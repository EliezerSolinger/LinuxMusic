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

#include <QTabWidget>
#include <QVBoxLayout>

namespace lmms::gui
{

DockPanel::DockPanel(QWidget* content, QTabWidget* tabs)
	: QWidget(tabs)
	, m_content(content)
	, m_tabs(tabs)
{
	auto layout = new QVBoxLayout(this);
	layout->setContentsMargins(0, 0, 0, 0);
	layout->setSpacing(0);

	// Editors are created as top-level windows (most are QMainWindows); embed them as plain widgets
	content->setParent(this);
	content->setWindowFlags(Qt::Widget);
	layout->addWidget(content);
	content->show();
}




bool DockPanel::isShownInDock() const
{
	return m_tabs->currentWidget() == this && m_tabs->isVisible() && m_tabs->height() > 0;
}




void DockPanel::setVisible(bool visible)
{
	// Hiding comes from QTabWidget switching tabs; let it through
	if (!visible || m_selecting)
	{
		QWidget::setVisible(visible);
		return;
	}

	// Someone asked to show the editor: select our tab and let the main window reveal the dock area
	m_selecting = true;
	m_tabs->setCurrentWidget(this);
	QWidget::setVisible(true);
	m_selecting = false;

	emit revealRequested(this);
}

} // namespace lmms::gui
