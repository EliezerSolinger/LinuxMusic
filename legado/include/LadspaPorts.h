/*
 * LadspaPorts.h - what each port of a LADSPA plugin is, and where its values live in a shared control block
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

#ifndef LMMS_LADSPA_PORTS_H
#define LMMS_LADSPA_PORTS_H

#include <cctype>
#include <cstddef>
#include <limits>
#include <vector>

#include <ladspa.h>

/**
 * Shared by the LADSPA effect in the program and by the plugin process that runs one isolated
 * (plugins/LadspaEffect/RemoteLadspaHost.cpp), so that both see the same ports in the same places. No Qt here: the
 * plugin process does not have it.
 */
namespace lmms::LadspaPorts
{

enum class Kind
{
	ChannelIn,         //!< an audio input the host connects to one of its channels
	ChannelOut,        //!< an audio output the host connects to one of its channels
	AudioRateInput,    //!< an audio-rate control input (a buffer of values)
	AudioRateOutput,   //!< an audio-rate control output
	ControlRateInput,  //!< one value a period
	ControlRateOutput
};

//! Whether @p name contains @p word, ignoring ASCII case (@p word is upper case)
inline auto containsWord(const char* name, const char* word) -> bool
{
	if (!name) { return false; }
	for (const char* start = name; *start; ++start)
	{
		auto here = start;
		auto wanted = word;
		while (*wanted && *here && std::toupper(static_cast<unsigned char>(*here)) == *wanted) { ++here; ++wanted; }
		if (!*wanted) { return true; }
	}
	return false;
}

//! What port @p port is. An audio port is one of the host's channels when its name says "in" or "out" in its direction
//! (as LadspaManager counts a plugin's channels); any other is an audio-rate control
inline auto kindOf(const LADSPA_Descriptor& descriptor, unsigned long port) -> Kind
{
	const auto what = descriptor.PortDescriptors[port];
	const auto* name = descriptor.PortNames ? descriptor.PortNames[port] : nullptr;
	if (LADSPA_IS_PORT_AUDIO(what))
	{
		if (LADSPA_IS_PORT_INPUT(what) && containsWord(name, "IN")) { return Kind::ChannelIn; }
		if (LADSPA_IS_PORT_OUTPUT(what) && containsWord(name, "OUT")) { return Kind::ChannelOut; }
		return LADSPA_IS_PORT_INPUT(what) ? Kind::AudioRateInput : Kind::AudioRateOutput;
	}
	return LADSPA_IS_PORT_INPUT(what) ? Kind::ControlRateInput : Kind::ControlRateOutput;
}

//! The place of a port that has none in the control block (a channel)
constexpr auto NoPlace = std::numeric_limits<std::size_t>::max();

/**
 * The control block shared with an isolated plugin: first the number of frames of the period (written by the host
 * before each one), then, processor by processor and port by port, one value for a control-rate port and
 * @p maxFrames values for an audio-rate one. The channels are in the shared audio buffers instead.
 */
struct Layout
{
	std::vector<std::size_t> places; //!< processor * portCount + port -> its first value, or NoPlace
	std::size_t size = 1;            //!< values in the whole block
};

inline auto layoutOf(const LADSPA_Descriptor& descriptor, std::size_t processors, std::size_t maxFrames) -> Layout
{
	auto layout = Layout{};
	layout.places.reserve(processors * descriptor.PortCount);
	for (std::size_t processor = 0; processor < processors; ++processor)
	{
		for (unsigned long port = 0; port < descriptor.PortCount; ++port)
		{
			switch (kindOf(descriptor, port))
			{
			case Kind::ChannelIn:
			case Kind::ChannelOut:
				layout.places.push_back(NoPlace);
				break;
			case Kind::AudioRateInput:
			case Kind::AudioRateOutput:
				layout.places.push_back(layout.size);
				layout.size += maxFrames;
				break;
			case Kind::ControlRateInput:
			case Kind::ControlRateOutput:
				layout.places.push_back(layout.size);
				layout.size += 1;
				break;
			}
		}
	}
	return layout;
}

//! The messages between the LADSPA effect and its plugin process (after RemotePluginBase's own)
enum Message
{
	//! To the process: load (library path, label, sample rate, max frames, processors, control block key).
	//! It answers IdLadspaLoaded (1 and nothing, or 0 and why)
	IdLadspaLoad = 64 + 10,
	IdLadspaLoaded,
	IdLadspaActivate,   //!< activate the instances (LADSPA activate())
	IdLadspaDeactivate, //!< deactivate them
	//! Run once on silence, then activate afresh (the latency port is then right); answered with IdLadspaMeasured
	IdLadspaMeasureLatency,
	IdLadspaMeasured,
};

} // namespace lmms::LadspaPorts

#endif // LMMS_LADSPA_PORTS_H
