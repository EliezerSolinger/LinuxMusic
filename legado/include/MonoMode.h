/*
 * MonoMode.h - an instrument track played one note at a time, legato and with glide
 *
 * This file is part of LinuxMusic, based on LMMS - https://lmms.io
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

#ifndef LMMS_MONO_MODE_H
#define LMMS_MONO_MODE_H

#include <array>
#include <atomic>
#include <cstdint>

#include "AutomatableModel.h"
#include "JournallingObject.h"
#include "LmmsTypes.h"
#include "Note.h"
#include "NotePlayHandle.h"

namespace lmms
{

class InstrumentTrack;

namespace gui
{
class InstrumentFunctionMonoView;
}

/**
 * Upstream's #6516: any instrument played one note at a time, as a monophonic synthesizer is.
 *
 * The notes held (the keys pressed, the notes of the clips playing) are kept in the order they came; one voice plays
 * the last of them. A new one takes the voice: legato, the voice moves to its key without starting again (no new
 * attack); otherwise the voice ends and a new one starts. When the note playing ends and another is still held, the
 * voice goes back to it. With a glide time, the voice's pitch slides from where it was to the new key whenever it
 * moves from one note to the next (only when they overlap: fingered portamento).
 *
 * The voice is a note of no length of its own: the notes held say when it ends. The clips' notes are dealt with in
 * the song's part of the period (on the audio thread, before the notes play), the keys from a device or the piano
 * under the audio engine's change lock: never while a note plays. The voice clears itself from here first thing as
 * it goes (NotePlayHandle's destructor), so nothing is done to a note on its way out.
 *
 * Instruments that take MIDI (ZynAddSubFX, VST, the soundfont players) get a note on for the new key and a note off
 * for the old one at once (their own legato, if they have one); the glide is the others' (they follow the note's
 * frequency).
 */
class LMMS_EXPORT MonoMode : public Model, public JournallingObject
{
	Q_OBJECT
public:
	explicit MonoMode(InstrumentTrack* track);
	~MonoMode() override = default;

	auto enabled() const -> bool { return m_enabledModel.value(); }

	/**
	 * A note starts on the track: @p note (its key, volume, panning), @p offset frames into the period, @p frames
	 * long (none: until noteOff() for its key, a key held), from @p origin on MIDI @p channel. [audio thread, song's
	 * part of the period; or under the audio engine's change lock]
	 */
	void noteOn(const Note& note, f_cnt_t offset, f_cnt_t frames, NotePlayHandle::Origin origin, int channel);
	//! The key @p key held (noteOn() without frames) is let go, @p offset frames into the period [as noteOn()]
	void noteOff(int key, f_cnt_t offset);
	//! The notes of clips that end at the tick starting @p offset frames into the period end the voice or take it
	//! back [audio thread, song's part: each tick, after its notes start]
	void advance(f_cnt_t offset);
	//! Nothing held, no voice (the song stops, the notes are silenced) [main thread, change lock]
	void reset();
	//! @p note goes: it is no longer the voice [NotePlayHandle's destructor, any thread]
	void forget(const NotePlayHandle* note);

	//! Whether @p key is held from a device or the piano
	auto isHeld(int key) const -> bool;

	void saveSettings(QDomDocument& doc, QDomElement& element) override;
	void loadSettings(const QDomElement& element) override;
	//! Off, as a new track (settings without it: an older project, a preset)
	void loadDefaults();
	auto nodeName() const -> QString override { return "monomode"; }

private:
	struct Held
	{
		int key = 0;
		volume_t volume = DefaultVolume;
		panning_t panning = DefaultPanning;
		std::int64_t end = 0; //!< the frame (see now()) it ends at; none, a key held
		bool live = false;
		NotePlayHandle::Origin origin = NotePlayHandle::Origin::MidiClip;
		int channel = 0;
	};
	static constexpr std::size_t MostHeld = 32;
	static constexpr std::int64_t Never = INT64_MAX;

	//! The frame the period being played starts at, counting from the engine's start
	static auto now() -> std::int64_t;
	auto glideFrames() const -> f_cnt_t;
	//! The voice moves to (or starts again at) @p held, @p offset frames into the period
	void play(const Held& held, f_cnt_t offset, bool fromAnother);
	void remove(std::size_t index);

	InstrumentTrack* m_track;
	BoolModel m_enabledModel;
	BoolModel m_legatoModel; //!< a new note moves the voice to its key; off, it starts the voice again
	FloatModel m_glideModel; //!< milliseconds

	//! Held for a few steps (spinning): the destructor of a note may run on any thread
	class Lock
	{
	public:
		explicit Lock(std::atomic_flag& flag) : m_flag{flag}
		{
			while (m_flag.test_and_set(std::memory_order_acquire))
			{
				while (m_flag.test(std::memory_order_relaxed)) {}
			}
		}
		~Lock() { m_flag.clear(std::memory_order_release); }
		Lock(const Lock&) = delete;
		auto operator=(const Lock&) -> Lock& = delete;

	private:
		std::atomic_flag& m_flag;
	};
	mutable std::atomic_flag m_lock = ATOMIC_FLAG_INIT;
	std::array<Held, MostHeld> m_held;
	std::size_t m_heldCount = 0;
	NotePlayHandle* m_voice = nullptr;
	int m_voiceKey = -1; //!< the key of the note the voice plays (the voice's own key moves with it)

	friend class gui::InstrumentFunctionMonoView;
};

} // namespace lmms

#endif // LMMS_MONO_MODE_H
