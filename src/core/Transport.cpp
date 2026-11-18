/*
 * Transport.cpp
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

#include "Transport.h"

#include <algorithm>
#include <cmath>

#include "AudioEngine.h"
#include "AudioTrack.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "Song.h"

namespace lmms
{

Transport::Transport(Song& song, AudioEngine& engine)
	: m_song{song}
	, m_engine{engine}
{
}




auto Transport::isRecording() const -> bool
{
	const auto current = state();
	return current == State::PreRoll || current == State::CountIn || current == State::Recording;
}




auto Transport::seconds(sample_rate_t sampleRate) const -> double
{
	return sampleRate > 0 ? static_cast<double>(frame()) / sampleRate : 0.0;
}




void Transport::play()
{
	// The song: started from its position when something else (or nothing) plays, else paused or resumed
	if (m_song.playMode() != Song::PlayMode::Song) { m_song.playSong(); }
	else { m_song.togglePause(); }
	updateState();
	emit stateChanged();
}




void Transport::stop()
{
	m_song.stop();
	updateState();
	emit stopped();
	emit stateChanged();
}




auto Transport::canRecord(const Track* track) -> bool
{
	return dynamic_cast<const AudioTrack*>(track) || dynamic_cast<const InstrumentTrack*>(track);
}




auto Transport::record(Track* fallback) -> RecordResult
{
	// While recording, the recording ends and the song plays on (punch out)
	if (m_song.isRecording())
	{
		m_song.stopRecording();
		updateState();
		emit stateChanged();
		return RecordResult::Ended;
	}

	const auto armedModel = [](Track* track) -> BoolModel* {
		if (auto* audioTrack = dynamic_cast<AudioTrack*>(track)) { return audioTrack->recordArmModel(); }
		if (auto* instrumentTrack = dynamic_cast<InstrumentTrack*>(track)) { return instrumentTrack->recordArmModel(); }
		return nullptr;
	};
	const auto& tracks = m_song.tracks();
	const bool armed = std::any_of(tracks.begin(), tracks.end(), [&](Track* track) {
		const auto* model = armedModel(track);
		return model && model->value();
	});
	if (!armed)
	{
		// As in REAPER: nothing armed, the fallback (the selected track) is armed, if it records
		auto* model = fallback ? armedModel(fallback) : nullptr;
		if (!model) { return RecordResult::NothingArmed; }
		model->setValue(true);
	}

	const bool started = m_song.record();
	updateState();
	emit stateChanged();
	return started ? RecordResult::Started : RecordResult::Failed;
}




void Transport::updateState()
{
	auto next = State::Playing;
	if (m_song.isPaused()) { next = State::Paused; }
	else if (!m_song.m_playing.load()) { next = State::Stopped; }
	else if (m_song.m_recording.load())
	{
		const auto start = m_recordingStartTick.load(std::memory_order_relaxed);
		const auto tick = m_song.getPlayPos(Song::PlayMode::Song).getTicks();
		if (start >= 0 && tick < start)
		{
			next = m_song.m_countInUntilTick.load(std::memory_order_relaxed) >= 0 ? State::CountIn : State::PreRoll;
		}
		else { next = State::Recording; }
	}
	m_state.store(next, std::memory_order_release);
}




void Transport::periodStarts(tick_t tick, float frameOffset, f_cnt_t framesPerPeriod)
{
	const bool follows = !m_positionSet.exchange(false, std::memory_order_relaxed) && tick == m_expectedTick
		&& frameOffset == m_expectedFrameOffset;
	m_jumped.store(!follows, std::memory_order_relaxed);
	if (follows) { m_frame.fetch_add(framesPerPeriod, std::memory_order_relaxed); }
	else
	{
		// Counted from the song's start, tick by tick as the engine plays them (the tempo map's tempo at each, or
		// the song's), plus the frames into the tick
		const auto sampleRate = m_engine.outputSampleRate();
		const auto& tempoMap = m_song.tempoMap();
		const auto frames = m_song.playMode() == Song::PlayMode::Song
			? tempoMap.framesBetween(0, tick, m_song.getTempo(), sampleRate)
			: static_cast<double>(tick) * Engine::framesPerTick();
		m_frame.store(static_cast<std::int64_t>(std::llround(frames + frameOffset)), std::memory_order_relaxed);
	}
}




void Transport::periodEnds(tick_t tick, float frameOffset)
{
	m_expectedTick = tick;
	m_expectedFrameOffset = frameOffset;
}




void Transport::positionSet()
{
	m_positionSet.store(true, std::memory_order_relaxed);
}

} // namespace lmms
