/*
 * TripleBufferTest.cpp
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

#include <array>
#include <atomic>
#include <thread>

#include "TripleBuffer.h"

class TripleBufferTest : public QObject
{
	Q_OBJECT

private slots:
	void nothingPublishedNothingTaken()
	{
		auto buffer = lmms::TripleBuffer<int>{7};
		QVERIFY(!buffer.update());
		QCOMPARE(buffer.readSlot(), 7);
	}

	void readerTakesTheLatest()
	{
		auto buffer = lmms::TripleBuffer<int>{0};
		for (int value = 1; value <= 3; ++value)
		{
			buffer.writeSlot() = value;
			buffer.publish();
		}
		QVERIFY(buffer.update());
		QCOMPARE(buffer.readSlot(), 3);
		QVERIFY(!buffer.update()); // taken once
		QCOMPARE(buffer.readSlot(), 3);

		buffer.writeSlot() = 4;
		buffer.publish();
		QVERIFY(buffer.update());
		QCOMPARE(buffer.readSlot(), 4);
	}

	void writerNeverWritesWhatTheReaderHolds()
	{
		auto buffer = lmms::TripleBuffer<int>{0};
		buffer.writeSlot() = 1;
		buffer.publish();
		QVERIFY(buffer.update());
		const int* held = &buffer.readSlot();
		for (int value = 2; value < 10; ++value)
		{
			QVERIFY(&buffer.writeSlot() != held);
			buffer.writeSlot() = value;
			buffer.publish();
		}
		QCOMPARE(*held, 1);
	}

	void valuesArriveWholeAcrossThreads()
	{
		// Each value fills a whole array; a torn read would mix two values
		using Block = std::array<int, 256>;
		auto buffer = lmms::TripleBuffer<Block>{Block{}};
		constexpr int Count = 200000;
		auto done = std::atomic<bool>{false};

		auto writer = std::thread{[&] {
			for (int value = 1; value <= Count; ++value)
			{
				buffer.writeSlot().fill(value);
				buffer.publish();
			}
			done = true;
		}};

		int last = 0;
		bool torn = false, backwards = false;
		while (true)
		{
			// Every value published before the writer said it was done is taken by the update() after
			const bool finished = done;
			if (!buffer.update())
			{
				if (finished) { break; }
				continue;
			}
			const auto& block = buffer.readSlot();
			const int value = block.front();
			for (const int sample : block) { torn = torn || sample != value; }
			backwards = backwards || value < last;
			last = value;
		}
		writer.join();
		QVERIFY(!torn);
		QVERIFY(!backwards);
		QCOMPARE(buffer.readSlot().front(), Count);
	}
};

QTEST_GUILESS_MAIN(TripleBufferTest)
#include "TripleBufferTest.moc"
