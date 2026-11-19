/*
 * LiveTakeOverlay.cpp
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

#include "LiveTakeOverlay.h"

#include <algorithm>

#include <QEvent>
#include <QPainter>

#include "Engine.h"
#include "PeakCache.h"
#include "Song.h"
#include "TrackContainerView.h"
#include "TrackView.h"

namespace lmms::gui
{

LiveTakeOverlay::LiveTakeOverlay(TrackView* trackView, QWidget* content)
	: QWidget{content}
	, m_trackView{trackView}
{
	setAttribute(Qt::WA_TransparentForMouseEvents);
	setAttribute(Qt::WA_NoSystemBackground);
	setGeometry(content->rect());
	content->installEventFilter(this);
	hide();
	m_timer.setInterval(50);
	connect(&m_timer, &QTimer::timeout, this, &LiveTakeOverlay::refresh);
	m_timer.start();
}




auto LiveTakeOverlay::eventFilter(QObject* watched, QEvent* event) -> bool
{
	if (watched == parentWidget() && event->type() == QEvent::Resize) { setGeometry(parentWidget()->rect()); }
	return QWidget::eventFilter(watched, event);
}




void LiveTakeOverlay::refresh()
{
	auto* song = Engine::getSong();
	auto recordingHere = false;
	if (song->isRecordingAudio())
	{
		song->refreshLiveTakes();
		const auto& takes = song->liveTakes();
		recordingHere = std::any_of(takes.begin(), takes.end(),
			[this](const auto& take) { return take.track == m_trackView->getTrack(); });
	}
	if (recordingHere)
	{
		if (!isVisible()) { show(); }
		raise(); // (over the clips, also those made while it shows)
		update();
	}
	else if (isVisible()) { hide(); }
}




auto LiveTakeOverlay::xOf(double tick) const -> double
{
	const auto* view = m_trackView->trackContainerView();
	return (tick - view->currentPosition().getTicks()) * view->pixelsPerBar() / TimePos::ticksPerBar();
}




void LiveTakeOverlay::paintEvent(QPaintEvent*)
{
	auto painter = QPainter{this};
	const auto framesPerTick = std::max(1.0, static_cast<double>(Engine::framesPerTick()));
	const auto red = QColor{224, 64, 64};
	const auto top = 1;
	const auto bottom = height() - 2;
	const auto middle = (top + bottom) / 2.0;
	const auto halfHeight = (bottom - top) / 2.0;

	for (const auto& take : Engine::getSong()->liveTakes())
	{
		if (take.track != m_trackView->getTrack()) { continue; }
		for (const auto& segment : take.segments)
		{
			const auto left = xOf(static_cast<double>(segment.startTick));
			const auto right = xOf(segment.startTick + segment.frames / framesPerTick);
			if (right < 0 || left > width()) { continue; }
			const auto area = QRectF{QPointF{left, static_cast<double>(top)}, QPointF{std::max(right, left + 1), static_cast<double>(bottom)}};
			auto fill = red;
			fill.setAlpha(70);
			painter.fillRect(area, fill);
			painter.setPen(red);
			painter.drawRect(area);

			// The waveform: the peaks under each pixel (the partial one last)
			const auto count = segment.peaks.size() + (segment.frames % PeakCache::FramesPerPeak != 0 ? 1 : 0);
			if (count == 0 || right - left < 1) { continue; }
			const auto peaksPerPixel = count / (right - left);
			painter.setPen(red.lighter(140));
			const auto first = std::max(0, static_cast<int>(left));
			const auto last = std::min(width() - 1, static_cast<int>(right));
			for (auto x = first; x <= last; ++x)
			{
				const auto from = static_cast<std::size_t>(std::max(0.0, (x - left) * peaksPerPixel));
				const auto to = std::min(count, std::max(from + 1, static_cast<std::size_t>((x + 1 - left) * peaksPerPixel)));
				auto low = 0.f, high = 0.f;
				for (auto index = from; index < to; ++index)
				{
					const auto& peak = index < segment.peaks.size() ? segment.peaks[index] : segment.partial;
					low = std::min(low, peak.min);
					high = std::max(high, peak.max);
				}
				painter.drawLine(QPointF(x, middle - std::clamp(high, -1.f, 1.f) * halfHeight),
					QPointF(x, middle - std::clamp(low, -1.f, 1.f) * halfHeight));
			}
		}
	}
}

} // namespace lmms::gui
