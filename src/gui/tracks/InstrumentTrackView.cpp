/*
 * InstrumentTrackView.cpp - implementation of InstrumentTrackView class
 *
 * Copyright (c) 2004-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "InstrumentTrackView.h"

#include <ranges>

#include <QAction>
#include <QApplication>
#include <QDragEnterEvent>
#include <QHBoxLayout>
#include <QMdiArea>
#include <QMdiSubWindow>
#include <QActionGroup>
#include <QMenu>
#include <QPushButton>
#include <QSpacerItem>
#include <QVBoxLayout>

#include "AudioEngine.h"
#include "AudioPortsModel.h"
#include "AutomatableButton.h"
#include "ConfigManager.h"
#include "Engine.h"
#include "FadeButton.h"
#include "GuiApplication.h"
#include "Instrument.h"
#include "InstrumentTrackWindow.h"
#include "Knob.h"
#include "MainWindow.h"
#include "MidiCCRackView.h"
#include "MidiClient.h"
#include "MidiCCRackView.h"
#include "MidiPortMenu.h"
#include "MidiRecorder.h"
#include "TrackChannelControls.h"
#include "TrackContainerView.h"
#include "Mixer.h"
#include "MixerChannelLcdSpinBox.h"
#include "MixerView.h"
#include "PinConnector.h"
#include "TrackLabelButton.h"


namespace lmms::gui
{


InstrumentTrackView::InstrumentTrackView( InstrumentTrack * _it, TrackContainerView* tcv ) :
	TrackView( _it, tcv ),
	m_window( nullptr ),
	m_lastPos( -1, -1 )
{
	setAcceptDrops( true );
	setFixedHeight(TwoRowHeight);

	m_tlb = new TrackLabelButton( this, getTrackSettingsWidget() );
	m_tlb->setCheckable( true );
	m_tlb->setIcon(determinePixmap(_it));
	m_tlb->show();

	connect( m_tlb, SIGNAL(toggled(bool)),
			this, SLOT(toggleInstrumentWindow(bool)));

	connect( _it, SIGNAL(nameChanged()),
			m_tlb, SLOT(update()));

	connect(ConfigManager::inst(), SIGNAL(valueChanged(QString,QString,QString)),
			this, SLOT(handleConfigChange(QString,QString,QString)));

	connect(_it, &InstrumentTrack::instrumentChanged, this, &InstrumentTrackView::onInstrumentChanged);

	m_mixerChannelNumber = new MixerChannelLcdSpinBox(2, getTrackSettingsWidget(), tr("Mixer channel"), this);
	if (trackContainerView()->trackHeadWidth() < COMPACT_TRACK_WIDTH) { m_mixerChannelNumber->hide(); }
	else { m_mixerChannelNumber->show(); }

	connect(trackContainerView(), &TrackContainerView::trackHeadWidthChanged, this, [this](int width){
		if (width < COMPACT_TRACK_WIDTH) { m_mixerChannelNumber->hide(); }
		else { m_mixerChannelNumber->show(); }
	});

	m_volumeKnob = new VolumeKnob(KnobType::Small17, tr("VOL"), getTrackSettingsWidget(), Knob::LabelRendering::LegacyFixedFontSize, tr("VOL"));
	m_volumeKnob->setModel( &_it->m_volumeModel );
	m_volumeKnob->setHintText( tr( "Volume:" ), "%" );
	m_volumeKnob->show();

	m_panningKnob = new Knob(KnobType::Small17, tr("PAN"), getTrackSettingsWidget(), Knob::LabelRendering::LegacyFixedFontSize, tr("Panning"));
	m_panningKnob->setModel( &_it->m_panningModel );
	m_panningKnob->setHintText(tr("Panning:"), "%");
	m_panningKnob->show();

	// The devices' menus serve a sequencing MIDI client (as ALSA's); the MIDI menu is made again when the driver
	// changes (see fillMidiMenu())
	m_midiMenu = new QMenu( tr( "MIDI" ), this );
	_it->m_midiPort.m_readablePortsMenu = new MidiPortMenu(MidiPort::Mode::Input);
	_it->m_midiPort.m_writablePortsMenu = new MidiPortMenu(MidiPort::Mode::Output);
	_it->m_midiPort.m_readablePortsMenu->setModel(&_it->m_midiPort);
	_it->m_midiPort.m_writablePortsMenu->setModel(&_it->m_midiPort);
	connect(&_it->m_midiPort, &MidiPort::midiClientChanged, this, &InstrumentTrackView::fillMidiMenu);
	connect(&_it->m_midiPort, &MidiPort::modeChanged, this, &InstrumentTrackView::midiConfigChanged);

	m_activityIndicator = new FadeButton(getTrackSettingsWidget());
	m_activityIndicator->setFixedSize(8, 28);
	m_activityIndicator->show();

	// Recording: R arms the track for the notes played into it; its menu chooses overdub or replace
	m_recordArmButton = new AutomatableButton(getTrackSettingsWidget(), tr("Record arm"));
	m_recordArmButton->setCheckable(true);
	m_recordArmButton->setObjectName("btn-arm");
	m_recordArmButton->setText(tr("R"));
	m_recordArmButton->setToolTip(tr("Arm for recording: the Record button records the notes played into this track "
		"(right-click: overdub, replace or a take in a lane, and quantizing)"));
	m_recordArmButton->setModel(_it->recordArmModel());
	m_recordArmButton->setContextMenuPolicy(Qt::CustomContextMenu);
	connect(m_recordArmButton, &QWidget::customContextMenuRequested, this, [this](const QPoint& where) {
		auto* track = model();
		auto menu = QMenu{};
		auto* modes = new QActionGroup(&menu);
		const auto addMode = [&](const QString& label, MidiRecorder::Mode mode) {
			auto* action = menu.addAction(label, [track, mode] {
				track->midiRecordModeModel()->setValue(static_cast<int>(mode));
			});
			action->setCheckable(true);
			action->setChecked(track->midiRecordModeModel()->value() == static_cast<int>(mode));
			modes->addAction(action);
		};
		addMode(tr("Overdub: the notes join what is there"), MidiRecorder::Mode::Overdub);
		addMode(tr("Replace: what was there where you record goes"), MidiRecorder::Mode::Replace);
		addMode(tr("Take in a lane: the recording goes on a new lane on top, heard; what was there stays below"),
			MidiRecorder::Mode::TakeLane);

		// Quantizing what is recorded: the grid and how far toward it
		menu.addSeparator();
		auto* quantize = menu.addMenu(tr("Quantize while recording"));
		auto* grids = new QActionGroup(quantize);
		for (const auto value : {0, 4, 8, 16, 32, 64})
		{
			auto* action = quantize->addAction(value == 0 ? tr("Off") : QString{"1/%1"}.arg(value), [track, value] {
				track->recordQuantizeModel()->setValue(value);
			});
			action->setCheckable(true);
			action->setChecked(track->recordQuantizeModel()->value() == value);
			grids->addAction(action);
		}
		quantize->addSeparator();
		auto* strengths = new QActionGroup(quantize);
		for (const auto value : {50, 75, 100})
		{
			auto* action = quantize->addAction(tr("Strength %1%").arg(value), [track, value] {
				track->recordQuantizeStrengthModel()->setValue(value);
			});
			action->setCheckable(true);
			action->setChecked(track->recordQuantizeStrengthModel()->value() == value);
			strengths->addAction(action);
		}
		menu.exec(m_recordArmButton->mapToGlobal(where));
	});

	// The MIDI input: whether it receives, from which devices, on which channel (omni by default)
	m_midiInputButton = new QPushButton(getTrackSettingsWidget());
	m_midiInputButton->setObjectName("btn-input");
	m_midiInputButton->setFocusPolicy(Qt::NoFocus);
	m_midiInputButton->setCursor(Qt::PointingHandCursor);
	auto* midiInputMenu = new QMenu(m_midiInputButton);
	connect(midiInputMenu, &QMenu::aboutToShow, this, [this, midiInputMenu] { fillMidiInputMenu(midiInputMenu); });
	m_midiInputButton->setMenu(midiInputMenu);
	connect(&_it->m_midiPort, &MidiPort::modeChanged, this, &InstrumentTrackView::updateMidiInputButton);
	connect(_it->m_midiPort.inputChannelModel(), &IntModel::dataChanged, this, &InstrumentTrackView::updateMidiInputButton);

	// Two rows, as the audio track's: the name, activity, arming and input on top; the channel, VOL and PAN below
	auto masterLayout = new QVBoxLayout(getTrackSettingsWidget());
	masterLayout->setContentsMargins(0, 1, 0, 0);
	masterLayout->setSpacing(1);
	auto top = new QHBoxLayout();
	top->setContentsMargins(0, 0, 0, 0);
	top->setSpacing(3);
	top->addWidget(m_tlb, 1);
	top->addWidget(m_activityIndicator);
	top->addWidget(m_recordArmButton);
	top->addWidget(m_midiInputButton);
	top->addSpacing(4);
	auto bottom = new QHBoxLayout();
	bottom->setContentsMargins(0, 0, 0, 0);
	bottom->setSpacing(3);
	bottom->addWidget(m_mixerChannelNumber);
	bottom->addStretch(1);
	bottom->addWidget(m_volumeKnob);
	bottom->addWidget(m_panningKnob);
	bottom->addSpacing(4);
	m_activityIndicator->setFixedSize(8, 18);
	masterLayout->addLayout(top);
	masterLayout->addLayout(bottom);
	masterLayout->addSpacerItem(new QSpacerItem(0, 0, QSizePolicy::Minimum, QSizePolicy::Expanding));

	connect( m_activityIndicator, SIGNAL(pressed()),
				this, SLOT(activityIndicatorPressed()));
	connect( m_activityIndicator, SIGNAL(released()),
				this, SLOT(activityIndicatorReleased()));
	connect( _it, SIGNAL(newNote()),
			 m_activityIndicator, SLOT(activate()));
	connect( _it, SIGNAL(endNote()),
	 		m_activityIndicator, SLOT(noteEnd()));
	connect(getGUI()->mainWindow(), &MainWindow::periodicUpdate, this, &InstrumentTrackView::corruptStateUpdate);

	setModel( _it );
	updateMidiInputButton(); // (it reads the model: once it is set)
	fillMidiMenu(); // (the same)

	onInstrumentChanged();
}




InstrumentTrackView::~InstrumentTrackView()
{
	if (m_pinConnectorView) { m_pinConnectorView->closeWindow(); }

	delete m_window;
	m_window = nullptr;

	delete model()->m_midiPort.m_readablePortsMenu;
	delete model()->m_midiPort.m_writablePortsMenu;
}




void InstrumentTrackView::toggleMidiCCRack()
{
	// Lazy creation: midiCCRackView is only created when accessed the first time.
	// this->model() returns pointer to the InstrumentTrack who owns this InstrumentTrackView.
	if (!m_midiCCRackView)
	{
		m_midiCCRackView = std::unique_ptr<MidiCCRackView>(new MidiCCRackView(this->model()));
	}

	if (m_midiCCRackView->parentWidget()->isVisible())
	{
		m_midiCCRackView->parentWidget()->hide();
	}
	else
	{
		m_midiCCRackView->parentWidget()->show();
		m_midiCCRackView->show();
	}
}




void InstrumentTrackView::togglePinConnector()
{
	const auto it = this->model();
	if (!it) { return; }

	const auto inst = it->instrumentInstance();
	if (!inst) { return; }

	auto ap = inst->audioPortsModel();
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




InstrumentTrackWindow* InstrumentTrackView::topLevelInstrumentTrackWindow()
{
	const auto subWindows = getGUI()->mainWindow()->workspace()->subWindowList(QMdiArea::ActivationHistoryOrder);

	for (QMdiSubWindow* sw : subWindows | std::views::reverse)
	{
		if (!sw->widget() || !sw->widget()->isVisible()) { continue; }

		if (auto itw = qobject_cast<InstrumentTrackWindow*>(sw->widget()))
		{
			return itw;
		}
	}

	return nullptr;
}




void InstrumentTrackView::createMixerLine()
{
	int channelIndex = getGUI()->mixerView()->addNewChannel();
	auto channel = Engine::mixer()->mixerChannel(channelIndex);

	channel->m_name = getTrack()->name();
	channel->setColor(getTrack()->color());

	assignMixerLine(channelIndex);
}




void InstrumentTrackView::assignMixerLine(int channelIndex)
{
	model()->mixerChannelModel()->setValue( channelIndex );

	getGUI()->mixerView()->setCurrentMixerChannel(channelIndex);
}



InstrumentTrackWindow * InstrumentTrackView::getInstrumentTrackWindow()
{
	if (!m_window)
	{
		m_window = new InstrumentTrackWindow(this);
	}

	return m_window;
}

void InstrumentTrackView::handleConfigChange(QString cls, QString attr, QString value)
{
	// When one instrument track window mode is turned on,
	// close windows except last opened one.
	if (cls == "ui" && attr == "oneinstrumenttrackwindow" && value.toInt())
	{
		m_tlb->setChecked(m_window && m_window == topLevelInstrumentTrackWindow());
	}
}

void InstrumentTrackView::modelChanged()
{
	TrackView::modelChanged();
	auto st = castModel<InstrumentTrack>();
	m_mixerChannelNumber->setModel(&st->m_mixerChannelModel);
	m_channelControls = TrackChannelControls::replace(m_channelControls, st, this, {.volume = m_volumeKnob,
		.panning = m_panningKnob, .mute = getTrackOperationsWidget()->muteButton(),
		.solo = getTrackOperationsWidget()->soloButton()});
}

void InstrumentTrackView::dragEnterEvent( QDragEnterEvent * _dee )
{
	InstrumentTrackWindow::dragEnterEventGeneric( _dee );
	if( !_dee->isAccepted() )
	{
		TrackView::dragEnterEvent( _dee );
	}
}




void InstrumentTrackView::dropEvent( QDropEvent * _de )
{
	getInstrumentTrackWindow()->dropEvent( _de );
	TrackView::dropEvent( _de );
}




void InstrumentTrackView::onInstrumentChanged()
{
	// Check if an instrument has been loaded, if not disable the track label button to prevent opening an empty instrument window.
	m_tlb->setEnabled(model()->m_instrument != nullptr);
}




void InstrumentTrackView::toggleInstrumentWindow( bool _on )
{
	if (_on && ConfigManager::inst()->value("ui", "oneinstrumenttrackwindow").toInt())
	{
		if (topLevelInstrumentTrackWindow())
		{
			topLevelInstrumentTrackWindow()->m_itv->m_tlb->setChecked(false);
		}
	}

	getInstrumentTrackWindow()->toggleVisibility( _on );
}




void InstrumentTrackView::activityIndicatorPressed()
{
	model()->processInEvent( MidiEvent( MidiNoteOn, 0, DefaultKey, MidiDefaultVelocity ) );
}




void InstrumentTrackView::activityIndicatorReleased()
{
	model()->processInEvent( MidiEvent( MidiNoteOff, 0, DefaultKey, 0 ) );
}





void InstrumentTrackView::midiInSelected()
{
	if( model() )
	{
		model()->m_midiPort.setReadable( m_midiInputAction->isChecked() );
	}
}




void InstrumentTrackView::midiOutSelected()
{
	if( model() )
	{
		model()->m_midiPort.setWritable( m_midiOutputAction->isChecked() );
	}
}




void InstrumentTrackView::fillMidiMenu()
{
	m_midiMenu->clear();
	auto& port = model()->m_midiPort;
	if (!port.midiClient()->isRaw())
	{
		// Each menu chooses the devices, and turns the input or output on
		m_midiInputAction = m_midiMenu->addMenu(port.m_readablePortsMenu);
		m_midiOutputAction = m_midiMenu->addMenu(port.m_writablePortsMenu);
	}
	else
	{
		// A raw client has no devices to choose: input and output on or off
		m_midiInputAction = m_midiMenu->addAction(QString{});
		m_midiOutputAction = m_midiMenu->addAction(QString{});
		m_midiInputAction->setCheckable(true);
		m_midiOutputAction->setCheckable(true);
		midiConfigChanged();
		connect(m_midiInputAction, &QAction::toggled, this, &InstrumentTrackView::midiInSelected);
		connect(m_midiOutputAction, &QAction::toggled, this, &InstrumentTrackView::midiOutSelected);
	}
	m_midiInputAction->setText(tr("Input"));
	m_midiOutputAction->setText(tr("Output"));

	auto* midiRackAction = m_midiMenu->addAction(tr("Open/Close MIDI CC Rack"));
	midiRackAction->setIcon(embed::getIconPixmap("midi_cc_rack"));
	connect(midiRackAction, &QAction::triggered, this, &InstrumentTrackView::toggleMidiCCRack);
}




void InstrumentTrackView::midiConfigChanged()
{
	if (!m_midiInputAction || !m_midiOutputAction || !model()) { return; }
	m_midiInputAction->setChecked( model()->m_midiPort.isReadable() );
	m_midiOutputAction->setChecked( model()->m_midiPort.isWritable() );
}




//FIXME: This is identical to SampleTrackView::createMixerMenu
QMenu * InstrumentTrackView::createMixerMenu(QString title, QString newMixerLabel)
{
	int channelIndex = model()->mixerChannelModel()->value();

	MixerChannel *mixerChannel = Engine::mixer()->mixerChannel( channelIndex );

	// If title allows interpolation, pass channel index and name
	if ( title.contains( "%2" ) )
	{
		title = title.arg( channelIndex ).arg( mixerChannel->m_name );
	}

	auto mixerMenu = new QMenu(title);

	mixerMenu->addAction( newMixerLabel, this, SLOT(createMixerLine()));
	mixerMenu->addSeparator();

	for (int i = 0; i < Engine::mixer()->numChannels(); ++i)
	{
		MixerChannel * currentChannel = Engine::mixer()->mixerChannel( i );

		if ( currentChannel != mixerChannel )
		{
			auto index = currentChannel->index();
			QString label = tr( "%1: %2" ).arg(index).arg(currentChannel->m_name);
			mixerMenu->addAction(label, [this, index](){
				assignMixerLine(index);
			});
		}
	}

	return mixerMenu;
}

void InstrumentTrackView::addPinConnectorAction(QMenu* menu)
{
	assert(menu != nullptr);
	const auto addAction = [=, this](const AudioPortsModel* pc = nullptr) {
		if (pc)
		{
			auto pcAction = menu->addAction(embed::getIconPixmap("pin_connector"), tr("Pin connector"),
				this, &InstrumentTrackView::togglePinConnector);

			pcAction->setToolTip(pc->getChannelCountText());
		}
		else
		{
			auto pcAction = menu->addAction(embed::getIconPixmap("pin_connector"), tr("Pin connector"));
			pcAction->setDisabled(true);
		}
	};

	const auto it = this->model();
	if (!it) { addAction(); return; }

	const auto inst = it->instrumentInstance();
	if (!inst) { addAction(); return; }

	const auto ap = inst->audioPortsModel();
	addAction(ap);
}

QPixmap InstrumentTrackView::determinePixmap(InstrumentTrack* instrumentTrack)
{
	if (instrumentTrack)
	{
		Instrument* instrument = instrumentTrack->instrument();

		if (instrument && instrument->descriptor())
		{
			const PixmapLoader* pl = instrument->key().isValid()
				? instrument->key().logo()
				: instrument->descriptor()->logo;

			if (pl)
			{
				return pl->pixmap();
			}
		}
	}

	return embed::getIconPixmap("instrument_track");
}

void InstrumentTrackView::corruptStateUpdate()
{
	if (model()->audioBusHandle()->isCorrupted())
	{
		m_activityIndicator->setState(FadeButton::State::Corrupted);
		m_activityIndicator->setToolTip(tr("Corrupted audio detected: muting affected channels"));
	}
	else
	{
		m_activityIndicator->setState(FadeButton::State::Normal);
		m_activityIndicator->setToolTip(QString{});
	}
}

void InstrumentTrackView::updateMidiInputButton()
{
	auto& port = model()->m_midiPort;
	const auto channel = port.inputChannel();
	m_midiInputButton->setText(!port.isReadable() ? QString{"-"} : channel == 0 ? tr("Omni") : QString::number(channel));
	m_midiInputButton->setToolTip(!port.isReadable() ? tr("MIDI input: off")
		: channel == 0 ? tr("MIDI input: all channels") : tr("MIDI input: channel %1").arg(channel));
}




void InstrumentTrackView::fillMidiInputMenu(QMenu* menu)
{
	menu->clear();
	auto& port = model()->m_midiPort;
	auto* receive = menu->addAction(tr("Receive MIDI"), [&port](bool on) { port.setReadable(on); });
	receive->setCheckable(true);
	receive->setChecked(port.isReadable());
	// The devices (with a sequencing MIDI client, as ALSA's or JACK's)
	if (!port.midiClient()->isRaw()) { menu->addMenu(port.m_readablePortsMenu)->setText(tr("Devices")); }
	menu->addSeparator();
	auto* channels = new QActionGroup(menu);
	for (int channel = 0; channel <= MidiChannelCount; ++channel)
	{
		auto* action = menu->addAction(channel == 0 ? tr("All channels (omni)") : tr("Channel %1").arg(channel),
			[&port, channel] { port.setInputChannel(channel); });
		action->setCheckable(true);
		action->setChecked(port.inputChannel() == channel);
		channels->addAction(action);
	}
}




} // namespace lmms::gui
