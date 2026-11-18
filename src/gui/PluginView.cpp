/*
 * PluginView.cpp - makes a plugin's view
 *
 * Copyright (c) 2007-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
 *
 * This file is part of LMMS - https://lmms.io
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

#include "PluginView.h"
#include "DummyPlugin.h"

namespace lmms
{

gui::PluginView* Plugin::createView(QWidget* parent)
{
	gui::PluginView* pv = instantiateView(parent);
	// (a view made on this plugin has it already: setting it again would disconnect what the view's constructor
	// connected to the plugin's signals, see ModelView::setModel)
	if (pv != nullptr && pv->model() != this)
	{
		pv->setModel(this);
	}
	return pv;
}

} // namespace lmms


namespace lmms
{

gui::PluginView* DummyPlugin::instantiateView(QWidget* parent)
{
	return new gui::PluginView(this, parent);
}

} // namespace lmms
