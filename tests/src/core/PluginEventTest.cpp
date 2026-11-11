/*
 * PluginEventTest.cpp - tests for plugin events and the lock-free queue that carries them
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

#include <set>
#include <thread>
#include <vector>

#include "LockFreeQueue.h"
#include "MidiEvent.h"
#include "PluginEvent.h"

using namespace lmms;

class PluginEventTest : public QObject
{
	Q_OBJECT

private slots:
	void Queue_FifoAndCapacity()
	{
		auto queue = LockFreeQueue<int, 4>{};
		QVERIFY(!queue.tryPop());
		for (int i = 0; i < 4; ++i) { QVERIFY(queue.tryPush(i)); }
		QVERIFY(!queue.tryPush(4)); // full
		for (int i = 0; i < 4; ++i) { QCOMPARE(*queue.tryPop(), i); }
		QVERIFY(!queue.tryPop());

		// Wraps around
		for (int round = 0; round < 10; ++round)
		{
			QVERIFY(queue.tryPush(round));
			QCOMPARE(*queue.tryPop(), round);
		}
	}

	void Queue_ManyProducersLoseNothing()
	{
		constexpr int Producers = 4;
		constexpr int PerProducer = 20000;
		auto queue = LockFreeQueue<int, 1024>{};

		auto producers = std::vector<std::thread>{};
		for (int p = 0; p < Producers; ++p)
		{
			producers.emplace_back([&queue, p] {
				for (int i = 0; i < PerProducer; ++i)
				{
					while (!queue.tryPush(p * PerProducer + i)) { std::this_thread::yield(); }
				}
			});
		}

		// Each producer's values arrive in its order, and all of them arrive
		auto last = std::vector<int>(Producers, -1);
		auto received = 0;
		while (received < Producers * PerProducer)
		{
			const auto value = queue.tryPop();
			if (!value) { std::this_thread::yield(); continue; }
			const int producer = *value / PerProducer;
			QVERIFY(*value > last[producer]);
			last[producer] = *value;
			++received;
		}
		for (auto& thread : producers) { thread.join(); }
		QVERIFY(!queue.tryPop());
	}

	void Event_FromMidiNotes()
	{
		auto on = PluginEvent::fromMidi(MidiEvent{MidiNoteOn, 3, 60, 127}, 17);
		QVERIFY(on);
		QCOMPARE(on->type, PluginEvent::Type::NoteOn);
		QCOMPARE(on->time, 17);
		QCOMPARE(on->channel, 3);
		QCOMPARE(on->key, 60);
		QCOMPARE(on->value, 1.0);

		// A note-on without velocity ends the note
		auto off = PluginEvent::fromMidi(MidiEvent{MidiNoteOn, 3, 60, 0}, 0);
		QCOMPARE(off->type, PluginEvent::Type::NoteOff);
	}

	void Event_MidiRoundTrip()
	{
		for (const auto& midi : {MidiEvent{MidiNoteOn, 1, 64, 100}, MidiEvent{MidiNoteOff, 2, 64, 0},
			MidiEvent{MidiControlChange, 5, 7, 99}, MidiEvent{MidiProgramChange, 0, 12, 0},
			MidiEvent{MidiChannelPressure, 9, 55, 0}, MidiEvent{MidiKeyPressure, 4, 60, 70}})
		{
			const auto event = PluginEvent::fromMidi(midi, 0);
			QVERIFY(event);
			const auto back = event->toMidi();
			QVERIFY(back);
			QCOMPARE(back->type(), midi.type());
			QCOMPARE(back->channel(), midi.channel());
			QCOMPARE(back->param(0), midi.param(0));
			if (midi.type() != MidiProgramChange && midi.type() != MidiChannelPressure)
			{
				QCOMPARE(back->param(1), midi.param(1));
			}
		}

		auto bend = MidiEvent{MidiPitchBend, 0};
		bend.setPitchBend(12345);
		QCOMPARE(PluginEvent::fromMidi(bend, 0)->toMidi()->pitchBend(), 12345);
	}

	void Event_SystemMessagesAreNotEvents()
	{
		QVERIFY(!PluginEvent::fromMidi(MidiEvent{MidiStart}, 0));
	}

	void Event_ExpressionsHaveNoMidi()
	{
		auto event = PluginEvent{};
		event.type = PluginEvent::Type::NoteExpression;
		QVERIFY(!event.toMidi());
	}

	void OutputEvents_StopAtCapacity()
	{
		auto storage = std::array<PluginEvent, 2>{};
		auto out = OutputEvents{storage.data(), storage.size()};
		QVERIFY(out.tryPush(PluginEvent::noteOn(0, 0, 60, 1.0)));
		QVERIFY(out.tryPush(PluginEvent::noteOn(1, 0, 61, 1.0)));
		QVERIFY(!out.tryPush(PluginEvent::noteOn(2, 0, 62, 1.0)));
		QCOMPARE(out.events().size(), std::size_t{2});
		out.clear();
		QVERIFY(out.events().empty());
	}
};

QTEST_GUILESS_MAIN(PluginEventTest)
#include "PluginEventTest.moc"
