/*
 * SongMarkers.cpp - named markers along the song (as REAPER's): "Chorus", "Verse 2"...
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

#include "SongMarkers.h"

#include <algorithm>

#include <QDomElement>

#include "TimePos.h"

namespace lmms
{

auto SongMarkers::at(tick_t tick) const -> std::optional<Marker>
{
	const auto found = std::find_if(m_markers.begin(), m_markers.end(), [tick](const Marker& m) { return m.tick == tick; });
	if (found == m_markers.end()) { return std::nullopt; }
	return *found;
}


auto SongMarkers::next(tick_t tick) const -> std::optional<Marker>
{
	const auto found = std::find_if(m_markers.begin(), m_markers.end(), [tick](const Marker& m) { return m.tick > tick; });
	if (found == m_markers.end()) { return std::nullopt; }
	return *found;
}


auto SongMarkers::previous(tick_t tick) const -> std::optional<Marker>
{
	const auto found = std::find_if(m_markers.rbegin(), m_markers.rend(), [tick](const Marker& m) { return m.tick < tick; });
	if (found == m_markers.rend()) { return std::nullopt; }
	return *found;
}


void SongMarkers::add(tick_t tick, const QString& name, const QColor& color)
{
	addJournalCheckPoint();
	tick = std::clamp(tick, 0, TimePos::MaxTicks);
	std::erase_if(m_markers, [tick](const Marker& m) { return m.tick == tick; });
	m_markers.push_back({tick, name, color});
	sort();
	emit changed();
}


void SongMarkers::rename(tick_t tick, const QString& name)
{
	for (auto& marker : m_markers)
	{
		if (marker.tick != tick || marker.name == name) { continue; }
		addJournalCheckPoint();
		marker.name = name;
		emit changed();
	}
}


void SongMarkers::setColor(tick_t tick, const QColor& color)
{
	for (auto& marker : m_markers)
	{
		if (marker.tick != tick || marker.color == color) { continue; }
		addJournalCheckPoint();
		marker.color = color;
		emit changed();
	}
}


void SongMarkers::move(tick_t from, tick_t to)
{
	to = std::clamp(to, 0, TimePos::MaxTicks);
	const auto moved = at(from);
	if (!moved || from == to) { return; }
	addJournalCheckPoint();
	std::erase_if(m_markers, [&](const Marker& m) { return m.tick == from || m.tick == to; });
	m_markers.push_back({to, moved->name, moved->color});
	sort();
	emit changed();
}


void SongMarkers::remove(tick_t tick)
{
	if (!at(tick)) { return; }
	addJournalCheckPoint();
	std::erase_if(m_markers, [tick](const Marker& m) { return m.tick == tick; });
	emit changed();
}


void SongMarkers::clear()
{
	if (m_markers.empty() && m_regions.empty()) { return; }
	m_markers.clear();
	m_regions.clear();
	emit changed();
}


auto SongMarkers::regionAt(tick_t tick) const -> std::optional<Region>
{
	auto best = std::optional<Region>{};
	for (const auto& region : m_regions)
	{
		if (tick < region.begin || tick >= region.end) { continue; }
		if (!best || region.end - region.begin < best->end - best->begin) { best = region; }
	}
	return best;
}


void SongMarkers::addRegion(tick_t begin, tick_t end, const QString& name, const QColor& color)
{
	if (end < begin) { std::swap(begin, end); }
	begin = std::clamp(begin, 0, TimePos::MaxTicks - 1);
	end = std::clamp(std::max(end, begin + 1), 1, TimePos::MaxTicks);
	addJournalCheckPoint();
	std::erase_if(m_regions, [begin](const Region& r) { return r.begin == begin; });
	m_regions.push_back({begin, end, name, color});
	sort();
	emit changed();
}


void SongMarkers::renameRegion(tick_t begin, const QString& name)
{
	for (auto& region : m_regions)
	{
		if (region.begin != begin || region.name == name) { continue; }
		addJournalCheckPoint();
		region.name = name;
		emit changed();
	}
}


void SongMarkers::setRegionColor(tick_t begin, const QColor& color)
{
	for (auto& region : m_regions)
	{
		if (region.begin != begin || region.color == color) { continue; }
		addJournalCheckPoint();
		region.color = color;
		emit changed();
	}
}


void SongMarkers::setRegionSpan(tick_t begin, tick_t newBegin, tick_t newEnd)
{
	const auto found = std::find_if(m_regions.begin(), m_regions.end(), [begin](const Region& r) { return r.begin == begin; });
	if (found == m_regions.end()) { return; }
	if (newEnd < newBegin) { std::swap(newBegin, newEnd); }
	newBegin = std::clamp(newBegin, 0, TimePos::MaxTicks - 1);
	newEnd = std::clamp(std::max(newEnd, newBegin + 1), 1, TimePos::MaxTicks);
	if (found->begin == newBegin && found->end == newEnd) { return; }
	addJournalCheckPoint();
	auto region = *found;
	region.begin = newBegin;
	region.end = newEnd;
	// (a region that began where this one now begins gives way)
	std::erase_if(m_regions, [&](const Region& r) { return r.begin == begin || r.begin == newBegin; });
	m_regions.push_back(region);
	sort();
	emit changed();
}


void SongMarkers::removeRegion(tick_t begin)
{
	if (std::none_of(m_regions.begin(), m_regions.end(), [begin](const Region& r) { return r.begin == begin; })) { return; }
	addJournalCheckPoint();
	std::erase_if(m_regions, [begin](const Region& r) { return r.begin == begin; });
	emit changed();
}


void SongMarkers::saveSettings(QDomDocument& document, QDomElement& element)
{
	for (const auto& marker : m_markers)
	{
		auto child = document.createElement("marker");
		child.setAttribute("tick", marker.tick);
		child.setAttribute("name", marker.name);
		if (marker.color.isValid()) { child.setAttribute("color", marker.color.name()); }
		element.appendChild(child);
	}
	for (const auto& region : m_regions)
	{
		auto child = document.createElement("region");
		child.setAttribute("begin", region.begin);
		child.setAttribute("end", region.end);
		child.setAttribute("name", region.name);
		if (region.color.isValid()) { child.setAttribute("color", region.color.name()); }
		element.appendChild(child);
	}
}


void SongMarkers::loadSettings(const QDomElement& element)
{
	m_markers.clear();
	m_regions.clear();
	for (auto child = element.firstChildElement("region"); !child.isNull(); child = child.nextSiblingElement("region"))
	{
		auto okBegin = false, okEnd = false;
		auto begin = child.attribute("begin").toInt(&okBegin);
		auto end = child.attribute("end").toInt(&okEnd);
		if (!okBegin || !okEnd) { continue; } // (a damaged project: left out)
		if (end < begin) { std::swap(begin, end); }
		begin = std::clamp(begin, 0, TimePos::MaxTicks - 1);
		const auto color = QColor{child.attribute("color")};
		std::erase_if(m_regions, [begin](const Region& r) { return r.begin == begin; });
		m_regions.push_back({begin, std::clamp(std::max(end, begin + 1), 1, TimePos::MaxTicks), child.attribute("name"),
			color.isValid() ? color : QColor{}});
	}
	for (auto child = element.firstChildElement("marker"); !child.isNull(); child = child.nextSiblingElement("marker"))
	{
		auto ok = false;
		const auto tick = child.attribute("tick").toInt(&ok);
		if (!ok) { continue; } // (a damaged project: the marker is left out)
		const auto color = QColor{child.attribute("color")};
		std::erase_if(m_markers, [tick](const Marker& m) { return m.tick == tick; });
		m_markers.push_back({std::clamp(tick, 0, TimePos::MaxTicks), child.attribute("name"),
			color.isValid() ? color : QColor{}});
	}
	sort();
	emit changed();
}


void SongMarkers::sort()
{
	std::sort(m_markers.begin(), m_markers.end(), [](const Marker& a, const Marker& b) { return a.tick < b.tick; });
	std::sort(m_regions.begin(), m_regions.end(), [](const Region& a, const Region& b) { return a.begin < b.begin; });
}

} // namespace lmms
