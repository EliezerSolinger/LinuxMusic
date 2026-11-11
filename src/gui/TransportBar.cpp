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
#include <QPainter>
#include <QToolButton>

#include "AudioEngine.h"
#include "embed.h"
#include "Engine.h"
#include "GuiApplication.h"
#include "LcdSpinBox.h"
#include "MainWindow.h"
#include "MeterDialog.h"
#include "Metronome.h"
#include "Mixer.h"
#include "PatternEditor.h"
#include "PianoRoll.h"
#include "Song.h"
#include "SongEditor.h"
#include "TimeDisplayWidget.h"

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

	const auto master = Engine::mixer()->mixerChannel(0);
	const float peak = std::max(master->m_peakLeft, master->m_peakRight);
	// Decay smoothly instead of jumping down
	m_out = std::max(std::clamp(peak, 0.f, 1.f), m_out * 0.8f);

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

	// Metronome
	m_metronomeButton = addButton("metronome", tr("Metronome"));
	m_metronomeButton->setCheckable(true);
	m_metronomeButton->setChecked(song->metronome().active());
	connect(m_metronomeButton, &QToolButton::toggled, this, [](bool on) {
		Engine::getSong()->metronome().setActive(on);
	});
	layout->addWidget(m_metronomeButton);

	// Part/Song: what the play button plays
	m_partButton = new QToolButton(this);
	m_partButton->setText(tr("Part"));
	m_partButton->setToolTip(tr("Play the pattern open in the Pattern Editor"));
	m_songButton = new QToolButton(this);
	m_songButton->setText(tr("Song"));
	m_songButton->setToolTip(tr("Play the whole song"));
	auto modeGroup = new QButtonGroup(this);
	for (auto button : {m_partButton, m_songButton})
	{
		button->setObjectName("transportModeButton");
		button->setCheckable(true);
		button->setFocusPolicy(Qt::NoFocus);
		modeGroup->addButton(button);
	}
	m_songButton->setChecked(true);
	auto modeLayout = new QVBoxLayout;
	modeLayout->setSpacing(0);
	modeLayout->addWidget(m_partButton);
	modeLayout->addWidget(m_songButton);
	layout->addLayout(modeLayout);

	// Play, record, record while playing, stop
	m_playButton = addButton("play", tr("Play/pause (Space)"));
	m_recordButton = addButton("record", tr("Record MIDI into the clip open in the Piano Roll"));
	m_recordWhilePlayingButton = addButton("record_accompany",
		tr("Record MIDI into the clip open in the Piano Roll while the song or pattern plays"));
	m_stopButton = addButton("stop", tr("Stop (Space)"));
	connect(m_playButton, &QToolButton::clicked, this, &TransportBar::play);
	connect(m_recordButton, &QToolButton::clicked, this, &TransportBar::record);
	connect(m_recordWhilePlayingButton, &QToolButton::clicked, this, &TransportBar::recordWhilePlaying);
	connect(m_stopButton, &QToolButton::clicked, this, &TransportBar::stop);
	for (auto button : {m_playButton, m_recordButton, m_recordWhilePlayingButton, m_stopButton})
	{
		layout->addWidget(button);
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

	// Master volume and pitch
	auto volume = new LcdSpinBox(3, this, tr("Master volume"));
	volume->setModel(&song->masterVolumeModel());
	volume->setLabel(tr("VOL"));
	volume->setToolTip(tr("Master volume (%)"));
	layout->addWidget(volume);

	auto pitch = new LcdSpinBox(2, this, tr("Master pitch"));
	pitch->setModel(&song->masterPitchModel());
	pitch->setLabel(tr("PITCH"));
	pitch->setToolTip(tr("Global transposition (semitones)"));
	layout->addWidget(pitch);

	layout->addSpacing(12);

	layout->addWidget(new TransportMeters(this));

	connect(song, &Song::playbackStateChanged, this, &TransportBar::updateState);
	connect(song, &Song::stopped, this, &TransportBar::updateState);
	updateState();
}




QToolButton* TransportBar::addButton(const char* icon, const QString& toolTip)
{
	auto button = new QToolButton(this);
	button->setIcon(embed::getIconPixmap(icon));
	button->setIconSize({24, 24});
	button->setToolTip(toolTip);
	button->setAutoRaise(true);
	button->setFocusPolicy(Qt::NoFocus);
	return button;
}




void TransportBar::play()
{
	// The editors know how to start, pause and resume their play mode
	const auto gui = getGUI();
	if (m_partButton->isChecked()) { gui->patternEditor()->playAction()->trigger(); }
	else { gui->songEditor()->playAction()->trigger(); }
}




void TransportBar::stop()
{
	getGUI()->songEditor()->stopAction()->trigger();
}




void TransportBar::record()
{
	getGUI()->pianoRoll()->recordAction()->trigger();
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

	// Follow what is playing, e.g. when playback was started from the Pattern Editor
	if (song->isPlaying())
	{
		if (song->playMode() == Song::PlayMode::Pattern) { m_partButton->setChecked(true); }
		else if (song->playMode() == Song::PlayMode::Song) { m_songButton->setChecked(true); }
	}
}

} // namespace lmms::gui
