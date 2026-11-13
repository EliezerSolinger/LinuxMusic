/*
 * SongMarkers.h - named markers along the song (as REAPER's): "Chorus", "Verse 2"...
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

#ifndef LMMS_SONG_MARKERS_H
#define LMMS_SONG_MARKERS_H

#include <optional>
#include <vector>

#include <QColor>
#include <QObject>
#include <QString>

#include "JournallingObject.h"
#include "LmmsTypes.h"
#include "lmms_export.h"

namespace lmms
{

/**
 * Markers along the song with a text, to note its parts (docs/plano-anotacoes-13-11.md, 9.7): "Chorus", "Verse 2"...
 * Each has a tick, a name and a colour. They are shown in the lane under the ruler with the tempo map's markers; a
 * marker "marks the tempo" when the tempo map has a change of tempo at its tick (TempoMap): the two lists stay
 * apart, so that the tempo plays exactly as before.
 *
 * Each edit is an undo step. Main thread.
 */
class LMMS_EXPORT SongMarkers : public QObject, public JournallingObject
{
	Q_OBJECT
public:
	struct Marker
	{
		tick_t tick = 0;
		QString name;
		QColor color;
		auto operator==(const Marker&) const -> bool = default;
	};

	auto markers() const -> const std::vector<Marker>& { return m_markers; }
	//! The marker at @p tick, if any
	auto at(tick_t tick) const -> std::optional<Marker>;
	//! The first marker after @p tick, and the last before it (for going from marker to marker)
	auto next(tick_t tick) const -> std::optional<Marker>;
	auto previous(tick_t tick) const -> std::optional<Marker>;

	//! Adds a marker at @p tick (replacing one there)
	void add(tick_t tick, const QString& name, const QColor& color = {});
	void rename(tick_t tick, const QString& name);
	void setColor(tick_t tick, const QColor& color);
	//! Moves the marker at @p from to @p to (replacing one there)
	void move(tick_t from, tick_t to);
	void remove(tick_t tick);
	//! Without markers, not an undo step (a project closing)
	void clear();

	void saveSettings(QDomDocument& document, QDomElement& element) override;
	void loadSettings(const QDomElement& element) override;
	auto nodeName() const -> QString override { return "markers"; }

signals:
	void changed();

private:
	void sort();

	std::vector<Marker> m_markers;
};

} // namespace lmms

#endif // LMMS_SONG_MARKERS_H
