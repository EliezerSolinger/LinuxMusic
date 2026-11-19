/*
 * MidiAppleTest.cpp
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

#include "lmmsconfig.h"

#ifdef LMMS_BUILD_APPLE

#include <array>
#include <chrono>
#include <mutex>
#include <thread>
#include <vector>

#include <CoreMIDI/CoreMIDI.h>
#include <unistd.h>

#include "AudioEngine.h"
#include "Engine.h"
#include "MidiApple.h"
#include "MidiEventProcessor.h"
#include "MidiPort.h"

using namespace lmms;
using namespace std::chrono_literals;

/**
 * The macOS MIDI client, against MIDI endpoints of the test's own (virtual, as another program publishes them):
 * plugged in and out while it runs, a subscription kept across that, the events received (with their source's name,
 * for learning a controller) and the bytes sent, from another thread than the main one.
 */
class MidiAppleTest : public QObject
{
	Q_OBJECT

	//! Collects what a port receives (on CoreMIDI's thread)
	struct Collector : MidiEventProcessor
	{
		void processInEvent(const MidiEvent& event, const TimePos&, f_cnt_t) override
		{
			const auto lock = std::lock_guard{mutex};
			events.push_back(event);
		}
		void processOutEvent(const MidiEvent&, const TimePos&, f_cnt_t) override {}
		auto taken() -> std::vector<MidiEvent>
		{
			const auto lock = std::lock_guard{mutex};
			return std::exchange(events, {});
		}
		std::mutex mutex;
		std::vector<MidiEvent> events;
	};

	//! What a destination of the test receives
	struct Received
	{
		std::mutex mutex;
		std::vector<std::uint8_t> bytes;
	};

	static void destinationRead(const MIDIPacketList* packets, void* received, void*)
	{
		auto* r = static_cast<Received*>(received);
		const auto lock = std::lock_guard{r->mutex};
		const auto* packet = &packets->packet[0];
		for (UInt32 i = 0; i < packets->numPackets; ++i)
		{
			r->bytes.insert(r->bytes.end(), packet->data, packet->data + packet->length);
			packet = MIDIPacketNext(packet);
		}
	}

	//! Lets CoreMIDI tell of changes (on this thread's run loop) and the client look again (a queued call)
	static void pump(std::chrono::milliseconds time = 300ms)
	{
		const auto until = std::chrono::steady_clock::now() + time;
		while (std::chrono::steady_clock::now() < until)
		{
			CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.02, false);
			QCoreApplication::processEvents();
		}
	}

	static void send(MIDIEndpointRef source, std::initializer_list<std::uint8_t> bytes)
	{
		auto buffer = std::array<Byte, 256>{};
		auto* list = reinterpret_cast<MIDIPacketList*>(buffer.data());
		auto* packet = MIDIPacketListInit(list);
		const auto data = std::vector<std::uint8_t>(bytes);
		MIDIPacketListAdd(list, buffer.size(), packet, 0, data.size(), data.data());
		MIDIReceived(source, list);
	}

	//! The events @p client received, handed to its ports, as the audio thread does with the engine's client (this
	//! client is the test's own: see MidiClient)
	static void deliver(MidiClient& client)
	{
		const auto guard = AudioEngine::changesGuard();
		client.deliverInEvents();
	}

	template<class Done>
	static auto waitFor(Done done) -> bool
	{
		for (int i = 0; i < 100 && !done(); ++i) { pump(20ms); }
		return done();
	}

	MIDIClientRef m_testClient = 0;
	QString m_tag = QString::number(getpid());

