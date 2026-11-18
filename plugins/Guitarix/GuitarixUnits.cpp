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

// Guitarix's factories, as Guitarix declares them (each unit's file is compiled as it is in Guitarix: see gx/)
#include "pluginlib.h"

namespace lmms::guitarix
{

namespace
{

// In the order of Guitarix's rack (gx_engine.cpp), its two stereo units last
const auto s_units = std::array{
	Unit{"ts9sim", &pluginlib::ts9sim::plugin},
	Unit{"aclipper", &pluginlib::aclipper::plugin},
	Unit{"mxrdis", &pluginlib::mxrdist::plugin},
	Unit{"bossds1", &pluginlib::bossds1::plugin},
	Unit{"bmp", &pluginlib::bmp::plugin},
	Unit{"flanger_mono_gx", &pluginlib::flanger_gx::plugin},
	Unit{"mbc", &pluginlib::mbc::plugin},
	Unit{"mbd", &pluginlib::mbd::plugin},
	Unit{"mbe", &pluginlib::mbe::plugin},
	Unit{"mbdel", &pluginlib::mbdel::plugin},
	Unit{"mbclip", &pluginlib::mbclipper::plugin},
	Unit{"bboom", &pluginlib::bassboom::plugin},
	Unit{"mbchor", &pluginlib::mbchor::plugin},
	Unit{"GCB_95", &pluginlib::gcb_95::plugin},
	Unit{"fuzzfacefm", &pluginlib::fuzzfacefm::plugin},
	Unit{"fuzzfacerm", &pluginlib::fuzzfacerm::plugin},
	Unit{"hornet", &pluginlib::hornet::plugin},
	Unit{"susta", &pluginlib::susta::plugin},
	Unit{"hfb", &pluginlib::hfb::plugin},
	Unit{"muff", &pluginlib::muff::plugin},
	Unit{"scream", &pluginlib::scream::plugin},
	Unit{"lpbboost", &pluginlib::lpbboost::plugin},
	Unit{"foxeylady", &pluginlib::foxeylady::plugin},
	Unit{"hogsfoot", &pluginlib::hogsfoot::plugin},
	Unit{"cstbt1", &pluginlib::cstb::plugin},
	Unit{"coverd", &pluginlib::coverd::plugin},
	Unit{"rangem", &pluginlib::rangem::plugin},
	Unit{"ruin", &pluginlib::ruin::plugin},
	Unit{"ffreak", &pluginlib::ffreak::plugin},
	Unit{"fumaster", &pluginlib::fumaster::plugin},
	Unit{"fuzzdrive", &pluginlib::fuzzdrive::plugin},
	Unit{"rolandwah", &pluginlib::rolandwah::plugin},
	Unit{"astrofuzz", &pluginlib::astrofuzz::plugin},
	Unit{"buffb", &pluginlib::buffb::plugin},
	Unit{"eldist", &pluginlib::eldist::plugin},
	Unit{"mole", &pluginlib::mole::plugin},
	Unit{"buzz", &pluginlib::buzz::plugin},
	Unit{"bfuzz", &pluginlib::bfuzz::plugin},
	Unit{"axface", &pluginlib::axface::plugin},
	Unit{"mbcs", &pluginlib::mbcs::plugin},
	Unit{"zita_rev1", &pluginlib::zita_rev1::plugin},
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
