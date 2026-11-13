/*
 * InputMonitorHandle.cpp - plays a sample track's audio input through the track (software monitoring)
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

#include "InputMonitorHandle.h"

#include <algorithm>
#include <cmath>

#include "AudioEngine.h"
#include "Engine.h"
#include "SampleFrame.h"
#include "SampleTrack.h"
#include "Song.h"

namespace lmms
{

InputMonitorHandle::InputMonitorHandle(SampleTrack* track)
	: PlayHandle{Type::InputMonitorHandle}
	, m_track{track}
{
	setAudioBusHandle(track->audioBusHandle());
}




auto InputMonitorHandle::monitoring(const SampleTrack& track) -> bool
{
	if (track.input().source != AudioInput::Source::Hardware) { return false; }
	switch (static_cast<SampleTrack::MonitorMode>(track.monitorMode()))
	{
	case SampleTrack::MonitorMode::On:
		return true;
	case SampleTrack::MonitorMode::Auto:
	{
		// Tape auto: heard while stopped or recording, not while only playing what was recorded
		const auto* song = Engine::getSong();
		return track.isRecordArmed() && (!song->isPlaying() || song->isRecordingAudio());
	}
	case SampleTrack::MonitorMode::Off:
	default:
		return false;
	}
}




void InputMonitorHandle::play(std::span<SampleFrame> buffer)
{
	const auto input = m_track->input();
	if (input.source != AudioInput::Source::Hardware) { return; }

	const auto capture = Engine::audioEngine()->capture();
	const auto frames = std::min<std::size_t>(buffer.size(), capture.frames());
	const auto left = input.firstChannel;
	const auto right = input.channelCount == 2 ? input.firstChannel + 1 : input.firstChannel;
	if (left >= capture.channels() || right >= capture.channels()) { return; }
	const float* leftSamples = capture.bufferPtr(static_cast<ch_cnt_t>(left));
	const float* rightSamples = capture.bufferPtr(static_cast<ch_cnt_t>(right));

	// The input meter shows the input whether it is heard or not (to set the level before recording)
	auto peak = 0.f;
	for (std::size_t frame = 0; frame < frames; ++frame)
	{
		peak = std::max({peak, std::abs(leftSamples[frame]), std::abs(rightSamples[frame])});
	}
	m_track->reportInputPeak(peak);

	if (m_track->isSilenced() || !monitoring(*m_track)) { return; }
	for (std::size_t frame = 0; frame < frames; ++frame)
	{
		buffer[frame][0] = leftSamples[frame];
		buffer[frame][1] = rightSamples[frame];
	}
}




bool InputMonitorHandle::isFromTrack(const Track* track) const
{
	return track == m_track;
}

} // namespace lmms
