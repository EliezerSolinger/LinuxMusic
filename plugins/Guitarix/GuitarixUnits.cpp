/*
 * GuitarixUnits.cpp - the Guitarix units built into LinuxMusic
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
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public
 * License along with this program (see COPYING); if not, write to the
 * Free Software Foundation, Inc., 51 Franklin Street, Fifth Floor,
 * Boston, MA 02110-1301 USA.
 *
 */

#include "GuitarixUnits.h"

#include <array>
#include <cstring>

#include <QtGlobal>

struct PluginDef;

// Guitarix's factories (each unit's file, compiled as it is in Guitarix: see gx/)
namespace pluginlib
{
namespace ts9sim { PluginDef* plugin(); }
} // namespace pluginlib

namespace lmms::guitarix
{

namespace
{

const auto s_units = std::array{
	Unit{"ts9sim", QT_TRANSLATE_NOOP("GuitarixEffect", "Distortion"), &pluginlib::ts9sim::plugin},
};

} // namespace


auto units() -> std::span<const Unit>
{
	return s_units;
}


auto unit(const char* id) -> const Unit*
{
	for (const auto& candidate : s_units)
	{
		if (std::strcmp(candidate.id, id) == 0) { return &candidate; }
	}
	return nullptr;
}

} // namespace lmms::guitarix
