/*
 * Transport.h
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

#ifndef LMMS_TRANSPORT_H
#define LMMS_TRANSPORT_H

#include <atomic>
#include <cstdint>

#include <QObject>

#include "LmmsTypes.h"
#include "lmms_export.h"

namespace lmms
{

class AudioEngine;
class Song;
class Track;

/**
 * The song's transport: what it is doing and where on the timeline it is, in frames, readable from any thread at
 * once, and the global actions the transport bar and the shortcuts call (play, stop, record), which no editor owns.
 *
 * The state is one atomic value: the song sets it when it starts, pauses, stops or starts recording, and the audio
 * thread when a recording's pre-roll or count-in ends (the interface reads it, by timer). The frame is where the
 * period being played starts, counted from the song's start along the tempo map exactly as the engine plays it.
 */
class LMMS_EXPORT Transport : public QObject
{
	Q_OBJECT
public:
	enum class State : int
	{
		Stopped,
		Paused,
		Playing,
		PreRoll,	//!< recording: playing the bars before the cursor first
		CountIn,	//!< recording: the pre-roll, with the metronome's count-in
		Recording
	};

	Transport(Song& song, AudioEngine& engine);

	auto state() const -> State { return m_state.load(std::memory_order_acquire); }
	auto isRecording() const -> bool;

	//! The timeline frame the period being played starts at (song mode), along the tempo map [any thread]
	auto frame() const -> std::int64_t { return m_frame.load(std::memory_order_relaxed); }
	//! Whether the period being played starts somewhere else than the last one ended (a jump, the cycle, a start)
	auto jumped() const -> bool { return m_jumped.load(std::memory_order_relaxed); }
	//! The same, as seconds of the song
	auto seconds(sample_rate_t sampleRate) const -> double;
	//! Where a recording's pre-roll (or count-in) ends and it records (-1: none) [any thread]
	auto recordingStartTick() const -> tick_t { return m_recordingStartTick.load(std::memory_order_relaxed); }

	// The global actions [main thread]

	//! Plays the song from its position, or pauses it while it plays, and resumes it
	void play();
	//! Stops the song and any recording (the piano roll's too: see stopped())
	void stop();

	enum class RecordResult
	{
		Started,
		Ended,			//!< it was recording: the recording ends and the song plays on (punch out)
		NothingArmed,	//!< no track records, and @p fallback did not either
		Failed			//!< the song said why (Song::recordingProblem)
	};
	/**
	 * Starts recording on the armed tracks; with none armed, @p fallback is armed first if it records (the selected
	 * track, as in REAPER). While recording, ends the recording.
	 */
	auto record(Track* fallback) -> RecordResult;
	//! Whether @p track can record (an audio track, an instrument track)
	static auto canRecord(const Track* track) -> bool;

signals:
	//! The state changed on the main thread (the audio thread's changes are read by timer)
	void stateChanged();
	//! The transport stopped: editors that record on their own stop too
	void stopped();

private:
	friend class Song;

	//! Works the state out from the song [any thread]
	void updateState();
	/**
	 * The period starting at tick @p tick and @p frameOffset frames into it: counted on from the last one when it
	 * follows it, else counted again from the song's start (a jump) [audio thread]
	 */
	void periodStarts(tick_t tick, float frameOffset, f_cnt_t framesPerPeriod);
	//! Where the period that was played ends, which the next one starts at if nothing jumps [audio thread]
	void periodEnds(tick_t tick, float frameOffset);
	//! The position was set (stopped or jumping): the next period counts again
	void positionSet();

	Song& m_song;
	AudioEngine& m_engine;
	std::atomic<State> m_state = State::Stopped;
	std::atomic<std::int64_t> m_frame = 0;
	std::atomic<bool> m_jumped = false;
	std::atomic<tick_t> m_recordingStartTick = -1; //!< where the recording's pre-roll ends

	// The audio thread's: what the next period would start at if nothing jumps
	tick_t m_expectedTick = -1;
	float m_expectedFrameOffset = 0.f;
	std::atomic<bool> m_positionSet = true;
};

} // namespace lmms

#endif // LMMS_TRANSPORT_H
