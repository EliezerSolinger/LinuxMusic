/*
 * EffectView.cpp - view-component for an effect
 *
 * Copyright (c) 2006-2007 Danny McRae <khjklujn/at/users.sourceforge.net>
 * Copyright (c) 2007-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "EffectView.h"

#include <QGraphicsOpacityEffect>
#include <QLayout>
#include <QMouseEvent>
#include <QPushButton>
#include <QPainter>

#include "AudioPortsModel.h"
#include "CaptionMenu.h"
#include "DockPanel.h"
#include "DummyEffect.h"
#include "EffectChain.h"
#include "Engine.h"
#include "FontHelper.h"
#include "GuiApplication.h"
#include "Knob.h"
#include "LedCheckBox.h"
#include "MainWindow.h"
#include "Mixer.h"
#include "PinConnector.h"
#include "SubWindow.h"
#include "TempoSyncKnob.h"
#include "embed.h"


namespace lmms::gui
{

EffectView::EffectView(Effect* _model, QWidget* _parent, AudioPortsModel* audioPorts) :
	PluginView( _model, _parent ),
	m_audioPorts(audioPorts),
	m_bg( embed::getIconPixmap( "effect_plugin" ) ),
	m_subWindow( nullptr ),
	m_controlView(nullptr),
	m_dragging(false)
{
	setFixedSize(EffectView::DEFAULT_WIDTH, EffectView::DEFAULT_HEIGHT);
	setFocusPolicy(Qt::StrongFocus);

	// Disable effects that are of type "DummyEffect"
	bool isEnabled = !dynamic_cast<DummyEffect *>( effect() );
	m_bypass = new LedCheckBox( this, "", isEnabled ? LedCheckBox::LedColor::Green : LedCheckBox::LedColor::Red );
	m_bypass->move( 3, 3 );
	m_bypass->setEnabled( isEnabled );
	m_bypass->setToolTip(tr("On/Off"));

	connect(getGUI()->mainWindow(), &MainWindow::periodicUpdate, this, &EffectView::corruptStateUpdate);

	m_wetDry = new Knob(KnobType::Bright26, tr("W/D"), this, Knob::LabelRendering::LegacyFixedFontSize);
	m_wetDry->move( 40 - m_wetDry->width() / 2, 5 );
	m_wetDry->setEnabled( isEnabled );
	m_wetDry->setHintText( tr( "Wet Level:" ), "" );


	m_autoQuit = new TempoSyncKnob(KnobType::Bright26, tr("DECAY"), this, Knob::LabelRendering::LegacyFixedFontSize);
	m_autoQuit->move( 78 - m_autoQuit->width() / 2, 5 );
	m_autoQuit->setEnabled(isEnabled && effect()->autoQuitEnabled());
	m_autoQuit->setHintText( tr( "Time:" ), "ms" );

	setModel( _model );

	if( effect()->controls()->controlCount() > 0 )
	{
		auto ctls_btn = new QPushButton(embed::getIconPixmap("gear", 20, 20), "", this);
		ctls_btn->setToolTip(tr("Controls"));
		ctls_btn->setGeometry(144, 12, 28, 28);
		ctls_btn->setFocusPolicy(Qt::NoFocus);
		connect( ctls_btn, SIGNAL(clicked()),
					this, SLOT(editControls()));

		m_controlView = effect()->controls()->createView();
		if (m_controlView)
		{
			m_subWindow = getGUI()->mainWindow()->addPluginTab(m_controlView);
		}
	}

	if (auto ap = m_audioPorts)
	{
		const auto formatString = tr("Pin connector\n%1");

		m_pinConnectorButton = new QPushButton(embed::getIconPixmap("tool", 20, 20), "", this);
		m_pinConnectorButton->setToolTip(formatString.arg(ap->getChannelCountText()));

		connect(ap, &AudioPortsModel::propertiesChanged, [=, this]() {
			m_pinConnectorButton->setToolTip(formatString.arg(m_audioPorts->getChannelCountText()));
		});

		m_pinConnectorButton->setGeometry(144 + 32, 12, 28, 28);
		connect(m_pinConnectorButton, &QPushButton::clicked, this, &EffectView::togglePinConnector);
	}

	m_opacityEffect = new QGraphicsOpacityEffect(this);
	m_opacityEffect->setOpacity(1);
	setGraphicsEffect(m_opacityEffect);

	//move above vst effect view creation
	//setModel( _model );
}




EffectView::~EffectView()
{
	if (m_pinConnectorView) { m_pinConnectorView->closeWindow(); }
	delete m_subWindow;
}




void EffectView::editControls()
{
	if( m_subWindow )
	{
		if (!m_controlView->isVisible())
		{
			m_subWindow->show();
			m_subWindow->raise();
			effect()->controls()->setViewVisible(true); // TODO is this even needed?
		}
		else
		{
			m_subWindow->hide();
			effect()->controls()->setViewVisible( false );
		}
	}
}




void EffectView::togglePinConnector()
{
	auto ap = m_audioPorts;
	if (!ap) { return; }

	if (!m_pinConnectorView)
	{
		m_pinConnectorView = ap->instantiateView();
	}
	else
	{
		m_pinConnectorView->toggleVisibility();
	}
}




void EffectView::moveUp()
{
	emit movedUp(this);
}




void EffectView::moveDown()
{
	emit movedDown(this);
}



void EffectView::deletePlugin()
{
	emit deletedPlugin(this);
}




void EffectView::contextMenuEvent( QContextMenuEvent * )
{
	QPointer<CaptionMenu> contextMenu = new CaptionMenu( model()->displayName(), this );
	contextMenu->addAction( embed::getIconPixmap( "arp_up" ),
						tr( "Move &up" ),
						this, SLOT(moveUp()));
	contextMenu->addAction( embed::getIconPixmap( "arp_down" ),
						tr( "Move &down" ),
						this, SLOT(moveDown()));
	contextMenu->addSeparator();
	contextMenu->addAction(tr("Re&place with..."), this, [this] { emit replaceRequested(this); });
	contextMenu->addAction(tr("&Load preset..."), this, [this] { emit loadPresetRequested(this); });
	if (!dynamic_cast<DummyEffect*>(effect()))
	{
		contextMenu->addAction(tr("&Save preset..."), this, [this] { emit savePresetRequested(this); });
	}

	// Effects with a sidechain input (more inputs than the main pair) on a mixer channel: where it comes from
	const auto mixer = Engine::mixer();
	const int channel = effect()->effectChain() ? mixer->channelOf(effect()->effectChain()) : -1;
	auto sidechainMenu = QMenu{tr("Sidechain source"), this};
	if (channel > 0 && m_audioPorts && m_audioPorts->in().channelCount() > 2)
	{
		for (int source = 1; source < mixer->numChannels(); ++source)
		{
			if (source == channel) { continue; }
			const bool sending = mixer->findChannelSend(source, channel, 0, 1) != nullptr;
			if (!sending && mixer->isInfiniteLoop(source, channel)) { continue; }
			auto action = sidechainMenu.addAction(tr("%1: %2").arg(source).arg(mixer->mixerChannel(source)->m_name),
				this, [mixer, source, channel] { mixer->toggleSidechainSend(source, channel); });
			action->setCheckable(true);
			action->setChecked(sending);
		}
		contextMenu->addMenu(&sidechainMenu);
	}

	contextMenu->addSeparator();
	contextMenu->addAction( embed::getIconPixmap( "cancel" ),
						tr( "&Remove this plugin" ),
						this, SLOT(deletePlugin()));
	contextMenu->addSeparator();
	contextMenu->exec( QCursor::pos() );
	delete contextMenu;
}



void EffectView::mousePressEvent(QMouseEvent* event)
{
	if (event->button() == Qt::LeftButton)
	{
		m_dragging = true;
		m_opacityEffect->setOpacity(0.3);
		QCursor c(Qt::SizeVerCursor);
		QApplication::setOverrideCursor(c);
		update();
	}
}

void EffectView::mouseReleaseEvent(QMouseEvent* event)
{
	if (event->button() == Qt::LeftButton)
	{
		m_dragging = false;
		m_opacityEffect->setOpacity(1);
		QApplication::restoreOverrideCursor();
		update();
	}
}

void EffectView::mouseMoveEvent(QMouseEvent* event)
{
	if (!m_dragging) { return; }
	if (event->pos().y() < 0)
	{
		moveUp();
	}
	else if (event->pos().y() > EffectView::DEFAULT_HEIGHT)
	{
		moveDown();
	}
}



void EffectView::paintEvent( QPaintEvent * )
{
	QPainter p( this );
	p.drawPixmap( 0, 0, m_bg );

	QFont f = adjustedToPixelSize(font(), DEFAULT_FONT_SIZE);
	f.setBold( true );
	p.setFont( f );

	QString elidedText = p.fontMetrics().elidedText( model()->displayName(), Qt::ElideRight, width() - 22 );

	p.setPen( palette().shadow().color() );
	p.drawText( 6, 55, elidedText );
	p.setPen( palette().text().color() );
	p.drawText( 5, 54, elidedText );
}




void EffectView::modelChanged()
{
	m_bypass->setModel( &effect()->m_enabledModel );
	m_wetDry->setModel( &effect()->m_wetDryModel );
	m_autoQuit->setModel( &effect()->m_autoQuitModel );
}

void EffectView::corruptStateUpdate()
{
	if (dynamic_cast<DummyEffect*>(effect())) { return; }

	if (effect()->isCorrupted())
	{
		m_bypass->setLedColor(LedCheckBox::LedColor::Red);
		m_bypass->setToolTip(tr("Corrupted audio detected: muting affected channels"));
	}
	else
	{
		m_bypass->setLedColor(LedCheckBox::LedColor::Green);
		m_bypass->setToolTip(tr("On/Off"));
	}
}

} // namespace lmms::gui
