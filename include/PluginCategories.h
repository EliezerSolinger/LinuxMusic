/*
 * PluginCategories.h - categories for organizing the bundled instruments and effects
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

#ifndef LMMS_PLUGIN_CATEGORIES_H
#define LMMS_PLUGIN_CATEGORIES_H

#include <QColor>
#include <QString>
#include <QStringList>

#include "lmms_export.h"
#include "Plugin.h"

namespace lmms
{

//! Categories used to organize the bundled (native) instruments and effects in the plugin browser
//! and the effect selection dialog, similar to how FL Studio groups its plugins.
namespace PluginCategories
{

//! @returns the translated category of the native plugin with descriptor @p desc, or an empty
//! string for plugins without a category (e.g. hosts of external plugins such as LV2 or LADSPA)
LMMS_EXPORT QString categoryOf(const Plugin::Descriptor* desc);

//! @returns the translated categories for plugins of type @p type, in display order
LMMS_EXPORT QStringList categories(Plugin::Type type);

//! @returns the translated subcategory (the second level inside a category that has many plugins, as the
//! synthesizers by how they make sound) of the plugin with descriptor @p desc, or an empty string
LMMS_EXPORT QString subcategoryOf(const Plugin::Descriptor* desc);

//! @returns the translated subcategories of the (translated) category @p category, in display order
LMMS_EXPORT QStringList subcategories(const QString& category);

//! How a category shows: its colour (EQ blue, dynamics yellow...) and the two letters on its generic icon
struct Style
{
	QColor color;
	QString code;
};
//! The style of the (translated) category @p category; a grey one for plugins of no category
LMMS_EXPORT auto styleOf(const QString& category) -> Style;
//! The style of the category of the plugin with descriptor @p desc
LMMS_EXPORT auto styleOf(const Plugin::Descriptor* desc) -> Style;

//! @returns whether the plugin with descriptor @p desc is retired: it is not offered any more (what it did is part
//! of the mixer), but projects that use it still load and play it
LMMS_EXPORT bool isRetired(const Plugin::Descriptor* desc);

} // namespace PluginCategories

} // namespace lmms

#endif // LMMS_PLUGIN_CATEGORIES_H
