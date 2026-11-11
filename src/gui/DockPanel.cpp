/*
 * DockPanel.cpp - tab page that hosts an editor docked in the main window
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

#include "DockPanel.h"

#include <QEvent>
#include <QTabWidget>
#include <QVBoxLayout>

namespace lmms::gui
{

DockPanel::DockPanel(QWidget* content, QTabWidget* tabs, bool closable)
	: QWidget(tabs)
	, m_content(content)
	, m_tabs(tabs)
	, m_closable(closable)
{
	auto layout = new QVBoxLayout(this);
	layout->setContentsMargins(0, 0, 0, 0);
	layout->setSpacing(0);

	// Windows are created as top-level widgets (editors are QMainWindows); embed them as plain widgets.
	// Plugin windows have a fixed design size, so keep them in the top left corner instead of stretching them.
	content->setParent(this);
	content->setWindowFlags(Qt::Widget);
	if (closable) { layout->addWidget(content, 0, Qt::AlignLeft | Qt::AlignTop); }
	else { layout->addWidget(content); }
	content->show();
	content->installEventFilter(this);

	if (closable) { QWidget::setVisible(false); }
}




bool DockPanel::isShownInDock() const
{
	return m_tabs->indexOf(const_cast<DockPanel*>(this)) >= 0
		&& m_tabs->currentWidget() == this && m_tabs->isVisible() && m_tabs->height() > 0;
}




void DockPanel::removeFromTabs()
{
	const int index = m_tabs->indexOf(this);
	if (index >= 0) { m_tabs->removeTab(index); }
	QWidget::setVisible(false);
}




void DockPanel::setVisible(bool visible)
{
	if (m_selecting)
	{
		QWidget::setVisible(visible);
		return;
	}

	if (!visible)
	{
		// QTabWidget hides the previous page after switching tabs, so a hide request while we are still the
		// current tab comes from someone closing the window
		if (m_closable && m_tabs->currentWidget() == this) { removeFromTabs(); }
		else { QWidget::setVisible(false); }
		return;
	}

	// Someone asked to show the window: select our tab and let the main window reveal the dock area
	m_selecting = true;
	if (m_tabs->indexOf(this) < 0)
	{
		m_tabs->addTab(this, m_content->windowIcon(), m_content->windowTitle());
	}
	m_tabs->setCurrentWidget(this);
	QWidget::setVisible(true);
	m_content->show();
	m_selecting = false;

	emit revealRequested(this);
}




bool DockPanel::eventFilter(QObject* watched, QEvent* event)
{
	if (watched == m_content
		&& (event->type() == QEvent::WindowTitleChange || event->type() == QEvent::WindowIconChange))
	{
		updateTab();
	}
	return QWidget::eventFilter(watched, event);
}




void DockPanel::updateTab()
{
	const int index = m_tabs->indexOf(this);
	if (index < 0 || !m_closable) { return; }
	m_tabs->setTabText(index, m_content->windowTitle());
	m_tabs->setTabIcon(index, m_content->windowIcon());
}

} // namespace lmms::gui
