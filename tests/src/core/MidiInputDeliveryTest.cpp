/*
 * MidiInputDeliveryTest.cpp - the MIDI received goes to the ports by the audio thread, never to a port deleted
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

#include <atomic>
#include <chrono>
#include <memory>
#include <mutex>
#include <random>
#include <thread>
#include <vector>

#include "AudioEngine.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "MidiClient.h"
#include "MidiEventProcessor.h"
#include "MidiPort.h"
#include "NotePlayHandle.h"
#include "Song.h"

using namespace lmms;

namespace
{

//! A port's owner that counts the events it is given, and whether they came on the thread rendering a period
class Recorder : public MidiEventProcessor
{
public:
	Recorder() : port{"recorder", Engine::audioEngine()->midiClient(), this, nullptr, MidiPort::Mode::Input} {}
	~Recorder() override { port.closeInput(); }

	void processInEvent(const MidiEvent&, const TimePos&, f_cnt_t) override
	{
		received.fetch_add(1, std::memory_order_relaxed);
		if (!AudioEngine::isRenderingThread()) { elsewhere.fetch_add(1, std::memory_order_relaxed); }
	}
	void processOutEvent(const MidiEvent&, const TimePos&, f_cnt_t) override {}

	std::atomic<int> received = 0;
	std::atomic<int> elsewhere = 0;
	MidiPort port; //!< (last: it goes first)
};

auto noteOn(int key = 60) -> MidiEvent { return MidiEvent{MidiNoteOn, 0, static_cast<int16_t>(key), 100}; }

auto client() -> MidiClient* { return Engine::audioEngine()->midiClient(); }

void renderPeriod() { Engine::audioEngine()->renderNextPeriod(); }

} // namespace


class MidiInputDeliveryTest : public QObject
{
	Q_OBJECT

private slots:
	void initTestCase()
	{
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase() { Engine::destroy(); }

	void init()
	{
		// (nothing left from a test before)
		renderPeriod();
	}

	//! An event received waits for the period, and goes to its port on the thread that renders it
	void deliveredByTheAudioThread()
	{
		auto recorder = Recorder{};
		client()->queueInEvent(&recorder.port, noteOn());
		QCOMPARE(recorder.received.load(), 0);
		renderPeriod();
		QCOMPARE(recorder.received.load(), 1);
		QCOMPARE(recorder.elsewhere.load(), 0);
		renderPeriod();
		QCOMPARE(recorder.received.load(), 1);
	}

	//! A raw client's event goes to every port
	void aRawEventGoesToEveryPort()
	{
		auto first = Recorder{};
		auto second = Recorder{};
		client()->queueInEvent(nullptr, noteOn());
		renderPeriod();
		QCOMPARE(first.received.load(), 1);
		QCOMPARE(second.received.load(), 1);
	}

	//! The events for a port deleted before they are delivered go nowhere: not to it (deleted), nor to a port made
	//! since, even at its address
	void aDeletedPortGetsNothing()
	{
		auto deleted = std::make_unique<Recorder>();
		const auto* address = &deleted->port;
		client()->queueInEvent(&deleted->port, noteOn());
		deleted.reset();
		auto made = std::make_unique<Recorder>();
		if (&made->port != address) { qInfo("(the new port is at another address)"); }
		renderPeriod();
		QCOMPARE(made->received.load(), 0);
	}

	//! A port whose input is closed (its owner going) is given nothing more
	void aClosedInputGetsNothing()
	{
		auto recorder = Recorder{};
		client()->queueInEvent(&recorder.port, noteOn());
		recorder.port.closeInput();
		renderPeriod();
		QCOMPARE(recorder.received.load(), 0);
	}

	//! A note received plays on its track; a track deleted with notes on their way plays nothing (it is gone)
	void aTrackPlaysTheNotesReceivedUnlessDeleted()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QVERIFY(track);
		renderPeriod();
		const auto notesOf = [](const InstrumentTrack* owner) {
			auto count = 0;
			for (const auto* handle : Engine::audioEngine()->playHandles())
			{
				if (const auto* note = dynamic_cast<const NotePlayHandle*>(handle))
				{
					count += note->instrumentTrack() == owner ? 1 : 0;
				}
			}
			return count;
		};
		client()->queueInEvent(track->midiPort(), noteOn(60));
		renderPeriod();
		renderPeriod();
		QCOMPARE(notesOf(track), 1);
		client()->queueInEvent(track->midiPort(), MidiEvent{MidiNoteOff, 0, 60, 0});
		client()->queueInEvent(track->midiPort(), noteOn(64));
		Track::destroy(track);
		renderPeriod();
		renderPeriod();
		QCOMPARE(notesOf(track), 0);
	}

	/**
	 * A driver's thread receiving all the time, for ports its routing holds, while the main thread makes and deletes
	 * them and another thread renders: every event goes to a port alive (the sanitizer builds check the memory), on
	 * the rendering thread.
	 */
	void portsComeAndGoWhileReceiving()
	{
		auto routing = std::mutex{};
		auto ports = std::vector<std::unique_ptr<Recorder>>{};
		for (int i = 0; i < 8; ++i) { ports.push_back(std::make_unique<Recorder>()); }
		auto stop = std::atomic<bool>{false};
		auto queued = std::atomic<int>{0};

		// (as a driver: the port taken and its event queued under the routing's lock)
		auto receiver = std::thread{[&] {
			auto random = std::minstd_rand{7};
			while (!stop.load())
			{
				const auto lock = std::lock_guard{routing};
				if (ports.empty()) { continue; }
				client()->queueInEvent(&ports[random() % ports.size()]->port, noteOn(36 + random() % 48));
				queued.fetch_add(1);
			}
		}};
		auto renderer = std::thread{[&] {
			while (!stop.load()) { renderPeriod(); }
		}};

		auto random = std::minstd_rand{11};
		auto elsewhere = 0;
		const auto end = std::chrono::steady_clock::now() + std::chrono::milliseconds{600};
		while (std::chrono::steady_clock::now() < end)
		{
			// (out of the routing under its lock, then deleted: as a track's port)
			auto leaving = std::unique_ptr<Recorder>{};
			{
				const auto lock = std::lock_guard{routing};
				const auto index = random() % ports.size();
				leaving = std::move(ports[index]);
				ports[index] = std::make_unique<Recorder>();
			}
			elsewhere += leaving->elsewhere.load();
			leaving.reset();
		}
		stop = true;
		receiver.join();
		renderer.join();
		for (const auto& recorder : ports) { elsewhere += recorder->elsewhere.load(); }
		QVERIFY(queued.load() > 0);
		QCOMPARE(elsewhere, 0);
	}
};

QTEST_GUILESS_MAIN(MidiInputDeliveryTest)
#include "MidiInputDeliveryTest.moc"
