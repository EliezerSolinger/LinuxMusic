/*
 * EffectBrowser.cpp - the effects page of the browser, to drag effects onto mixer channels
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

#include "EffectBrowser.h"

#include <map>

#include <QApplication>
#include <QHeaderView>
#include <QLabel>
#include <QLineEdit>
#include <QMouseEvent>
#include <QTreeWidget>
#include <QVBoxLayout>

#include "EffectSelectDialog.h"
#include "Engine.h"
#include "GuiApplication.h"
#include "Mixer.h"
#include "MixerView.h"
#include "PluginCategories.h"
#include "PluginInstance.h"
#include "StringPairDrag.h"
#include "embed.h"

namespace lmms::gui
{

namespace
{

constexpr int DragDataRole = Qt::UserRole;

//! Starts a StringPairDrag with the effect under the mouse
class EffectTree : public QTreeWidget
{
public:
	using QTreeWidget::QTreeWidget;

protected:
	void mousePressEvent(QMouseEvent* event) override
	{
		if (event->button() == Qt::LeftButton) { m_pressPos = event->pos(); }
		QTreeWidget::mousePressEvent(event);
	}

	void mouseMoveEvent(QMouseEvent* event) override
	{
		if (!(event->buttons() & Qt::LeftButton)
			|| (event->pos() - m_pressPos).manhattanLength() < QApplication::startDragDistance())
		{
			return QTreeWidget::mouseMoveEvent(event);
		}
		const auto item = itemAt(m_pressPos);
		if (!item || item->data(0, DragDataRole).toString().isEmpty()) { return; }
		new StringPairDrag("effect", item->data(0, DragDataRole).toString(),
			embed::getIconPixmap("effect_plugin"), this);
	}

private:
	QPoint m_pressPos;
};

} // namespace




EffectBrowser::EffectBrowser(QWidget* parent) :
	SideBarWidget(tr("Effects"), embed::getIconPixmap("fx_tab"), parent)
{
	auto view = new QWidget(contentParent());
	addContentWidget(view);

	auto layout = new QVBoxLayout(view);
	layout->setContentsMargins(5, 5, 5, 5);
	layout->setSpacing(5);

	auto hint = new QLabel(tr("Drag an effect onto a mixer channel or its effects, or double-click it to add it to "
		"the selected mixer channel."), view);
	hint->setWordWrap(true);

	auto searchBar = new QLineEdit(view);
	searchBar->setPlaceholderText(tr("Search"));
	searchBar->setClearButtonEnabled(true);
	searchBar->addAction(embed::getIconPixmap("zoom"), QLineEdit::LeadingPosition);
	connect(searchBar, &QLineEdit::textChanged, this, &EffectBrowser::filter);

	m_tree = new EffectTree(view);
	m_tree->setColumnCount(1);
	m_tree->header()->setVisible(false);
	m_tree->setIndentation(10);
	m_tree->setSelectionMode(QAbstractItemView::SingleSelection);

	// A double click adds the effect to the selected mixer channel
	connect(m_tree, &QTreeWidget::itemDoubleClicked, this, [](QTreeWidgetItem* item) {
		const auto data = item->data(0, DragDataRole).toString();
		const auto mixerView = getGUI()->mixerView();
		if (data.isEmpty() || !mixerView || !mixerView->currentMixerChannel()) { return; }
		auto& chain = Engine::mixer()->mixerChannel(mixerView->currentMixerChannel()->channelIndex())->m_fxChain;
		if (auto effect = EffectSelectDialog::instantiateDragged(data, &chain)) { chain.appendEffect(std::move(effect)); }
	});

	layout->addWidget(hint);
	layout->addWidget(searchBar);
	layout->addWidget(m_tree, 1);

	fill();
}




void EffectBrowser::fill()
{
	// By category for LinuxMusic's own effects, by host for the others (LADSPA, LV2...)
	auto groups = std::map<QString, std::vector<std::pair<QString, QString>>>{};
	for (const auto& key : EffectSelectDialog::availableEffects())
	{
		if (!key.isValid()) { continue; }
		QString group;
		QString name;
		if (key.desc->subPluginFeatures)
		{
			group = key.desc->displayName;
			name = key.displayName();
		}
		else
		{
			group = PluginCategories::categoryOf(key.desc);
			if (group.isEmpty()) { group = tr("Other"); }
			name = key.desc->displayName;
		}
		groups[group].emplace_back(name, EffectSelectDialog::dragData(key));
	}

	for (auto& [group, effects] : groups)
	{
		std::sort(effects.begin(), effects.end(), [](const auto& a, const auto& b) {
			return QString::localeAwareCompare(a.first, b.first) < 0;
		});
		auto groupItem = new QTreeWidgetItem(m_tree, QStringList{group});
		for (const auto& [name, data] : effects)
		{
			auto item = new QTreeWidgetItem(groupItem, QStringList{name});
			item->setData(0, DragDataRole, data);
			item->setToolTip(0, name);
		}
	}
}




void EffectBrowser::filter(const QString& text)
{
	for (int g = 0; g < m_tree->topLevelItemCount(); ++g)
	{
		auto groupItem = m_tree->topLevelItem(g);
		int shown = 0;
		for (int i = 0; i < groupItem->childCount(); ++i)
		{
			const bool matches = text.isEmpty() || groupItem->child(i)->text(0).contains(text, Qt::CaseInsensitive);
			groupItem->child(i)->setHidden(!matches);
			shown += matches ? 1 : 0;
		}
		groupItem->setHidden(shown == 0);
		groupItem->setExpanded(!text.isEmpty());
	}
}

} // namespace lmms::gui
