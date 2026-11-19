/*
 * BrowserPanel.h - the browsers (plugins, files...) in one panel with tabs on top
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

#ifndef LMMS_GUI_BROWSER_PANEL_H
#define LMMS_GUI_BROWSER_PANEL_H

#include <QWidget>

class QButtonGroup;
class QHBoxLayout;
class QLabel;
class QStackedWidget;
class QTabBar;

namespace lmms::gui
{

class SideBarWidget;

/**
 * The plugin and file browsers in one panel, as in the LMMS redesign concept: under the panel's "Browser" tab, a
 * row of small icon buttons selects a browser, which is named below them. It replaces the old side bar with
 * vertical buttons.
 * The selected tab and whether the panel is shown are remembered.
 */
class BrowserPanel : public QWidget
{
	Q_OBJECT
public:
	//! @param key names its settings (which page it shows, whether it is shown)
	explicit BrowserPanel(QWidget* parent, const QString& key = "browser");

	/**
	 * Adds a browser as a tab; its own title bar is hidden, since the tab names it.
	 * @param id names the browser in the settings (e.g. "projects"), so that it is remembered by name
	 */
	void addBrowser(SideBarWidget* browser, const QString& id);

	//! The index of the browser named @p id, or -1
	auto indexOf(const QString& id) const -> int;

	//! Selects the tab remembered from the last session; call after adding all browsers
	void restoreState();

	//! Shows or hides the panel and remembers it
	void setShown(bool shown);
	//! Whether the user wants it shown (when its page is shown)
	auto isShown() const -> bool { return m_shown; }
	//! Its page is the one shown (@p active) or not: it shows when both its page and the user want it
	void setPageActive(bool active);

	//! Shows browser @p index, and remembers it for the next session if @p remember
	void select(int index, bool remember = true);

private:
	QTabBar* m_header;
	QButtonGroup* m_buttons;
	QHBoxLayout* m_buttonRow;
	QStackedWidget* m_pages;
	QStringList m_ids;
	QString m_key;
	bool m_shown = true;
	bool m_pageActive = true;
};

} // namespace lmms::gui

#endif // LMMS_GUI_BROWSER_PANEL_H
