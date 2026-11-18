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
#include "gx_faust_plugins.h"
#include "pluginlib.h"

namespace lmms::guitarix
{

namespace
{

// In the order of Guitarix's rack (gx_engine.cpp: its mono units, then its stereo ones)
const auto s_units = std::array{
	Unit{"gain", &gx_engine::gx_effects::gain::plugin},
	Unit{"low_highpass", &gx_engine::gx_effects::low_high_pass::plugin},
	Unit{"highbooster", &gx_engine::gx_effects::highbooster::plugin},
	Unit{"eqs", &gx_engine::gx_effects::selecteq::plugin},
	Unit{"crybaby", &makeCrybaby},
	Unit{"poweramp", &makePoweramp},
	Unit{"wah", &makeWah},
	Unit{"gx_distortion", &gx_engine::gx_effects::gx_distortion::plugin},
	Unit{"bitdowner", &gx_engine::gx_effects::bitdowner::plugin},
	Unit{"thick_distortion", &gx_engine::gx_effects::thick_distortion::plugin},
	Unit{"ts9sim", &pluginlib::ts9sim::plugin},
	Unit{"aclipper", &pluginlib::aclipper::plugin},
	Unit{"mxrdis", &pluginlib::mxrdist::plugin},
	Unit{"bossds1", &pluginlib::bossds1::plugin},
	Unit{"bmp", &pluginlib::bmp::plugin},
	Unit{"IR", &gx_engine::gx_effects::impulseresponse::plugin},
	Unit{"compressor", &gx_engine::gx_effects::compressor::plugin},
	Unit{"expander", &gx_engine::gx_effects::expander::plugin},
	Unit{"overdrive", &gx_engine::gx_effects::overdrive::plugin},
	Unit{"echo", &gx_engine::gx_effects::echo::plugin},
	Unit{"delay", &gx_engine::gx_effects::delay::plugin},
	Unit{"freeverb", &gx_engine::gx_effects::freeverb::plugin},
	Unit{"biquad", &gx_engine::gx_effects::biquad::plugin},
	Unit{"tremolo", &gx_engine::gx_effects::tremolo::plugin},
	Unit{"phaser_mono", &gx_engine::gx_effects::phaser_mono::plugin},
	Unit{"chorus_mono", &gx_engine::gx_effects::chorus_mono::plugin},
	Unit{"flanger_mono", &gx_engine::gx_effects::flanger_mono::plugin},
	Unit{"flanger_mono_gx", &pluginlib::flanger_gx::plugin},
	Unit{"feedback", &gx_engine::gx_effects::gx_feedback::plugin},
	Unit{"eq", &gx_engine::gx_effects::peak_eq::plugin},
	Unit{"dide", &gx_engine::gx_effects::digital_delay::plugin},
	Unit{"amp.tonestack", &makeTonestack},
	Unit{"abgate", &pluginlib::abgate::plugin},
	Unit{"ring_modulator", &gx_engine::gx_effects::ring_modulator::plugin},
	Unit{"graphiceq", &gx_engine::gx_effects::graphiceq::plugin},
	Unit{"univibe_mono", &makeVibeMono, &vibeMonoLfoState},
	Unit{"mbc", &pluginlib::mbc::plugin},
	Unit{"mbd", &pluginlib::mbd::plugin},
	Unit{"mbe", &pluginlib::mbe::plugin},
	Unit{"mbdel", &pluginlib::mbdel::plugin},
	Unit{"mbclip", &pluginlib::mbclipper::plugin},
	Unit{"bboom", &pluginlib::bassboom::plugin},
	Unit{"mbchor", &pluginlib::mbchor::plugin},
	Unit{"GCB_95", &pluginlib::gcb_95::plugin},
	Unit{"duckDelay", &gx_engine::gx_effects::duck_delay::plugin},
	Unit{"reversedelay", &pluginlib::reversedelay::plugin},
	Unit{"baxandall", &gx_engine::gx_effects::baxandall::plugin},
	Unit{"distortion2", &gx_engine::gx_effects::distortion2::plugin},
	Unit{"fuzzface", &gx_engine::gx_effects::fuzzface::plugin},
	Unit{"trbuff", &gx_engine::gx_effects::trbuff::plugin},
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
	Unit{"chorus", &gx_engine::gx_effects::chorus::plugin},
	Unit{"flanger", &gx_engine::gx_effects::flanger::plugin},
	Unit{"phaser", &gx_engine::gx_effects::phaser::plugin},
	Unit{"stereodelay", &gx_engine::gx_effects::stereodelay::plugin},
	Unit{"stereoecho", &gx_engine::gx_effects::stereoecho::plugin},
	Unit{"moog", &gx_engine::gx_effects::moog::plugin},
	Unit{"ampmodul", &gx_engine::gx_amps::gx_ampmodul::plugin},
	Unit{"tonemodul", &gx_engine::gx_effects::tonecontroll::plugin},
	Unit{"didest", &gx_engine::gx_effects::digital_delay_st::plugin},
	Unit{"stereoverb", &gx_engine::gx_effects::stereoverb::plugin},
	Unit{"zita_rev1", &pluginlib::zita_rev1::plugin},
	Unit{"univibe", &makeVibeStereo, &vibeLfoState},
	Unit{"mbcs", &pluginlib::mbcs::plugin},
	Unit{"dattorros_progenitor", &gx_engine::gx_effects::dattorros_progenitor::plugin},
	Unit{"panoram_enhancer", &gx_engine::gx_effects::panoram_enhancer::plugin},
	Unit{"bassEnhancer", &gx_engine::gx_effects::bass_enhancer::plugin},
	Unit{"ringModulatorSt", &gx_engine::gx_effects::ring_modulator_st::plugin},
	Unit{"duckDelaySt", &gx_engine::gx_effects::duck_delay_st::plugin},
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
