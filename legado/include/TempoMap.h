/*
 * TempoMap.h - the song's tempo along its timeline: markers with jumps and ramps
 *
 * This file is part of LinuxMusic, a fork of LMMS - https://lmms.io
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public
 * License as published by the Free Software Foundation; either
 * version 2 of the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * General Public License for more details.
 *
 * You should have received a copy of the GNU General Public
 * License along with this program (see COPYING); if not, write to the
 * Free Software Foundation, Inc., 51 Franklin Street, Fifth Floor,
 * Boston, MA 02110-1301 USA.
 *
 */

#ifndef LMMS_TEMPO_MAP_H
#define LMMS_TEMPO_MAP_H

#include <optional>
#include <vector>

#include <QString>

#include <QObject>

#include "LmmsTypes.h"
#include "SerializingObject.h"
#include "lmms_export.h"

namespace lmms
{

class AutomatableModel;

/**
 * The song's tempo along its timeline (docs/plano-gravacao.md, "Mapa de tempo"): markers, each with a tempo from its
 * tick on. A marker either holds its tempo up to the next one (the tempo jumps there) or ramps to the next one's,
 * linearly in ticks. Without markers the song keeps its one tempo, as before.
 *
 * While the song plays, the tempo the map gives at each tick becomes the song's (Song::processNextBuffer()), the
 * same way an automation of the tempo did; the engine's tempo is whole BPM, so a ramp goes by steps of one.
 *
 * A marker may also change the time signature: a bar starts there, in the new one (a bar the change cuts short ends
 * there). The time signature at the start is the song's (Song::getTimeSigModel()). The tick stays 1/192 of a whole
 * note in every time signature: only where the bars are changes (barAt(), barStart()).
 *
 * Changes are made on the main thread under the audio engine's change lock, which the audio thread holds while it
 * reads the markers.
 */
class LMMS_EXPORT TempoMap : public QObject, public SerializingObject
{
	Q_OBJECT
public:
	//! A time signature: @p numerator beats of a 1/@p denominator note to the bar
	struct Meter
	{
		int numerator = 4;
		int denominator = 4;
		auto ticksPerBar() const -> tick_t;
		auto ticksPerBeat() const -> tick_t;
		auto operator==(const Meter&) const -> bool = default;
	};

	struct Marker
	{
		tick_t tick = 0;
		double bpm = 120;
		bool ramp = false; //!< the tempo goes from this marker's to the next one's
		//! The time signature from here on (a bar starts here); none: the one before goes on. Not at the song's
		//! start, where it is the song's.
		std::optional<Meter> meter = std::nullopt;
		auto operator==(const Marker&) const -> bool = default;
	};

	//! The bar a tick is in
	struct Bar
	{
		int index = 0; //!< from 0
		tick_t start = 0;
		Meter meter;
		//! Its length (the last one before a change of the time signature may be cut short)
		tick_t length = 0;
	};

	//! The time signatures a numerator and a denominator may have
	static constexpr int MaxNumerator = 32;
	static auto isValid(const Meter& meter) -> bool;

	TempoMap();

	auto markers() const -> const std::vector<Marker>& { return m_markers; }
	auto isEmpty() const -> bool { return m_markers.empty(); }

	//! The time signature at the song's start (the song's) [main thread]
	void setStartMeter(const Meter& meter);
	auto startMeter() const -> const Meter& { return m_startMeter; }
	//! Whether a marker changes the time signature
	auto hasMeterChanges() const -> bool { return m_segments.size() > 1; }
	auto meterAt(tick_t tick) const -> Meter;
	//! The bar @p tick is in
	auto barAt(tick_t tick) const -> Bar;
	//! Where bar @p index (from 0) starts
	auto barStart(int index) const -> tick_t;
	/**
	 * A position in bars: the bar's index and how far into it, in its time signature (a bar cut short by a change
	 * of the time signature ends before its whole). The snap of the Arrangement goes by it, so that a step of a
	 * fraction of a bar is that fraction of each bar.
	 */
	auto toBars(tick_t tick) const -> double;
	//! The tick at a position in bars (one past the end of a bar cut short: where the next one starts)
	auto fromBars(double bars) const -> tick_t;
	//! @p tick on the grid of @p bars (whole bars, or a fraction of each bar): the nearest point (a tie to the
	//! right), or the one at or before it
	auto quantize(tick_t tick, double bars, bool roundDown = false) const -> tick_t;
	//! @p from moved toward @p to by the whole steps of @p bars nearest to the distance
	auto moveBySteps(tick_t from, tick_t to, double bars) const -> tick_t;

	//! Calls @p visit(bar) for each bar from the one @p from is in until @p to
	template<typename Visit>
	void forEachBar(tick_t from, tick_t to, Visit&& visit) const
	{
		for (auto bar = barAt(std::max(from, tick_t{0})); bar.start < to; bar = barAt(bar.start + bar.length))
		{
			visit(bar);
		}
	}

	//! @returns the tempo at @p tick (the first marker's before it)
	auto tempoAt(double tick) const -> double;

	/**
	 * @returns the frames the song plays from tick @p from to tick @p to: tick by tick, at the whole BPM the engine
	 *          plays each tick at (the map's, or @p songBpm without markers)
	 */
	auto framesBetween(tick_t from, tick_t to, double songBpm, sample_rate_t sampleRate) const -> double;

	//! Replaces all the markers (e.g. to go back to what they were)
	void setMarkers(std::vector<Marker> markers);

	//! Adds a marker, or replaces the one at its tick. The first marker added also gets one at the song's start,
	//! with @p startBpm, so that the map says what the tempo is everywhere.
	void setMarker(const Marker& marker, double startBpm);
	/**
	 * Changes the time signature from @p tick on (none: the one before goes on there). A marker made for it keeps
	 * the tempo as it was (in a ramp, the ramp's tempo there, ramping on). At the song's start it is the song's.
	 */
	void setMeter(tick_t tick, std::optional<Meter> meter, double startBpm);
	void removeMarker(tick_t tick);
	//! Moves the marker at @p from to @p to (the one at the song's start stays there)
	void moveMarker(tick_t from, tick_t to);
	void clear();

	//! Makes the change since the markers were @p before an undo step (a command: the markers before and after)
	void recordChange(std::vector<Marker> before);

	/**
	 * Turns the automations of @p tempo (an older project's) into markers: each point a marker, a ramp to the next
	 * where the automation was not discrete. The automations no longer change the tempo afterwards.
	 * @returns whether there was an automation to turn
	 */
	auto takeOverAutomation(AutomatableModel* tempo, double startBpm) -> bool;

	void saveSettings(QDomDocument& document, QDomElement& element) override;
	void loadSettings(const QDomElement& element) override;
	auto nodeName() const -> QString override { return "tempomap"; }

signals:
	void changed();

private:
	void sort();
	//! The markers in order, one on each tick, without a time signature at the start or a wrong one [change lock]
	void tidy();
	//! The bars, from the markers' time signatures [change lock]
	void updateSegments();
	//! The ticks of a snap step of @p bars in the time signature at the start
	auto stepTicks(double bars) const -> tick_t;

	//! A stretch of bars in one time signature
	struct Segment
	{
		tick_t start;
		int firstBar;
		Meter meter;
	};

	std::vector<Marker> m_markers;
	Meter m_startMeter;
	std::vector<Segment> m_segments;
};

} // namespace lmms

#endif // LMMS_TEMPO_MAP_H
