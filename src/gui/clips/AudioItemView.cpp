/*
 * AudioItemView.cpp
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

#include "AudioItemView.h"

#include <QMouseEvent>
#include <QToolTip>
#include <cmath>
#include <numbers>
#include <algorithm>
#include <QActionGroup>
#include <QApplication>
#include <QMenu>
#include <QPainter>
#include <QPainterPath>

#include "AudioEngine.h"
#include "AudioItem.h"
#include "AudioTrack.h"
#include "Engine.h"
#include "PathUtil.h"
#include "SampleThumbnailCache.h"
#include "SampleThumbnailRenderer.h"
#include "Song.h"
#include "TrackView.h"

namespace lmms::gui
{

AudioItemView::AudioItemView(AudioItem* item, TrackView* view)
	: ClipView{item, view}
	, m_item{item}
{
	connect(item, &AudioItem::itemChanged, this, &AudioItemView::updateItem);
	updateItem();
	setStyle(QApplication::style());
}




void AudioItemView::updateItem()
{
	if (m_dragging == Handle::None)
	{
		m_gainDb = m_item->gainDb();
		m_fadeInMs = m_item->fadeInMs();
		m_fadeOutMs = m_item->fadeOutMs();
	}
	auto tip = PathUtil::toAbsolute(m_item->file());
	if (!tip.isEmpty())
	{
		tip += '\n' + (m_item->isStreamed() ? tr("Streamed from disk") : tr("Loaded into RAM"));
		tip += '\n' + tr("Gain %1 dB, fades %2 ms and %3 ms").arg(m_item->gainDb(), 0, 'f', 1)
			.arg(m_item->fadeInMs(), 0, 'f', 0).arg(m_item->fadeOutMs(), 0, 'f', 0);
	}
	setToolTip(tip);
	setNeedsUpdate(true);
	update();
}




void AudioItemView::paintEvent(QPaintEvent*)
{
	auto painter = QPainter{this};
	const bool muted = m_item->getTrack()->isMutedInMix() || m_item->isMuted();
	auto background = painter.background().color();
	if (muted) { background = background.darker(150); }
	if (isSelected()) { background = background.darker(150); }
	// A lane not heard: dimmed
	const auto* track = dynamic_cast<const AudioTrack*>(m_item->getTrack());
	const bool heard = !track || m_item->lane() == track->playingLane();
	if (!heard) { background = background.darker(170); }
	painter.fillRect(rect(), QColor{0, 0, 0});
	auto gradient = QLinearGradient{0, 0, 0, static_cast<qreal>(height())};
	gradient.setColorAt(0, background);
	gradient.setColorAt(1, background.darker(300));
	painter.fillRect(rect(), gradient);

	auto itemColor = m_item->color().value_or(m_item->getTrack()->color().value_or(painter.pen().brush().color()));
	if (!heard) { itemColor = itemColor.darker(160); }
	const auto ticksPerBar = static_cast<double>(TimePos::ticksPerBar());
	const auto pixelsPerTick = pixelsPerBar() / ticksPerBar;

	// The waveform: the part of the file under the item (it starts startTimeOffset() before the item)
	if (const auto* peaks = m_item->peaks(); peaks && peaks->sampleCount() > 0)
	{
		const auto fileTicks = static_cast<double>(m_item->fileLength().getTicks());
		const auto left = m_item->startTimeOffset().getTicks() * pixelsPerTick;
		const auto width = fileTicks * pixelsPerTick;
		if (width >= 1)
		{
			painter.setPen(itemColor);
			const auto parameters = SampleThumbnailRenderer::VisualizeParameters{
				.cache = peaks,
				.rect = QRect(static_cast<int>(left), BORDER_WIDTH + 1, static_cast<int>(width), height() - 2 * BORDER_WIDTH - 2),
				.amplification = std::min(m_gainDb <= AudioItem::MinGainDb ? 0.f
					: static_cast<float>(std::pow(10.0, m_gainDb / 20.0)), 4.f),
				.reversed = false
			};
			SampleThumbnailRenderer::visualize(parameters, painter);
		}
	}

	// The fades as heard (a crossfade over an overlap shows), and their handles
	const auto [fadeIn, fadeOut] = heardFades();
	painter.setRenderHint(QPainter::Antialiasing, true);
	painter.setPen(Qt::NoPen);
	painter.setBrush(QColor{0, 0, 0, 110});
	if (const auto in = fadeIn * pixelsPerMs(); in >= 1)
	{
		auto path = QPainterPath{};
		path.moveTo(0, 0);
		path.lineTo(0, height());
		// (equal power: the curve of sin)
		for (int step = 0; step <= 16; ++step)
		{
			const auto x = in * step / 16.0;
			path.lineTo(x, height() * (1.0 - std::sin(std::numbers::pi / 2 * step / 16.0)));
		}
		path.lineTo(0, 0);
		painter.drawPath(path);
	}
	if (const auto out = fadeOut * pixelsPerMs(); out >= 1)
	{
		auto path = QPainterPath{};
		path.moveTo(width(), 0);
		path.lineTo(width(), height());
		for (int step = 0; step <= 16; ++step)
		{
			const auto x = width() - out * step / 16.0;
			path.lineTo(x, height() * (1.0 - std::sin(std::numbers::pi / 2 * step / 16.0)));
		}
		path.lineTo(width(), 0);
		painter.drawPath(path);
	}
	if (underMouse() || m_dragging != Handle::None)
	{
		painter.setBrush(QColor{230, 230, 230});
		const auto handle = [&](double x) { painter.drawRect(QRectF{x - 3, 0, 6, 6}); };
		handle(std::clamp(m_fadeInMs * pixelsPerMs(), 3.0, width() - 3.0));
		handle(std::clamp(width() - m_fadeOutMs * pixelsPerMs(), 3.0, width() - 3.0));
		// The gain: a line along the top, lower as the gain is lower
		const auto gainY = std::clamp((AudioItem::MaxGainDb - m_gainDb) / (AudioItem::MaxGainDb - AudioItem::MinGainDb)
			* height(), 1.0, height() - 1.0);
		painter.setPen(QColor{230, 230, 230, 160});
		painter.drawLine(QPointF{0, gainY}, QPointF{static_cast<double>(width()), gainY});
	}
	painter.setRenderHint(QPainter::Antialiasing, false);

	// The name, and the gain if not 0 dB
	auto name = PathUtil::cleanName(m_item->file());
	if (std::abs(m_gainDb) >= 0.05) { name += QString{"  %1%2 dB"}.arg(m_gainDb > 0 ? "+" : "").arg(m_gainDb, 0, 'f', 1); }
	paintTextLabel(name, painter);

	painter.setPen(background.lighter(135));
	painter.drawRect(1, 1, rect().right() - BORDER_WIDTH, rect().bottom() - BORDER_WIDTH);
	painter.setPen(background.darker(200));
	painter.drawRect(0, 0, rect().right(), rect().bottom());
}




void AudioItemView::constructContextMenu(QMenu* menu)
{
	auto* track = dynamic_cast<AudioTrack*>(m_item->getTrack());
	if (!track || m_item->file().isEmpty()) { return; }
	menu->addSeparator();

	// Where the audio is while it plays, as Ableton's RAM switch
	auto* storageMenu = menu->addMenu(m_item->isStreamed() ? tr("Memory: from disk") : tr("Memory: in RAM"));
	auto* storages = new QActionGroup{storageMenu};
	const auto addStorage = [&](const QString& text, AudioItem::Storage storage) {
		auto* action = storageMenu->addAction(text);
		action->setCheckable(true);
		action->setChecked(m_item->storage() == storage);
		storages->addAction(action);
		connect(action, &QAction::triggered, m_item, [track, item = m_item, storage] {
			auto settings = AudioTrack::settingsOf(*item);
			settings.storage = storage;
			track->changeItem(item, settings);
		});
	};
	addStorage(tr("Automatic (files of %1 s or more from disk)").arg(AudioItem::StreamAboveSeconds),
		AudioItem::Storage::Automatic);
	addStorage(tr("Load into RAM"), AudioItem::Storage::Memory);
	addStorage(tr("Stream from disk"), AudioItem::Storage::Disk);

	if (track->laneCount() > 1 && m_item->lane() != track->playingLane())
	{
		menu->addAction(tr("Hear this lane"), m_item, [track, lane = m_item->lane()] { track->setPlayingLane(lane); });
		menu->addAction(tr("Comp: use this item on the lane heard"), m_item, [track, item = m_item] { track->compItem(item); });
	}

	menu->addAction(tr("Reset gain and fades"), m_item, [track, item = m_item] {
		auto settings = AudioTrack::settingsOf(*item);
		settings.gainDb = 0.0;
		settings.fadeInMs = settings.fadeOutMs = AudioItem::DefaultFadeMs;
		track->changeItem(item, settings);
	});
}

auto AudioItemView::verticalPlace(int contentHeight) const -> std::pair<int, int>
{
	const auto* track = dynamic_cast<const AudioTrack*>(m_item->getTrack());
	const auto lanes = track ? track->laneCount() : 1;
	if (lanes <= 1) { return {0, contentHeight - 1}; }
	// Each lane a band across the track, the first on top
	const auto laneHeight = std::max(contentHeight / lanes, 4);
	return {m_item->lane() * laneHeight, laneHeight - 1};
}




auto AudioItemView::pixelsPerMs() const -> double
{
	const auto rate = static_cast<double>(Engine::audioEngine()->outputSampleRate());
	const auto framesPerTick = std::max(1.0, static_cast<double>(Engine::framesPerTick()));
	return rate / 1000.0 / framesPerTick * pixelsPerBar() / TimePos::ticksPerBar();
}




auto AudioItemView::heardFades() const -> std::pair<double, double>
{
	auto fadeIn = m_fadeInMs;
	auto fadeOut = m_fadeOutMs;
	const auto* track = dynamic_cast<const AudioTrack*>(m_item->getTrack());
	if (!track) { return {fadeIn, fadeOut}; }
	const auto msPerTick = static_cast<double>(Engine::framesPerTick()) * 1000.0
		/ Engine::audioEngine()->outputSampleRate();
	const auto begin = m_item->startPosition().getTicks();
	const auto end = m_item->endPosition().getTicks();
	for (const auto* other : track->items())
	{
		if (other == m_item || other->lane() != m_item->lane() || other->isMuted()) { continue; }
		const auto otherBegin = other->startPosition().getTicks();
		const auto otherEnd = other->endPosition().getTicks();
		// (as the player does: an item inside another does not cross)
		if (otherBegin < begin && otherEnd > begin && otherEnd < end) { fadeIn = std::max(fadeIn, (otherEnd - begin) * msPerTick); }
		if (otherBegin > begin && otherBegin < end && otherEnd > end) { fadeOut = std::max(fadeOut, (end - otherBegin) * msPerTick); }
	}
	const auto half = (end - begin) * msPerTick / 2;
	return {std::min(fadeIn, half), std::min(fadeOut, half)};
}




auto AudioItemView::handleAt(QPoint position) const -> Handle
{
	if (position.y() > 7 || const_cast<AudioItemView*>(this)->fixedClips()) { return Handle::None; }
	const auto inX = std::clamp(m_fadeInMs * pixelsPerMs(), 3.0, width() - 3.0);
	const auto outX = std::clamp(width() - m_fadeOutMs * pixelsPerMs(), 3.0, width() - 3.0);
	if (std::abs(position.x() - inX) <= 5) { return Handle::FadeIn; }
	if (std::abs(position.x() - outX) <= 5) { return Handle::FadeOut; }
	return position.x() > inX + 5 && position.x() < outX - 5 ? Handle::Gain : Handle::None;
}




void AudioItemView::mousePressEvent(QMouseEvent* event)
{
	const auto handle = event->button() == Qt::LeftButton && event->modifiers() == Qt::NoModifier
		? handleAt(event->position().toPoint()) : Handle::None;
	if (handle == Handle::None) { ClipView::mousePressEvent(event); return; }
	m_dragging = handle;
	m_dragStart = event->position().toPoint();
	m_startValue = handle == Handle::FadeIn ? m_fadeInMs : handle == Handle::FadeOut ? m_fadeOutMs : m_gainDb;
	event->accept();
}




void AudioItemView::mouseMoveEvent(QMouseEvent* event)
{
	if (m_dragging == Handle::None)
	{
		ClipView::mouseMoveEvent(event);
		if (!(event->buttons() & Qt::LeftButton))
		{
			const auto handle = handleAt(event->position().toPoint());
			if (handle == Handle::Gain) { setCursor(Qt::SizeVerCursor); }
			else if (handle != Handle::None) { setCursor(Qt::SizeHorCursor); }
		}
		update();
		return;
	}
	const auto delta = event->position().toPoint() - m_dragStart;
	auto text = QString{};
	switch (m_dragging)
	{
	case Handle::FadeIn:
		m_fadeInMs = std::max(0.0, m_startValue + delta.x() / pixelsPerMs());
		text = tr("Fade in: %1 ms").arg(m_fadeInMs, 0, 'f', 0);
		break;
	case Handle::FadeOut:
		m_fadeOutMs = std::max(0.0, m_startValue - delta.x() / pixelsPerMs());
		text = tr("Fade out: %1 ms").arg(m_fadeOutMs, 0, 'f', 0);
		break;
	case Handle::Gain:
		// A dB for every two pixels; with Shift, a tenth
		m_gainDb = std::clamp(m_startValue - delta.y() * (event->modifiers() & Qt::ShiftModifier ? 0.05 : 0.5),
			AudioItem::MinGainDb, AudioItem::MaxGainDb);
		text = m_gainDb <= AudioItem::MinGainDb ? tr("Gain: silent") : tr("Gain: %1 dB").arg(m_gainDb, 0, 'f', 1);
		break;
	case Handle::None: break;
	}
	QToolTip::showText(event->globalPosition().toPoint(), text, this);
	setNeedsUpdate(true);
	update();
}




void AudioItemView::mouseReleaseEvent(QMouseEvent* event)
{
	if (m_dragging == Handle::None) { ClipView::mouseReleaseEvent(event); return; }
	m_dragging = Handle::None;
	QToolTip::hideText();
	if (auto* track = dynamic_cast<AudioTrack*>(m_item->getTrack()))
	{
		auto settings = AudioTrack::settingsOf(*m_item);
		settings.gainDb = m_gainDb;
		settings.fadeInMs = m_fadeInMs;
		settings.fadeOutMs = m_fadeOutMs;
		track->changeItem(m_item, settings); // (one undo step)
	}
	updateItem();
}

} // namespace lmms::gui
