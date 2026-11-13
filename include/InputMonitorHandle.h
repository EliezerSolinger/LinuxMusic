/*
 * InputMonitorHandle.h - plays a sample track's audio input through the track (software monitoring)
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

#ifndef LMMS_INPUT_MONITOR_HANDLE_H
#define LMMS_INPUT_MONITOR_HANDLE_H

#include "PlayHandle.h"

namespace lmms
{

class AudioInputTrack;

/**
 * Software monitoring (docs/plano-gravacao.md, "Gravação"): while it is on, each period the track's input channels
 * from the audio device's capture go into the track, through its effects and the mixer, as the track's clips do.
 * A mono input is heard in both channels.
 *
 * - On: always.
 * - Auto (tape auto): while the track is armed, when the song is stopped or recording, not while it only plays.
 *
 * The track keeps one while it has an input and monitoring is not off (AudioInputTrack::updateInputMonitor()).
 */
class InputMonitorHandle : public PlayHandle
{
public:
	explicit InputMonitorHandle(AudioInputTrack* track);

	void play(std::span<SampleFrame> buffer) override;
	bool isFinished() const override { return false; }
	bool isFromTrack(const Track* track) const override;

	//! @returns whether the track's input is heard now
	static auto monitoring(const AudioInputTrack& track) -> bool;

private:
	AudioInputTrack* m_track;
};

} // namespace lmms

#endif // LMMS_INPUT_MONITOR_HANDLE_H
