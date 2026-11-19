/*
 * MidiByteParserTest.cpp
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

#include <array>
#include <initializer_list>
#include <vector>

#include <QtTest>

#include "MidiByteParser.h"
#include "MidiEventToByteSeq.h"

using namespace lmms;

//! The MIDI byte stream's parser (MidiByteParser, for the raw, JACK and CoreMIDI drivers) and its encoder
class MidiByteParserTest : public QObject
{
	Q_OBJECT

	static auto parse(std::initializer_list<int> bytes) -> std::vector<MidiEvent>
	{
		auto parser = MidiByteParser{};
		auto events = std::vector<MidiEvent>{};
		for (const auto byte : bytes)
		{
			parser.feed(static_cast<std::uint8_t>(byte), nullptr, [&](const MidiEvent& e) { events.push_back(e); });
		}
		return events;
	}

private slots:
	void channelMessages()
	{
		const auto events = parse({0x92, 60, 100, 0xC3, 7, 0xD1, 99, 0xE0, 0x00, 0x40, 0xB5, 74, 127, 0xA0, 61, 5});
		QCOMPARE(events.size(), std::size_t{6});
		QCOMPARE(events[0].type(), MidiNoteOn);
		QCOMPARE(events[0].channel(), 2);
		QCOMPARE(events[0].key(), 60);
		QCOMPARE(events[0].velocity(), 100);
		QCOMPARE(events[1].type(), MidiProgramChange); // (two bytes: the next status is not eaten as its data)
		QCOMPARE(events[1].program(), 7);
		QCOMPARE(events[2].type(), MidiChannelPressure);
		QCOMPARE(events[2].channelPressure(), 99);
		QCOMPARE(events[3].type(), MidiPitchBend);
		QCOMPARE(events[3].pitchBend(), 8192); // the middle: LSB 0, MSB 64
		QCOMPARE(events[4].controllerNumber(), 74);
		QCOMPARE(events[4].controllerValue(), 127);
		QCOMPARE(events[5].type(), MidiKeyPressure);
	}

	void runningStatus()
	{
		const auto events = parse({0x90, 60, 100, 62, 90, 64, 0}); // three note-ons, one status
		QCOMPARE(events.size(), std::size_t{3});
		QCOMPARE(events[1].key(), 62);
		QCOMPARE(events[2].velocity(), 0);
	}

	//! Real-time bytes anywhere, even inside a message, without breaking it or the running status
	void realTimeInsideAMessage()
	{
		const auto events = parse({0x90, 0xF8, 60, 0xFA, 100, 0xFE, 62, 90, 0xFC, 0xFB});
		QCOMPARE(events.size(), std::size_t{6});
		QCOMPARE(events[0].type(), MidiSync);
		QCOMPARE(events[1].type(), MidiStart);
		QCOMPARE(events[2].type(), MidiNoteOn);
		QCOMPARE(events[2].key(), 60);
		QCOMPARE(events[3].key(), 62); // (active sensing dropped, the running status kept)
		QCOMPARE(events[4].type(), MidiStop);
		QCOMPARE(events[5].type(), MidiContinue);
	}

	void songPositionAndSystemCommon()
	{
		const auto events = parse({0xF2, 0x10, 0x02, 0x90, 60, 100, 0xF6, 61, 100});
		QCOMPARE(events.size(), std::size_t{2});
		QCOMPARE(events[0].type(), MidiSongPosition);
		QCOMPARE(events[0].param(0), 0x10 + 2 * 128);
		QCOMPARE(events[1].key(), 60); // (the tune request cancelled the running status: 61 100 dropped)
	}

	void sysExSkippedAndJoiningMidStream()
	{
		const auto events = parse({60, 100, 0xF0, 0x7E, 0x00, 0x90, 0xF7, 0x91, 64, 1});
		QCOMPARE(events.size(), std::size_t{1}); // (data without a status, and the SysEx with what looks like status)
		QCOMPARE(events[0].channel(), 1);
		QCOMPARE(events[0].key(), 64);
	}

	//! What the encoder writes, the parser reads back the same
	void encodeAndParseBack()
	{
		const auto originals = std::vector<MidiEvent>{
			MidiEvent{MidiNoteOn, 3, 60, 100}, MidiEvent{MidiNoteOff, 3, 60, 0}, MidiEvent{MidiControlChange, 0, 7, 99},
			MidiEvent{MidiProgramChange, 9, 42, 0}, MidiEvent{MidiChannelPressure, 1, 77, 0},
			MidiEvent{MidiPitchBend, 2, 12345, 0}, MidiEvent{MidiSongPosition, 0, 1000, 0}, MidiEvent{MidiSync},
			MidiEvent{MidiStart}, MidiEvent{MidiContinue}, MidiEvent{MidiStop}};
		auto parser = MidiByteParser{};
		auto back = std::vector<MidiEvent>{};
		for (const auto& event : originals)
		{
			auto bytes = std::array<std::uint8_t, 3>{};
			const auto size = writeToByteSeq(event, bytes.data(), bytes.size());
			QVERIFY(size > 0);
			for (std::size_t i = 0; i < size; ++i)
			{
				parser.feed(bytes[i], nullptr, [&](const MidiEvent& e) { back.push_back(e); });
			}
		}
		QCOMPARE(back.size(), originals.size());
		for (std::size_t i = 0; i < back.size(); ++i)
		{
			QCOMPARE(back[i].type(), originals[i].type());
			QCOMPARE(back[i].param(0), originals[i].param(0));
			if (originals[i].type() < MidiSysEx)
			{
				QCOMPARE(back[i].channel(), originals[i].channel());
				QCOMPARE(back[i].param(1), originals[i].param(1));
			}
		}
	}
};

QTEST_GUILESS_MAIN(MidiByteParserTest)
#include "MidiByteParserTest.moc"
