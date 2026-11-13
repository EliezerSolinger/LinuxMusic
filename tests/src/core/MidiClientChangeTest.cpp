/*
 * MidiClientChangeTest.cpp
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
#include <QSignalSpy>
#include <QtTest>

#include <algorithm>
#include <set>

#include "AudioEngine.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "MidiClient.h"
#include "MidiEventProcessor.h"
#include "MidiPort.h"
#include "Song.h"

using namespace lmms;

namespace
{

//! A sequencing client with the devices it is given, which keeps the connections made
class FakeSequencer : public MidiClient
{
public:
	explicit FakeSequencer(QStringList devices) : m_devices{std::move(devices)} {}

	void processOutEvent(const MidiEvent&, const TimePos&, const MidiPort*) override {}
	auto readablePorts() const -> QStringList override { return m_devices; }
	auto writablePorts() const -> QStringList override { return m_devices; }
	void subscribeReadablePort(MidiPort*, const QString& device, bool subscribe) override
	{
		if (subscribe) { readable.insert(device); }
		else { readable.erase(device); }
	}
	void subscribeWritablePort(MidiPort*, const QString& device, bool subscribe) override
	{
		if (subscribe) { writable.insert(device); }
		else { writable.erase(device); }
	}
	auto has(const MidiPort* port) const -> bool
	{
		return std::find(m_midiPorts.begin(), m_midiPorts.end(), port) != m_midiPorts.end();
	}

	std::set<QString> readable;
	std::set<QString> writable;

private:
	QStringList m_devices;
};

class NoProcessor : public MidiEventProcessor
{
public:
	void processInEvent(const MidiEvent&, const TimePos&, f_cnt_t) override {}
	void processOutEvent(const MidiEvent&, const TimePos&, f_cnt_t) override {}
};

} // namespace

class MidiClientChangeTest : public QObject
{
	Q_OBJECT

private slots:
	void initTestCase()
	{
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	//! A port moved to another client leaves the first one, and stays connected to the devices both have
	void aPortKeepsTheDevicesTheNewClientHas()
	{
		auto first = FakeSequencer{{"keyboard", "pads", "synth"}};
		auto second = FakeSequencer{{"keyboard", "synth"}};
		auto processor = NoProcessor{};
		{
			auto port = MidiPort{"track", &first, &processor, nullptr, MidiPort::Mode::Duplex};
			port.subscribeReadablePort("keyboard");
			port.subscribeReadablePort("pads");
			port.subscribeWritablePort("synth");
			QCOMPARE(first.readable, (std::set<QString>{"keyboard", "pads"}));

			auto changed = QSignalSpy{&port, &MidiPort::midiClientChanged};
			port.setMidiClient(&second);
			QCOMPARE(changed.count(), 1);
			QCOMPARE(port.midiClient(), &second);
			QVERIFY(!first.has(&port));
			QVERIFY(second.has(&port));
			QVERIFY(first.readable.empty());
			QVERIFY(first.writable.empty());
			QCOMPARE(second.readable, std::set<QString>{"keyboard"});
			QCOMPARE(second.writable, std::set<QString>{"synth"});
			QCOMPARE(port.subscribedReadablePorts(), QStringList{"keyboard"});
			QCOMPARE(port.readablePorts().keys(), (QList<QString>{"keyboard", "synth"}));
			QVERIFY(port.isReadable());
			QVERIFY(port.isWritable());
		}
		// (gone, it left the client it was on)
		QVERIFY(second.ports().empty());
	}

	//! The engine opens the MIDI client again: the tracks' ports move to it, as they were
	void theEngineMovesTheTracksPorts()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QVERIFY(track);
		auto* engine = Engine::audioEngine();
		auto* before = engine->midiClient();
		QVERIFY(std::find(before->ports().begin(), before->ports().end(), track->midiPort()) != before->ports().end());
		track->midiPort()->setReadable(true);
		track->midiPort()->setInputChannel(3);

		QCOMPARE(engine->reopenMidiClient(), QString{});
		auto* after = engine->midiClient();
		QCOMPARE(track->midiPort()->midiClient(), after);
		QVERIFY(std::find(after->ports().begin(), after->ports().end(), track->midiPort()) != after->ports().end());
		QVERIFY(track->midiPort()->isReadable());
		QCOMPARE(track->midiPort()->inputChannel(), 3);

		// A track made after it is on the new client
		auto* another = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QCOMPARE(another->midiPort()->midiClient(), after);
	}
};

QTEST_GUILESS_MAIN(MidiClientChangeTest)
#include "MidiClientChangeTest.moc"
