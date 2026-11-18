/*
 * NoteOffRoutingTest.cpp - a note's end is sent where its start went, though the routing changed between (#8030)
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

#include <vector>

#include "AudioDummy.h"
#include "AudioEngine.h"
#include "ConfigManager.h"
#include "Engine.h"
#include "MidiClient.h"
#include "MidiEventProcessor.h"
#include "MidiPort.h"

using namespace lmms;

namespace
{

//! A client that keeps what the ports send
class Recording : public MidiClient
{
public:
	void processOutEvent(const MidiEvent& event, const TimePos&, const MidiPort*) override { sent.push_back(event); }
	std::vector<MidiEvent> sent;
};

class NoProcessor : public MidiEventProcessor
{
public:
	void processInEvent(const MidiEvent&, const TimePos&, f_cnt_t) override {}
	void processOutEvent(const MidiEvent&, const TimePos&, f_cnt_t) override {}
};

} // namespace


class NoteOffRoutingTest : public QObject
{
	Q_OBJECT

private slots:
	void initTestCase()
	{
		ConfigManager::inst()->setValue("audioengine", "audiodev", AudioDummy::name());
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase() { Engine::destroy(); }

	//! Sent out: the output channel changed while a note was held (its end was not sent: the channel did not match)
	void outChannelChangedMidNote()
	{
		auto client = Recording{};
		auto processor = NoProcessor{};
		auto port = MidiPort{"out", &client, &processor, nullptr, MidiPort::Mode::Output};
		port.setOutputChannel(1);
		port.processOutEvent(MidiEvent{MidiNoteOn, 0, 60, 100});
		port.setOutputChannel(3);
		port.processOutEvent(MidiEvent{MidiNoteOff, 0, 60, 0});
		QCOMPARE(client.sent.size(), std::size_t{2});
		QCOMPARE(client.sent[1].type(), MidiNoteOff);
		QCOMPARE(client.sent[1].key(), 60);
		QCOMPARE(client.sent[1].channel(), 0);
		// (and a note off for a note never sent goes as before: here, not at all)
		port.processOutEvent(MidiEvent{MidiNoteOff, 0, 61, 0});
		QCOMPARE(client.sent.size(), std::size_t{2});
	}

	//! Sent out: the fixed note changed while a note was held (its end went to the new key)
	void fixedNoteChangedMidNote()
	{
		auto client = Recording{};
		auto processor = NoProcessor{};
		auto port = MidiPort{"out", &client, &processor, nullptr, MidiPort::Mode::Output};
		port.setFixedOutputNote(70);
		port.processOutEvent(MidiEvent{MidiNoteOn, 0, 60, 100});
		port.setFixedOutputNote(72);
		port.processOutEvent(MidiEvent{MidiNoteOff, 0, 60, 0});
		QCOMPARE(client.sent.size(), std::size_t{2});
		QCOMPARE(client.sent[0].key(), 70);
		QCOMPARE(client.sent[1].type(), MidiNoteOff);
		QCOMPARE(client.sent[1].key(), 70);
	}
};

QTEST_GUILESS_MAIN(NoteOffRoutingTest)
#include "NoteOffRoutingTest.moc"
