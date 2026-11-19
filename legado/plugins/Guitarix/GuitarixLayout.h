/*
 * GuitarixLayout.h - a Guitarix unit's controls, laid out as Guitarix lays them out
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

#ifndef LMMS_GUI_GUITARIX_LAYOUT_H
#define LMMS_GUI_GUITARIX_LAYOUT_H

class QWidget;

namespace lmms
{

class GuitarixControls;

namespace gui::guitarix
{

/**
 * The controls of @p controls' unit as Guitarix's rack shows them, from the unit's own description of them (its
 * load_ui): its glade (GtkBuilder XML, inline or in a file of Guitarix's builder directory: its "rackbox", the unit
 * expanded) read here into Qt widgets. Every unit here has one, but one (trbuff) that describes no UI, which shows
 * none, as in Guitarix. (A unit that only had the calls of Guitarix's stack builder would get its parameters in a
 * row: none does.) The widgets are Guitarix's (GuitarixWidgets.h) on the units' parameters.
 */
auto buildControls(GuitarixControls* controls, QWidget* parent) -> QWidget*;

} // namespace gui::guitarix

} // namespace lmms

#endif // LMMS_GUI_GUITARIX_LAYOUT_H
