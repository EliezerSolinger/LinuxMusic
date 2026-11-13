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

#include <QActionGroup>
#include <QApplication>
#include <QMenu>
#include <QPainter>
#include <QPainterPath>

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
	painter.fillRect(rect(), QColor{0, 0, 0});
	auto gradient = QLinearGradient{0, 0, 0, static_cast<qreal>(height())};
	gradient.setColorAt(0, background);
	gradient.setColorAt(1, background.darker(300));
	painter.fillRect(rect(), gradient);

	const auto itemColor = m_item->color().value_or(m_item->getTrack()->color().value_or(painter.pen().brush().color()));
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
				.amplification = std::min(m_item->gain(), 4.f),
				.reversed = false
			};
			SampleThumbnailRenderer::visualize(parameters, painter);
		}
	}

	// The fades: shaded corners
	const auto rate = static_cast<double>(Engine::audioEngine()->outputSampleRate());
	const auto framesPerTick = std::max(1.0, static_cast<double>(Engine::framesPerTick()));
	const auto fadePixels = [&](double ms) { return ms * rate / 1000.0 / framesPerTick * pixelsPerTick; };
	painter.setRenderHint(QPainter::Antialiasing, true);
	painter.setPen(Qt::NoPen);
	painter.setBrush(QColor{0, 0, 0, 110});
	if (const auto in = fadePixels(m_item->fadeInMs()); in >= 2)
	{
		auto path = QPainterPath{};
		path.moveTo(0, 0);
		path.lineTo(in, 0);
		path.lineTo(0, height());
		path.closeSubpath();
		painter.drawPath(path);
	}
	if (const auto out = fadePixels(m_item->fadeOutMs()); out >= 2)
	{
		auto path = QPainterPath{};
		path.moveTo(width(), 0);
		path.lineTo(width() - out, 0);
		path.lineTo(width(), height());
		path.closeSubpath();
		painter.drawPath(path);
	}
	painter.setRenderHint(QPainter::Antialiasing, false);

	// The name, and the gain if not 0 dB
	auto name = PathUtil::cleanName(m_item->file());
	if (std::abs(m_item->gainDb()) >= 0.05) { name += QString{"  %1%2 dB"}.arg(m_item->gainDb() > 0 ? "+" : "").arg(m_item->gainDb(), 0, 'f', 1); }
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

	menu->addAction(tr("Reset gain and fades"), m_item, [track, item = m_item] {
		auto settings = AudioTrack::settingsOf(*item);
		settings.gainDb = 0.0;
		settings.fadeInMs = settings.fadeOutMs = AudioItem::DefaultFadeMs;
		track->changeItem(item, settings);
	});
}

} // namespace lmms::gui
