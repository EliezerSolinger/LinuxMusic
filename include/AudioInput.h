/*
 * AudioInput.h - where a track records from
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

#ifndef LMMS_AUDIO_INPUT_H
#define LMMS_AUDIO_INPUT_H

namespace lmms
{

/**
 * Where a track takes audio from to record it (stage 3 of the LinuxMusic roadmap, docs/etapa-3-gravacao.md). Audio
 * input, arming and monitoring are the tracks', as in REAPER: the mixer's channels mix what the tracks give them.
 */
struct AudioInput
{
	enum class Source
	{
		None,		//!< nothing to record
		Hardware	//!< inputs of the audio interface
	};

	Source source = Source::None;
	int firstChannel = 0; //!< zero-based input channel of the audio interface
	int channelCount = 2; //!< 1 = mono, 2 = stereo pair starting at firstChannel

	bool operator==(const AudioInput&) const = default;
};

} // namespace lmms

#endif // LMMS_AUDIO_INPUT_H
