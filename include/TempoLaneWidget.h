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

namespace lmms
{
class TempoMap;
}

namespace lmms::gui
{

class TimeLineWidget;

/**
 * The tempo map's markers along the song, under the ruler (docs/plano-gravacao.md, "Mapa de tempo"):
 * - a double-click adds a marker there (with the tempo there), or edits the tempo of the marker under the mouse;
 * - dragging a marker moves it (on sixteenths of a bar);
 * - the context menu edits a marker's tempo, makes it jump or ramp to the next one, or removes it.
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

	TimeLineWidget* m_ruler;
	TempoMap* m_map;
	int m_headWidth;
	std::optional<tick_t> m_dragged; //!< the marker being dragged
};

} // namespace lmms::gui

#endif // LMMS_GUI_TEMPO_LANE_WIDGET_H
