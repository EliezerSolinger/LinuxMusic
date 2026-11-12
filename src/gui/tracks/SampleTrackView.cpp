/*
 * SampleTrackView.cpp
 *
 * Copyright (c) 2005-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "SampleTrackView.h"

#include <QApplication>
#include <QHBoxLayout>
#include <QActionGroup>
#include <QMenu>
#include <QPushButton>
#include <QSpacerItem>
#include <QVBoxLayout>

#include "AudioDevice.h"
#include "AudioEngine.h"
#include "AutomatableButton.h"
#include "ConfigManager.h"
#include "DeprecationHelper.h"
#include "MainWindow.h"
#include "embed.h"
#include "Engine.h"
#include "FadeButton.h"
#include "Mixer.h"
#include "MixerChannelLcdSpinBox.h"
#include "MixerView.h"
#include "GuiApplication.h"
#include "Knob.h"
#include "SampleClip.h"
#include "SampleTrackWindow.h"
#include "SongEditor.h"
#include "StringPairDrag.h"
#include "TrackChannelControls.h"
#include "TrackContainerView.h"
#include "TrackLabelButton.h"


namespace lmms::gui
{


SampleTrackView::SampleTrackView( SampleTrack * _t, TrackContainerView* tcv ) :
	TrackView( _t, tcv )
{
	setFixedHeight( 32 );

	m_tlb = new TrackLabelButton(this, getTrackSettingsWidget());
	m_tlb->setCheckable(true);
	connect(m_tlb, SIGNAL(clicked(bool)),
			this, SLOT(showEffects()));
	m_tlb->setIcon(embed::getIconPixmap("sample_track"));
	m_tlb->show();

	m_mixerChannelNumber = new MixerChannelLcdSpinBox(2, getTrackSettingsWidget(), tr("Mixer channel"), this);
	if (trackContainerView()->trackHeadWidth() < COMPACT_TRACK_WIDTH) { m_mixerChannelNumber->hide(); }
	else { m_mixerChannelNumber->show(); }

	connect(trackContainerView(), &TrackContainerView::trackHeadWidthChanged, this, [this](int width){
		if (width < COMPACT_TRACK_WIDTH) { m_mixerChannelNumber->hide(); }
		else { m_mixerChannelNumber->show(); }
	});

	m_volumeKnob = new VolumeKnob(KnobType::Small17, tr("VOL"), getTrackSettingsWidget(), Knob::LabelRendering::LegacyFixedFontSize, tr("Track volume"));
	m_volumeKnob->setModel( &_t->m_volumeModel );
	m_volumeKnob->setHintText(tr("Volume:"), "%");
	m_volumeKnob->show();

	m_panningKnob = new Knob(KnobType::Small17, tr("PAN"), getTrackSettingsWidget(), Knob::LabelRendering::LegacyFixedFontSize, tr("Panning"));
	m_panningKnob->setModel( &_t->m_panningModel );
	m_panningKnob->setHintText( tr( "Panning:" ), "%" );
	m_panningKnob->show();

	m_activityIndicator = new FadeButton(getTrackSettingsWidget());
	m_activityIndicator->setFixedSize(8, 28);
	m_activityIndicator->show();

	// Recording: arm the track, and choose its input and monitoring
	m_recordArmButton = new AutomatableButton(getTrackSettingsWidget(), tr("Record arm"));
	m_recordArmButton->setCheckable(true);
	m_recordArmButton->setObjectName("btn-arm");
	m_recordArmButton->setText(tr("R"));
	m_recordArmButton->setToolTip(tr("Arm for recording: the Record button records this track's input"));
	m_recordArmButton->setModel(_t->recordArmModel());

	m_inputButton = new QPushButton(getTrackSettingsWidget());
	m_inputButton->setObjectName("btn-input");
	m_inputButton->setFocusPolicy(Qt::NoFocus);
	m_inputButton->setCursor(Qt::PointingHandCursor);
	auto inputMenu = new QMenu(m_inputButton);
	connect(inputMenu, &QMenu::aboutToShow, this, [this, inputMenu] { fillInputMenu(inputMenu); });
	m_inputButton->setMenu(inputMenu);
	connect(_t, &SampleTrack::inputChanged, this, &SampleTrackView::updateInputButton);

	auto masterLayout = new QVBoxLayout(getTrackSettingsWidget());
	masterLayout->setContentsMargins(0, 1, 0, 0);
	auto layout = new QHBoxLayout();
	layout->setContentsMargins(0, 0, 0, 0);
	layout->setSpacing(3);
	layout->addWidget(m_tlb);
	//layout->addStretch();
	layout->addWidget(m_mixerChannelNumber);
	layout->addWidget(m_activityIndicator);
	layout->addWidget(m_recordArmButton);
	layout->addWidget(m_inputButton);
	layout->addWidget(m_volumeKnob);
	layout->addWidget(m_panningKnob);
	layout->addSpacing(4);
	masterLayout->addLayout(layout);
	masterLayout->addSpacerItem(new QSpacerItem(0, 0, QSizePolicy::Minimum, QSizePolicy::Expanding));

	connect(_t, SIGNAL(playingChanged()), this, SLOT(updateIndicator()));
	connect(getGUI()->mainWindow(), &MainWindow::periodicUpdate, this, &SampleTrackView::corruptStateUpdate);

	setModel( _t );
	updateInputButton();

	m_window = new SampleTrackWindow(this);
	m_window->toggleVisibility(false);
}




void SampleTrackView::updateIndicator()
{
	if (model()->isPlaying()) { m_activityIndicator->activateOnce(); }
	else { m_activityIndicator->noteEnd(); }
}




SampleTrackView::~SampleTrackView()
{
	if(m_window != nullptr)
	{
		m_window->setSampleTrackView(nullptr);
		m_window->parentWidget()->hide();
	}
	m_window = nullptr;
}



//FIXME: This is identical to InstrumentTrackView::createMixerMenu
QMenu * SampleTrackView::createMixerMenu(QString title, QString newMixerLabel)
{
	int channelIndex = model()->mixerChannelModel()->value();

	MixerChannel *mixerChannel = Engine::mixer()->mixerChannel(channelIndex);

	// If title allows interpolation, pass channel index and name
	if (title.contains("%2"))
	{
		title = title.arg(channelIndex).arg(mixerChannel->m_name);
	}

	auto mixerMenu = new QMenu(title);

	mixerMenu->addAction(newMixerLabel, this, SLOT(createMixerLine()));
	mixerMenu->addSeparator();

	for (int i = 0; i < Engine::mixer()->numChannels(); ++i)
	{
		MixerChannel * currentChannel = Engine::mixer()->mixerChannel(i);

		if (currentChannel != mixerChannel)
		{
			const auto index = currentChannel->index();
			QString label = tr("%1: %2").arg(index).arg(currentChannel->m_name);
			mixerMenu->addAction(label, [this, index](){
				assignMixerLine(index);
			});
		}
	}

	return mixerMenu;
}




void SampleTrackView::updateInputButton()
{
	const auto& input = model()->input();
	if (input.source != AudioInput::Source::Hardware)
	{
		m_inputButton->setText(tr("-"));
		m_inputButton->setToolTip(tr("Input: none"));
		return;
	}
	const auto first = input.firstChannel + 1;
	m_inputButton->setText(input.channelCount == 2 ? QString{"%1/%2"}.arg(first).arg(first + 1)
		: QString::number(first));
	m_inputButton->setToolTip(input.channelCount == 2
		? tr("Input: %1 and %2 (stereo)").arg(first).arg(first + 1)
		: tr("Input: %1 (mono)").arg(first));
}




void SampleTrackView::fillInputMenu(QMenu* menu)
{
	menu->clear();
	auto* track = model();
	const auto* device = Engine::audioEngine()->audioDev();
	const auto inputs = device ? static_cast<int>(device->inputChannels()) : 0;
	const auto current = track->input();

	auto* choices = new QActionGroup(menu);
	const auto addChoice = [&](const QString& label, const AudioInput& input) {
		auto* action = menu->addAction(label, [track, input] { track->setInput(input); });
		action->setCheckable(true);
		action->setChecked(current == input);
		choices->addAction(action);
	};

	addChoice(tr("No input"), AudioInput{});
	if (inputs == 0)
	{
		menu->addAction(tr("The audio device has no inputs"))->setEnabled(false);
	}
	else
	{
		menu->addSection(tr("Mono"));
		for (int channel = 0; channel < inputs; ++channel)
		{
			addChoice(QString{"%1: %2"}.arg(channel + 1).arg(device->inputChannelName(static_cast<ch_cnt_t>(channel))),
				AudioInput{AudioInput::Source::Hardware, channel, 1});
		}
		if (inputs >= 2)
		{
			menu->addSection(tr("Stereo"));
			for (int channel = 0; channel + 1 < inputs; channel += 2)
			{
				addChoice(tr("%1/%2: %3 + %4").arg(channel + 1).arg(channel + 2)
					.arg(device->inputChannelName(static_cast<ch_cnt_t>(channel)))
					.arg(device->inputChannelName(static_cast<ch_cnt_t>(channel + 1))),
					AudioInput{AudioInput::Source::Hardware, channel, 2});
			}
		}
	}

	// Software monitoring: the input through the track's effects and the mixer
	menu->addSection(tr("Monitoring"));
	auto* monitoring = new QActionGroup(menu);
	const auto mode = track->monitorModel()->value();
	const auto addMode = [&](const QString& label, SampleTrack::MonitorMode value) {
		auto* action = menu->addAction(label, [track, value] { track->monitorModel()->setValue(static_cast<int>(value)); });
		action->setCheckable(true);
		action->setChecked(mode == static_cast<int>(value));
		monitoring->addAction(action);
	};
	addMode(tr("Off"), SampleTrack::MonitorMode::Off);
	addMode(tr("On"), SampleTrack::MonitorMode::On);
	addMode(tr("Auto (while armed, unless playing without recording)"), SampleTrack::MonitorMode::Auto);
}




void SampleTrackView::showEffects()
{
	m_window->toggleVisibility(m_window->parentWidget()->isHidden());
}



void SampleTrackView::modelChanged()
{
	TrackView::modelChanged();

	auto st = castModel<SampleTrack>();
	m_volumeKnob->setModel(&st->m_volumeModel);
	m_mixerChannelNumber->setModel(&st->m_mixerChannelModel);
	m_channelControls = TrackChannelControls::replace(m_channelControls, st, this, {.volume = m_volumeKnob,
		.panning = m_panningKnob, .mute = getTrackOperationsWidget()->muteButton(),
		.solo = getTrackOperationsWidget()->soloButton()});
}




void SampleTrackView::dragEnterEvent(QDragEnterEvent *dee)
{
	StringPairDrag::processDragEnterEvent(dee, QString("samplefile"));
}




void SampleTrackView::dropEvent(QDropEvent *de)
{
	QString type  = StringPairDrag::decodeKey(de);
	QString value = StringPairDrag::decodeValue(de);

	if (type == "samplefile")
	{
		int trackHeadWidth = trackContainerView()->trackHeadWidth();

		const int deX = position(de).x();
		int xPos = deX < trackHeadWidth
				? trackHeadWidth
				: deX;

		const float snapSize = getGUI()->songEditor()->m_editor->getSnapSize();
		TimePos clipPos = trackContainerView()->fixedClips()
				? TimePos(0)
				: TimePos(((xPos - trackHeadWidth) / trackContainerView()->pixelsPerBar()
							* TimePos::ticksPerBar()) + trackContainerView()->currentPosition()
						).quantize(snapSize, true);

		auto sClip = static_cast<SampleClip*>(getTrack()->createClip(clipPos));
		if (sClip) { sClip->setSampleFile(value); }
	}
}




void SampleTrackView::createMixerLine()
{
	int channelIndex = getGUI()->mixerView()->addNewChannel();
	auto channel = Engine::mixer()->mixerChannel(channelIndex);

	channel->m_name = getTrack()->name();
	channel->setColor(getTrack()->color());

	assignMixerLine(channelIndex);
}




void SampleTrackView::assignMixerLine(int channelIndex)
{
	model()->mixerChannelModel()->setValue(channelIndex);

	getGUI()->mixerView()->setCurrentMixerChannel(channelIndex);
}

void SampleTrackView::corruptStateUpdate()
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


} // namespace lmms::gui
