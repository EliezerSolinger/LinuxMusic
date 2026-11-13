/*
 * TempoLaneWidget.h - the tempo map's lane under the Song Editor's ruler
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

#ifndef LMMS_GUI_TEMPO_LANE_WIDGET_H
#define LMMS_GUI_TEMPO_LANE_WIDGET_H

#include <optional>

#include <QWidget>

#include "LmmsTypes.h"

class QMenu;

#include "TempoMap.h"

namespace lmms::gui
{

class TimeLineWidget;

/**
 * The tempo map's markers along the song, under the ruler (docs/plano-gravacao.md, "Mapa de tempo"):
 * - a double-click adds a marker there (with the tempo there), or edits the tempo of the marker under the mouse;
 * - dragging a marker moves it (on sixteenths of a bar);
 * - the context menu edits a marker's tempo, makes it jump or ramp to the next one, or removes it.
 * Above the tempo, the named markers (SongMarkers) and the regions: spans with a name, as REAPER's. A region is
 * drawn by dragging on its row, moved by dragging it, resized by its edges, and a click on it makes it the cycle.
 * It maps ticks to pixels as the ruler does.
 */
class TempoLaneWidget : public QWidget
{
	Q_OBJECT
public:
	TempoLaneWidget(TimeLineWidget* ruler, TempoMap* map, int headWidth, QWidget* parent);

	void setHeadWidth(int width);

protected:
	void paintEvent(QPaintEvent* event) override;
	void mousePressEvent(QMouseEvent* event) override;
	void mouseMoveEvent(QMouseEvent* event) override;
	void mouseReleaseEvent(QMouseEvent* event) override;
	void mouseDoubleClickEvent(QMouseEvent* event) override;
	void contextMenuEvent(QContextMenuEvent* event) override;

private:
	auto xOf(tick_t tick) const -> int;
	auto tickAt(int x) const -> tick_t; //!< on sixteenths of a bar
	auto markerAt(int x) const -> std::optional<tick_t>;
	void editTempo(tick_t tick);
	void addMarker(tick_t tick);
	/**
	 * Asks for a time signature from @p tick on (at the song's start, the song's): @p current to begin with; for a
	 * marker that has one, the change can be taken away
	 */
	void editMeter(tick_t tick);
	//! A time signature asked for: none if cancelled; a null one to take the change away
	auto askMeter(const TempoMap::Meter& current, bool removable) -> std::optional<std::optional<TempoMap::Meter>>;
	//! Makes the change of the tempo map since @p before an undo step
	void recordTempoEdit(std::vector<TempoMap::Marker> before);
	std::vector<TempoMap::Marker> m_tempoBefore; //!< the markers when a drag began

	// The named markers (SongMarkers), in the row above the tempo
	auto namedMarkerAt(int x) const -> std::optional<tick_t>;
	static auto markerColor(const QColor& color) -> QColor;
	void addNamed(tick_t tick);
	void renameNamed(tick_t tick);
	void namedMenu(QMenu& menu, int x);

	// The regions (SongMarkers' too), in the row between
	enum class RegionDrag { None, Create, Move, Begin, End };
	struct RegionHit { tick_t begin; RegionDrag part; };
	auto regionAt(int x) const -> std::optional<RegionHit>;
	static auto regionColor(const QColor& color) -> QColor;
	void addRegion(tick_t begin, tick_t end);
	void addRegionFromCycle();
	void renameRegion(tick_t begin);
	void regionMenu(QMenu& menu, int x);
	static void cycleOver(tick_t begin, tick_t end);
	void paintRegions(QPainter& painter);

	RegionDrag m_regionDrag = RegionDrag::None;
	tick_t m_regionOrigin = 0; //!< the dragged region's begin, as it was
	tick_t m_pressTick = 0;
	bool m_regionMoved = false;
	std::pair<tick_t, tick_t> m_regionPreview{}; //!< the span while dragging (applied on release, one undo step)

	TimeLineWidget* m_ruler;
	TempoMap* m_map;
	int m_headWidth;
	std::optional<tick_t> m_dragged; //!< the tempo marker being dragged
	std::optional<tick_t> m_draggedNamed; //!< the named marker being dragged
};

} // namespace lmms::gui

#endif // LMMS_GUI_TEMPO_LANE_WIDGET_H
