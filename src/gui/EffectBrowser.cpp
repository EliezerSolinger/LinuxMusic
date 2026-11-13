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

#include "CategoryIcon.h"
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

	auto searchBar = m_search = new QLineEdit(view);
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

	layout->addWidget(searchBar);
	layout->addWidget(m_tree, 1);

	fill();
}




void EffectBrowser::focusSearch()
{
	m_search->setFocus(Qt::OtherFocusReason);
	m_search->selectAll();
}




void EffectBrowser::fill()
{
	// By category for LinuxMusic's own effects, by host for the others (LADSPA, LV2...)
	struct Entry
	{
		QString name;
		QString data;
		QPixmap logo;
	};
	auto groups = std::map<QString, std::vector<Entry>>{};
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
		const auto* logo = key.logo();
		groups[group].push_back({name, EffectSelectDialog::dragData(key), logo ? logo->pixmap() : QPixmap{}});
	}

	for (auto& [group, effects] : groups)
	{
		std::sort(effects.begin(), effects.end(), [](const auto& a, const auto& b) {
			return QString::localeAwareCompare(a.name, b.name) < 0;
		});
		// Coloured by category (EQ blue, dynamics yellow...): its icon on the group, each effect its own logo framed
		// in that colour, or the category's icon
		const auto style = PluginCategories::styleOf(group);
		auto groupItem = new QTreeWidgetItem(m_tree, QStringList{group});
		groupItem->setIcon(0, categoryIcon(style));
		for (const auto& effect : effects)
		{
			auto item = new QTreeWidgetItem(groupItem, QStringList{effect.name});
			item->setIcon(0, pluginIcon(effect.logo, style));
			item->setData(0, DragDataRole, effect.data);
			item->setToolTip(0, effect.name);
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
