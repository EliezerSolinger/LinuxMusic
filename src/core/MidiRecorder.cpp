/*
 * MidiRecorder.cpp - records the MIDI input of armed instrument tracks into clips on the timeline
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

#include "MidiRecorder.h"

#include <algorithm>

#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Note.h"
#include "TimePos.h"

namespace lmms
{

void MidiRecorder::start(const std::vector<InstrumentTrack*>& tracks, tick_t fromTick)
{
	m_tracks.clear();
	m_fromTick = fromTick;
	for (auto* track : tracks)
	{
		auto& state = m_tracks[track];
		state.mode = static_cast<Mode>(track->midiRecordModeModel()->value());
	}
}




auto MidiRecorder::isRecording(const InstrumentTrack* track) const -> bool
{
	return m_tracks.contains(const_cast<InstrumentTrack*>(track));
}




void MidiRecorder::noteOn(InstrumentTrack* track, int key, volume_t volume, tick_t atTick)
{
	const auto found = m_tracks.find(track);
	if (found == m_tracks.end() || atTick < m_fromTick) { return; }
	found->second.held[key] = Held{atTick, volume};
}




void MidiRecorder::noteOff(InstrumentTrack* track, int key, tick_t atTick)
{
	const auto found = m_tracks.find(track);
	if (found == m_tracks.end()) { return; }
	auto& state = found->second;
	const auto held = state.held.find(key);
	if (held == state.held.end()) { return; }
	addNote(track, state, key, held->second.volume, held->second.start, atTick);
	state.held.erase(held);
}




void MidiRecorder::addNote(InstrumentTrack* track, TrackState& state, int key, volume_t volume, tick_t start,
	tick_t end)
{
	// The clip that holds the note's place, or a new one from the bar it begins in
	MidiClip* clip = nullptr;
	for (int index = 0; index < track->numOfClips(); ++index)
	{
		auto* candidate = dynamic_cast<MidiClip*>(track->getClip(index));
		if (candidate && candidate->startPosition() <= start && start < candidate->endPosition())
		{
			clip = candidate;
			break;
		}
	}
	if (!clip)
	{
		track->addJournalCheckPoint();
		clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{start - start % TimePos::ticksPerBar()}));
		if (!clip) { return; }
	}
	if (state.clips.insert(clip).second) { clip->addJournalCheckPoint(); }

	const auto length = std::max<tick_t>(1, end - start);
	const auto* note = clip->addNote(Note{TimePos{length}, TimePos{start - clip->startPosition()}, key, volume}, false);
	state.added.insert(note);
}




void MidiRecorder::stop(tick_t atTick)
{
	for (auto& [track, state] : m_tracks)
	{
		// The keys still down end where the recording ends
		for (const auto& [key, held] : state.held)
		{
			addNote(track, state, key, held.volume, held.start, std::max(atTick, held.start + 1));
		}
		state.held.clear();

		// Replace: what was in the recorded stretch before goes
		if (state.mode == Mode::Replace)
		{
			for (auto* clip : state.clips)
			{
				auto toRemove = std::vector<Note*>{};
				for (auto* note : clip->notes())
				{
					const auto position = clip->startPosition() + note->pos();
					if (!state.added.contains(note) && position >= m_fromTick && position < atTick)
					{
						toRemove.push_back(note);
					}
				}
				for (auto* note : toRemove) { clip->removeNote(note); }
			}
		}
	}
	m_tracks.clear();
}




void MidiRecorder::forget(const InstrumentTrack* track)
{
	m_tracks.erase(const_cast<InstrumentTrack*>(track));
}

} // namespace lmms
