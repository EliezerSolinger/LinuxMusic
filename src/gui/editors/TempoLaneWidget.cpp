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
#include <QComboBox>
#include <QDialog>
#include <QDialogButtonBox>
#include <QFormLayout>
#include <QInputDialog>
#include <QPushButton>
#include <QSpinBox>
#include <QLineEdit>
#include <QColorDialog>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>

#include "Engine.h"
#include "ExportWizard.h"
#include "GuiApplication.h"
#include "MainWindow.h"
#include "Song.h"
#include "TempoMap.h"
#include "UndoCommand.h"
#include "ProjectJournal.h"
#include "TimeLineWidget.h"
#include "TimePos.h"

namespace lmms::gui
{

namespace
{
constexpr int MarkerRow = 16; //!< the named markers, above
constexpr int RegionTop = MarkerRow;
constexpr int RegionRow = 15; //!< the regions, between
constexpr int TempoTop = RegionTop + RegionRow;
constexpr int TempoRow = 18; //!< the tempo, below
constexpr int LaneHeight = MarkerRow + RegionRow + TempoRow;
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
	setToolTip(tr("Markers above (M adds one at the playhead; right-click to add, rename, color or remove); regions "
		"in the middle (drag to draw one, click one to cycle over it, drag its edges to resize it); tempo below "
		"(double-click to add a change of tempo); drag any of them to move it"));
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
	painter.drawLine(0, TempoTop - 1, width(), TempoTop - 1);
	painter.drawLine(0, height() - 1, width(), height() - 1);

	auto font = painter.font();
	font.setPixelSize(10);
	painter.setFont(font);

	painter.setClipRect(m_headWidth, 0, width() - m_headWidth, height());

	paintRegions(painter);

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
	const auto top = TempoTop;
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
		auto text = marker.ramp ? bpm + QString{" ↗"} : bpm;
		// A change of the time signature, after the tempo
		if (marker.meter) { text += QString{"  %1/%2"}.arg(marker.meter->numerator).arg(marker.meter->denominator); }
		painter.drawText(QRect{x + 3, top, 120, rowHeight}, Qt::AlignVCenter, text);
	}

	painter.setClipping(false);
	painter.fillRect(0, 0, m_headWidth, height(), QColor{30, 35, 41});
	painter.setPen(QColor{150, 160, 172});
	painter.drawText(QRect{6, 0, m_headWidth - 6, MarkerRow}, Qt::AlignVCenter, tr("MARKERS"));
	painter.drawText(QRect{6, RegionTop, m_headWidth - 6, RegionRow}, Qt::AlignVCenter, tr("REGIONS"));
	painter.drawText(QRect{6, TempoTop, m_headWidth - 6, TempoRow}, Qt::AlignVCenter, tr("TEMPO"));
}