private slots:
	void initTestCase()
	{
		Engine::init(true);
		QCOMPARE(MIDIClientCreate(CFSTR("MidiAppleTest"), nullptr, nullptr, &m_testClient), noErr);
	}

	void cleanupTestCase()
	{
		MIDIClientDispose(m_testClient);
		Engine::destroy();
	}

	void aDevicePluggedInIsReceivedFrom()
	{
		auto client = MidiApple{};
		auto collector = Collector{};
		auto port = MidiPort{"test in", &client, &collector, nullptr, MidiPort::Mode::Input};
		const auto name = QString{"LinuxMusic test source %1"}.arg(m_tag);

		// Subscribed before the device is there (as a project saved with it, opened before it is plugged in)
		client.subscribeReadablePort(&port, name, true);

		auto source = MIDIEndpointRef{0};
		const auto cfName = name.toCFString();
		QCOMPARE(MIDISourceCreate(m_testClient, cfName, &source), noErr);
		CFRelease(cfName);
		QVERIFY2(waitFor([&] { return client.readablePorts().contains(name); }), "the source plugged in was not seen");

		send(source, {0x90, 60, 100, 62, 90, 0xF8, 0xC1, 5, 0xE0, 0x00, 0x40});
		QVERIFY(waitFor([&] {
			deliver(client);
			const auto lock = std::lock_guard{collector.mutex};
			return collector.events.size() >= 5;
		}));
		const auto events = collector.taken();
		QCOMPARE(events.size(), std::size_t{5});
		QCOMPARE(events[0].key(), 60);
		QCOMPARE(events[1].key(), 62); // (running status)
		QCOMPARE(events[2].type(), MidiSync);
		QCOMPARE(events[3].program(), 5);
		QCOMPARE(events[4].pitchBend(), 8192);
		QCOMPARE(client.sourcePortName(events[0]), name); // (learning a controller)

		// Unplugged: gone from the list; plugged in again: received from, the subscription kept
		MIDIEndpointDispose(source);
		QVERIFY2(waitFor([&] { return !client.readablePorts().contains(name); }), "the source unplugged stayed");
		const auto again = name.toCFString();
		QCOMPARE(MIDISourceCreate(m_testClient, again, &source), noErr);
		CFRelease(again);
		QVERIFY(waitFor([&] { return client.readablePorts().contains(name); }));
		send(source, {0x80, 60, 0});
		QVERIFY2(waitFor([&] {
			deliver(client);
			const auto lock = std::lock_guard{collector.mutex};
			return !collector.events.empty();
		}),
			"plugged in again, it was not received from");
		QCOMPARE(collector.taken().front().type(), MidiNoteOff);
		MIDIEndpointDispose(source);
	}

	void eventsAreSentToADestination()
	{
		auto client = MidiApple{};
		auto collector = Collector{};
		auto port = MidiPort{"test out", &client, &collector, nullptr, MidiPort::Mode::Output};
		auto received = Received{};
		{
			// (made here, read on CoreMIDI's thread: handed over under its mutex, for TSan to see it, as CoreMIDI
			// does not show the hand-over to it)
			const auto lock = std::lock_guard{received.mutex};
			received.bytes.reserve(64);
		}
		const auto name = QString{"LinuxMusic test destination %1"}.arg(m_tag);
		auto destination = MIDIEndpointRef{0};
		const auto cfName = name.toCFString();
		QCOMPARE(MIDIDestinationCreate(m_testClient, cfName, &MidiAppleTest::destinationRead, &received, &destination), noErr);
		CFRelease(cfName);
		QVERIFY2(waitFor([&] { return client.writablePorts().contains(name); }), "the destination was not seen");
		client.subscribeWritablePort(&port, name, true);

		// From another thread than the main one (as the audio thread sends)
		auto sender = std::thread{[&] {
			client.processOutEvent(MidiEvent{MidiNoteOn, 4, 64, 99}, TimePos{}, &port);
			client.processOutEvent(MidiEvent{MidiProgramChange, 4, 12, 0}, TimePos{}, &port);
			client.processOutEvent(MidiEvent{MidiPitchBend, 4, 0x3FFF, 0}, TimePos{}, &port);
			client.processOutEvent(MidiEvent{MidiStart}, TimePos{}, &port);
		}};
		sender.join();
		const auto expected = std::vector<std::uint8_t>{0x94, 64, 99, 0xC4, 12, 0xE4, 0x7F, 0x7F, 0xFA};
		QVERIFY(waitFor([&] { const auto lock = std::lock_guard{received.mutex}; return received.bytes.size() >= expected.size(); }));
		const auto lock = std::lock_guard{received.mutex};
		QCOMPARE(received.bytes, expected);
		MIDIEndpointDispose(destination);
	}
};

QTEST_GUILESS_MAIN(MidiAppleTest)
#include "MidiAppleTest.moc"

#else

class MidiAppleTest : public QObject
{
	Q_OBJECT
private slots:
	void notOnThisSystem() { QSKIP("CoreMIDI is macOS's"); }
};

QTEST_GUILESS_MAIN(MidiAppleTest)
#include "MidiAppleTest.moc"

#endif
