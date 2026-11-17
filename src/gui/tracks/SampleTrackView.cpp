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

#include "LiveTakeOverlay.h"

#include <algorithm>
#include <cmath>

#include <QApplication>
#include <QHBoxLayout>
#include <QActionGroup>
#include <QMenu>
#include <QPainter>
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
#include "AudioItem.h"
#include "AudioTrack.h"
#include "SampleClip.h"
#include "SampleTrackWindow.h"
#include "SongEditor.h"
#include "StringPairDrag.h"
#include "TrackChannelControls.h"
#include "TrackContainerView.h"
#include "TrackLabelButton.h"
#include "SampleTrack.h"


namespace lmms::gui
{

namespace
{

/**
 * The level of a track's input, on a scale from -60 to 0 dBFS: green, yellow from -12, red from -3, with the
 * latest peak held for a moment. It shows the input whether it is heard or not, to set the level before recording.
 */
class InputLevelMeter : public QWidget
{
public:
	explicit InputLevelMeter(QWidget* parent)
		: QWidget{parent}
	{
		setFixedSize(4, 26);
		setToolTip(QObject::tr("Input level"));
	}

	void setPeak(float peak)
	{
		m_level = std::max(peak, m_level * 0.8f);
		if (peak >= m_hold) { m_hold = peak; m_holdAge = 0; }
		else if (++m_holdAge > 40) { m_hold = std::max(m_hold * 0.9f, m_level); }
		update();
	}

protected:
	void paintEvent(QPaintEvent*) override
	{
		auto painter = QPainter{this};
		painter.fillRect(rect(), QColor{20, 23, 28});
		const auto heightOf = [this](float level) {
			const auto db = level > 0.f ? 20.f * std::log10(level) : -100.f;
			return static_cast<int>(std::clamp((db + 60.f) / 60.f, 0.f, 1.f) * height());
		};
		const auto bar = heightOf(m_level);
		const auto color = m_level >= 0.708f ? QColor{230, 60, 60}       // -3 dBFS
			: m_level >= 0.251f ? QColor{230, 200, 60}                     // -12 dBFS
			: QColor{71, 253, 133};
		painter.fillRect(0, height() - bar, width(), bar, color);
		const auto hold = heightOf(m_hold);
		if (hold > 0) { painter.fillRect(0, height() - hold, width(), 1, m_hold >= 0.708f ? QColor{255, 80, 80} : color); }
	}

private:
	float m_level = 0.f;
	float m_hold = 0.f;
	int m_holdAge = 0;
};

} // namespace


SampleTrackView::SampleTrackView( AudioInputTrack * _t, TrackContainerView* tcv ) :
	TrackView( _t, tcv )
{
	// The audio track records (two rows: what it is and records, then where it goes); the sample track plays
	// samples on the grid (one row, nothing to record)
	auto* audioTrack = dynamic_cast<AudioTrack*>(_t);
	setFixedHeight(TwoRowHeight);

	// The take growing while this track records
	new LiveTakeOverlay(this, getTrackContentWidget());

	m_tlb = new TrackLabelButton(this, getTrackSettingsWidget());
	m_tlb->setCheckable(true);
	connect(m_tlb, SIGNAL(clicked(bool)),
			this, SLOT(showEffects()));
	m_tlb->setIcon(embed::getIconPixmap(audioTrack ? "audio_track" : "sample_track"));
	m_tlb->show();

	m_mixerChannelNumber = new MixerChannelLcdSpinBox(2, getTrackSettingsWidget(), tr("Mixer channel"), this);
	if (trackContainerView()->trackHeadWidth() < COMPACT_TRACK_WIDTH) { m_mixerChannelNumber->hide(); }
	else { m_mixerChannelNumber->show(); }

	connect(trackContainerView(), &TrackContainerView::trackHeadWidthChanged, this, [this](int width){
		if (width < COMPACT_TRACK_WIDTH) { m_mixerChannelNumber->hide(); }
		else { m_mixerChannelNumber->show(); }
	});

	m_volumeKnob = new VolumeKnob(KnobType::Small17, tr("VOL"), getTrackSettingsWidget(), Knob::LabelRendering::LegacyFixedFontSize, tr("Track volume"));
	m_volumeKnob->setModel( _t->volumeModel() );
	m_volumeKnob->setHintText(tr("Volume:"), "%");
	m_volumeKnob->show();

	m_panningKnob = new Knob(KnobType::Small17, tr("PAN"), getTrackSettingsWidget(), Knob::LabelRendering::LegacyFixedFontSize, tr("Panning"));
	m_panningKnob->setModel( _t->panningModel() );
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
	connect(_t, &AudioInputTrack::inputChanged, this, &SampleTrackView::updateInputButton);

	auto* inputMeter = new InputLevelMeter(getTrackSettingsWidget());
	m_inputMeter = inputMeter;
	connect(getGUI()->mainWindow(), &MainWindow::periodicUpdate, inputMeter, [this, inputMeter] {
		if (inputMeter->isVisible()) { inputMeter->setPeak(model()->takeInputPeak()); }
	});

	auto masterLayout = new QVBoxLayout(getTrackSettingsWidget());
	masterLayout->setContentsMargins(0, 1, 0, 0);
	masterLayout->setSpacing(1);
	if (audioTrack)
	{
		// As in REAPER: the name, activity, arming and input on top; the mixer channel, input meter, VOL and PAN below
		auto top = new QHBoxLayout();
		top->setContentsMargins(0, 0, 0, 0);
		top->setSpacing(3);
		top->addWidget(m_tlb, 1);
		top->addWidget(m_activityIndicator);
		top->addWidget(m_recordArmButton);
		top->addWidget(m_inputButton);
		top->addSpacing(4);
		auto bottom = new QHBoxLayout();
		bottom->setContentsMargins(0, 0, 0, 0);
		bottom->setSpacing(3);
		bottom->addWidget(m_mixerChannelNumber);
		bottom->addWidget(m_inputMeter, 1);
		bottom->addWidget(m_volumeKnob);
		bottom->addWidget(m_panningKnob);
		bottom->addSpacing(4);
		m_activityIndicator->setFixedSize(8, 18);
		masterLayout->addLayout(top);
		masterLayout->addLayout(bottom);
	}
	else
	{
		// Playing samples on the grid: nothing is recorded here (the audio track records)
		m_recordArmButton->hide();
		m_inputButton->hide();
		m_inputMeter->hide();
		auto top = new QHBoxLayout();
		top->setContentsMargins(0, 0, 0, 0);
		top->setSpacing(3);
		top->addWidget(m_tlb, 1);
		top->addWidget(m_activityIndicator);
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
	}
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
	m_inputMeter->setVisible(input.source == AudioInput::Source::Hardware);
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
	const auto addMode = [&](const QString& label, AudioInputTrack::MonitorMode value) {
		auto* action = menu->addAction(label, [track, value] { track->monitorModel()->setValue(static_cast<int>(value)); });
		action->setCheckable(true);
		action->setChecked(mode == static_cast<int>(value));
		monitoring->addAction(action);
	};
	addMode(tr("Off"), AudioInputTrack::MonitorMode::Off);
	addMode(tr("On"), AudioInputTrack::MonitorMode::On);
	addMode(tr("Auto (while armed, unless playing without recording)"), AudioInputTrack::MonitorMode::Auto);
}




void SampleTrackView::showEffects()
{
	m_window->toggleVisibility(m_window->parentWidget()->isHidden());
}



void SampleTrackView::modelChanged()
{
	TrackView::modelChanged();

	auto st = castModel<AudioInputTrack>();
	m_volumeKnob->setModel(st->volumeModel());
	m_mixerChannelNumber->setModel(st->mixerChannelModel());
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
				: trackContainerView()->snap(TimePos(((xPos - trackHeadWidth) / trackContainerView()->pixelsPerBar()
							* TimePos::ticksPerBar()) + trackContainerView()->currentPosition()), snapSize, true);

		auto* clip = getTrack()->createClip(clipPos);
		if (auto* sampleClip = dynamic_cast<SampleClip*>(clip)) { sampleClip->setSampleFile(value); }
		else if (auto* item = dynamic_cast<AudioItem*>(clip)) { item->setFile(value); }
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


namespace lmms
{

gui::TrackView * SampleTrack::createView( gui::TrackContainerView* tcv )
{
	return new gui::SampleTrackView( this, tcv );
}


gui::TrackView* AudioTrack::createView(gui::TrackContainerView* view)
{
	return new gui::SampleTrackView{this, view};
}

} // namespace lmms
