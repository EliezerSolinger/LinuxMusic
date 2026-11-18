/*
 * PluginBrowser.cpp - implementation of the plugin-browser
 *
 * Copyright (c) 2005-2009 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "PluginBrowser.h"

#include "CategoryIcon.h"

#include <map>

#include <QHeaderView>
#include <QLabel>
#include <QLineEdit>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QStyleOption>
#include <QTreeWidget>

#include "embed.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "Song.h"
#include "StringPairDrag.h"
#include "TrackContainerView.h"
#include "PluginCategories.h"
#include "PluginFactory.h"

namespace lmms::gui
{


PluginBrowser::PluginBrowser( QWidget * _parent ) :
	SideBarWidget( tr( "Instrument Plugins" ),
				embed::getIconPixmap( "plugins" ).transformed( QTransform().rotate( 90 ) ), _parent )
{
	setWindowTitle( tr( "Instrument browser" ) );
	m_view = new QWidget( contentParent() );
	//m_view->setFrameShape( QFrame::NoFrame );

	addContentWidget( m_view );

	auto view_layout = new QVBoxLayout(m_view);
	view_layout->setContentsMargins(5, 5, 5, 5);
	view_layout->setSpacing( 5 );


	auto searchBar = new QLineEdit(m_view);
	searchBar->setPlaceholderText(tr("Search"));
	searchBar->setMaxLength(64);
	searchBar->setClearButtonEnabled(true);
	searchBar->addAction(embed::getIconPixmap("zoom"), QLineEdit::LeadingPosition);

	m_descTree = new QTreeWidget( m_view );
	m_descTree->setColumnCount( 1 );
	m_descTree->header()->setVisible( false );
	m_descTree->setIndentation( 10 );
	m_descTree->setSelectionMode( QAbstractItemView::NoSelection );

	connect( searchBar, SIGNAL( textEdited( const QString& ) ),
			this, SLOT( onFilterChanged( const QString& ) ) );

	view_layout->addWidget( searchBar );
	view_layout->addWidget( m_descTree );

	// Add plugins to the tree
	addPlugins();

	// Resize
	m_descTree->header()->setSectionResizeMode( QHeaderView::ResizeToContents );

	// Hide empty roots
	updateRootVisibilities();
}


void PluginBrowser::updateRootVisibility( int rootIndex )
{
	QTreeWidgetItem * root = m_descTree->topLevelItem( rootIndex );
	root->setHidden( !root->childCount() );
}


void PluginBrowser::updateRootVisibilities()
{
	int rootCount = m_descTree->topLevelItemCount();
	for (int rootIndex = 0; rootIndex < rootCount; ++rootIndex)
	{
		updateRootVisibility( rootIndex );
	}
}


void PluginBrowser::onFilterChanged( const QString & filter )
{
	// A plugin shows when its name has the text; a subcategory, while one of its plugins shows
	const auto filterItem = [&](auto&& self, QTreeWidgetItem* item) -> bool {
		if (auto* desc = static_cast<PluginDescWidget*>(m_descTree->itemWidget(item, 0)))
		{
			const bool shown = desc->name().contains(filter, Qt::CaseInsensitive);
			item->setHidden(!shown);
			return shown;
		}
		auto anyShown = false;
		for (int child = 0; child < item->childCount(); ++child) { anyShown = self(self, item->child(child)) || anyShown; }
		return anyShown;
	};
	for (int rootIndex = 0; rootIndex < m_descTree->topLevelItemCount(); ++rootIndex)
	{
		auto* root = m_descTree->topLevelItem(rootIndex);
		for (int child = 0; child < root->childCount(); ++child)
		{
			auto* item = root->child(child);
			const bool shown = filterItem(filterItem, item);
			// (a subcategory with nothing found goes; a plugin was set by filterItem)
			if (!m_descTree->itemWidget(item, 0)) { item->setHidden(!shown); }
		}
	}
}


void PluginBrowser::addPlugins()
{
	// Add a root node to the plugin tree with the specified `label` and return it
	const auto addRoot = [this](auto label)
	{
		const auto root = new QTreeWidgetItem();
		root->setText(0, label);
		// Coloured by category, like the effects
		root->setIcon(0, categoryIcon(PluginCategories::styleOf(QString{label})));
		m_descTree->addTopLevelItem(root);
		return root;
	};

	// Add the plugin identified by `key` to the tree under the root node `root`
	const auto addPlugin = [this](const auto& key, auto root)
	{
		const auto item = new QTreeWidgetItem();
		root->addChild(item);
		m_descTree->setItemWidget(item, 0, new PluginDescWidget(key, m_descTree));
	};

	// Remove any existing plugins from the tree
	m_descTree->clear();

	// Fetch and sort all instrument plugin descriptors
	auto descs = getPluginFactory()->descriptors(Plugin::Type::Instrument);
	std::sort(descs.begin(), descs.end(),
		[](auto d1, auto d2)
		{
			return qstricmp(d1->displayName, d2->displayName) < 0;
		}
	);

	// Add a root node per category for the native plugins (with a node per subcategory inside it, when it has
	// them); plugins without a category go last
	auto categoryRoots = std::map<QString, QTreeWidgetItem*>{};
	auto subcategoryRoots = std::map<std::pair<QString, QString>, QTreeWidgetItem*>{};
	for (const auto& category : PluginCategories::categories(Plugin::Type::Instrument))
	{
		const auto root = addRoot(category);
		root->setExpanded(true);
		categoryRoots[category] = root;
		for (const auto& subcategory : PluginCategories::subcategories(category))
		{
			auto* item = new QTreeWidgetItem(root, {subcategory});
			item->setFlags(Qt::ItemIsEnabled);
			auto font = item->font(0);
			font.setPointSizeF(font.pointSizeF() * 0.9);
			item->setFont(0, font);
			item->setForeground(0, QColor{140, 150, 163});
			item->setExpanded(true);
			subcategoryRoots[{category, subcategory}] = item;
		}
	}
	QTreeWidgetItem* otherRoot = nullptr;
	const auto rootFor = [&](const Plugin::Descriptor* desc) {
		const auto category = PluginCategories::categoryOf(desc);
		if (const auto it = subcategoryRoots.find({category, PluginCategories::subcategoryOf(desc)});
			it != subcategoryRoots.end())
		{
			return it->second;
		}
		if (const auto it = categoryRoots.find(category); it != categoryRoots.end())
		{
			return it->second;
		}
		if (!otherRoot) { otherRoot = addRoot(tr("Other")); }
		return otherRoot;
	};

	// Add all of the descriptors to the tree
	for (const auto desc : descs)
	{
		if (desc->subPluginFeatures)
		{
			// Fetch and sort all subplugins for this plugin descriptor
			auto subPluginKeys = Plugin::Descriptor::SubPluginFeatures::KeyList{};
			desc->subPluginFeatures->listSubPluginKeys(desc, subPluginKeys);
			std::sort(subPluginKeys.begin(), subPluginKeys.end(),
				[](const auto& l, const auto& r)
				{
					return QString::compare(l.displayName(), r.displayName(), Qt::CaseInsensitive) < 0;
				}
			);

			// Create a root node for this plugin and add the subplugins under it
			const auto root = addRoot(desc->displayName);
			for (const auto& key : subPluginKeys) { addPlugin(key, root); }
		}
		else
		{
			addPlugin(Plugin::Descriptor::SubPluginFeatures::Key(desc, desc->name), rootFor(desc));
		}
	}
	// (a subcategory none of whose plugins was built here goes)
	for (auto& [key, item] : subcategoryRoots) { item->setHidden(item->childCount() == 0); }
}




PluginDescWidget::PluginDescWidget(const PluginKey &_pk,
							QWidget * _parent ) :
	QWidget( _parent ),
	m_pluginKey( _pk ),
	m_logo( _pk.logo()->pixmap() ),
	m_mouseOver( false )
{
	setFixedHeight( DEFAULT_HEIGHT );
	setMouseTracking( true );
	setCursor( Qt::PointingHandCursor );
	setToolTip(_pk.desc->subPluginFeatures
		? _pk.description()
		: tr(_pk.desc->description));
}




QString PluginDescWidget::name() const
{
	return m_pluginKey.displayName();
}




void PluginDescWidget::paintEvent( QPaintEvent * )
{

	QPainter p( this );

	// Paint everything according to the style sheet
	QStyleOption o;
	o.initFrom( this );
	style()->drawPrimitive( QStyle::PE_Widget, &o, &p, this );

	// Draw the rest
	const int s = 16 + ( 32 * ( qBound( 24, height(), 60 ) - 24 ) ) /
								( 60 - 24 );
	const QSize logo_size( s, s );
	QPixmap logo = m_logo.scaled( logo_size, Qt::KeepAspectRatio,
						Qt::SmoothTransformation );
	p.drawPixmap( 4, 4, logo );

	QFont f = p.font();
	if ( m_mouseOver )
	{
		f.setBold( true );
	}

	p.setFont( f );
	p.drawText( 10 + logo_size.width(), 15, m_pluginKey.displayName());
}


void PluginDescWidget::enterEvent(QEnterEvent* event)
{
	m_mouseOver = true;

	QWidget::enterEvent(event);
}




void PluginDescWidget::leaveEvent( QEvent * _e )
{
	m_mouseOver = false;

	QWidget::leaveEvent( _e );
}




void PluginDescWidget::mousePressEvent( QMouseEvent * _me )
{
	Engine::setDndPluginKey(&m_pluginKey);
	if ( _me->button() == Qt::LeftButton )
	{
		new StringPairDrag("instrument",
			QString::fromUtf8(m_pluginKey.desc->name), m_logo, this);
		leaveEvent( _me );
	}
}


void PluginDescWidget::contextMenuEvent(QContextMenuEvent* e)
{
	QMenu contextMenu(this);
	contextMenu.addAction(
		tr("Send to new instrument track"),
		[=, this]{ openInNewInstrumentTrack(m_pluginKey.desc->name); }
	);
	contextMenu.exec(e->globalPos());
}


void PluginDescWidget::openInNewInstrumentTrack(QString value)
{
	TrackContainer* tc = Engine::getSong();
	auto it = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, tc));
	auto ilt = new InstrumentLoaderThread(this, it, value);
	ilt->start();
}


} // namespace lmms::gui
