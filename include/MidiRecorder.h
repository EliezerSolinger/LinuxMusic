/*
 * MidiRecorder.h - records the MIDI input of armed instrument tracks into clips on the timeline
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

#ifndef LMMS_MIDI_RECORDER_H
#define LMMS_MIDI_RECORDER_H

#include <map>
#include <set>
#include <vector>

#include "LmmsTypes.h"
#include "lmms_export.h"
#include "volume.h"

namespace lmms
{

class InstrumentTrack;
class MidiClip;
class Note;

/**
 * MIDI recording on the timeline (docs/plano-gravacao.md, "Gravação MIDI"): while the song records, the notes played
 * into an armed instrument track become notes of a clip of the track, where they were heard. Main thread only (the
 * tracks hand their notes over from the MIDI thread, see InstrumentTrack::processInEvent()).
 *
 * - Overdub (the default): the notes go into the clip that holds their place, or into a new clip from the bar they
 *   begin in; what was there stays.
 * - Replace: as overdub, and at the end the notes that were there before, in the recorded stretch, are removed.
 * - Take in a lane: the clips there were go a lane down, and the recording goes into new clips on the lane heard
 *   (as an audio track's passes; see Track::insertLanesOnTop()).
 * The notes go into the clips of the lane heard.
 */
class LMMS_EXPORT MidiRecorder
{
public:
	enum class Mode
	{
		Overdub,
		Replace,
		TakeLane
	};

	//! Starts recording the tracks, from @p fromTick on
	void start(const std::vector<InstrumentTrack*>& tracks, tick_t fromTick);

	//! Ends recording at @p atTick: notes still held end there
	void stop(tick_t atTick);

	auto isRecording() const -> bool { return !m_tracks.empty(); }
	auto isRecording(const InstrumentTrack* track) const -> bool;

	void noteOn(InstrumentTrack* track, int key, volume_t volume, tick_t atTick);
	void noteOff(InstrumentTrack* track, int key, tick_t atTick);

	//! Forgets a track that is going away
	void forget(const InstrumentTrack* track);

private:
	struct Held
	{
		tick_t start;
		volume_t volume;
	};

	struct TrackState
	{
		Mode mode = Mode::Overdub;
		std::map<int, Held> held; //!< the keys down, by key
		std::set<MidiClip*> clips; //!< the clips the recording went into
		std::set<const Note*> added; //!< the notes the recording added
	};

	void addNote(InstrumentTrack* track, TrackState& state, int key, volume_t volume, tick_t start, tick_t end);

	std::map<InstrumentTrack*, TrackState> m_tracks;
	tick_t m_fromTick = 0;
};

} // namespace lmms

#endif // LMMS_MIDI_RECORDER_H
