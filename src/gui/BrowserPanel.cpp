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

#include <QButtonGroup>
#include <QHBoxLayout>
#include <QLabel>
#include <QStackedWidget>
#include <QTabBar>
#include <QToolButton>
#include <QVBoxLayout>

#include "ConfigManager.h"
#include "SideBarWidget.h"

namespace lmms::gui
{

BrowserPanel::BrowserPanel(QWidget* parent) :
	QWidget(parent),
	m_header(new QTabBar(this)),
	m_buttons(new QButtonGroup(this)),
	m_buttonRow(new QHBoxLayout),
	m_title(new QLabel(this)),
	m_pages(new QStackedWidget(this))
{
	setObjectName("browserPanel");
	setMinimumWidth(220);

	// Named like the other areas of the window
	m_header->setObjectName("browserHeader");
	m_header->setDocumentMode(true);
	m_header->setExpanding(false);
	m_header->addTab(tr("Browser"));

	m_buttons->setExclusive(true);
	// Aligned with the browsers' contents, which have 6 px margins
	m_buttonRow->setContentsMargins(6, 6, 6, 0);
	m_buttonRow->setSpacing(3);
	m_buttonRow->addStretch(1);

	m_title->setObjectName("browserTitle");
	m_title->setContentsMargins(6, 8, 6, 0);
	auto titleFont = m_title->font();
	titleFont.setBold(true);
	m_title->setFont(titleFont);

	auto layout = new QVBoxLayout(this);
	layout->setContentsMargins(0, 0, 0, 0);
	layout->setSpacing(0);
	layout->addWidget(m_header);
	layout->addLayout(m_buttonRow);
	layout->addWidget(m_title);
	layout->addWidget(m_pages, 1);

	connect(m_buttons, &QButtonGroup::idClicked, this, [this](int index) { select(index); });
}




void BrowserPanel::addBrowser(SideBarWidget* browser)
{
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
	m_buttonRow->insertWidget(index, button);

	if (index == 0) { select(0, false); }
}




void BrowserPanel::restoreState()
{
	const int index = ConfigManager::inst()->value("ui", "browsertab", "0").toInt();
	select(index >= 0 && index < m_pages->count() ? index : 0);

	setVisible(ConfigManager::inst()->value("ui", "browservisible", "1").toInt() != 0);
}




void BrowserPanel::setShown(bool shown)
{
	setVisible(shown);
	ConfigManager::inst()->setValue("ui", "browservisible", shown ? "1" : "0");
}




void BrowserPanel::select(int index, bool remember)
{
	if (index < 0 || index >= m_pages->count()) { return; }

	m_pages->setCurrentIndex(index);
	if (auto button = m_buttons->button(index)) { button->setChecked(true); }

	const auto browser = qobject_cast<SideBarWidget*>(m_pages->widget(index));
	m_title->setText(browser ? browser->title().toUpper() : QString{});
	if (remember) { ConfigManager::inst()->setValue("ui", "browsertab", QString::number(index)); }
}

} // namespace lmms::gui
