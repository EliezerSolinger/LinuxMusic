/*
 * Lv2Isolation.h - what the LV2 effect and the process that runs it isolated (RemoteLv2Host) say to each other
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

#ifndef LMMS_LV2_ISOLATION_H
#define LMMS_LV2_ISOLATION_H

#include <cstddef>
#include <string>
#include <vector>

/**
 * No Qt here: the plugin process does not have it.
 *
 * The effect in the program works out how each port of each instance is connected and sends it as a plan (it alone
 * knows the ports: Lv2Proc): the process follows the plan and needs no rules of its own. Audio ports are connected to
 * the shared audio buffers (inputs and outputs counted as the effect counts its channels); control ports to places in
 * a control block shared with the program, whose first value is the period's frames; an audio-rate control (CV) input
 * takes the one value at its place for the whole period, as in the program; event ports get an empty sequence (an
 * effect gets no events in the program either).
 */
namespace lmms::Lv2Isolation
{

enum class Kind : char
{
	AudioIn = 'a',     //!< index: the shared input channel
	AudioOut = 'A',    //!< index: the shared output channel
	ControlIn = 'c',   //!< index: its place in the control block
	ControlOut = 'C',  //!< index: its place in the control block
	CvIn = 'v',        //!< index: its value's place in the control block
	CvOut = 'V',       //!< a buffer of its own in the process
	EventsIn = 'e',    //!< an empty sequence each period
	EventsOut = 'E',   //!< room for the plugin to write in
	None = 'u'         //!< connected to nothing (as an unknown port in the program)
};

struct Port
{
	Kind kind = Kind::None;
	std::size_t index = 0;
	std::size_t eventCapacity = 0; //!< bytes, for event ports
};

//! One instance's ports, in port order
using Plan = std::vector<Port>;

//! "kind index capacity;" for each port, instances separated by '|'
inline auto encode(const std::vector<Plan>& plans) -> std::string
{
	auto text = std::string{};
	for (const auto& plan : plans)
	{
		if (!text.empty()) { text += '|'; }
		for (const auto& port : plan)
		{
			text += static_cast<char>(port.kind);
			text += std::to_string(port.index) + ' ' + std::to_string(port.eventCapacity) + ';';
		}
	}
	return text;
}

//! The plans in @p text; empty when it is not one (it comes from the other process)
inline auto decode(const std::string& text) -> std::vector<Plan>
{
	auto plans = std::vector<Plan>(1);
	std::size_t at = 0;
	while (at < text.size())
	{
		if (text[at] == '|') { plans.emplace_back(); ++at; continue; }
		auto port = Port{};
		switch (text[at])
		{
		case 'a': case 'A': case 'c': case 'C': case 'v': case 'V': case 'e': case 'E': case 'u':
			port.kind = static_cast<Kind>(text[at]);
			break;
		default:
			return {};
		}
		++at;
		const auto space = text.find(' ', at);
		const auto end = text.find(';', at);
		if (space == std::string::npos || end == std::string::npos || space > end) { return {}; }
		try
		{
			port.index = static_cast<std::size_t>(std::stoull(text.substr(at, space - at)));
			port.eventCapacity = static_cast<std::size_t>(std::stoull(text.substr(space + 1, end - space - 1)));
		}
		catch (...) { return {}; }
		plans.back().push_back(port);
		at = end + 1;
	}
	return plans;
}

//! The messages (after RemotePluginBase's own and LadspaPorts' ones)
enum Message
{
	//! To the process: load (bundle path, plugin URI, sample rate, max frames, plans, control block key, whether
	//! the worker has a thread (not when only rendering, as in the program), the feature URIs offered without data).
	//! It answers IdLv2Loaded (1 and nothing, or 0 and why)
	IdLv2Load = 64 + 30,
	IdLv2Loaded,
	IdLv2Activate,
	IdLv2Deactivate,
	//! Run once on silence, then activate afresh (the latency port is then right); answered with IdLv2Measured
	IdLv2MeasureLatency,
	IdLv2Measured,
};

} // namespace lmms::Lv2Isolation

#endif // LMMS_LV2_ISOLATION_H
