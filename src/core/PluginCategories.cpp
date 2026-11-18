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
	const char* color; //!< its colour, for its icons
	const char* code; //!< two letters on its generic icon
};

// Categories are listed in display order
const auto s_instrumentCategories = std::array{
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Synthesizers"),
		{"tripleoscillator", "monstro", "watsyn", "organic", "bitinvader", "xpressive", "zynaddsubfx", "lb302",
			"vibedstrings"}, "#4afd85", "SY"},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Samplers"),
		{"audiofileprocessor", "sf2player", "gigplayer", "patman", "slicert"}, "#f0913a", "SA"},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Drums"), {"kicker", "avldrums"}, "#e85a4f", "DR"},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Physical modeling"), {"malletsstk"}, "#c8a06b", "PM"},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Chiptune"), {"freeboy", "nes", "sid", "opulenz", "sfxr"}, "#9b6be8", "CH"},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Plugin hosts"), {"vestige", "carlarack", "carlapatchbay"}, "#9aa3ad", "PH"},
};

const auto s_effectCategories = std::array{
	Category{QT_TRANSLATE_NOOP("PluginCategories", "EQ & filters"),
		{"eq", "dualfilter", "crossovereq", "bassbooster"}, "#3d8be8", "EQ"},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Dynamics"), {"compressor", "dynamicsprocessor", "lomm"}, "#e8c23d", "DY"},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Delay & reverb"), {"delay", "multitapecho", "reverbsc"}, "#9b6be8", "RV"},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Modulation"), {"flanger"}, "#4fc96b", "MO"},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Distortion"), {"bitcrush", "waveshaper", "slewdistortion", "ampandir"}, "#e85a4f", "DS"},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Pitch & frequency"),
		{"granularpitchshifter", "frequencyshifter"}, "#f0913a", "PI"},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Stereo & utility"),
		{"stereoenhancer", "dispersion"}, "#9aa3ad", "ST"},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Analysis"), {"analyzer", "oscilloscope", "vectorscope"}, "#3dc8d8", "AN"},
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Control"), {"peakcontrollereffect"}, "#d36bb8", "CT"},
	// (Guitarix's pedals, amps and cabinets: its units are the plugin's sub-plugins, see plugins/Guitarix)
	Category{QT_TRANSLATE_NOOP("PluginCategories", "Guitarix"), {"guitarix"}, "#e8743d", "GX"},
};

//! A second level inside a category, when it has many plugins (the synthesizers, by how they make sound)
struct Subcategory
{
	const char* category; //!< the category's untranslated name
	const char* name; //!< untranslated, in the "PluginCategories" translation context
	QStringList plugins;
};

// In display order inside their category
const auto s_subcategories = std::array{
	Subcategory{"Synthesizers", QT_TRANSLATE_NOOP("PluginCategories", "Subtractive"),
		{"tripleoscillator", "lb302", "monstro"}},
	Subcategory{"Synthesizers", QT_TRANSLATE_NOOP("PluginCategories", "Wavetable"), {"watsyn", "bitinvader"}},
	Subcategory{"Synthesizers", QT_TRANSLATE_NOOP("PluginCategories", "Additive"), {"organic", "zynaddsubfx"}},
	Subcategory{"Synthesizers", QT_TRANSLATE_NOOP("PluginCategories", "Strings"), {"vibedstrings"}},
	Subcategory{"Synthesizers", QT_TRANSLATE_NOOP("PluginCategories", "Formulas"), {"xpressive"}},
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




QString subcategoryOf(const Plugin::Descriptor* desc)
{
	const auto name = QString::fromUtf8(desc->name);
	for (const auto& subcategory : s_subcategories)
	{
		if (subcategory.plugins.contains(name)) { return translated(subcategory.name); }
	}
	return {};
}




QStringList subcategories(const QString& category)
{
	auto result = QStringList{};
	for (const auto& subcategory : s_subcategories)
	{
		if (translated(subcategory.category) == category) { result << translated(subcategory.name); }
	}
	return result;
}




bool isRetired(const Plugin::Descriptor* desc)
{
	// What these did is in every mixer channel: the trim, the fader, PAN, phase, swap and MONO (the user's notes of
	// 13/11/2026)
	static const auto retired = QStringList{"amplifier", "stereomatrix"};
	return desc && retired.contains(QString::fromUtf8(desc->name));
}

auto styleOf(const QString& category) -> Style
{
	for (const auto& list : {std::span<const Category>{s_instrumentCategories}, std::span<const Category>{s_effectCategories}})
	{
		for (const auto& entry : list)
		{
			if (translated(entry.name) == category) { return {QColor{entry.color}, QString::fromLatin1(entry.code)}; }
		}
	}
	// The plugins of no category (hosted ones, by host: LADSPA, LV2...)
	return {QColor{"#7c8591"}, category.left(2).toUpper()};
}




auto styleOf(const Plugin::Descriptor* desc) -> Style
{
	return styleOf(categoryOf(desc));
}

} // namespace lmms::PluginCategories
