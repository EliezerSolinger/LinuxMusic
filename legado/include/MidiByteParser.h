/*
 * MidiByteParser.h - MIDI events out of the bytes of a MIDI stream
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

#ifndef LMMS_MIDI_BYTE_PARSER_H
#define LMMS_MIDI_BYTE_PARSER_H

#include <array>
#include <cstdint>

#include "MidiEvent.h"

namespace lmms
{

/**
 * The events of a MIDI byte stream, one byte at a time, as the MIDI 1.0 specification describes it, for the drivers
 * that get bytes (raw devices, JACK, CoreMIDI): running status; real-time messages anywhere, even inside another
 * message, without touching the running status (clock, start, continue, stop, reset; active sensing and tick are
 * dropped); song position; system exclusive and the other system common messages skipped (they cancel the running
 * status); data without a status dropped (a stream joined in the middle). No allocation, no lock: one parser per
 * stream, used by one thread at a time.
 */
class MidiByteParser
{
public:
	//! Feeds @p byte; calls @p deliver with each MidiEvent it completes (@p sourcePort goes in the event)
	template<class Deliver>
	void feed(std::uint8_t byte, const void* sourcePort, Deliver&& deliver)
	{
		if (byte >= 0xF8)
		{
			// Real-time: anywhere, without changing the running status
			switch (byte)
			{
			case MidiSync: case MidiStart: case MidiContinue: case MidiStop: case MidiSystemReset:
				deliver(MidiEvent{static_cast<MidiEventTypes>(byte), 0, 0, 0, sourcePort});
				break;
			default: // (active sensing, tick, undefined)
				break;
			}
			if (byte == MidiSystemReset) { m_status = 0; }
			return;
		}

		if (byte & 0x80)
		{
			// A status byte: a new message
			m_inSysEx = byte == MidiSysEx;
			m_count = 0;
			if (byte >= 0xF0)
			{
				// System common: only the song position is kept (two data bytes); each cancels the running status
				m_status = byte == MidiSongPosition ? byte : 0;
				m_channel = 0;
				m_needed = byte == MidiSongPosition ? 2 : 0;
				return;
			}
			m_status = byte & 0xF0;
			m_channel = byte & 0x0F;
			m_needed = (m_status == MidiProgramChange || m_status == MidiChannelPressure) ? 1 : 2;
			return;
		}

		// A data byte: of the message being read, or of the next one with the same status (running status)
		if (m_inSysEx || m_status == 0) { return; }
		m_data[m_count++] = byte;
		if (m_count < m_needed) { return; }
		m_count = 0;

		const auto type = static_cast<MidiEventTypes>(m_status);
		switch (type)
		{
		case MidiNoteOff: case MidiNoteOn: case MidiKeyPressure: case MidiControlChange:
			deliver(MidiEvent{type, static_cast<std::int8_t>(m_channel), m_data[0], m_data[1], sourcePort});
			break;
		case MidiProgramChange: case MidiChannelPressure:
			deliver(MidiEvent{type, static_cast<std::int8_t>(m_channel), m_data[0], 0, sourcePort});
			break;
		case MidiPitchBend:
			deliver(MidiEvent{type, static_cast<std::int8_t>(m_channel), static_cast<std::int16_t>(m_data[0] | (m_data[1] << 7)),
				0, sourcePort});
			break;
		case MidiSongPosition:
			deliver(MidiEvent{type, 0, static_cast<std::int16_t>(m_data[0] | (m_data[1] << 7)), 0, sourcePort});
			m_status = 0; // (system common: no running status)
			break;
		default:
			break;
		}
	}

private:
	std::uint8_t m_status = 0;	//!< the running status (the channel's bits apart), 0 for none
	std::uint8_t m_channel = 0;
	std::uint8_t m_needed = 0;	//!< data bytes of a message of this status
	std::uint8_t m_count = 0;	//!< data bytes read of the message
	std::array<std::uint8_t, 2> m_data = {};
	bool m_inSysEx = false;
};

} // namespace lmms

#endif // LMMS_MIDI_BYTE_PARSER_H
