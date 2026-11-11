/*
 * Host.h - Defines the public host API for native LMMS plugins
 *
 * Copyright (c) 2026 Dalton Messmer <messmer.dalton/at/gmail.com>
 *
 * This file is part of LMMS - https://lmms.io
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

#ifndef LMMS_HOST_H
#define LMMS_HOST_H

#include <cstdint>

namespace lmms {

/**
 * @brief Public host API for native LMMS plugins.
 *
 * Once the New Plugin API (#8275) is fully implemented, this interface
 * will be used for all communication from plugin to host.
 *
 * NOTE: Neither the Host nor Plugin sides of the API are API-stable or ABI-stable. Expect major changes!
 */
class Host
{
public:
	//! Tells the host what kind of change took place
	enum class AudioPortsRescanFlag : std::uint32_t
	{
		//! The names of audio port(s) changed
		Names        = 1 << 0,

		//! The audio port flags changed (main/aux)
		Flags        = 1 << 1,

		//! The input or output channel counts changed
		ChannelCount = 1 << 2,

		//! The port types changed (mono/stereo/surround-sound/...)
		Type         = 1 << 3,

		//! The in-place buffer mapping changed
		InPlace      = 1 << 4,

		//! The number of audio ports changed
		List         = 1 << 5
	}

	friend constexpr auto operator|(AudioPortsRescanFlag l, AudioPortsRescanFlag r) noexcept -> AudioPortsRescanFlag
	{
		return static_cast<AudioPortsRescanFlag>(static_cast<std::uint32_t>(l) | static_cast<std::uint32_t>(r));
	}

	static constexpr auto AudioPortsRescanAll = AudioPortsRescanFlag{0x3F};

	/**
	 * Tells the host to rescan the plugin's audio ports.
	 *
	 * @note The host will scan the plugin's audio ports immediately after instantiation, so this
	 *       does not need to be called then.
	 *
	 * @param flags what aspect(s) of the audio port(s) changed. Multiple flags may be OR'd together.
	 *
	 * [main-thread]
	 */
	virtual void rescanAudioPorts(AudioPortsRescanFlag flags) = 0;

	/**
	 * Allows a plugin to switch between local buffers and RemotePlugin buffers.
	 *
	 * @todo In the future, once any plugin can run in a RemotePlugin sandbox, this should
	 *       probably be controlled by the host side.
	 *
	 * [thread-safe]
	 */
	virtual void useRemotePluginBuffers(bool remote) = 0;


	// TODO: get plugin state
};

} // namespace lmms

#endif // LMMS_HOST_H
