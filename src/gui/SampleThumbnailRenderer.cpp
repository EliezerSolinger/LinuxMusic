/*
 * SampleThumbnailRenderer.cpp
 *
 * Copyright (c) 2026 Khoi Dau <casboi86@gmail.com>
 * Copyright (c) 2026 saker <sakertooth@gmail.com>
 *
 * This file is part of LMMS - https://lmms.io
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

#include "SampleThumbnailRenderer.h"

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <numeric>

#include <QFileInfo>
#include <QPainter>

#include "SampleThumbnailCache.h"

namespace lmms::gui {

void SampleThumbnailRenderer::visualize(VisualizeParameters parameters, QPainter& painter)
{
	// Whatever it is asked: a range outside the sample (a view's points from the sample before the one it now has),
	// an empty one (start and end together) or not a number (points over a sample of no frames) draws what is
	// inside the sample, or nothing (upstream's #8647 crashed in here, opening the AudioFileProcessor)
	if (!std::isfinite(parameters.sampleStart) || !std::isfinite(parameters.sampleEnd)) { return; }
	parameters.sampleStart = std::clamp(parameters.sampleStart, 0.0, 1.0);
	parameters.sampleEnd = std::clamp(parameters.sampleEnd, 0.0, 1.0);
	if (parameters.sampleEnd <= parameters.sampleStart) { return; }

	// (no peaks yet: those of a file streamed from disk are built in the background)
	if (!(parameters.rect.width() >= 1) || !parameters.cache || parameters.cache->sampleCount() == 0) { return; }

	// Get the number of samples that will be drawn in this rectangle
	const auto numSamples = (parameters.sampleEnd - parameters.sampleStart) * parameters.cache->sampleCount();

	// For our rectangle, how many samples each pixel has
	const auto samplesPerPixel = numSamples / parameters.rect.width();

	// We want a thumbnail that has the same SPP as our own rectangle
	// However, it needs to cover the full sample
	// (a width between one peak and one per frame: zoomed in further than a frame a pixel, the finest)
	const auto wanted = std::clamp(std::round(parameters.cache->sampleCount() / samplesPerPixel), 1.0,
		static_cast<double>(parameters.cache->sampleCount()));
	const auto& nearestThumbnail = parameters.cache->nearestThumbnailForWidth(static_cast<f_cnt_t>(wanted));
	if (nearestThumbnail.width() == 0) { return; }

	const auto yScale = parameters.rect.height() / 2 * parameters.amplification;
	const auto yCenter = parameters.rect.center().y();

	// Calculate how far along we should be in the thumbnail based on where we begin and end in the sample
	const auto nearestThumbnailStart = parameters.sampleStart * nearestThumbnail.width();
	const auto nearestThumbnailEnd = parameters.sampleEnd * nearestThumbnail.width();
	const auto nearestThumbnailRange = nearestThumbnailEnd - nearestThumbnailStart;

	painter.save();
	painter.setRenderHint(QPainter::Antialiasing, true);

	for (double out = 0; out < parameters.rect.width(); ++out)
	{
		// The peaks under this pixel: at least one (when zoomed in further than the thumbnail's resolution, a peak
		// covers several pixels)
		const auto width = static_cast<std::ptrdiff_t>(nearestThumbnail.width());
		const auto peakStart = std::clamp(static_cast<std::ptrdiff_t>(
			nearestThumbnailStart + (out / parameters.rect.width()) * nearestThumbnailRange), std::ptrdiff_t{0}, width - 1);
		const auto peakEnd = std::clamp(static_cast<std::ptrdiff_t>(
			nearestThumbnailStart + ((out + 1) / parameters.rect.width()) * nearestThumbnailRange), peakStart + 1, width);

		const auto peak = parameters.reversed
			? std::accumulate(nearestThumbnail.rbegin() + peakStart, nearestThumbnail.rbegin() + peakEnd,
				  SampleThumbnailCache::Thumbnail::Peak{})
			: std::accumulate(nearestThumbnail.begin() + peakStart, nearestThumbnail.begin() + peakEnd,
				  SampleThumbnailCache::Thumbnail::Peak{});

		const auto yMin = yCenter - peak.min * yScale;
		const auto yMax = yCenter - peak.max * yScale;
		const auto x = parameters.rect.x() + out;
		painter.drawLine(x, yMin, x, yMax);
	}

	painter.restore();
}

} // namespace lmms::gui
