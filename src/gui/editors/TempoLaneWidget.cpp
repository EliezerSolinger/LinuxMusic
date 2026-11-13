/*
 * TempoLaneWidget.cpp - the tempo map's lane under the Song Editor's ruler
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

#include "TempoLaneWidget.h"

#include <algorithm>
#include <cmath>

#include <QContextMenuEvent>
#include <QInputDialog>
#include <QLineEdit>
#include <QColorDialog>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>

#include "Engine.h"
#include "Song.h"
#include "TempoMap.h"
#include "TimeLineWidget.h"
#include "TimePos.h"

namespace lmms::gui
{

namespace
{
constexpr int MarkerRow = 16; //!< the named markers, above
constexpr int TempoRow = 18; //!< the tempo, below
constexpr int LaneHeight = MarkerRow + TempoRow;
constexpr int GrabDistance = 5; //!< how close to a marker the mouse takes it, in pixels
}




TempoLaneWidget::TempoLaneWidget(TimeLineWidget* ruler, TempoMap* map, int headWidth, QWidget* parent)
	: QWidget{parent}
	, m_ruler{ruler}
	, m_map{map}
	, m_headWidth{headWidth}
{
	setFixedHeight(LaneHeight);
	setMouseTracking(true);
	setToolTip(tr("Markers above (M adds one at the playhead; right-click to add, rename, color or remove), tempo "
		"below (double-click to add a change of tempo); drag either to move it"));
	connect(map, &TempoMap::changed, this, qOverload<>(&QWidget::update));
	connect(&Engine::getSong()->markers(), &SongMarkers::changed, this, qOverload<>(&QWidget::update));
}




void TempoLaneWidget::setHeadWidth(int width)
{
	m_headWidth = width;
	update();
}




auto TempoLaneWidget::xOf(tick_t tick) const -> int
{
	return m_ruler->markerX(TimePos{tick});
}




auto TempoLaneWidget::tickAt(int x) const -> tick_t
{
	// The ruler's own mapping, inverted
	const auto bar = TimePos::ticksPerBar();
	const auto start = m_ruler->markerX(TimePos{0});
	const auto pixelsPerBar = std::max(1, m_ruler->markerX(TimePos{bar}) - start);
	const auto tick = static_cast<double>(x - start) * bar / pixelsPerBar;
	const auto step = std::max(1, bar / 16);
	return std::clamp(static_cast<tick_t>(std::lround(tick / step)) * step, 0, TimePos::MaxTicks);
}




auto TempoLaneWidget::markerAt(int x) const -> std::optional<tick_t>
{
	for (const auto& marker : m_map->markers())
	{
		if (std::abs(xOf(marker.tick) - x) <= GrabDistance) { return marker.tick; }
	}
	return std::nullopt;
}




auto TempoLaneWidget::namedMarkerAt(int x) const -> std::optional<tick_t>
{
	// The flag reaches right of its tick
	for (const auto& marker : Engine::getSong()->markers().markers())
	{
		const auto at = xOf(marker.tick);
		if (x >= at - GrabDistance && x <= at + 14) { return marker.tick; }
	}
	return std::nullopt;
}




auto TempoLaneWidget::markerColor(const QColor& color) -> QColor
{
	return color.isValid() ? color : QColor{232, 190, 64};
}




void TempoLaneWidget::paintEvent(QPaintEvent*)
{
	auto painter = QPainter{this};
	painter.fillRect(rect(), QColor{24, 28, 33});
	painter.fillRect(QRect{0, 0, width(), MarkerRow}, QColor{20, 23, 27});
	painter.setPen(QColor{12, 14, 17});
	painter.drawLine(0, MarkerRow - 1, width(), MarkerRow - 1);
	painter.drawLine(0, height() - 1, width(), height() - 1);

	auto font = painter.font();
	font.setPixelSize(10);
	painter.setFont(font);

	painter.setClipRect(m_headWidth, 0, width() - m_headWidth, height());

	// The named markers: a flag in their colour and their name
	const auto& named = Engine::getSong()->markers().markers();
	for (std::size_t index = 0; index < named.size(); ++index)
	{
		const auto& marker = named[index];
		const auto x = xOf(marker.tick);
		const auto color = markerColor(marker.color);
		painter.setPen(color);
		painter.drawLine(x, 0, x, height() - 2);
		const auto room = index + 1 < named.size() ? xOf(named[index + 1].tick) - x - 4 : 400;
		const auto text = painter.fontMetrics().elidedText(marker.name.isEmpty() ? QString::number(index + 1)
			: marker.name, Qt::ElideRight, std::max(room - 6, 10));
		const auto flag = QRect{x, 1, painter.fontMetrics().horizontalAdvance(text) + 8, MarkerRow - 3};
		painter.fillRect(flag, color);
		painter.setPen(color.lightnessF() > 0.55 ? QColor{20, 22, 26} : QColor{245, 247, 250});
		painter.drawText(flag.adjusted(4, 0, 0, 0), Qt::AlignVCenter, text);
	}

	// The tempo, below
	const auto& markers = m_map->markers();
	const auto top = MarkerRow;
	const auto rowHeight = TempoRow;
	if (markers.empty())
	{
		painter.setPen(QColor{110, 118, 128});
		painter.drawText(QRect{m_headWidth + 6, top, width(), rowHeight}, Qt::AlignVCenter,
			tr("%1 BPM - double-click to add a change of tempo").arg(Engine::getSong()->getTempo()));
	}
	for (std::size_t index = 0; index < markers.size(); ++index)
	{
		const auto& marker = markers[index];
		const auto x = xOf(marker.tick);

		// A ramp: a line up or down to the next marker
		if (marker.ramp && index + 1 < markers.size())
		{
			const auto& next = markers[index + 1];
			const auto low = std::min(marker.bpm, next.bpm);
			const auto high = std::max(marker.bpm, next.bpm);
			const auto yOf = [&](double bpm) {
				return top + (high > low ? static_cast<int>(rowHeight - 3 - (bpm - low) / (high - low) * (rowHeight - 6))
					: rowHeight / 2);
			};
			painter.setPen(QColor{71, 253, 133, 140});
			painter.drawLine(x, yOf(marker.bpm), xOf(next.tick), yOf(next.bpm));
		}

		painter.setPen(QColor{71, 253, 133});
		painter.drawLine(x, top, x, height() - 2);
		const auto bpm = std::abs(marker.bpm - std::round(marker.bpm)) < 0.005
			? QString::number(static_cast<int>(std::round(marker.bpm)))
			: QString::number(marker.bpm, 'f', 2);
		painter.setPen(QColor{209, 216, 228});
		painter.drawText(QRect{x + 3, top, 80, rowHeight}, Qt::AlignVCenter, marker.ramp ? bpm + QString{" ↗"} : bpm);
	}

	painter.setClipping(false);
	painter.fillRect(0, 0, m_headWidth, height(), QColor{30, 35, 41});
	painter.setPen(QColor{150, 160, 172});
	painter.drawText(QRect{6, 0, m_headWidth - 6, MarkerRow}, Qt::AlignVCenter, tr("MARKERS"));
	painter.drawText(QRect{6, MarkerRow, m_headWidth - 6, TempoRow}, Qt::AlignVCenter, tr("TEMPO"));
}




void TempoLaneWidget::mousePressEvent(QMouseEvent* event)
{
	const auto x = static_cast<int>(event->position().x());
	const auto upper = event->position().y() < MarkerRow;
	if (event->button() == Qt::LeftButton && x > m_headWidth)
	{
		if (upper) { m_draggedNamed = namedMarkerAt(x); }
		// The tempo marker at the song's start stays there
		else if (const auto marker = markerAt(x); marker && *marker > 0) { m_dragged = marker; }
	}
	QWidget::mousePressEvent(event);
}




void TempoLaneWidget::mouseMoveEvent(QMouseEvent* event)
{
	const auto x = static_cast<int>(event->position().x());
	if (m_draggedNamed)
	{
		const auto to = tickAt(x);
		if (to != *m_draggedNamed && !Engine::getSong()->markers().at(to))
		{
			Engine::getSong()->markers().move(*m_draggedNamed, to);
			m_draggedNamed = to;
		}
	}
	if (m_dragged)
	{
		const auto to = std::max(1, tickAt(x));
		if (to != *m_dragged)
		{
			m_map->moveMarker(*m_dragged, to);
			m_dragged = to;
		}
	}
	const auto upper = event->position().y() < MarkerRow;
	const auto over = x > m_headWidth && (upper ? namedMarkerAt(x).has_value() : markerAt(x).has_value());
	setCursor(m_dragged || m_draggedNamed || over ? Qt::SizeHorCursor : Qt::ArrowCursor);
	QWidget::mouseMoveEvent(event);
}




void TempoLaneWidget::mouseReleaseEvent(QMouseEvent* event)
{
	if (m_dragged || m_draggedNamed) { Engine::getSong()->setModified(); }
	m_dragged.reset();
	m_draggedNamed.reset();
	QWidget::mouseReleaseEvent(event);
}




void TempoLaneWidget::mouseDoubleClickEvent(QMouseEvent* event)
{
	const auto x = static_cast<int>(event->position().x());
	if (x <= m_headWidth) { return; }
	if (event->position().y() < MarkerRow)
	{
		// Above: a named marker is renamed, or one is added
		if (const auto marker = namedMarkerAt(x)) { renameNamed(*marker); }
		else { addNamed(tickAt(x)); }
		return;
	}
	if (const auto marker = markerAt(x)) { editTempo(*marker); }
	else { addMarker(tickAt(x)); }
}




void TempoLaneWidget::contextMenuEvent(QContextMenuEvent* event)
{
	const auto x = event->pos().x();
	if (x <= m_headWidth) { return; }
	auto menu = QMenu{this};
	if (event->pos().y() < MarkerRow)
	{
		namedMenu(menu, x);
		menu.exec(event->globalPos());
		return;
	}
	if (const auto marker = markerAt(x))
	{
		const auto tick = *marker;
		const auto& markers = m_map->markers();
		const auto found = std::find_if(markers.begin(), markers.end(),
			[tick](const TempoMap::Marker& m) { return m.tick == tick; });
		menu.addAction(tr("Edit tempo..."), this, [this, tick] { editTempo(tick); });
		auto* ramp = menu.addAction(tr("Ramp to the next marker"));
		ramp->setCheckable(true);
		ramp->setChecked(found != markers.end() && found->ramp);
		connect(ramp, &QAction::toggled, this, [this, tick](bool on) {
			for (auto marker : m_map->markers())
			{
				if (marker.tick != tick) { continue; }
				marker.ramp = on;
				m_map->setMarker(marker, Engine::getSong()->getTempo());
				Engine::getSong()->setModified();
				break;
			}
		});
		if (tick > 0)
		{
			menu.addSeparator();
			menu.addAction(tr("Remove"), this, [this, tick] {
				m_map->removeMarker(tick);
				Engine::getSong()->setModified();
			});
		}
	}
	else
	{
		const auto tick = tickAt(x);
		menu.addAction(tr("Add a change of tempo here"), this, [this, tick] { addMarker(tick); });
		menu.addAction(tr("Add a marker here..."), this, [this, tick] { addNamed(tick); });
	}
	menu.exec(event->globalPos());
}




void TempoLaneWidget::addMarker(tick_t tick)
{
	auto* song = Engine::getSong();
	const auto current = m_map->isEmpty() ? static_cast<double>(song->getTempo()) : m_map->tempoAt(tick);
	auto ok = false;
	const auto bpm = QInputDialog::getDouble(this, tr("Change of tempo"), tr("Tempo from here on (BPM):"), current,
		MinTempo, MaxTempo, 2, &ok);
	if (!ok) { return; }
	m_map->setMarker({tick, bpm, false}, song->getTempo());
	song->setModified();
}




void TempoLaneWidget::editTempo(tick_t tick)
{
	for (auto marker : m_map->markers())
	{
		if (marker.tick != tick) { continue; }
		auto ok = false;
		const auto bpm = QInputDialog::getDouble(this, tr("Change of tempo"), tr("Tempo from here on (BPM):"),
			marker.bpm, MinTempo, MaxTempo, 2, &ok);
		if (!ok) { return; }
		marker.bpm = bpm;
		m_map->setMarker(marker, Engine::getSong()->getTempo());
		Engine::getSong()->setModified();
		return;
	}
}

void TempoLaneWidget::addNamed(tick_t tick)
{
	auto& markers = Engine::getSong()->markers();
	auto ok = false;
	const auto name = QInputDialog::getText(this, tr("Marker"), tr("Name (e.g. Chorus):"), QLineEdit::Normal,
		tr("Marker %1").arg(markers.markers().size() + 1), &ok);
	if (!ok) { return; }
	markers.add(tick, name);
	Engine::getSong()->setModified();
}




void TempoLaneWidget::renameNamed(tick_t tick)
{
	auto& markers = Engine::getSong()->markers();
	const auto marker = markers.at(tick);
	if (!marker) { return; }
	auto ok = false;
	const auto name = QInputDialog::getText(this, tr("Marker"), tr("Name:"), QLineEdit::Normal, marker->name, &ok);
	if (!ok) { return; }
	markers.rename(tick, name);
	Engine::getSong()->setModified();
}




void TempoLaneWidget::namedMenu(QMenu& menu, int x)
{
	auto* song = Engine::getSong();
	auto& markers = song->markers();
	const auto marker = namedMarkerAt(x);
	if (!marker)
	{
		const auto tick = tickAt(x);
		menu.addAction(tr("Add a marker here..."), this, [this, tick] { addNamed(tick); });
		return;
	}
	const auto tick = *marker;
	menu.addAction(tr("Rename..."), this, [this, tick] { renameNamed(tick); });
	menu.addAction(tr("Color..."), this, [this, tick] {
		auto& markers = Engine::getSong()->markers();
		const auto color = QColorDialog::getColor(markerColor(markers.at(tick)->color), this, tr("Marker color"));
		if (color.isValid()) { markers.setColor(tick, color); Engine::getSong()->setModified(); }
	});
	// Marking the tempo: a change of tempo at the marker (the tempo map's)
	const auto& tempo = m_map->markers();
	const bool marksTempo = std::any_of(tempo.begin(), tempo.end(), [tick](const TempoMap::Marker& m) { return m.tick == tick; });
	auto* marks = menu.addAction(tr("Marks a change of tempo"));
	marks->setCheckable(true);
	marks->setChecked(marksTempo);
	connect(marks, &QAction::toggled, this, [this, tick](bool on) {
		if (on) { addMarker(tick); }
		else if (tick > 0) { m_map->removeMarker(tick); Engine::getSong()->setModified(); }
	});
	menu.addAction(tr("Play from here"), this, [tick] {
		Engine::getSong()->setPlayPos(tick, Song::PlayMode::Song);
		if (!Engine::getSong()->isPlaying()) { Engine::getSong()->setPlayPos(tick, Song::PlayMode::None); }
	});
	menu.addSeparator();
	menu.addAction(tr("Remove"), this, [tick] {
		Engine::getSong()->markers().remove(tick);
		Engine::getSong()->setModified();
	});
	(void)markers;
}

} // namespace lmms::gui
