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
constexpr int LaneHeight = 18;
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
	setToolTip(tr("Tempo: double-click to add a change of tempo, drag a marker to move it, right-click to edit it"));
	connect(map, &TempoMap::changed, this, qOverload<>(&QWidget::update));
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




void TempoLaneWidget::paintEvent(QPaintEvent*)
{
	auto painter = QPainter{this};
	painter.fillRect(rect(), QColor{24, 28, 33});
	painter.setPen(QColor{12, 14, 17});
	painter.drawLine(0, height() - 1, width(), height() - 1);

	auto font = painter.font();
	font.setPixelSize(10);
	painter.setFont(font);

	painter.setClipRect(m_headWidth, 0, width() - m_headWidth, height());
	const auto& markers = m_map->markers();
	if (markers.empty())
	{
		painter.setPen(QColor{110, 118, 128});
		painter.drawText(QRect{m_headWidth + 6, 0, width(), height()}, Qt::AlignVCenter,
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
				return high > low ? static_cast<int>(height() - 3 - (bpm - low) / (high - low) * (height() - 6)) : height() / 2;
			};
			painter.setPen(QColor{71, 253, 133, 140});
			painter.drawLine(x, yOf(marker.bpm), xOf(next.tick), yOf(next.bpm));
		}

		painter.setPen(QColor{71, 253, 133});
		painter.drawLine(x, 0, x, height() - 2);
		const auto bpm = std::abs(marker.bpm - std::round(marker.bpm)) < 0.005
			? QString::number(static_cast<int>(std::round(marker.bpm)))
			: QString::number(marker.bpm, 'f', 2);
		painter.setPen(QColor{209, 216, 228});
		painter.drawText(QRect{x + 3, 0, 80, height()}, Qt::AlignVCenter, marker.ramp ? bpm + QString{" ↗"} : bpm);
	}

	painter.setClipping(false);
	painter.fillRect(0, 0, m_headWidth, height(), QColor{30, 35, 41});
	painter.setPen(QColor{150, 160, 172});
	painter.drawText(QRect{6, 0, m_headWidth - 6, height()}, Qt::AlignVCenter, tr("TEMPO"));
}




void TempoLaneWidget::mousePressEvent(QMouseEvent* event)
{
	const auto x = static_cast<int>(event->position().x());
	if (event->button() == Qt::LeftButton && x > m_headWidth)
	{
		// The marker at the song's start stays there
		if (const auto marker = markerAt(x); marker && *marker > 0) { m_dragged = marker; }
	}
	QWidget::mousePressEvent(event);
}




void TempoLaneWidget::mouseMoveEvent(QMouseEvent* event)
{
	const auto x = static_cast<int>(event->position().x());
	if (m_dragged)
	{
		const auto to = std::max(1, tickAt(x));
		if (to != *m_dragged)
		{
			m_map->moveMarker(*m_dragged, to);
			m_dragged = to;
		}
	}
	setCursor(m_dragged || (x > m_headWidth && markerAt(x)) ? Qt::SizeHorCursor : Qt::ArrowCursor);
	QWidget::mouseMoveEvent(event);
}




void TempoLaneWidget::mouseReleaseEvent(QMouseEvent* event)
{
	if (m_dragged) { Engine::getSong()->setModified(); }
	m_dragged.reset();
	QWidget::mouseReleaseEvent(event);
}




void TempoLaneWidget::mouseDoubleClickEvent(QMouseEvent* event)
{
	const auto x = static_cast<int>(event->position().x());
	if (x <= m_headWidth) { return; }
	if (const auto marker = markerAt(x)) { editTempo(*marker); }
	else { addMarker(tickAt(x)); }
}




void TempoLaneWidget::contextMenuEvent(QContextMenuEvent* event)
{
	const auto x = event->pos().x();
	if (x <= m_headWidth) { return; }
	auto menu = QMenu{this};
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

} // namespace lmms::gui
