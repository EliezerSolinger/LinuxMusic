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

#include <vector>

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
 * Changes are made on the main thread under the audio engine's change lock, which the audio thread holds while it
 * reads the markers.
 */
class LMMS_EXPORT TempoMap : public QObject, public SerializingObject
{
	Q_OBJECT
public:
	struct Marker
	{
		tick_t tick = 0;
		double bpm = 120;
		bool ramp = false; //!< the tempo goes from this marker's to the next one's
		auto operator==(const Marker&) const -> bool = default;
	};

	auto markers() const -> const std::vector<Marker>& { return m_markers; }
	auto isEmpty() const -> bool { return m_markers.empty(); }

	//! @returns the tempo at @p tick (the first marker's before it)
	auto tempoAt(double tick) const -> double;

	//! Adds a marker, or replaces the one at its tick. The first marker added also gets one at the song's start,
	//! with @p startBpm, so that the map says what the tempo is everywhere.
	void setMarker(const Marker& marker, double startBpm);
	void removeMarker(tick_t tick);
	//! Moves the marker at @p from to @p to (the one at the song's start stays there)
	void moveMarker(tick_t from, tick_t to);
	void clear();

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

	std::vector<Marker> m_markers;
};

} // namespace lmms

#endif // LMMS_TEMPO_MAP_H
