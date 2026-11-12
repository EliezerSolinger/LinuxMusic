/*
 * SpscRingTest.cpp
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

#include <QObject>
#include <QtTest>

#include <atomic>
#include <thread>
#include <vector>

#include "SpscRing.h"

class SpscRingTest : public QObject
{
	Q_OBJECT

private slots:
	void writesWhatFitsAndReadsWhatIsThere()
	{
		auto ring = lmms::SpscRing<int>{5};
		QCOMPARE(ring.capacity(), std::size_t{8});
		const int values[] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10};
		QCOMPARE(ring.write(values, 10), std::size_t{8});
		QCOMPARE(ring.writeSpace(), std::size_t{0});
		int out[10] = {};
		QCOMPARE(ring.read(out, 3), std::size_t{3});
		QCOMPARE(out[2], 3);
		QCOMPARE(ring.write(nullptr, 2, -1), std::size_t{2}); // fill values, across the end of the storage
		QCOMPARE(ring.read(out, 10), std::size_t{7});
		QCOMPARE(out[0], 4);
		QCOMPARE(out[4], 8);
		QCOMPARE(out[5], -1);
		QCOMPARE(out[6], -1);
		QCOMPARE(ring.readSpace(), std::size_t{0});
	}

	void streamArrivesInOrderAcrossThreads()
	{
		auto ring = lmms::SpscRing<int>{1000};
		constexpr int Count = 2'000'000;
		auto writer = std::thread{[&] {
			int next = 0;
			int block[97];
			while (next < Count)
			{
				const int count = std::min(97, Count - next);
				for (int index = 0; index < count; ++index) { block[index] = next + index; }
				next += static_cast<int>(ring.write(block, count));
			}
		}};
		int expected = 0;
		bool ordered = true;
		int block[131];
		while (expected < Count)
		{
			const auto count = ring.read(block, 131);
			for (std::size_t index = 0; index < count; ++index) { ordered = ordered && block[index] == expected++; }
		}
		writer.join();
		QVERIFY(ordered);
	}
};

QTEST_GUILESS_MAIN(SpscRingTest)
#include "SpscRingTest.moc"
