/*
 * SampleThumbnailRendererTest.cpp
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

#include <cmath>
#include <limits>
#include <vector>

#include <QImage>
#include <QPainter>
#include <QtTest>

#include "SampleBuffer.h"
#include "SampleFrame.h"
#include "SampleThumbnailCache.h"
#include "SampleThumbnailRenderer.h"

using namespace lmms;
using namespace lmms::gui;

/**
 * Drawing a waveform takes whatever it is given (upstream's #8647: opening the AudioFileProcessor crashed in it):
 * samples of no, one and a few frames, ranges outside the sample, empty, reversed or not numbers, rectangles of a
 * pixel or very wide. It draws inside its rectangle, or nothing.
 */
class SampleThumbnailRendererTest : public QObject
{
	Q_OBJECT

	static auto bufferOf(int frames) -> SampleBuffer
	{
		auto data = std::vector<SampleFrame>(static_cast<std::size_t>(frames));
		for (int i = 0; i < frames; ++i)
		{
			const auto value = static_cast<float>(std::sin(i * 0.05));
			data[static_cast<std::size_t>(i)] = SampleFrame{value, value};
		}
		return SampleBuffer{std::move(data), 44100};
	}

	//! How many pixels were drawn on
	static auto draw(const SampleThumbnailCache& cache, double start, double end, int width, bool reversed = false)
		-> int
	{
		auto image = QImage{std::max(width, 1), 40, QImage::Format_ARGB32};
		image.fill(Qt::transparent);
		{
			auto painter = QPainter{&image};
			painter.setPen(Qt::white);
			SampleThumbnailRenderer::visualize({.cache = &cache, .rect = QRectF{0, 0, static_cast<double>(width), 40},
				.sampleStart = start, .sampleEnd = end, .reversed = reversed}, painter);
		}
		auto drawn = 0;
		for (int y = 0; y < image.height(); ++y)
		{
			for (int x = 0; x < image.width(); ++x) { drawn += qAlpha(image.pixel(x, y)) > 0; }
		}
		return drawn;
	}

private slots:
	void anythingItIsGiven()
	{
		const auto nan = std::numeric_limits<double>::quiet_NaN();
		const auto infinity = std::numeric_limits<double>::infinity();
		for (const int frames : {0, 1, 2, 3, 5, 1000})
		{
			const auto cache = SampleThumbnailCache{bufferOf(frames)};
			for (const int width : {0, 1, 7, 300, 20000})
			{
				for (const auto& [start, end] : std::vector<std::pair<double, double>>{{0, 1}, {0.25, 0.5}, {0.5, 0.5},
					{0.7, 0.2}, {-1, 2}, {0.9, 3}, {nan, 1}, {0, nan}, {0, infinity}, {1e-9, 2e-9}})
				{
					draw(cache, start, end, width);
					draw(cache, start, end, width, true);
				}
			}
		}
	}

	void aWaveformIsDrawn()
	{
		const auto cache = SampleThumbnailCache{bufferOf(1000)};
		QVERIFY(draw(cache, 0, 1, 300) > 300);
		// A range partly outside: what is inside
		QVERIFY(draw(cache, 0.5, 4, 300) > 0);
		// Nothing for an empty range or no sample
		QCOMPARE(draw(cache, 0.5, 0.5, 300), 0);
		QCOMPARE(draw(SampleThumbnailCache{bufferOf(0)}, 0, 1, 300), 0);
	}
};

QTEST_MAIN(SampleThumbnailRendererTest)
#include "SampleThumbnailRendererTest.moc"
