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

// Guitarix's Vibe, its file as it is (but for its LFOs: each Vibe's own here, see vendor/gx/plugins/vibe.cc),
// compiled here (and only here)
#include "vibe.cc"

#include <cstring>
#include <new>

#include "GuitarixUnits.h"

namespace lmms::guitarix
{

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

} // namespace lmms::guitarix