void TempoLaneWidget::mousePressEvent(QMouseEvent* event)
{
	const auto x = static_cast<int>(event->position().x());
	const auto y = event->position().y();
	const auto upper = y < MarkerRow;
	if (event->button() == Qt::LeftButton && x > m_headWidth && y >= RegionTop && y < TempoTop)
	{
		// A region: its edge resizes it, its body moves it (or makes it the cycle, if it does not move), and
		// the row elsewhere draws a new one
		m_pressTick = tickAt(x);
		m_regionMoved = false;
		if (const auto hit = regionAt(x))
		{
			const auto region = std::find_if(Engine::getSong()->markers().regions().begin(),
				Engine::getSong()->markers().regions().end(), [&](const auto& r) { return r.begin == hit->begin; });
			m_regionDrag = hit->part;
			m_regionOrigin = hit->begin;
			m_regionPreview = {region->begin, region->end};
		}
		else
		{
			m_regionDrag = RegionDrag::Create;
			m_regionPreview = {m_pressTick, m_pressTick};
		}
		update();
	}
	else if (event->button() == Qt::LeftButton && x > m_headWidth)
	{
		if (upper) { m_draggedNamed = namedMarkerAt(x); }
		// The tempo marker at the song's start stays there
		else if (const auto marker = markerAt(x); marker && *marker > 0)
		{
			m_dragged = marker;
			m_tempoBefore = m_map->markers(); // (the drag is one undo step)
		}
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
	if (m_regionDrag != RegionDrag::None)
	{
		const auto tick = tickAt(x);
		if (tick != m_pressTick) { m_regionMoved = true; }
		auto& [begin, end] = m_regionPreview;
		const auto step = std::max(1, TimePos::ticksPerBar() / 16);
		switch (m_regionDrag)
		{
		case RegionDrag::Create:
			begin = std::min(m_pressTick, tick);
			end = std::max(m_pressTick, tick);
			break;
		case RegionDrag::Move:
		{
			const auto& regions = Engine::getSong()->markers().regions();
			const auto found = std::find_if(regions.begin(), regions.end(),
				[this](const auto& r) { return r.begin == m_regionOrigin; });
			if (found == regions.end()) { break; }
			const auto length = found->end - found->begin;
			begin = std::clamp(found->begin + tick - m_pressTick, 0, TimePos::MaxTicks - length);
			end = begin + length;
			break;
		}
		case RegionDrag::Begin: begin = std::min(tick, end - step); break;
		case RegionDrag::End: end = std::max(tick, begin + step); break;
		case RegionDrag::None: break;
		}
		begin = std::max(begin, 0);
		update();
	}
	const auto y = event->position().y();
	const auto upper = y < MarkerRow;
	const auto inRegions = y >= RegionTop && y < TempoTop;
	auto cursor = Qt::ArrowCursor;
	if (x > m_headWidth && inRegions)
	{
		const auto hit = m_regionDrag != RegionDrag::None ? std::optional{RegionHit{0, m_regionDrag}} : regionAt(x);
		if (hit && (hit->part == RegionDrag::Begin || hit->part == RegionDrag::End)) { cursor = Qt::SizeHorCursor; }
		else if (hit && hit->part == RegionDrag::Move) { cursor = m_regionMoved ? Qt::ClosedHandCursor : Qt::PointingHandCursor; }
		else if (hit) { cursor = Qt::IBeamCursor; }
	}
	else
	{
		const auto over = x > m_headWidth && !inRegions
			&& (upper ? namedMarkerAt(x).has_value() : markerAt(x).has_value());
		if (m_dragged || m_draggedNamed || over) { cursor = Qt::SizeHorCursor; }
	}
	setCursor(cursor);
	QWidget::mouseMoveEvent(event);
}




void TempoLaneWidget::mouseReleaseEvent(QMouseEvent* event)
{
	if (m_dragged || m_draggedNamed) { Engine::getSong()->setModified(); }
	if (m_dragged) { recordTempoEdit(std::exchange(m_tempoBefore, {})); }
	if (m_regionDrag != RegionDrag::None && event->button() == Qt::LeftButton)
	{
		const auto drag = std::exchange(m_regionDrag, RegionDrag::None);
		const auto [begin, end] = m_regionPreview;
		auto& markers = Engine::getSong()->markers();
		if (drag == RegionDrag::Create)
		{
			if (m_regionMoved && end > begin) { addRegion(begin, end); }
		}
		else if (!m_regionMoved)
		{
			// A click: the region becomes the cycle
			const auto& regions = markers.regions();
			const auto found = std::find_if(regions.begin(), regions.end(),
				[this](const auto& r) { return r.begin == m_regionOrigin; });
			if (found != regions.end()) { cycleOver(found->begin, found->end); }
		}
		else
		{
			markers.setRegionSpan(m_regionOrigin, begin, end);
			Engine::getSong()->setModified();
		}
		update();
	}
	m_dragged.reset();
	m_draggedNamed.reset();
	QWidget::mouseReleaseEvent(event);
}




void TempoLaneWidget::mouseDoubleClickEvent(QMouseEvent* event)
{
	const auto x = static_cast<int>(event->position().x());
	if (x <= m_headWidth) { return; }
	const auto y = event->position().y();
	if (y >= RegionTop && y < TempoTop)
	{
		// The regions: one is renamed, or one is added over the cycle (or a bar, without one)
		m_regionDrag = RegionDrag::None;
		if (const auto hit = regionAt(x)) { renameRegion(hit->begin); }
		else
		{
			const auto& timeline = Engine::getSong()->getTimeline(Song::PlayMode::Song);
			const auto tick = tickAt(x);
			if (timeline.loopEnd() > timeline.loopBegin() && tick >= timeline.loopBegin().getTicks()
				&& tick < timeline.loopEnd().getTicks())
			{
				addRegionFromCycle();
			}
			else
			{
				// The song's bar it is in (in its time signature)
				const auto bar = m_map->barAt(tick);
				addRegion(bar.start, bar.start + bar.length);
			}
		}
		return;
	}
	if (y < MarkerRow)
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
	if (event->pos().y() >= RegionTop && event->pos().y() < TempoTop)
	{
		regionMenu(menu, x);
		menu.exec(event->globalPos());
		return;
	}
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
		menu.addAction(tr("Time signature..."), this, [this, tick] { editMeter(tick); });
		auto* ramp = menu.addAction(tr("Ramp to the next marker"));
		ramp->setCheckable(true);
		ramp->setChecked(found != markers.end() && found->ramp);
		connect(ramp, &QAction::toggled, this, [this, tick](bool on) {
			for (auto marker : m_map->markers())
			{
				if (marker.tick != tick) { continue; }
				marker.ramp = on;
				auto before = m_map->markers();
				m_map->setMarker(marker, Engine::getSong()->getTempo());
				recordTempoEdit(std::move(before));
				Engine::getSong()->setModified();
				break;
			}
		});
		if (tick > 0)
		{
			menu.addSeparator();
			menu.addAction(tr("Remove"), this, [this, tick] {
				auto before = m_map->markers();
				m_map->removeMarker(tick);
				recordTempoEdit(std::move(before));
				Engine::getSong()->setModified();
			});
		}
	}
	else
	{
		const auto tick = tickAt(x);
		menu.addAction(tr("Add a change of tempo here"), this, [this, tick] { addMarker(tick); });
		// A new time signature starts a bar: at the start of the nearest one
		const auto barStart = m_map->quantize(tick, 1);
		menu.addAction(tr("Change the time signature here..."), this, [this, barStart] { editMeter(barStart); });
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
	auto before = m_map->markers();
	m_map->setMarker({tick, bpm, false}, song->getTempo());
	recordTempoEdit(std::move(before));
	song->setModified();
}




void TempoLaneWidget::editMeter(tick_t tick)
{
	auto* song = Engine::getSong();
	if (tick <= 0)
	{
		// At the start it is the song's (the time signature display's)
		auto& model = song->getTimeSigModel();
		const auto chosen = askMeter({model.getNumerator(), model.getDenominator()}, false);
		if (!chosen || !*chosen) { return; }
		model.setNumerator((*chosen)->numerator);
		model.setDenominator((*chosen)->denominator);
		song->setModified();
		return;
	}
	const auto& markers = m_map->markers();
	const auto found = std::find_if(markers.begin(), markers.end(),
		[tick](const TempoMap::Marker& marker) { return marker.tick == tick; });
	const bool hasOne = found != markers.end() && found->meter;
	const auto chosen = askMeter(hasOne ? *found->meter : m_map->meterAt(tick), hasOne);
	if (!chosen) { return; }
	auto before = m_map->markers();
	m_map->setMeter(tick, *chosen, song->getTempo());
	recordTempoEdit(std::move(before));
	song->setModified();
}




auto TempoLaneWidget::askMeter(const TempoMap::Meter& current, bool removable)
	-> std::optional<std::optional<TempoMap::Meter>>
{
	auto dialog = QDialog{this};
	dialog.setWindowTitle(tr("Time signature"));
	auto* layout = new QFormLayout{&dialog};
	auto* numerator = new QSpinBox{&dialog};
	numerator->setRange(1, TempoMap::MaxNumerator);
	numerator->setValue(current.numerator);
	auto* denominator = new QComboBox{&dialog};
	for (const auto value : {1, 2, 4, 8, 16, 32}) { denominator->addItem(QString::number(value), value); }
	denominator->setCurrentIndex(std::max(0, denominator->findData(current.denominator)));
	layout->addRow(tr("Beats to the bar:"), numerator);
	layout->addRow(tr("Note value of a beat:"), denominator);
	auto* buttons = new QDialogButtonBox{QDialogButtonBox::Ok | QDialogButtonBox::Cancel, &dialog};
	auto removed = false;
	if (removable)
	{
		// The time signature before goes on from here
		auto* remove = buttons->addButton(tr("Take the change away"), QDialogButtonBox::DestructiveRole);
		connect(remove, &QPushButton::clicked, &dialog, [&] {
			removed = true;
			dialog.accept();
		});
	}
	connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
	connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
	layout->addRow(buttons);
	if (dialog.exec() != QDialog::Accepted) { return std::nullopt; }
	if (removed) { return std::optional<TempoMap::Meter>{}; }
	return std::optional<TempoMap::Meter>{TempoMap::Meter{numerator->value(), denominator->currentData().toInt()}};
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
		auto before = m_map->markers();
		m_map->setMarker(marker, Engine::getSong()->getTempo());
		recordTempoEdit(std::move(before));
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
		else if (tick > 0)
		{
			auto before = m_map->markers();
			m_map->removeMarker(tick);
			recordTempoEdit(std::move(before));
			Engine::getSong()->setModified();
		}
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




auto TempoLaneWidget::regionColor(const QColor& color) -> QColor
{
	return color.isValid() ? color : QColor{92, 152, 226};
}




auto TempoLaneWidget::regionAt(int x) const -> std::optional<RegionHit>
{
	// An edge first (of the shortest region, if several meet), then the shortest region under the mouse
	auto best = std::optional<RegionHit>{};
	auto bestLength = TimePos::MaxTicks + 1;
	for (const auto& region : Engine::getSong()->markers().regions())
	{
		const auto left = xOf(region.begin);
		const auto right = xOf(region.end);
		const auto length = region.end - region.begin;
		// (a region narrower than its grips is moved, not resized)
		const auto grips = right - left > 3 * GrabDistance;
		auto part = RegionDrag::None;
		if (grips && std::abs(x - left) <= GrabDistance) { part = RegionDrag::Begin; }
		else if (grips && std::abs(x - right) <= GrabDistance) { part = RegionDrag::End; }
		else if (x >= left && x < right) { part = RegionDrag::Move; }
		if (part == RegionDrag::None) { continue; }
		const auto isEdge = part != RegionDrag::Move;
		const auto bestIsEdge = best && best->part != RegionDrag::Move;
		if (!best || (isEdge && !bestIsEdge) || (isEdge == bestIsEdge && length < bestLength))
		{
			best = RegionHit{region.begin, part};
			bestLength = length;
		}
	}
	return best;
}




void TempoLaneWidget::paintRegions(QPainter& painter)
{
	const auto& regions = Engine::getSong()->markers().regions();
	// The longer ones below, so a short region over a long one shows
	auto order = std::vector<std::size_t>(regions.size());
	for (std::size_t index = 0; index < order.size(); ++index) { order[index] = index; }
	std::stable_sort(order.begin(), order.end(), [&](std::size_t a, std::size_t b) {
		return regions[a].end - regions[a].begin > regions[b].end - regions[b].begin;
	});

	const auto& timeline = Engine::getSong()->getTimeline(Song::PlayMode::Song);
	const auto drawRegion = [&](tick_t begin, tick_t end, const QString& name, const QColor& base, bool dragged) {
		const auto left = xOf(begin);
		const auto right = std::max(left + 2, xOf(end));
		const auto band = QRect{left, RegionTop + 1, right - left, RegionRow - 3};
		auto fill = base;
		// The region that is the cycle shows brighter
		const auto isCycle = timeline.loopBegin().getTicks() == begin && timeline.loopEnd().getTicks() == end;
		fill.setAlpha(dragged ? 200 : isCycle ? 170 : 110);
		painter.fillRect(band, fill);
		painter.setPen(base.lighter(130));
		painter.drawLine(band.topLeft(), band.bottomLeft());
		painter.drawLine(band.topRight(), band.bottomRight());
		painter.setPen(QColor{245, 247, 250});
		const auto text = painter.fontMetrics().elidedText(name, Qt::ElideRight, std::max(band.width() - 8, 0));
		painter.drawText(band.adjusted(4, 0, -4, 0), Qt::AlignVCenter, text);
	};

	const auto dragging = m_regionDrag != RegionDrag::None && m_regionDrag != RegionDrag::Create && m_regionMoved;
	for (const auto index : order)
	{
		const auto& region = regions[index];
		if (dragging && region.begin == m_regionOrigin) { continue; }
		drawRegion(region.begin, region.end, region.name.isEmpty() ? QString::number(index + 1) : region.name,
			regionColor(region.color), false);
	}
	if (m_regionDrag != RegionDrag::None && m_regionMoved)
	{
		const auto& [begin, end] = m_regionPreview;
		auto name = QString{};
		auto color = QColor{};
		for (const auto& region : regions)
		{
			if (m_regionDrag != RegionDrag::Create && region.begin == m_regionOrigin) { name = region.name; color = region.color; }
		}
		if (end > begin) { drawRegion(begin, end, name, regionColor(color), true); }
	}
}




void TempoLaneWidget::addRegion(tick_t begin, tick_t end)
{
	auto& markers = Engine::getSong()->markers();
	auto ok = false;
	const auto name = QInputDialog::getText(this, tr("Region"), tr("Name (e.g. Chorus):"), QLineEdit::Normal,
		tr("Region %1").arg(markers.regions().size() + 1), &ok);
	if (!ok) { return; }
	markers.addRegion(begin, end, name);
	Engine::getSong()->setModified();
}




void TempoLaneWidget::addRegionFromCycle()
{
	const auto& timeline = Engine::getSong()->getTimeline(Song::PlayMode::Song);
	if (timeline.loopEnd() <= timeline.loopBegin()) { return; }
	addRegion(timeline.loopBegin().getTicks(), timeline.loopEnd().getTicks());
}




void TempoLaneWidget::renameRegion(tick_t begin)
{
	auto& markers = Engine::getSong()->markers();
	const auto& regions = markers.regions();
	const auto found = std::find_if(regions.begin(), regions.end(), [begin](const auto& r) { return r.begin == begin; });
	if (found == regions.end()) { return; }
	auto ok = false;
	const auto name = QInputDialog::getText(this, tr("Region"), tr("Name:"), QLineEdit::Normal, found->name, &ok);
	if (!ok) { return; }
	markers.renameRegion(begin, name);
	Engine::getSong()->setModified();
}




void TempoLaneWidget::cycleOver(tick_t begin, tick_t end)
{
	auto& timeline = Engine::getSong()->getTimeline(Song::PlayMode::Song);
	timeline.setLoopPoints(TimePos{begin}, TimePos{end});
	timeline.setLoopEnabled(true);
}




void TempoLaneWidget::regionMenu(QMenu& menu, int x)
{
	const auto hit = regionAt(x);
	if (!hit)
	{
		const auto& timeline = Engine::getSong()->getTimeline(Song::PlayMode::Song);
		auto* fromCycle = menu.addAction(tr("Add a region over the cycle..."), this, [this] { addRegionFromCycle(); });
		fromCycle->setEnabled(timeline.loopEnd() > timeline.loopBegin());
		// The song's bar it is in (in its time signature)
		const auto bar = m_map->barAt(tickAt(x));
		menu.addAction(tr("Add a region of a bar here..."), this,
			[this, bar] { addRegion(bar.start, bar.start + bar.length); });
		return;
	}
	const auto begin = hit->begin;
	const auto& regions = Engine::getSong()->markers().regions();
	const auto region = *std::find_if(regions.begin(), regions.end(), [begin](const auto& r) { return r.begin == begin; });
	menu.addAction(tr("Rename..."), this, [this, begin] { renameRegion(begin); });
	menu.addAction(tr("Color..."), this, [this, region] {
		const auto color = QColorDialog::getColor(regionColor(region.color), this, tr("Region color"));
		if (color.isValid())
		{
			Engine::getSong()->markers().setRegionColor(region.begin, color);
			Engine::getSong()->setModified();
		}
	});
	menu.addSeparator();
	menu.addAction(tr("Cycle over the region"), this, [region] { cycleOver(region.begin, region.end); });
	menu.addAction(tr("Play from its start"), this, [region] {
		auto* song = Engine::getSong();
		song->setPlayPos(region.begin, Song::PlayMode::Song);
		if (!song->isPlaying()) { song->playSong(); }
	});
	menu.addAction(tr("Export the region as audio..."), this, [region] {
		auto wizard = ExportWizard{getGUI()->mainWindow()};
		wizard.setSpan(TimePos{region.begin}, TimePos{region.end}, region.name);
		wizard.exec();
	});
	menu.addSeparator();
	menu.addAction(tr("Remove"), this, [begin] {
		Engine::getSong()->markers().removeRegion(begin);
		Engine::getSong()->setModified();
	});
}

void TempoLaneWidget::recordTempoEdit(std::vector<TempoMap::Marker> before)
{
	m_map->recordChange(std::move(before));
}

} // namespace lmms::gui
