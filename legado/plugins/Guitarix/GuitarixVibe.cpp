/*
 * GuitarixVibe.cpp - Guitarix's Vibe, and its LFOs' state, one for each instance
 *
 * This file is part of LinuxMusic, based on LMMS - https://lmms.io
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

// Guitarix's Vibe, its file as it is, compiled here (and only here): its LFOs (Faust code generated to be embedded)
// keep their state in the file, which this file reaches
#include "vibe.cc"

#include <cstring>
#include <new>

#include "GuitarixUnits.h"

namespace lmms::guitarix
{

namespace
{

std::atomic_flag s_lfoLock;
std::atomic_flag s_monoLfoLock;

//! Copies @p from to @p to, and on: one array after another
template<typename... Arrays>
void copyOut(std::byte* to, const Arrays&... arrays)
{
	((std::memcpy(to, &arrays, sizeof(arrays)), to += sizeof(arrays)), ...);
}

template<typename... Arrays>
void copyIn(const std::byte* from, Arrays&... arrays)
{
	((std::memcpy(&arrays, from, sizeof(arrays)), from += sizeof(arrays)), ...);
}

// (what their code changes as it runs: the oscillators; their rate is the same for all, their controls are each
// effect's, set before it runs)
namespace stereo = ::vibe_lfo_sine;
namespace mono = ::vibe_mono_lfo_sine;

} // namespace


//! A Vibe built on zeroed memory: its filters' state (fparams' x1 and y1), its feedback and steps are set by
//! neither its constructor nor its init_vibes; zeroed, every one starts the same (as fresh memory is, mostly, where
//! Guitarix runs). Its del_instance's delete frees it (the memory is ::operator new's, as a new expression's).
auto makeVibe(bool stereo) -> PluginDef*
{
	using pluginlib::vibe::Vibe;
	void* memory = ::operator new(sizeof(Vibe));
	std::memset(memory, 0, sizeof(Vibe));
	return new (memory) Vibe(stereo);
}

auto makeVibeMono() -> PluginDef* { return makeVibe(false); }
auto makeVibeStereo() -> PluginDef* { return makeVibe(true); }


const FileState vibeLfoState{
	sizeof(stereo::iVec0) + sizeof(stereo::fRec0) + sizeof(stereo::fRec1),
	[](std::byte* to) { copyOut(to, stereo::iVec0, stereo::fRec0, stereo::fRec1); },
	[](const std::byte* from) { copyIn(from, stereo::iVec0, stereo::fRec0, stereo::fRec1); },
	&s_lfoLock,
};

const FileState vibeMonoLfoState{
	sizeof(mono::iVec0) + sizeof(mono::fRec0) + sizeof(mono::fRec1) + sizeof(mono::fRec2),
	[](std::byte* to) { copyOut(to, mono::iVec0, mono::fRec0, mono::fRec1, mono::fRec2); },
	[](const std::byte* from) { copyIn(from, mono::iVec0, mono::fRec0, mono::fRec1, mono::fRec2); },
	&s_monoLfoLock,
};

} // namespace lmms::guitarix
