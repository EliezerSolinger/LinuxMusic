/*
 * DockPanel.h - tab page that hosts an editor docked in the main window
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

#ifndef LMMS_GUI_DOCK_PANEL_H
#define LMMS_GUI_DOCK_PANEL_H

#include <QWidget>

class QTabWidget;

namespace lmms::gui
{

//! A tab page in one of the main window's dock areas, wrapping a single editor.
//!
//! Lots of code shows an editor with `editor->parentWidget()->show()`, which used to show the editor's
//! MDI subwindow. When the editor is docked, its parent is a DockPanel, and showing it selects its tab
//! and asks the main window to reveal the dock area instead.
class DockPanel : public QWidget
{
	Q_OBJECT
public:
	DockPanel(QWidget* content, QTabWidget* tabs);

	QWidget* content() const { return m_content; }
	QTabWidget* tabs() const { return m_tabs; }

	//! @returns true if this panel is the selected tab of a dock area that is not collapsed
	bool isShownInDock() const;

	void setVisible(bool visible) override;

signals:
	//! Emitted when something asks to show this panel, so the main window can reveal its dock area
	void revealRequested(DockPanel* panel);

private:
	QWidget* m_content;
	QTabWidget* m_tabs;
	bool m_selecting = false;
};

} // namespace lmms::gui

#endif // LMMS_GUI_DOCK_PANEL_H
