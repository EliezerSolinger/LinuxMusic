/*
 * PluginEvent.h - events between the host and a plugin, delivered with the audio
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

#ifndef LMMS_PLUGIN_EVENT_H
#define LMMS_PLUGIN_EVENT_H

#include <array>
#include <cstdint>
#include <optional>
#include <span>

#include "LmmsTypes.h"
#include "lmms_export.h"

namespace lmms
{

class MidiEvent;

/**
 * An event between the host and a plugin, as in CLAP: notes with optional note IDs, per-note expressions, and
 * raw MIDI for everything else. Each event happens at a frame of the period it is delivered with.
 *
 * Fields that do not apply are -1 ("any"), e.g. a note-off with key -1 ends all notes of its channel.
 */
struct LMMS_EXPORT PluginEvent
{
	enum class Type : std::uint8_t
	{
		NoteOn,			//!< key, channel, velocity (value) and an optional note ID
		NoteOff,		//!< key, channel, release velocity (value)
		NoteChoke,		//!< ends the note right away, without release
		NoteEnd,		//!< plugin -> host: a voice finished
		NoteExpression,	//!< expression of a note or of all notes of a channel (value, see Expression)
		Midi			//!< raw MIDI message (midi), for everything else
	};

	//! Per-note expressions; the ranges follow CLAP
	enum class Expression : std::uint8_t
	{
		Volume,		//!< 0 to 4, linear gain
		Pan,		//!< 0 (left) to 1 (right), 0.5 is center
		Tuning,		//!< -120 to 120 semitones
		Vibrato,	//!< 0 to 1
		Brightness,	//!< 0 to 1
		Pressure	//!< 0 to 1
	};

	f_cnt_t time = 0;		//!< frame within the period
	Type type = Type::Midi;
	Expression expression = Expression::Volume;
	std::int16_t port = 0;
	std::int16_t channel = -1;	//!< 0 to 15
	std::int16_t key = -1;		//!< 0 to 127
	std::int32_t noteId = -1;
	double value = 0.0;			//!< velocity (0 to 1) or expression value
	std::array<std::uint8_t, 3> midi = {};

	static auto noteOn(f_cnt_t time, int channel, int key, double velocity, std::int32_t noteId = -1) -> PluginEvent;
	static auto noteOff(f_cnt_t time, int channel, int key, double velocity = 0.0, std::int32_t noteId = -1)
		-> PluginEvent;
	static auto midiMessage(f_cnt_t time, std::uint8_t status, std::uint8_t data1, std::uint8_t data2) -> PluginEvent;

	/**
	 * Converts a MIDI event: note on/off become note events (a note-on with velocity 0 is a note-off), channel
	 * messages become raw MIDI.
	 *
	 * @returns nothing for messages without a channel (system exclusive, real-time...)
	 */
	static auto fromMidi(const MidiEvent& event, f_cnt_t time) -> std::optional<PluginEvent>;

	//! The event as a MIDI message, for plugins that speak MIDI; nothing for events MIDI cannot express
	auto toMidi() const -> std::optional<MidiEvent>;

	//! The event as MIDI bytes (status, data1, data2); nothing for events MIDI cannot express
	auto toMidiBytes() const -> std::optional<std::array<std::uint8_t, 3>>;
};


//! The events a plugin receives with a period, sorted by time
using InputEvents = std::span<const PluginEvent>;

//! Where a plugin puts the events it sends to the host during a period
class LMMS_EXPORT OutputEvents
{
public:
	OutputEvents(PluginEvent* storage, std::size_t capacity) : m_storage{storage}, m_capacity{capacity} {}

	//! @returns false if there is no room left for this period
	auto tryPush(const PluginEvent& event) -> bool
	{
		if (m_size >= m_capacity) { return false; }
		m_storage[m_size++] = event;
		return true;
	}

	auto events() const -> std::span<const PluginEvent> { return {m_storage, m_size}; }
	void clear() { m_size = 0; }

private:
	PluginEvent* m_storage;
	std::size_t m_capacity;
	std::size_t m_size = 0;
};

} // namespace lmms

#endif // LMMS_PLUGIN_EVENT_H
