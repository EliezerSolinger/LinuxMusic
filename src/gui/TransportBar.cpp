/*
 * TransportBar.cpp - the main window's central transport bar
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

#include "TransportBar.h"

#include <algorithm>
#include <QAction>
#include <QButtonGroup>
#include <QHBoxLayout>
#include <QActionGroup>
#include <QMenu>
#include <QMessageBox>
#include <QPainter>
#include <QToolButton>

#include "AudioEngine.h"
#include "ConfigManager.h"
#include "embed.h"
#include "Engine.h"
#include "GuiApplication.h"
#include "InstrumentTrack.h"
#include "LcdSpinBox.h"
#include "MainWindow.h"
#include "MeterDialog.h"
#include "Metronome.h"
#include "Mixer.h"
#include "PatternEditor.h"
#include "PianoRoll.h"
#include "SampleTrack.h"
#include "AudioTrack.h"
#include "Song.h"
#include "SongEditor.h"
#include "TrackView.h"
#include "TimeDisplayWidget.h"
#include "Timeline.h"

namespace lmms::gui
{

TransportMeters::TransportMeters(QWidget* parent)
	: QWidget(parent)
{
	setObjectName("transportMeters");
	setToolTip(tr("CPU load and master output level"));
	connect(getGUI()->mainWindow(), &MainWindow::periodicUpdate, this, &TransportMeters::updateLevels);
}




QSize TransportMeters::sizeHint() const
{
	return {110, 36};
}




void TransportMeters::updateLevels()
{
	m_cpu = std::clamp(Engine::audioEngine()->cpuLoad() / 100.f, 0.f, 1.f);

	// The mixer view consumes the master's peaks, so show its RMS level here
	const auto& meter = Engine::mixer()->mixerChannel(0)->m_meter;
	m_out = std::clamp(std::max(meter.rms(0), meter.rms(1)), 0.f, 1.f);

	update();
}




void TransportMeters::paintEvent(QPaintEvent*)
{
	QPainter p(this);
	p.setRenderHint(QPainter::Antialiasing);

	const auto text = palette().color(QPalette::Text);
	const QColor accent(43, 212, 168);
	const QColor groove(28, 30, 33);
	const QColor hot(232, 84, 72);

	QFont font = p.font();
	font.setPixelSize(10);
	p.setFont(font);

	const int labelWidth = 30;
	const int barHeight = 6;
	const auto drawRow = [&](int y, const QString& label, float value) {
		p.setPen(text);
		p.drawText(QRect(0, y - 6, labelWidth, 12), Qt::AlignLeft | Qt::AlignVCenter, label);
		const QRectF bar(labelWidth, y - barHeight / 2., width() - labelWidth - 1, barHeight);
		p.setPen(Qt::NoPen);
		p.setBrush(groove);
		p.drawRoundedRect(bar, 2, 2);
		p.setBrush(value > 0.95f ? hot : accent);
		p.drawRoundedRect(QRectF(bar.x(), bar.y(), bar.width() * value, bar.height()), 2, 2);
	};

	drawRow(height() / 3, tr("CPU"), m_cpu);
	drawRow(height() * 2 / 3, tr("OUT"), m_out);
}




TransportBar::TransportBar(QWidget* parent)
	: QWidget(parent)
{
	setObjectName("transportBar");
	setAttribute(Qt::WA_StyledBackground); // let the theme give the transport its own background

	auto song = Engine::getSong();
	auto layout = new QHBoxLayout(this);
	layout->setContentsMargins(8, 4, 8, 4);
	layout->setSpacing(4);

	// The transport, after Logic's: its buttons together in one rounded group, each lit in its colour when on
	// (play green, record red, cycle yellow, metronome blue)
	auto group = new QWidget(this);
	group->setObjectName("transportGroup");
	group->setAttribute(Qt::WA_StyledBackground);
	auto groupLayout = new QHBoxLayout(group);
	groupLayout->setContentsMargins(3, 2, 3, 2);
	groupLayout->setSpacing(1);
	layout->addWidget(group);

	m_stopButton = addButton("stop", tr("Stop (Space)"), "transportStop");
	m_playButton = addButton("play", tr("Play/pause the song (Space)"), "transportPlay");
	m_playButton->setCheckable(true);
	m_recordButton = addButton("record", tr("Record the armed tracks: audio into sample tracks, notes into "
		"instrument tracks (with no track armed: record MIDI into the clip open in the Piano Roll) (R)"),
		"transportRecord");
	m_recordButton->setCheckable(true);

	// The recording's pre-roll and count-in, in the button's menu
	auto recordMenu = new QMenu(m_recordButton);
	auto preRollMenu = recordMenu->addMenu(tr("Pre-roll"));
	auto preRolls = new QActionGroup(preRollMenu);
	const auto preRoll = ConfigManager::inst()->value("recording", "prerollbars", "0").toInt();
	for (const int bars : {0, 1, 2, 4})
	{
		auto action = preRollMenu->addAction(bars == 0 ? tr("Off") : tr("%n bar(s)", "", bars));
		action->setCheckable(true);
		action->setChecked(bars == preRoll);
		preRolls->addAction(action);
		connect(action, &QAction::triggered, this, [bars] {
			ConfigManager::inst()->setValue("recording", "prerollbars", QString::number(bars));
		});
	}
	auto countIn = recordMenu->addAction(tr("Count-in: the metronome clicks during the pre-roll"));
	countIn->setCheckable(true);
	countIn->setChecked(ConfigManager::inst()->value("recording", "countin", "0").toInt() != 0);
	connect(countIn, &QAction::toggled, this, [](bool on) {
		ConfigManager::inst()->setValue("recording", "countin", on ? "1" : "0");
	});
	recordMenu->addSeparator();
	auto recordMidi = recordMenu->addAction(embed::getIconPixmap("record_accompany"),
		tr("Record MIDI into the clip open in the Piano Roll while the song plays"));
	connect(recordMidi, &QAction::triggered, this, &TransportBar::recordWhilePlaying);
	m_recordButton->setMenu(recordMenu);
	m_recordButton->setPopupMode(QToolButton::MenuButtonPopup);

	// Cycle: the song plays the range selected on the ruler over and over (C)
	m_cycleButton = addButton("cycle", tr("Cycle: play the range selected on the ruler over and over (C)"),
		"transportCycle");
	m_cycleButton->setCheckable(true);
	auto& timeline = song->getTimeline(Song::PlayMode::Song);
	m_cycleButton->setChecked(timeline.loopEnabled());
	connect(m_cycleButton, &QToolButton::toggled, this, [](bool on) {
		Engine::getSong()->getTimeline(Song::PlayMode::Song).setLoopEnabled(on);
	});
	connect(&timeline, &Timeline::loopEnabledChanged, m_cycleButton, &QToolButton::setChecked);

	m_metronomeButton = addButton("metronome", tr("Metronome"), "transportMetronome");
	m_metronomeButton->setCheckable(true);
	m_metronomeButton->setChecked(song->metronome().active());
	connect(m_metronomeButton, &QToolButton::toggled, this, [](bool on) {
		Engine::getSong()->metronome().setActive(on);
	});

	connect(m_playButton, &QToolButton::clicked, this, &TransportBar::play);
	connect(m_recordButton, &QToolButton::clicked, this, &TransportBar::record);
	connect(m_stopButton, &QToolButton::clicked, this, &TransportBar::stop);
	for (auto button : {m_stopButton, m_playButton, m_recordButton, m_cycleButton, m_metronomeButton})
	{
		groupLayout->addWidget(button);
	}

	layout->addSpacing(12);

	// Clock, tempo and time signature
	layout->addWidget(new TimeDisplayWidget);

	auto tempo = new LcdSpinBox(3, this, tr("Tempo"));
	tempo->setModel(&song->tempoModel());
	tempo->setLabel(tr("TEMPO"));
	tempo->setToolTip(tr("Tempo in BPM"));
	layout->addWidget(tempo);

	auto timeSig = new MeterDialog(this, true);
	timeSig->setModel(&song->getTimeSigModel());
	layout->addWidget(timeSig);

	layout->addSpacing(12);

	// The master's mute (the master's level is its fader, on the mixer)
	m_masterMuteButton = addButton("speaker", tr("Mute the master"), "transportMute");
	m_masterMuteButton->setCheckable(true);
	auto& masterMute = Engine::mixer()->mixerChannel(0)->m_muteModel;
	const auto showMute = [this](bool muted) {
		m_masterMuteButton->setChecked(muted);
		m_masterMuteButton->setIcon(embed::getIconPixmap(muted ? "speaker_slash" : "speaker"));
	};
	showMute(masterMute.value());
	connect(m_masterMuteButton, &QToolButton::toggled, this, [](bool on) {
		Engine::mixer()->mixerChannel(0)->m_muteModel.setValue(on);
	});
	connect(&masterMute, &BoolModel::dataChanged, this, [showMute] {
		showMute(Engine::mixer()->mixerChannel(0)->m_muteModel.value());
	});
	layout->addWidget(m_masterMuteButton);

	layout->addSpacing(12);

	layout->addWidget(new TransportMeters(this));

	connect(song, &Song::playbackStateChanged, this, &TransportBar::updateState);
	connect(song, &Song::recordingProblem, this, [this](const QString& message) {
		QMessageBox::warning(this, tr("Recording"), message);
	});
	connect(song, &Song::stopped, this, &TransportBar::updateState);
	updateState();
}




QToolButton* TransportBar::addButton(const char* icon, const QString& toolTip, const char* name)
{
	auto button = new QToolButton(this);
	button->setObjectName(name);
	button->setIcon(embed::getIconPixmap(icon));
	button->setIconSize({24, 24});
	button->setToolTip(toolTip);
	button->setAutoRaise(true);
	button->setFocusPolicy(Qt::NoFocus);
	return button;
}




void TransportBar::play()
{
	// The song (the Pattern Editor has its own play button); the editor knows how to start, pause and resume it
	getGUI()->songEditor()->playAction()->trigger();
}




void TransportBar::stop()
{
	getGUI()->songEditor()->stopAction()->trigger();
}




void TransportBar::record()
{
	const auto song = Engine::getSong();

	// While recording, the button ends the recording and the song plays on (punch out)
	if (song->isRecording())
	{
		song->stopRecording();
		return;
	}

	// The tracks that record: audio tracks (their input) and instrument tracks (the notes played)
	const auto armedModel = [](Track* track) -> BoolModel* {
		if (auto* audioTrack = dynamic_cast<AudioTrack*>(track)) { return audioTrack->recordArmModel(); }
		if (auto* instrumentTrack = dynamic_cast<InstrumentTrack*>(track)) { return instrumentTrack->recordArmModel(); }
		return nullptr;
	};
	const auto& tracks = song->tracks();
	const bool armed = std::any_of(tracks.begin(), tracks.end(), [&](Track* track) {
		const auto* model = armedModel(track);
		return model && model->value();
	});
	if (!armed)
	{
		// As in REAPER: nothing armed, the selected track is armed, if it records
		auto* selected = getGUI()->songEditor()->m_editor->selectedTrack();
		auto* model = selected ? armedModel(selected->getTrack()) : nullptr;
		if (!model)
		{
			QMessageBox::information(this, tr("Record"), tr("No track is armed. Arm a track with its R button, or "
				"select an audio or instrument track (click its header) and press Record again: it is armed."));
			return;
		}
		model->setValue(true);
	}
	song->record();
	updateState();
}




void TransportBar::toggleCycle()
{
	m_cycleButton->toggle();
}




void TransportBar::recordWhilePlaying()
{
	getGUI()->pianoRoll()->recordAccompanyAction()->trigger();
}




void TransportBar::updateState()
{
	const auto song = Engine::getSong();
	const bool playing = song->isPlaying() && !song->isPaused();
	m_playButton->setIcon(embed::getIconPixmap(playing ? "pause" : "play"));
	m_playButton->setChecked(playing);
	m_recordButton->setChecked(song->isRecording());
}

} // namespace lmms::gui
