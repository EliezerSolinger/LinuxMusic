/*
 * EffectBrowser.h - the effects page of the browser, to drag effects onto mixer channels
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

#ifndef LMMS_GUI_EFFECT_BROWSER_H
#define LMMS_GUI_EFFECT_BROWSER_H

#include "SideBarWidget.h"

class QTreeWidget;

namespace lmms::gui
{

/**
 * Lists the effects by category. An effect can be dragged onto a mixer channel, its effects rack or a track's
 * effects tab, or added to the selected mixer channel with a double click.
 */
class EffectBrowser : public SideBarWidget
{
	Q_OBJECT
public:
	explicit EffectBrowser(QWidget* parent);

private:
	void fill();
	void filter(const QString& text);

	QTreeWidget* m_tree;
};

} // namespace lmms::gui

#endif // LMMS_GUI_EFFECT_BROWSER_H
