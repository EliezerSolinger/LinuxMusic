/*
 * PluginEvent.cpp - events between the host and a plugin, delivered with the audio
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

#include "PluginEvent.h"

#include <algorithm>
#include <cmath>

#include "MidiEvent.h"

namespace lmms
{

namespace
{

auto toVelocity(int midiVelocity) -> double { return std::clamp(midiVelocity, 0, 127) / 127.0; }

auto toMidiVelocity(double velocity) -> std::uint8_t
{
	return static_cast<std::uint8_t>(std::clamp(static_cast<int>(std::lround(velocity * 127.0)), 0, 127));
}

} // namespace


auto PluginEvent::noteOn(f_cnt_t time, int channel, int key, double velocity, std::int32_t noteId) -> PluginEvent
{
	auto e = PluginEvent{};
	e.time = time;
	e.type = Type::NoteOn;
	e.channel = static_cast<std::int16_t>(channel);
	e.key = static_cast<std::int16_t>(key);
	e.value = velocity;
	e.noteId = noteId;
	return e;
}

auto PluginEvent::noteOff(f_cnt_t time, int channel, int key, double velocity, std::int32_t noteId) -> PluginEvent
{
	auto e = noteOn(time, channel, key, velocity, noteId);
	e.type = Type::NoteOff;
	return e;
}

auto PluginEvent::midiMessage(f_cnt_t time, std::uint8_t status, std::uint8_t data1, std::uint8_t data2) -> PluginEvent
{
	auto e = PluginEvent{};
	e.time = time;
	e.type = Type::Midi;
	e.channel = static_cast<std::int16_t>(status & 0x0f);
	e.midi = {status, static_cast<std::uint8_t>(data1 & 0x7f), static_cast<std::uint8_t>(data2 & 0x7f)};
	return e;
}

auto PluginEvent::fromMidi(const MidiEvent& event, f_cnt_t time) -> std::optional<PluginEvent>
{
	const auto channel = std::clamp<int>(event.channel(), 0, 15);
	switch (event.type())
	{
		case MidiNoteOn:
			if (event.velocity() > 0) { return noteOn(time, channel, std::clamp<int>(event.key(), 0, 127), toVelocity(event.velocity())); }
			return noteOff(time, channel, std::clamp<int>(event.key(), 0, 127));
		case MidiNoteOff:
			return noteOff(time, channel, std::clamp<int>(event.key(), 0, 127), toVelocity(event.velocity()));
		case MidiKeyPressure:
		case MidiControlChange:
			return midiMessage(time, static_cast<std::uint8_t>(event.type() | channel),
				static_cast<std::uint8_t>(event.param(0)), static_cast<std::uint8_t>(event.param(1)));
		case MidiProgramChange:
		case MidiChannelPressure:
			return midiMessage(time, static_cast<std::uint8_t>(event.type() | channel),
				static_cast<std::uint8_t>(event.param(0)), 0);
		case MidiPitchBend:
		{
			const auto bend = std::clamp<int>(event.pitchBend(), 0, 16383);
			return midiMessage(time, static_cast<std::uint8_t>(MidiPitchBend | channel),
				static_cast<std::uint8_t>(bend & 0x7f), static_cast<std::uint8_t>(bend >> 7));
		}
		default:
			return std::nullopt;
	}
}

auto PluginEvent::toMidiBytes() const -> std::optional<std::array<std::uint8_t, 3>>
{
	const auto ch = static_cast<std::uint8_t>(std::clamp<int>(channel, 0, 15));
	switch (type)
	{
		case Type::NoteOn:
			if (key < 0) { return std::nullopt; }
			// A velocity of 0 would mean note-off in MIDI
			return std::array<std::uint8_t, 3>{static_cast<std::uint8_t>(MidiNoteOn | ch), static_cast<std::uint8_t>(key),
				std::max<std::uint8_t>(toMidiVelocity(value), 1)};
		case Type::NoteOff:
		case Type::NoteChoke:
			if (key < 0) { return std::array<std::uint8_t, 3>{static_cast<std::uint8_t>(MidiControlChange | ch), 123, 0}; }
			return std::array<std::uint8_t, 3>{static_cast<std::uint8_t>(MidiNoteOff | ch), static_cast<std::uint8_t>(key),
				toMidiVelocity(value)};
		case Type::Midi:
			return midi;
		default:
			return std::nullopt;
	}
}

auto PluginEvent::toMidi() const -> std::optional<MidiEvent>
{
	const auto bytes = toMidiBytes();
	if (!bytes) { return std::nullopt; }

	const auto status = static_cast<MidiEventTypes>((*bytes)[0] & 0xf0);
	const auto ch = static_cast<std::int8_t>((*bytes)[0] & 0x0f);
	if (status == MidiPitchBend)
	{
		auto event = MidiEvent{MidiPitchBend, ch};
		event.setPitchBend(static_cast<std::uint16_t>((*bytes)[1] | ((*bytes)[2] << 7)));
		return event;
	}
	return MidiEvent{status, ch, (*bytes)[1], (*bytes)[2]};
}

} // namespace lmms
