/*
 * ConvolverTest.cpp
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
#include <random>
#include <vector>

#include <QObject>
#include <QtTest>

#include "Convolver.h"

using namespace lmms;

class ConvolverTest : public QObject
{
	Q_OBJECT

private slots:
	void matchesDirectConvolutionOneBlockLate_data()
	{
		QTest::addColumn<int>("irLength");
		QTest::addColumn<int>("chunk");
		QTest::newRow("short IR, periods of 256") << 100 << 256;
		QTest::newRow("several partitions, odd periods") << 1000 << 97;
		QTest::newRow("long IR, small periods") << 5000 << 33;
	}

	void matchesDirectConvolutionOneBlockLate()
	{
		QFETCH(int, irLength);
		QFETCH(int, chunk);
		auto random = std::mt19937{7};
		auto noise = std::uniform_real_distribution<float>{-1.f, 1.f};
		auto ir = std::vector<float>(irLength);
		for (auto& x : ir) { x = noise(random) * std::exp(-3.f * (&x - ir.data()) / irLength); }
		auto input = std::vector<float>(8000);
		for (auto& x : input) { x = noise(random); }

		// Direct convolution, the reference
		auto expected = std::vector<float>(input.size(), 0.f);
		for (std::size_t n = 0; n < input.size(); ++n)
		{
			double sum = 0.0;
			for (std::size_t k = 0; k < ir.size() && k <= n; ++k) { sum += ir[k] * input[n - k]; }
			expected[n] = static_cast<float>(sum);
		}

		auto convolver = Convolver{ir};
		auto output = input;
		for (std::size_t at = 0; at < output.size(); at += chunk)
		{
			const auto frames = std::min<std::size_t>(chunk, output.size() - at);
			convolver.process(output.data() + at, static_cast<int>(frames));
		}
		// A block late: the output's frame n is the convolution's n - BlockSize
		auto worst = 0.f;
		for (std::size_t n = Convolver::BlockSize; n < output.size(); ++n)
		{
			worst = std::max(worst, std::abs(output[n] - expected[n - Convolver::BlockSize]));
		}
		for (int n = 0; n < Convolver::BlockSize; ++n) { QCOMPARE(output[n], 0.f); }
		QVERIFY2(worst < 1e-3f, qPrintable(QString::number(worst)));
		QCOMPARE(convolver.partitions(), (irLength + Convolver::BlockSize - 1) / Convolver::BlockSize);
	}
};

QTEST_GUILESS_MAIN(ConvolverTest)
#include "ConvolverTest.moc"
