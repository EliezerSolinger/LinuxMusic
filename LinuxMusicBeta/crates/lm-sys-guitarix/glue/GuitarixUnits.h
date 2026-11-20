/*
 * GuitarixUnits.h - the Guitarix units built into LinuxMusic
 *
 * The units' DSP is Guitarix's own (https://github.com/brummer10/guitarix), by Hermann Meyer, James Warden, Andreas
 * Degert, Pete Shorthose and its other authors, GPL-2.0 or later; with zita-resampler by Fons Adriaensen (GPL-3.0
 * or later), so that they sound as in Guitarix: see gx/COPYING and the headers of the files in gx/.
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

#ifndef LMMS_GUITARIX_UNITS_H
#define LMMS_GUITARIX_UNITS_H

#include <cstddef>
#include <span>

struct PluginDef;

namespace lmms::guitarix
{


//! A unit of Guitarix: its id (Guitarix's) and how one is made (Guitarix's own factory: a PluginDef, the unit's
//! interface, with its name and category, deleted by its delete_instance)
struct Unit
{
	const char* id;
	PluginDef* (*make)();
	//! What the unit does on the main thread when its parameters changed (Guitarix calls its convolvers'
	//! check_update every 200 ms on its UI's thread: a new impulse response), with no period rendered meanwhile
	void (*update)(PluginDef*) = nullptr;
};

//! Guitarix's units that are a choice among others (GuitarixSelectors.cpp)
auto makeCrybaby() -> PluginDef*;
auto makeWah() -> PluginDef*;
auto makeTonestack() -> PluginDef*;
auto makePoweramp() -> PluginDef*;

//! Guitarix's convolving units (GuitarixConvolution.cpp), and their updates
auto makeCabinet() -> PluginDef*;
auto makePreamp() -> PluginDef*;
void updateCabinet(PluginDef* plugin);
void updatePreamp(PluginDef* plugin);
//! The amp head's presence (Guitarix's ContrastConvolver), and the amp head's amps (its ampstack)
auto makeContrast() -> PluginDef*;
void updateContrast(PluginDef* plugin);
auto makeAmpstack() -> PluginDef*;

//! Guitarix's Detune (GuitarixDetune.cpp), and its update (its latency setting)
auto makeDetune() -> PluginDef*;
void updateDetune(PluginDef* plugin);

//! The amp head (GuitarixAmp.cpp)
auto makeAmpHead() -> PluginDef*;
void updateAmpHead(PluginDef* plugin);

//! The Vibe (GuitarixVibe.cpp), mono and stereo, each made on zeroed memory (see there)
auto makeVibeMono() -> PluginDef*;
auto makeVibeStereo() -> PluginDef*;

//! How many frames a unit processes at a time: always the same, as in Guitarix (its JACK period). LinuxMusicBeta:
//! the block the effect gives the unit, set by the shim on the thread that starts or updates the unit, before it
//! does (lm-sys-guitarix, shim.cpp); the legacy program worked it out from its engine's period
auto blockFrames() -> unsigned int;
//! Sets what blockFrames() gives on this thread
void setBlockFrames(unsigned int frames);

//! Every unit, in the browser's order
auto units() -> std::span<const Unit>;

//! The unit @p id, or none
auto unit(const char* id) -> const Unit*;

} // namespace lmms::guitarix

#endif // LMMS_GUITARIX_UNITS_H
