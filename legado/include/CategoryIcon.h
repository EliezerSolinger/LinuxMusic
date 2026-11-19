/*
 * CategoryIcon.h - the icon of a plugin category: its colour and two letters
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

#ifndef LMMS_GUI_CATEGORY_ICON_H
#define LMMS_GUI_CATEGORY_ICON_H

#include <QPixmap>

#include "PluginCategories.h"

namespace lmms::gui
{

//! A plugin category's generic icon, @p size pixels square: a rounded square of its colour with its two letters
auto categoryIcon(const PluginCategories::Style& style, int size = 16) -> QPixmap;

//! A plugin's icon in the explorer: its own logo, framed by its category's colour; its category's icon without one
auto pluginIcon(const QPixmap& logo, const PluginCategories::Style& style, int size = 16) -> QPixmap;

} // namespace lmms::gui

#endif // LMMS_GUI_CATEGORY_ICON_H
