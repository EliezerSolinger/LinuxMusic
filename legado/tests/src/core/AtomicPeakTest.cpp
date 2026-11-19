/*
 * AtomicPeakTest.cpp
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

#include <QtTest>

#include <cmath>
#include <limits>
#include <thread>
#include <vector>

#include "AtomicPeak.h"

using namespace lmms;

class AtomicPeakTest : public QObject
{
	Q_OBJECT

private slots:
	void raisesOnlyUpwards()
	{
		auto peak = AtomicPeak{};
		peak.raise(0.5f);
		peak.raise(0.25f);
		QCOMPARE(peak.value(), 0.5f);
		peak.raise(std::numeric_limits<float>::quiet_NaN());
		QCOMPARE(peak.value(), 0.5f);
	}

	void takesAndResets()
	{
		auto peak = AtomicPeak{};
		peak.raise(0.75f);
		QCOMPARE(peak.take(), 0.75f);
		QCOMPARE(peak.value(), 0.f);
		peak.set(0.3f);
		QCOMPARE(peak.take(0.1f), 0.3f);
		QCOMPARE(peak.value(), 0.1f);
	}

	//! As a meter takes it: only a peak above what it shows, which then starts again
	void takesOnlyAboveAThreshold()
	{
		auto peak = AtomicPeak{};
		peak.set(0.4f);
		QVERIFY(!peak.takeAbove(0.5f).has_value());
		QCOMPARE(peak.value(), 0.4f);
		const auto taken = peak.takeAbove(0.3f);
		QVERIFY(taken.has_value());
		QCOMPARE(*taken, 0.4f);
		QCOMPARE(peak.value(), 0.f);
	}

	//! Threads raising at once while another takes: the highest of everything raised is never lost
	void theHighestSurvivesConcurrentRaisesAndTakes()
	{
		auto peak = AtomicPeak{};
		constexpr auto Threads = 4;
		constexpr auto Values = 20000;
		auto highestTaken = 0.f;
		auto raisers = std::vector<std::thread>{};
		auto done = std::atomic<int>{0};
		for (int t = 0; t < Threads; ++t)
		{
			raisers.emplace_back([&, t] {
				for (int v = 0; v < Values; ++v) { peak.raise(static_cast<float>(v * Threads + t)); }
				++done;
			});
		}
		while (done.load() < Threads) { highestTaken = std::max(highestTaken, peak.take()); }
		for (auto& raiser : raisers) { raiser.join(); }
		highestTaken = std::max(highestTaken, peak.take());
		QCOMPARE(highestTaken, static_cast<float>((Values - 1) * Threads + Threads - 1));
	}
};

QTEST_GUILESS_MAIN(AtomicPeakTest)
#include "AtomicPeakTest.moc"
