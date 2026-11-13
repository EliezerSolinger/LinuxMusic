/*
 * TempoMap.cpp - the song's tempo along its timeline: markers with jumps and ramps
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

#include "TempoMap.h"

#include <algorithm>
#include <cmath>

#include <QDomElement>

#include "AudioEngine.h"
#include "AutomationClip.h"
#include "AutomationTrack.h"
#include "Engine.h"
#include "ProjectJournal.h"
#include "UndoCommand.h"
#include "Song.h"
#include "TimePos.h"

namespace lmms
{

auto TempoMap::Meter::ticksPerBar() const -> tick_t
{
	return DefaultTicksPerBar * numerator / std::max(1, denominator);
}


auto TempoMap::Meter::ticksPerBeat() const -> tick_t
{
	return DefaultTicksPerBar / std::max(1, denominator);
}


auto TempoMap::isValid(const Meter& meter) -> bool
{
	// A beat of a whole number of ticks: a note value of a power of two, down to 1/32
	const bool powerOfTwo = meter.denominator > 0 && (meter.denominator & (meter.denominator - 1)) == 0;
	return meter.numerator >= 1 && meter.numerator <= MaxNumerator && powerOfTwo && meter.denominator <= 32;
}




TempoMap::TempoMap()
{
	updateSegments();
}




void TempoMap::setStartMeter(const Meter& meter)
{
	if (meter == m_startMeter) { return; }
	{
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		m_startMeter = meter;
		updateSegments();
	}
	emit changed();
}




auto TempoMap::meterAt(tick_t tick) const -> Meter
{
	return barAt(tick).meter;
}




auto TempoMap::barAt(tick_t tick) const -> Bar
{
	tick = std::max(tick, tick_t{0});
	// The last stretch that starts at or before the tick (the first one starts at 0)
	const auto next = std::upper_bound(m_segments.begin(), m_segments.end(), tick,
		[](tick_t at, const Segment& segment) { return at < segment.start; });
	const auto& segment = *std::prev(next);
	const auto length = std::max(segment.meter.ticksPerBar(), tick_t{1});
	const auto inSegment = (tick - segment.start) / length;
	auto bar = Bar{segment.firstBar + inSegment, segment.start + inSegment * length, segment.meter, length};
	// The last bar before a change of the time signature ends where the change is
	if (next != m_segments.end() && bar.start + bar.length > next->start) { bar.length = next->start - bar.start; }
	return bar;
}




auto TempoMap::barStart(int index) const -> tick_t
{
	index = std::max(index, 0);
	const auto next = std::upper_bound(m_segments.begin(), m_segments.end(), index,
		[](int at, const Segment& segment) { return at < segment.firstBar; });
	const auto& segment = *std::prev(next);
	const auto start = static_cast<std::int64_t>(segment.start)
		+ static_cast<std::int64_t>(index - segment.firstBar) * std::max(segment.meter.ticksPerBar(), tick_t{1});
	return static_cast<tick_t>(std::min<std::int64_t>(start, TimePos::MaxTicks));
}




namespace
{

//! Float errors of positions in bars stay well below a tick in the smallest bar (1/32 of 192 ticks)
constexpr double BarsTolerance = 1e-7;

//! @p value / @p step rounded to the nearest whole number (a tie up), or down
auto steps(double value, double step, bool roundDown) -> double
{
	const auto ratio = value / step;
	return roundDown ? std::floor(ratio + BarsTolerance) : std::floor(ratio + 0.5 + BarsTolerance);
}

} // namespace


auto TempoMap::stepTicks(double bars) const -> tick_t
{
	// As TimePos::quantize() reckons it
	return static_cast<tick_t>(m_startMeter.ticksPerBar() * static_cast<float>(bars));
}




auto TempoMap::toBars(tick_t tick) const -> double
{
	const auto bar = barAt(tick);
	return bar.index + static_cast<double>(std::max(tick, tick_t{0}) - bar.start)
		/ std::max(bar.meter.ticksPerBar(), tick_t{1});
}




auto TempoMap::fromBars(double bars) const -> tick_t
{
	if (bars <= 0) { return 0; }
	const auto index = static_cast<int>(std::min(std::floor(bars + BarsTolerance), double{TimePos::MaxTicks}));
	const auto bar = barAt(barStart(index));
	const auto into = std::max(bars - index, 0.0) * bar.meter.ticksPerBar();
	return std::min(bar.start + static_cast<tick_t>(std::llround(into)), bar.start + bar.length);
}




auto TempoMap::quantize(tick_t tick, double bars, bool roundDown) const -> tick_t
{
	if (bars <= 0) { return tick; }
	// In one time signature, in ticks as the snap always went (a step of a fraction of a tick left out)
	if (!hasMeterChanges())
	{
		return std::max(tick_t{0}, TimePos::quantize(tick, stepTicks(bars), roundDown));
	}
	const auto before = fromBars(steps(toBars(tick), bars, true) * bars);
	if (roundDown || before == tick) { return before; }
	// The nearest in ticks (in a bar cut short, the next bar's start may be nearer than its last step)
	const auto after = fromBars((steps(toBars(tick), bars, true) + 1) * bars);
	return tick - before < after - tick ? before : after;
}




auto TempoMap::moveBySteps(tick_t from, tick_t to, double bars) const -> tick_t
{
	if (bars <= 0) { return to; }
	if (!hasMeterChanges()) { return std::max(tick_t{0}, from + TimePos::quantize(to - from, stepTicks(bars))); }
	const auto start = toBars(from);
	return fromBars(start + steps(toBars(to) - start, bars, false) * bars);
}




void TempoMap::updateSegments()
{
	m_segments.clear();
	m_segments.push_back({0, 0, m_startMeter});
	for (const auto& marker : m_markers)
	{
		if (!marker.meter || marker.tick <= 0) { continue; }
		const auto last = m_segments.back();
		const auto length = std::max(last.meter.ticksPerBar(), tick_t{1});
		// A bar the change cuts short counts as one
		const auto bars = (marker.tick - last.start + length - 1) / length;
		m_segments.push_back({marker.tick, last.firstBar + bars, *marker.meter});
	}
}




void TempoMap::tidy()
{
	sort();
	// One marker on a tick (the first one)
	const auto last = std::unique(m_markers.begin(), m_markers.end(),
		[](const Marker& a, const Marker& b) { return a.tick == b.tick; });
	m_markers.erase(last, m_markers.end());
	for (auto& marker : m_markers)
	{
		// At the start the time signature is the song's
		if (marker.meter && (marker.tick == 0 || !isValid(*marker.meter))) { marker.meter.reset(); }
	}
}




auto TempoMap::tempoAt(double tick) const -> double
{
	if (m_markers.empty()) { return 0; }

	// The last marker at or before the tick (the first one before all of them)
	const auto next = std::upper_bound(m_markers.begin(), m_markers.end(), tick,
		[](double at, const Marker& marker) { return at < static_cast<double>(marker.tick); });
	if (next == m_markers.begin()) { return m_markers.front().bpm; }
	const auto& marker = *std::prev(next);
	if (!marker.ramp || next == m_markers.end()) { return marker.bpm; }

	// A ramp: linear in ticks, to the next marker's tempo
	const auto span = static_cast<double>(next->tick - marker.tick);
	const auto progress = std::clamp((tick - static_cast<double>(marker.tick)) / span, 0.0, 1.0);
	return marker.bpm + (next->bpm - marker.bpm) * progress;
}




auto TempoMap::framesBetween(tick_t from, tick_t to, double songBpm, sample_rate_t sampleRate) const -> double
{
	const auto framesPerTickAt = [&](double bpm) {
		// As Engine::framesPerTick() reckons it, for the whole BPM the engine plays at
		return sampleRate * 60.0 * 4 / DefaultTicksPerBar / std::clamp(std::round(bpm), 1.0, 1e6);
	};
	if (to <= from) { return 0; }
	if (m_markers.empty()) { return (to - from) * framesPerTickAt(songBpm); }
	auto frames = 0.0;
	for (auto tick = from; tick < to; ++tick) { frames += framesPerTickAt(tempoAt(tick)); }
	return frames;
}




void TempoMap::setMarkers(std::vector<Marker> markers)
{
	{
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		m_markers = std::move(markers);
		tidy();
		updateSegments();
	}
	emit changed();
}




void TempoMap::setMarker(const Marker& marker, double startBpm)
{
	{
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		auto added = marker;
		added.tick = std::clamp(added.tick, 0, TimePos::MaxTicks);
		added.bpm = std::clamp(added.bpm, static_cast<double>(MinTempo), static_cast<double>(MaxTempo));
		if (m_markers.empty() && added.tick > 0) { m_markers.push_back({0, startBpm, false}); }
		const auto same = std::find_if(m_markers.begin(), m_markers.end(),
			[&](const Marker& existing) { return existing.tick == added.tick; });
		if (same != m_markers.end()) { *same = added; }
		else { m_markers.push_back(added); }
		tidy();
		updateSegments();
	}
	emit changed();
}




void TempoMap::setMeter(tick_t tick, std::optional<Meter> meter, double startBpm)
{
	tick = std::clamp(tick, 0, TimePos::MaxTicks);
	if (tick == 0 || (meter && !isValid(*meter))) { return; }
	{
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		const auto at = std::find_if(m_markers.begin(), m_markers.end(),
			[tick](const Marker& marker) { return marker.tick == tick; });
		if (at != m_markers.end()) { at->meter = meter; }
		else if (meter)
		{
			if (m_markers.empty()) { m_markers.push_back({0, startBpm, false}); }
			// The tempo there, and a ramp on through it if it is in one (the same line in two)
			const auto next = std::upper_bound(m_markers.begin(), m_markers.end(), tick,
				[](tick_t at, const Marker& marker) { return at < marker.tick; });
			const bool inRamp = next != m_markers.end() && std::prev(next)->ramp;
			m_markers.push_back({tick, tempoAt(tick), inRamp, meter});
		}
		tidy();
		updateSegments();
	}
	emit changed();
}




void TempoMap::removeMarker(tick_t tick)
{
	{
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		std::erase_if(m_markers, [tick](const Marker& marker) { return marker.tick == tick; });

		// Only the one at the song's start left says nothing more than the song's tempo: none then
		if (m_markers.size() == 1 && m_markers.front().tick == 0) { m_markers.clear(); }
		updateSegments();
	}
	emit changed();
}




void TempoMap::moveMarker(tick_t from, tick_t to)
{
	if (from == 0 || from == to) { return; }
	{
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		const auto moved = std::find_if(m_markers.begin(), m_markers.end(),
			[from](const Marker& marker) { return marker.tick == from; });
		if (moved == m_markers.end()) { return; }
		auto marker = *moved;
		m_markers.erase(moved);
		marker.tick = std::clamp(to, 1, TimePos::MaxTicks);
		std::erase_if(m_markers, [&](const Marker& other) { return other.tick == marker.tick; });
		m_markers.push_back(marker);
		tidy();
		updateSegments();
	}
	emit changed();
}




void TempoMap::clear()
{
	if (m_markers.empty()) { return; }
	{
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		m_markers.clear();
		updateSegments();
	}
	emit changed();
}




void TempoMap::sort()
{
	std::sort(m_markers.begin(), m_markers.end(), [](const Marker& a, const Marker& b) { return a.tick < b.tick; });
}




auto TempoMap::takeOverAutomation(AutomatableModel* tempo, double startBpm) -> bool
{
	auto markers = std::vector<Marker>{};
	for (auto* clip : AutomationClip::clipsForModel(tempo))
	{
		// Only clips that automate the tempo alone are taken over; one that also automates other things keeps
		// doing so (and the tempo too, as before)
		if (clip->objects().size() != 1 || clip->getTimeMap().isEmpty()) { continue; }
		// The song-global automation's points are on the song's timeline; a clip's from its start
		const bool global = clip->getTrack() == Engine::getSong()->globalAutomationTrack();
		const auto offset = global ? 0 : clip->startPosition().getTicks();
		const bool ramp = clip->progressionType() != AutomationClip::ProgressionType::Discrete;
		const auto& points = clip->getTimeMap();
		for (auto point = points.begin(); point != points.end(); ++point)
		{
			// The value after the point (a point may jump: in is the value up to it, out from it on)
			markers.push_back({offset + point.key(), static_cast<double>(point.value().getOutValue()), ramp});
		}
		clip->clearObjects();
		clip->clear();
	}
	if (markers.empty()) { return false; }

	{
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		m_markers = std::move(markers);
		for (auto& marker : m_markers)
		{
			marker.tick = std::clamp(marker.tick, 0, TimePos::MaxTicks);
			marker.bpm = std::clamp(marker.bpm, static_cast<double>(MinTempo), static_cast<double>(MaxTempo));
		}
		sort();
		// Several clips may have had points on one tick: the last one counts
		const auto last = std::unique(m_markers.rbegin(), m_markers.rend(),
			[](const Marker& a, const Marker& b) { return a.tick == b.tick; });
		m_markers.erase(m_markers.begin(), last.base());
		if (m_markers.front().tick > 0) { m_markers.insert(m_markers.begin(), {0, startBpm, false}); }
		updateSegments();
	}
	emit changed();
	return true;
}




void TempoMap::saveSettings(QDomDocument& document, QDomElement& element)
{
	for (const auto& marker : m_markers)
	{
		auto markerElement = document.createElement("marker");
		markerElement.setAttribute("tick", marker.tick);
		markerElement.setAttribute("bpm", marker.bpm);
		markerElement.setAttribute("ramp", marker.ramp ? 1 : 0);
		if (marker.meter)
		{
			markerElement.setAttribute("numerator", marker.meter->numerator);
			markerElement.setAttribute("denominator", marker.meter->denominator);
		}
		element.appendChild(markerElement);
	}
}




void TempoMap::loadSettings(const QDomElement& element)
{
	auto markers = std::vector<Marker>{};
	for (auto markerElement = element.firstChildElement("marker"); !markerElement.isNull();
		markerElement = markerElement.nextSiblingElement("marker"))
	{
		auto bpm = markerElement.attribute("bpm").toDouble();
		if (!std::isfinite(bpm)) { bpm = DefaultTempo; }
		auto marker = Marker{std::clamp(markerElement.attribute("tick").toInt(), 0, TimePos::MaxTicks),
			std::clamp(bpm, static_cast<double>(MinTempo), static_cast<double>(MaxTempo)),
			markerElement.attribute("ramp").toInt() != 0};
		// (a time signature that is not valid is left out, see tidy())
		if (markerElement.hasAttribute("numerator"))
		{
			marker.meter = Meter{markerElement.attribute("numerator").toInt(), markerElement.attribute("denominator").toInt()};
		}
		markers.push_back(marker);
	}
	{
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		m_markers = std::move(markers);
		tidy();
		updateSegments();
	}
	emit changed();
}

namespace
{

//! A change of the tempo map: its markers before and after
class TempoMapCommand : public UndoCommand
{
public:
	TempoMapCommand(std::vector<TempoMap::Marker> before, std::vector<TempoMap::Marker> after)
		: m_before{std::move(before)}
		, m_after{std::move(after)}
	{
	}
	void apply() override { Engine::getSong()->tempoMap().setMarkers(m_after); }
	void revert() override { Engine::getSong()->tempoMap().setMarkers(m_before); }
	auto text() const -> QString override { return QObject::tr("Change the tempo"); }

private:
	std::vector<TempoMap::Marker> m_before;
	std::vector<TempoMap::Marker> m_after;
};

} // namespace




void TempoMap::recordChange(std::vector<Marker> before)
{
	auto after = m_markers;
	if (before == after) { return; }
	// (already done: applying it again changes nothing)
	Engine::projectJournal()->execute(std::make_unique<TempoMapCommand>(std::move(before), std::move(after)));
}

} // namespace lmms
