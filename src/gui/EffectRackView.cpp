/*
 * EffectRackView.cpp - view for effectChain model
 *
 * Copyright (c) 2006-2007 Danny McRae <khjklujn@netscape.net>
 * Copyright (c) 2008-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "EffectRackView.h"

#include <QApplication>
#include <QDir>
#include <QDropEvent>
#include <QFileDialog>
#include <QMessageBox>
#include <QAction>
#include <QPushButton>
#include <QScrollArea>
#include <QVBoxLayout>

#include "ConfigManager.h"
#include "DataFile.h"
#include "DeprecationHelper.h"
#include "EffectSelectDialog.h"
#include "EffectView.h"
#include "PluginInstance.h"
#include "GroupBox.h"
#include "StringPairDrag.h"


namespace lmms::gui
{

EffectRackView::EffectRackView( EffectChain* model, QWidget* parent ) :
	QWidget( parent ),
	ModelView( nullptr, this )
{
	auto mainLayout = new QVBoxLayout(this);
	mainLayout->setContentsMargins(5, 5, 5, 5);

	m_effectsGroupBox = new GroupBox( tr( "EFFECTS CHAIN" ) );
	mainLayout->addWidget( m_effectsGroupBox );

	auto effectsLayout = new QVBoxLayout(m_effectsGroupBox);
	effectsLayout->setSpacing( 0 );
	effectsLayout->setContentsMargins( 2, m_effectsGroupBox->titleBarHeight() + 2, 2, 2 );

	m_scrollArea = new QScrollArea;
	m_scrollArea->setVerticalScrollBarPolicy( Qt::ScrollBarAlwaysOn );
	m_scrollArea->setHorizontalScrollBarPolicy( Qt::ScrollBarAlwaysOff );
	m_scrollArea->setPalette( QApplication::palette( m_scrollArea ) );
	m_scrollArea->setFrameStyle( QFrame::NoFrame );
	m_scrollArea->setWidget( new QWidget );

	effectsLayout->addWidget( m_scrollArea );

	auto addButton = new QPushButton;
	addButton->setText( tr( "Add effect" ) );
	addButton->setFocusPolicy(Qt::NoFocus);

	effectsLayout->addWidget( addButton );

	connect( addButton, SIGNAL(clicked()), this, SLOT(addEffect()));

	// Effects dragged from the browser go where they are dropped
	setAcceptDrops(true);


	m_lastY = 0;

	setModel( model );
}



EffectRackView::~EffectRackView()
{
	clearViews();
}





void EffectRackView::clearViews()
{
	while( m_effectViews.size() )
	{
		EffectView * e = m_effectViews[m_effectViews.size() - 1];
		m_effectViews.pop_back();
		delete e;
	}
}




void EffectRackView::moveUp( EffectView* view )
{
	fxChain()->moveUp( view->effect() );
	if( view != m_effectViews.first() )
	{
		int i = 0;
		for( QVector<EffectView *>::Iterator it = m_effectViews.begin(); 
					it != m_effectViews.end(); it++, i++ )
		{
			if( *it == view )
			{
				break;
			}
		}

		EffectView * temp = m_effectViews[ i - 1 ];

		m_effectViews[i - 1] = view;
		m_effectViews[i] = temp;

		update();
	}
}




void EffectRackView::moveDown( EffectView* view )
{
	if( view != m_effectViews.last() )
	{
		// moving next effect up is the same
		moveUp( *( std::find( m_effectViews.begin(), m_effectViews.end(), view ) + 1 ) );
	}
}




auto EffectRackView::viewOf(const Effect* effect) const -> EffectView*
{
	const auto it = std::find_if(m_effectViews.begin(), m_effectViews.end(),
		[effect](EffectView* view) { return view->effect() == effect; });
	return it != m_effectViews.end() ? *it : nullptr;
}




auto EffectRackView::indexOf(EffectView* view) const -> int
{
	const auto& effects = fxChain()->effects();
	for (auto i = std::size_t{0}; i < effects.size(); ++i)
	{
		if (effects[i]->effect() == view->effect()) { return static_cast<int>(i); }
	}
	return -1;
}




void EffectRackView::replace(EffectView* view, std::unique_ptr<PluginInstance> effect)
{
	const int index = indexOf(view);
	if (index < 0 || !effect) { return; }

	// The view goes before its effect does
	m_effectViews.erase(std::find(m_effectViews.begin(), m_effectViews.end(), view));
	delete view;

	fxChain()->replaceEffect(static_cast<std::size_t>(index), std::move(effect));
	update();
}




void EffectRackView::replacePlugin(EffectView* view)
{
	EffectSelectDialog esd(this);
	esd.exec();
	if (esd.result() == QDialog::Rejected) { return; }
	replace(view, esd.instantiateSelectedPlugin(fxChain()));
}




auto EffectRackView::presetDir() -> QString
{
	const auto dir = ConfigManager::inst()->userPresetsDir() + "effects/";
	QDir{}.mkpath(dir);
	return dir;
}




void EffectRackView::loadPreset(EffectView* view)
{
	const auto fileName = QFileDialog::getOpenFileName(this, tr("Load effect preset"), presetDir(),
		tr("Effect presets (*.xfx)"));
	if (fileName.isEmpty()) { return; }

	auto file = DataFile{fileName};
	const auto element = file.content().firstChildElement();
	if (file.type() != DataFile::Type::EffectSettings || element.isNull())
	{
		QMessageBox::warning(this, tr("Load effect preset"), tr("%1 is not an effect preset.").arg(fileName));
		return;
	}
	replace(view, fxChain()->loadEffect(element));
}




void EffectRackView::savePreset(EffectView* view)
{
	const int index = indexOf(view);
	if (index < 0) { return; }

	auto fileName = QFileDialog::getSaveFileName(this, tr("Save effect preset"),
		presetDir() + view->effect()->displayName() + ".xfx", tr("Effect presets (*.xfx)"));
	if (fileName.isEmpty()) { return; }
	if (!fileName.endsWith(".xfx")) { fileName += ".xfx"; }

	auto file = DataFile{DataFile::Type::EffectSettings};
	auto content = file.content();
	EffectChain::saveEffect(*fxChain()->effects()[index], file, content);
	file.writeFile(fileName);
}




void EffectRackView::dragEnterEvent(QDragEnterEvent* event)
{
	StringPairDrag::processDragEnterEvent(event, "effect");
}




void EffectRackView::dropEvent(QDropEvent* event)
{
	if (StringPairDrag::decodeKey(event) != "effect") { return; }
	auto effect = EffectSelectDialog::instantiateDragged(StringPairDrag::decodeValue(event), fxChain());
	if (!effect) { return; }

	// Before the first effect whose middle is below the drop
	const auto y = m_scrollArea->widget()->mapFrom(this, position(event)).y();
	auto index = std::size_t{0};
	for (const auto view : m_effectViews)
	{
		if (view->y() + view->height() / 2 > y) { break; }
		++index;
	}
	fxChain()->insertEffect(index, std::move(effect));
	update();
	event->acceptProposedAction();
}




void EffectRackView::deletePlugin( EffectView* view )
{
	Effect * e = view->effect();
	m_effectViews.erase( std::find( m_effectViews.begin(), m_effectViews.end(), view ) );
	delete view;

	// The chain owns the effect and deletes it
	fxChain()->removeEffect( e );
	update();
}




void EffectRackView::update()
{
	QWidget * w = m_scrollArea->widget();

	// One view per effect, in the chain's order
	auto ordered = QVector<EffectView*>{};
	for (const auto& instance : fxChain()->effects())
	{
		Effect* effect = instance->effect();
		const auto found = std::find_if(m_effectViews.begin(), m_effectViews.end(),
			[effect](EffectView* view) { return view->model() == effect; });
		if (found != m_effectViews.end())
		{
			ordered.append(*found);
			m_effectViews.erase(found);
			continue;
		}

		auto view = new EffectView(effect, w, instance->audioPortsModel());
		connect(view, &EffectView::movedUp, this, &EffectRackView::moveUp);
		connect(view, &EffectView::movedDown, this, &EffectRackView::moveDown);
		connect(view, &EffectView::deletedPlugin, this, &EffectRackView::deletePlugin, Qt::QueuedConnection);
		connect(view, &EffectView::replaceRequested, this, &EffectRackView::replacePlugin, Qt::QueuedConnection);
		connect(view, &EffectView::loadPresetRequested, this, &EffectRackView::loadPreset, Qt::QueuedConnection);
		connect(view, &EffectView::savePresetRequested, this, &EffectRackView::savePreset, Qt::QueuedConnection);

		QAction* moveUpAction = new QAction(view);
		moveUpAction->setShortcut(keySequence(Qt::Key_Up, Qt::AltModifier));
		moveUpAction->setShortcutContext(Qt::WidgetShortcut);
		connect(moveUpAction, &QAction::triggered, view, &EffectView::moveUp);
		view->addAction(moveUpAction);

		QAction* moveDownAction = new QAction(view);
		moveDownAction->setShortcut(keySequence(Qt::Key_Down, Qt::AltModifier));
		moveDownAction->setShortcutContext(Qt::WidgetShortcut);
		connect(moveDownAction, &QAction::triggered, view, &EffectView::moveDown);
		view->addAction(moveDownAction);

		view->show();
		ordered.append(view);
	}

	// The views of effects that are gone
	qDeleteAll(m_effectViews);
	m_effectViews = ordered;

	const int EffectViewMargin = 3;
	m_lastY = EffectViewMargin;
	for (const auto view : m_effectViews)
	{
		view->move(EffectViewMargin, m_lastY);
		m_lastY += view->height();
	}

	w->setFixedSize(EffectView::DEFAULT_WIDTH + 2 * EffectViewMargin, m_lastY);

	QWidget::update();
}




void EffectRackView::addEffect()
{
	EffectSelectDialog esd( this );
	esd.exec();

	if( esd.result() == QDialog::Rejected )
	{
		return;
	}

	auto instance = esd.instantiateSelectedPlugin( fxChain() );
	Effect* fx = instance->effect();

	fxChain()->appendEffect(std::move(instance));
	update();

	// Find the effectView, and show the controls
	for (const auto& effectView : m_effectViews)
	{
		if (effectView->effect() == fx)
		{
			effectView->editControls();
			break;
		}
	}


}




void EffectRackView::modelChanged()
{
	//clearViews();
	m_effectsGroupBox->setModel( &fxChain()->m_enabledModel );
	connect( fxChain(), SIGNAL(aboutToClear()), this, SLOT(clearViews()));
	update();
}




QSize EffectRackView::sizeHint() const
{
	// Use the formula from InstrumentTrackWindow.cpp
	return QSize{EffectRackView::DEFAULT_WIDTH, 254 /* INSTRUMENT_HEIGHT */ - 4 - 1};
}




} // namespace lmms::gui
