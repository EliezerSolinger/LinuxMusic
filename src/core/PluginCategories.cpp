/*
 * PluginCategories.cpp - categories for organizing the bundled instruments and effects
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

#include "PluginCategories.h"

#include <array>
#include <span>

#include <QCoreApplication>

namespace lmms::PluginCategories
{

namespace
{

struct Category
{
	const char* name; //!< untranslated, in the "PluginCategories" translation context
	QStringList plugins; //!< descriptor names (plugin library names)
};

// Categories are listed in display order
const auto s_instrumentCategories = std::array{
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Synthesizers"),
		{"tripleoscillator", "monstro", "watsyn", "organic", "bitinvader", "xpressive", "zynaddsubfx", "lb302"}},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Samplers"),
		{"audiofileprocessor", "sf2player", "gigplayer", "patman", "slicert"}},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Drums"), {"kicker", "avldrums"}},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Physical modeling"), {"vibedstrings", "malletsstk"}},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Chiptune"), {"freeboy", "nes", "sid", "opulenz", "sfxr"}},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Plugin hosts"), {"vestige", "carlarack", "carlapatchbay"}},
};

const auto s_effectCategories = std::array{
	Category{QT_TRANSLATE_NOOP("PluginCategories", "EQ & filters"),
		{"eq", "dualfilter", "crossovereq", "bassbooster"}},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Dynamics"), {"compressor", "dynamicsprocessor", "lomm"}},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Delay & reverb"), {"delay", "multitapecho", "reverbsc"}},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Modulation"), {"flanger"}},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Distortion"), {"bitcrush", "waveshaper", "slewdistortion"}},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Pitch & frequency"),
		{"granularpitchshifter", "frequencyshifter"}},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Stereo & utility"),
		{"amplifier", "stereoenhancer", "stereomatrix", "dispersion"}},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Analysis"), {"analyzer", "oscilloscope", "vectorscope"}},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Control"), {"peakcontrollereffect"}},
};

QString translated(const char* name)
{
	return QCoreApplication::translate("PluginCategories", name);
}

} // namespace




QString categoryOf(const Plugin::Descriptor* desc)
{
	const auto name = QString::fromUtf8(desc->name);
	for (const auto& list : {std::span<const Category>{s_instrumentCategories}, std::span<const Category>{s_effectCategories}})
	{
		for (const auto& category : list)
		{
			if (category.plugins.contains(name)) { return translated(category.name); }
		}
	}
	return {};
}




QStringList categories(Plugin::Type type)
{
	QStringList result;
	const auto list = type == Plugin::Type::Instrument
		? std::span<const Category>{s_instrumentCategories}
		: std::span<const Category>{s_effectCategories};
	for (const auto& category : list) { result << translated(category.name); }
	return result;
}

} // namespace lmms::PluginCategories
