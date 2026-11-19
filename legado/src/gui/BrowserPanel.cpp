/*
 * BrowserPanel.cpp - the browsers (plugins, files...) in one panel with tabs on top
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

#include "BrowserPanel.h"

#include <algorithm>

#include <QButtonGroup>
#include <QHBoxLayout>
#include <QLabel>
#include <QStackedWidget>
#include <QTabBar>
#include <QToolButton>
#include <QVBoxLayout>

#include "ConfigManager.h"
#include "embed.h"
#include "SideBarWidget.h"

namespace lmms::gui
{

BrowserPanel::BrowserPanel(QWidget* parent, const QString& key) :
	QWidget(parent),
	m_header(new QTabBar(this)),
	m_buttons(new QButtonGroup(this)),
	m_buttonRow(new QHBoxLayout),
	m_pages(new QStackedWidget(this)),
	m_key(key)
{
	setObjectName("browserPanel");
	setMinimumWidth(220);

	// Named like the other areas of the window
	m_header->setObjectName("browserHeader");
	m_header->setDocumentMode(true);
	m_header->setExpanding(false);
	m_header->addTab(embed::getIconPixmap("explorer"), tr("Explorer"));

	m_buttons->setExclusive(true);
	// The buttons side by side, centered
	m_buttonRow->setContentsMargins(6, 6, 6, 2);
	m_buttonRow->setSpacing(0);
	m_buttonRow->addStretch(1);
	m_buttonRow->addStretch(1);

	auto layout = new QVBoxLayout(this);
	layout->setContentsMargins(0, 0, 0, 0);
	layout->setSpacing(0);
	layout->addWidget(m_header);
	layout->addLayout(m_buttonRow);
	layout->addWidget(m_pages, 1);

	connect(m_buttons, &QButtonGroup::idClicked, this, [this](int index) { select(index); });
}




void BrowserPanel::addBrowser(SideBarWidget* browser, const QString& id)
{
	m_ids << id;
	browser->setHeaderVisible(false);
	m_pages->addWidget(browser); // the stack shows the selected page only

	// A small button with the browser's icon; the selected browser is named below the buttons
	auto button = new QToolButton(this);
	button->setObjectName("browserButton");
	button->setIcon(browser->icon());
	button->setIconSize(QSize{16, 16});
	button->setFixedSize(26, 22);
	button->setCheckable(true);
	button->setAutoRaise(true);
	button->setToolTip(browser->title());
	const int index = m_pages->count() - 1;
	m_buttons->addButton(button, index);
	m_buttonRow->insertWidget(index + 1, button); // (after the leading stretch)

	if (index == 0) { select(0, false); }
}




void BrowserPanel::restoreState()
{
	const auto config = ConfigManager::inst();
	auto id = config->value("ui", m_key + "page");
	if (id.isEmpty() && m_key == "browser")
	{
		// Settings from before the pages had names, which kept the index in the order of back then
		const auto before = QStringList{"plugins", "favorites", "projects", "samples", "presets", "files"};
		id = before.value(config->value("ui", "browsertab", "0").toInt());
	}
	select(std::max(indexOf(id), 0));

	m_shown = config->value("ui", m_key + "visible", "1").toInt() != 0;
	setVisible(m_shown && m_pageActive);
}




void BrowserPanel::setShown(bool shown)
{
	m_shown = shown;
	setVisible(m_shown && m_pageActive);
	ConfigManager::inst()->setValue("ui", m_key + "visible", shown ? "1" : "0");
}




void BrowserPanel::setPageActive(bool active)
{
	m_pageActive = active;
	setVisible(m_shown && m_pageActive);
}




void BrowserPanel::select(int index, bool remember)
{
	if (index < 0 || index >= m_pages->count()) { return; }

	m_pages->setCurrentIndex(index);
	if (auto button = m_buttons->button(index)) { button->setChecked(true); }

	if (remember) { ConfigManager::inst()->setValue("ui", m_key + "page", m_ids.value(index)); }
}




auto BrowserPanel::indexOf(const QString& id) const -> int
{
	return m_ids.indexOf(id);
}

} // namespace lmms::gui
