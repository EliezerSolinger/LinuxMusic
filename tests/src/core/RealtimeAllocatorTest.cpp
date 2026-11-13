/*
 * RealtimeAllocatorTest.cpp
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

#include <algorithm>
#include <cstdint>
#include <cstring>
#include <map>
#include <random>
#include <thread>
#include <vector>

#include "RealtimeAllocator.h"

using namespace lmms;

class RealtimeAllocatorTest : public QObject
{
	Q_OBJECT
private slots:
	void initTestCase()
	{
		QVERIFY(!RealtimeAllocator::allocate(16)); // nothing before the pool is made
		RealtimeAllocator::init(8 << 20);
		QVERIFY(RealtimeAllocator::isInitialized());
		QVERIFY(RealtimeAllocator::isWhole());
	}

	//! Random allocations and frees, each block filled with its own pattern: none overlaps another, each is aligned,
	//! and freeing everything leaves the pool whole (every block merged back)
	void randomUseKeepsBlocksApartAndMergesBack()
	{
		auto random = std::mt19937{1234};
		auto blocks = std::map<std::uint8_t*, std::pair<std::size_t, std::uint8_t>>{};
		std::uint8_t pattern = 0;
		const auto check = [&](std::uint8_t* memory, std::size_t size, std::uint8_t value) {
			return std::all_of(memory, memory + size, [value](std::uint8_t byte) { return byte == value; });
		};
		for (int step = 0; step < 200000; ++step)
		{
			const bool allocate = blocks.empty() || random() % 100 < 55;
			if (allocate)
			{
				// Mostly small, sometimes large
				const auto size = random() % 10 == 0 ? random() % 100000 : random() % 600;
				auto* memory = static_cast<std::uint8_t*>(RealtimeAllocator::allocate(size));
				if (!memory) { continue; } // the pool is full: a fallback
				QCOMPARE(reinterpret_cast<std::uintptr_t>(memory) % 16, std::uintptr_t{0});
				QVERIFY(RealtimeAllocator::owns(memory));
				++pattern;
				std::memset(memory, pattern, size);
				blocks[memory] = {size, pattern};
			}
			else
			{
				auto it = blocks.begin();
				std::advance(it, random() % blocks.size());
				QVERIFY2(check(it->first, it->second.first, it->second.second), "a block was written by another");
				RealtimeAllocator::release(it->first);
				blocks.erase(it);
			}
		}
		for (const auto& [memory, block] : blocks)
		{
			QVERIFY(check(memory, block.first, block.second));
			RealtimeAllocator::release(memory);
		}
		QVERIFY(RealtimeAllocator::isWhole());
		QCOMPARE(RealtimeAllocator::stats().usedBytes, std::size_t{0});
	}

	void alignedAllocations()
	{
		auto memories = std::vector<void*>{};
		for (const std::size_t alignment : {32, 64, 128, 4096})
		{
			auto* memory = RealtimeAllocator::allocateAligned(1000, alignment);
			QVERIFY(memory);
			QCOMPARE(reinterpret_cast<std::uintptr_t>(memory) % alignment, std::uintptr_t{0});
			QVERIFY(RealtimeAllocator::owns(memory));
			std::memset(memory, 0xab, 1000);
			memories.push_back(memory);
		}
		for (auto* memory : memories) { RealtimeAllocator::releaseAligned(memory); }
		QVERIFY(RealtimeAllocator::isWhole());
	}

	//! What the pool cannot hold is not given (the caller asks the system), and counted
	void tooLargeFallsBack()
	{
		const auto before = RealtimeAllocator::stats().fallbacks;
		QVERIFY(!RealtimeAllocator::allocate(64 << 20));
		QVERIFY(!RealtimeAllocator::allocate(~std::size_t{0}));
		QCOMPARE(RealtimeAllocator::stats().fallbacks, before + 2);
		int onTheStack = 0;
		QVERIFY(!RealtimeAllocator::owns(&onTheStack));
	}

	//! Several threads allocating and freeing at once, some freeing what others allocated
	void threadsAtOnce()
	{
		constexpr int Threads = 6;
		auto handedOver = std::vector<std::vector<void*>>(Threads);
		auto threads = std::vector<std::thread>{};
		for (int index = 0; index < Threads; ++index)
		{
			threads.emplace_back([index, &handedOver] {
				auto random = std::mt19937{static_cast<unsigned>(index)};
				auto mine = std::vector<void*>{};
				mine.reserve(256);
				for (int step = 0; step < 50000; ++step)
				{
					if (mine.size() < 200 && random() % 2)
					{
						if (auto* memory = RealtimeAllocator::allocate(random() % 2000))
						{
							static_cast<std::uint8_t*>(memory)[0] = static_cast<std::uint8_t>(index);
							mine.push_back(memory);
						}
					}
					else if (!mine.empty())
					{
						RealtimeAllocator::release(mine.back());
						mine.pop_back();
					}
				}
				handedOver[index] = std::move(mine);
			});
		}
		for (auto& thread : threads) { thread.join(); }
		// Freed by another thread than the one that allocated
		for (auto& memories : handedOver)
		{
			for (auto* memory : memories) { RealtimeAllocator::release(memory); }
		}
		QVERIFY(RealtimeAllocator::isWhole());
	}

	void deferredFrees()
	{
		auto* memory = std::malloc(100);
		QVERIFY(RealtimeAllocator::deferFree(memory));
		RealtimeAllocator::drainDeferred();
		QVERIFY(RealtimeAllocator::stats().deferredFrees >= 1);
	}
};

QTEST_GUILESS_MAIN(RealtimeAllocatorTest)
#include "RealtimeAllocatorTest.moc"
