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
#include <cmath>
#include <numeric>

#include <QAction>
#include <QActionGroup>
#include <QFontDatabase>
#include <QHBoxLayout>
#include <QIntValidator>
#include <QLabel>
#include <QLineEdit>
#include <QMenu>
#include <QMessageBox>
#include <QMouseEvent>
#include <QPainter>
#include <QStyle>
#include <QTimer>
#include <QToolButton>
#include <QWheelEvent>
#include <QWidgetAction>

#include "AudioEngine.h"
#include "ConfigManager.h"
#include "embed.h"
#include "Engine.h"
#include "GuiApplication.h"
#include "MainWindow.h"
#include "MeterDialog.h"
#include "Metronome.h"
#include "Mixer.h"
#include "PianoRoll.h"
#include "Song.h"
#include "SongEditor.h"
#include "Timeline.h"
#include "TrackView.h"
#include "Transport.h"


namespace lmms::gui
{

namespace
{

//! The transport's colours by state: playing green, recording red, counting in yellow (design 03)
const QColor PlayingColor{11, 213, 86};
const QColor RecordingColor{229, 72, 77};
const QColor CountInColor{242, 194, 90};
const QColor StoppedColor{140, 150, 163};

//! A monospaced font, for numbers that do not move as they change
auto clockFont(int pixelSize) -> QFont
{
	auto font = QFontDatabase::systemFont(QFontDatabase::FixedFont);
	font.setPixelSize(pixelSize);
	font.setStyleHint(QFont::Monospace);
	return font;
}

} // namespace




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




TransportClock::TransportClock(bool large, const QString& configKey, Format fallback, QWidget* parent) :
	QWidget(parent),
	m_large(large),
	m_configKey(configKey),
	m_format(fallback)
{
	setObjectName(large ? "transportPosition" : "transportTime");
	setAttribute(Qt::WA_StyledBackground, false);
	setCursor(Qt::PointingHandCursor);
	const auto saved = ConfigManager::inst()->value("transport", configKey, QString::number(static_cast<int>(fallback)));
	setFormat(static_cast<Format>(std::clamp(saved.toInt(), 0, static_cast<int>(Format::Timecode))));
	// (redrawn with the window's periodic update; without the window, as the tests make it, when asked)
	if (getGUI() && getGUI()->mainWindow())
	{
		connect(getGUI()->mainWindow(), &MainWindow::periodicUpdate, this, qOverload<>(&QWidget::update));
	}
}




void TransportClock::setFormat(Format format)
{
	m_format = format;
	setToolTip(tr("%1 (click to show it another way)").arg(formatName(format)));
	ConfigManager::inst()->setValue("transport", m_configKey, QString::number(static_cast<int>(format)));
	updateGeometry();
	update();
}




auto TransportClock::formatName(Format format) -> QString
{
	switch (format)
	{
	case Format::BarsBeats: return tr("Bars and beats");
	case Format::MinutesSeconds: return tr("Minutes and seconds");
	case Format::Samples: return tr("Samples");
	case Format::Timecode: return tr("Timecode (SMPTE)");
	}
	return {};
}




auto TransportClock::formatted(Format format, int ms, std::int64_t frames) -> QString
{
	switch (format)
	{
	case Format::BarsBeats:
	{
		// In the song's bars (whose time signatures the tempo map changes)
		const auto position = Engine::getSong()->musicalPosition();
		return QString{"%1.%2.%3"}.arg(position.bar + 1, 3, 10, QChar{'0'}).arg(position.beat + 1)
			.arg(position.tick, 3, 10, QChar{'0'});
	}
	case Format::MinutesSeconds:
	{
		const auto hours = ms / 3600000;
		const auto minutes = (ms / 60000) % 60;
		const auto seconds = QString{"%1.%2"}.arg((ms / 1000) % 60, 2, 10, QChar{'0'}).arg(ms % 1000, 3, 10, QChar{'0'});
		return hours > 0 ? QString{"%1:%2:%3"}.arg(hours).arg(minutes, 2, 10, QChar{'0'}).arg(seconds)
			: QString{"%1:%2"}.arg(minutes).arg(seconds);
	}
	case Format::Samples:
		return QString::number(frames);
	case Format::Timecode:
	{
		// Hours, minutes, seconds and frames at 30 frames per second (non-drop)
		constexpr int Fps = 30;
		const auto videoFrames = ms * Fps / 1000;
		return QString{"%1:%2:%3:%4"}.arg(videoFrames / (Fps * 3600), 2, 10, QChar{'0'})
			.arg((videoFrames / (Fps * 60)) % 60, 2, 10, QChar{'0'})
			.arg((videoFrames / Fps) % 60, 2, 10, QChar{'0'})
			.arg(videoFrames % Fps, 2, 10, QChar{'0'});
	}
	}
	return {};
}




auto TransportClock::text() const -> QString
{
	auto* song = Engine::getSong();
	const auto ms = song->getMilliseconds();
	// The frames: where the period being played starts while the song plays (exact, along the tempo map); stopped,
	// worked out from the position
	const auto& transport = song->transport();
	const bool playing = transport.state() != Transport::State::Stopped && transport.state() != Transport::State::Paused
		&& song->playMode() == Song::PlayMode::Song;
	const auto frames = playing ? transport.frame()
		: static_cast<std::int64_t>(std::llround(ms / 1000.0 * Engine::audioEngine()->outputSampleRate()));
	return formatted(m_format, ms, frames);
}




void TransportClock::setStateColor(const QColor& color)
{
	if (m_stateColor == color) { return; }
	m_stateColor = color;
	update();
}




QSize TransportClock::sizeHint() const
{
	// Wide enough for the longest of its format (the samples of an hour)
	const auto font = clockFont(m_large ? 22 : 14);
	const auto widest = [&] {
		switch (m_format)
		{
		case Format::BarsBeats: return QString{"000.0.000"};
		case Format::MinutesSeconds: return QString{"0:00:00.000"};
		case Format::Samples: return QString{"000000000"};
		case Format::Timecode: return QString{"00:00:00:00"};
		}
		return QString{"000.0.000"};
	}();
	return {QFontMetrics{font}.horizontalAdvance(widest) + 20, m_large ? 34 : 26};
}




void TransportClock::paintEvent(QPaintEvent*)
{
	auto p = QPainter{this};
	p.setRenderHint(QPainter::Antialiasing);
	const auto frame = QRectF{rect()}.adjusted(0.5, 0.5, -0.5, -0.5);
	p.setPen(QPen{m_stateColor.isValid() ? m_stateColor : m_borderColor, m_large ? 1.5 : 1.0});
	p.setBrush(m_backgroundColor);
	p.drawRoundedRect(frame, 4, 4);
	p.setFont(clockFont(m_large ? 22 : 14));
	p.setPen(m_large ? m_textColor : m_textColor.darker(115));
	p.drawText(rect(), Qt::AlignCenter, text());
}




void TransportClock::mousePressEvent(QMouseEvent* event)
{
	if (event->button() != Qt::LeftButton) { return; }
	// The formats, the one shown checked
	auto menu = QMenu{this};
	auto* formats = new QActionGroup(&menu);
	for (const auto format : {Format::BarsBeats, Format::MinutesSeconds, Format::Samples, Format::Timecode})
	{
		auto* action = menu.addAction(formatName(format));
		action->setCheckable(true);
		action->setChecked(format == m_format);
		formats->addAction(action);
		connect(action, &QAction::triggered, this, [this, format] { setFormat(format); });
	}
	menu.exec(mapToGlobal(QPoint{0, height()}));
}




TransportTempo::TransportTempo(QWidget* parent) :
	QWidget(parent)
{
	setObjectName("transportTempo");
	setCursor(Qt::SizeVerCursor);
	setToolTip(tr("The song's tempo (BPM): drag up or down, turn the wheel, double-click to type it; a click "
		"offers tapping it and the tempo map"));
	auto* song = Engine::getSong();
	connect(&song->tempoModel(), &IntModel::dataChanged, this, qOverload<>(&QWidget::update));
}




QSize TransportTempo::sizeHint() const
{
	return {QFontMetrics{clockFont(14)}.horizontalAdvance("♩ 000") + 22, 26};
}




void TransportTempo::paintEvent(QPaintEvent*)
{
	auto p = QPainter{this};
	p.setRenderHint(QPainter::Antialiasing);
	const auto frame = QRectF{rect()}.adjusted(0.5, 0.5, -0.5, -0.5);
	p.setPen(m_tapping ? PlayingColor : QColor{59, 66, 74});
	p.setBrush(QColor{38, 43, 48});
	p.drawRoundedRect(frame, 4, 4);
	p.setFont(clockFont(14));
	p.setPen(QColor{209, 216, 228});
	const auto text = m_tapping ? tr("Tap") : QString{"♩ %1"}.arg(Engine::getSong()->getTempo());
	p.drawText(rect(), Qt::AlignCenter, text);
}




void TransportTempo::setTempo(int bpm)
{
	auto& model = Engine::getSong()->tempoModel();
	model.setValue(static_cast<float>(std::clamp(bpm, static_cast<int>(MinTempo), static_cast<int>(MaxTempo))));
}




void TransportTempo::mousePressEvent(QMouseEvent* event)
{
	if (event->button() != Qt::LeftButton) { return; }
	if (m_tapping)
	{
		tap();
		return;
	}
	m_pressY = event->position().toPoint().y();
	m_pressTempo = Engine::getSong()->getTempo();
	m_dragged = false;
}




void TransportTempo::mouseMoveEvent(QMouseEvent* event)
{
	if (!(event->buttons() & Qt::LeftButton) || m_tapping) { return; }
	// Up: faster, a beat per minute every 3 pixels
	const auto moved = m_pressY - event->position().toPoint().y();
	if (std::abs(moved) < 3 && !m_dragged) { return; }
	m_dragged = true;
	setTempo(m_pressTempo + moved / 3);
}




void TransportTempo::mouseReleaseEvent(QMouseEvent* event)
{
	if (event->button() != Qt::LeftButton || m_tapping) { return; }
	// A click (not a drag): what can be done with it
	if (!m_dragged) { showMenu(); }
	m_dragged = false;
}




void TransportTempo::mouseDoubleClickEvent(QMouseEvent* event)
{
	if (event->button() == Qt::LeftButton && !m_tapping) { startTyping(); }
}




void TransportTempo::wheelEvent(QWheelEvent* event)
{
	// A beat per minute a notch (ten with Shift)
	const auto notches = event->angleDelta().y() / 120;
	if (notches == 0) { return; }
	const auto step = event->modifiers() & Qt::ShiftModifier ? 10 : 1;
	setTempo(Engine::getSong()->getTempo() + notches * step);
	event->accept();
}




void TransportTempo::showMenu()
{
	auto menu = QMenu{this};
	menu.addAction(tr("Type the tempo..."), this, &TransportTempo::startTyping);
	menu.addAction(tr("Tap the tempo"), this, &TransportTempo::startTapping);
	menu.addSeparator();
	menu.addAction(tr("Change the tempo from the playhead on (tempo map)..."), this, [] {
		// On the composition page, where the tempo map is (under the ruler)
		auto* window = getGUI()->mainWindow();
		window->showPage(MainWindow::Page::Composition);
		getGUI()->songEditor()->m_editor->changeTempoAtPlayhead();
	});
	menu.exec(mapToGlobal(QPoint{0, height()}));
}




void TransportTempo::startTyping()
{
	if (m_editor) { return; }
	m_editor = new QLineEdit(QString::number(Engine::getSong()->getTempo()), this);
	m_editor->setObjectName("transportTempoEditor");
	m_editor->setValidator(new QIntValidator(static_cast<int>(MinTempo), static_cast<int>(MaxTempo), m_editor));
	m_editor->setAlignment(Qt::AlignCenter);
	m_editor->setGeometry(rect());
	m_editor->selectAll();
	m_editor->show();
	m_editor->setFocus();
	const auto finish = [this](bool keep) {
		if (!m_editor) { return; }
		auto* editor = std::exchange(m_editor, nullptr);
		if (keep && editor->hasAcceptableInput()) { setTempo(editor->text().toInt()); }
		editor->deleteLater();
		update();
	};
	connect(m_editor, &QLineEdit::returnPressed, this, [finish] { finish(true); });
	connect(m_editor, &QLineEdit::editingFinished, this, [finish] { finish(true); });
}




void TransportTempo::startTapping()
{
	m_tapping = true;
	m_taps.clear();
	m_sinceTap.invalidate();
	setCursor(Qt::PointingHandCursor);
	setToolTip(tr("Click on each beat; it stops two seconds after the last click"));
	update();
	// Ends by itself if no tap comes (and two seconds after the last one: see tap())
	QTimer::singleShot(4000, this, [this] {
		if (m_tapping && !m_sinceTap.isValid()) { stopTapping(); }
	});
}




void TransportTempo::tap()
{
	if (m_sinceTap.isValid())
	{
		const auto interval = m_sinceTap.restart();
		// A pause of more than two seconds starts the count again
		if (interval > 2000) { m_taps.clear(); }
		else
		{
			m_taps.push_back(interval);
			// The last eight intervals
			if (m_taps.size() > 8) { m_taps.erase(m_taps.begin()); }
			const auto average = std::accumulate(m_taps.begin(), m_taps.end(), 0.0) / static_cast<double>(m_taps.size());
			if (average > 0) { setTempo(static_cast<int>(std::lround(60000.0 / average))); }
		}
	}
	else { m_sinceTap.start(); }
	// Two seconds without a tap: done
	QTimer::singleShot(2100, this, [this] {
		if (m_tapping && m_sinceTap.isValid() && m_sinceTap.elapsed() >= 2000) { stopTapping(); }
	});
}




void TransportTempo::stopTapping()
{
	m_tapping = false;
	setCursor(Qt::SizeVerCursor);
	setToolTip(tr("The song's tempo (BPM): drag up or down, turn the wheel, double-click to type it; a click "
		"offers tapping it and the tempo map"));
	update();
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

	// The buttons together in one rounded group, each lit in its colour when on (play green, record red, cycle
	// green): to the start, stop, play, record, cycle
	auto group = new QWidget(this);
	group->setObjectName("transportGroup");
	group->setAttribute(Qt::WA_StyledBackground);
	auto groupLayout = new QHBoxLayout(group);
	groupLayout->setContentsMargins(3, 2, 3, 2);
	groupLayout->setSpacing(1);
	layout->addWidget(group);

	m_toStartButton = addButton("back_to_start", tr("To the start of the song"), "transportToStart");
	m_stopButton = addButton("stop", tr("Stop (Space)"), "transportStop");
	m_playButton = addButton("play", tr("Play/pause the song (Space)"), "transportPlay");
	m_playButton->setCheckable(true);
	m_recordButton = addButton("record", recordToolTip(), "transportRecord");
	m_recordButton->setCheckable(true);
	auto recordMenu = new QMenu(m_recordButton);
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

	connect(m_toStartButton, &QToolButton::clicked, this, &TransportBar::toStart);
	connect(m_playButton, &QToolButton::clicked, this, &TransportBar::play);
	connect(m_recordButton, &QToolButton::clicked, this, &TransportBar::record);
	connect(m_stopButton, &QToolButton::clicked, this, &TransportBar::stop);
	for (auto button : {m_toStartButton, m_stopButton, m_playButton, m_recordButton, m_cycleButton})
	{
		groupLayout->addWidget(button);
	}

	layout->addSpacing(8);

	// Where the song is: large, in bars; beside it smaller, in time; and the state in a word
	m_position = new TransportClock(true, "positionformat", TransportClock::Format::BarsBeats, this);
	m_time = new TransportClock(false, "timeformat", TransportClock::Format::MinutesSeconds, this);
	layout->addWidget(m_position);
	layout->addWidget(m_time);
	m_state = new QLabel(this);
	m_state->setObjectName("transportState");
	m_state->setMinimumWidth(QFontMetrics{m_state->font()}.horizontalAdvance(tr("COUNT-IN 4...")) + 8);
	layout->addWidget(m_state);

	layout->addSpacing(8);

	// The tempo, the time signature, the metronome and the pre-roll
	layout->addWidget(new TransportTempo(this));

	m_timeSigButton = new QToolButton(this);
	m_timeSigButton->setObjectName("transportTimeSig");
	m_timeSigButton->setToolTip(tr("The song's time signature (the tempo map changes it along the song)"));
	m_timeSigButton->setPopupMode(QToolButton::InstantPopup);
	m_timeSigButton->setFocusPolicy(Qt::NoFocus);
	auto* timeSigMenu = new QMenu(m_timeSigButton);
	auto* timeSigAction = new QWidgetAction(timeSigMenu);
	auto* timeSig = new MeterDialog(timeSigMenu, true);
	timeSig->setModel(&song->getTimeSigModel());
	timeSigAction->setDefaultWidget(timeSig);
	timeSigMenu->addAction(timeSigAction);
	m_timeSigButton->setMenu(timeSigMenu);
	const auto showTimeSig = [this] {
		auto& meter = Engine::getSong()->getTimeSigModel();
		m_timeSigButton->setText(QString{"%1/%2"}.arg(meter.getNumerator()).arg(meter.getDenominator()));
	};
	showTimeSig();
	connect(&song->getTimeSigModel(), &Model::dataChanged, this, showTimeSig);
	layout->addWidget(m_timeSigButton);

	m_metronomeButton = addButton("metronome", tr("Metronome"), "transportMetronome");
	m_metronomeButton->setCheckable(true);
	m_metronomeButton->setChecked(song->metronome().active());
	connect(m_metronomeButton, &QToolButton::toggled, this, [](bool on) {
		Engine::getSong()->metronome().setActive(on);
	});
	layout->addWidget(m_metronomeButton);

	// The recording's pre-roll and count-in, in a button of their own that shows the bars
	m_preRollButton = new QToolButton(this);
	m_preRollButton->setObjectName("transportPreRoll");
	m_preRollButton->setPopupMode(QToolButton::InstantPopup);
	m_preRollButton->setFocusPolicy(Qt::NoFocus);
	m_preRollButton->setToolTip(tr("Bars played before a recording starts, and whether the metronome counts them in"));
	auto* preRollMenu = new QMenu(m_preRollButton);
	auto* preRolls = new QActionGroup(preRollMenu);
	for (const int bars : {0, 1, 2, 4})
	{
		auto* action = preRollMenu->addAction(bars == 0 ? tr("No pre-roll") : tr("%n bar(s) of pre-roll", "", bars));
		action->setCheckable(true);
		action->setData(bars);
		preRolls->addAction(action);
		connect(action, &QAction::triggered, this, [bars] {
			ConfigManager::inst()->setValue("recording", "prerollbars", QString::number(bars));
		});
	}
	preRollMenu->addSeparator();
	auto* countIn = preRollMenu->addAction(tr("Count-in: the metronome clicks during the pre-roll"));
	countIn->setCheckable(true);
	countIn->setObjectName("countIn");
	connect(countIn, &QAction::toggled, this, [](bool on) {
		ConfigManager::inst()->setValue("recording", "countin", on ? "1" : "0");
	});
	m_preRollButton->setMenu(preRollMenu);
	layout->addWidget(m_preRollButton);
	// (the settings change it too)
	connect(ConfigManager::inst(), &ConfigManager::valueChanged, this, [this](const QString& cls) {
		if (cls == "recording") { updatePreRollButton(); }
	});
	updatePreRollButton();

	layout->addSpacing(8);

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

	layout->addSpacing(8);

	layout->addWidget(new TransportMeters(this));

	connect(song, &Song::playbackStateChanged, this, &TransportBar::updateState);
	// The audio thread's changes (a pre-roll that ends) are read by timer; the main thread's come at once
	connect(&song->transport(), &Transport::stateChanged, this, &TransportBar::updateState);
	connect(getGUI()->mainWindow(), &MainWindow::periodicUpdate, this, &TransportBar::updateState);
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
	button->setIconSize({20, 20});
	button->setToolTip(toolTip);
	button->setAutoRaise(true);
	button->setFocusPolicy(Qt::NoFocus);
	return button;
}




void TransportBar::play()
{
	// The song (the Pattern Editor has its own play button)
	Engine::getSong()->transport().play();
}




void TransportBar::stop()
{
	Engine::getSong()->transport().stop();
}




void TransportBar::toStart()
{
	// Playing, it goes on from the start; stopped, it waits there
	Engine::getSong()->setPlayPos(0, Song::PlayMode::Song);
	updateState();
}




void TransportBar::record()
{
	// With nothing armed, the selected track is armed (as in REAPER), if it records
	auto* selected = getGUI()->songEditor()->m_editor->selectedTrack();
	const auto result = Engine::getSong()->transport().record(selected ? selected->getTrack() : nullptr);
	if (result == Transport::RecordResult::NothingArmed)
	{
		QMessageBox::information(this, tr("Record"), tr("No track is armed. Arm a track with its R button, or "
			"select an audio or instrument track (click its header) and press Record again: it is armed."));
	}
	updateState();
}




auto TransportBar::recordToolTip() -> QString
{
	return tr("Record the armed tracks: audio into audio tracks, notes into instrument tracks (with no track armed, "
		"the selected one is armed) (R)");
}




void TransportBar::toggleCycle()
{
	m_cycleButton->toggle();
}




void TransportBar::recordWhilePlaying()
{
	getGUI()->pianoRoll()->recordAccompanyAction()->trigger();
}




void TransportBar::updatePreRollButton()
{
	const auto bars = ConfigManager::inst()->value("recording", "prerollbars", "0").toInt();
	const bool countIn = ConfigManager::inst()->value("recording", "countin", "0").toInt() != 0;
	m_preRollButton->setText(bars == 0 ? tr("No pre-roll") : tr("Pre-roll %1").arg(bars));
	for (auto* action : m_preRollButton->menu()->actions())
	{
		const QSignalBlocker blocker{action};
		if (action->objectName() == "countIn") { action->setChecked(countIn); }
		else if (action->isCheckable()) { action->setChecked(action->data().toInt() == bars); }
	}
}




auto TransportBar::stateText() const -> QString
{
	return m_state->text();
}




void TransportBar::updateState()
{
	auto* song = Engine::getSong();
	const auto& transport = song->transport();
	const auto state = transport.state();
	const bool playing = state != Transport::State::Stopped && state != Transport::State::Paused;
	m_playButton->setIcon(embed::getIconPixmap(playing ? "pause" : "play"));
	m_playButton->setChecked(playing);
	m_recordButton->setChecked(transport.isRecording());
	const bool waiting = state == Transport::State::PreRoll || state == Transport::State::CountIn;
	m_recordButton->setToolTip(waiting ? tr("Recording after the pre-roll...") : recordToolTip());

	// The state in a word, in its colour, and the position's border with it
	auto text = QString{};
	auto color = StoppedColor;
	switch (state)
	{
	case Transport::State::Stopped: text = tr("STOPPED"); break;
	case Transport::State::Paused: text = tr("PAUSED"); break;
	case Transport::State::Playing: text = tr("PLAYING"); color = PlayingColor; break;
	case Transport::State::Recording: text = tr("RECORDING"); color = RecordingColor; break;
	case Transport::State::PreRoll:
	case Transport::State::CountIn:
	{
		// The bars still before the recording
		const auto start = transport.recordingStartTick();
		const auto now = song->getPlayPos(Song::PlayMode::Song).getTicks();
		const auto bars = start > now ? (start - now + song->ticksPerBar() - 1) / song->ticksPerBar() : 0;
		text = (state == Transport::State::CountIn ? tr("COUNT-IN %1...") : tr("PRE-ROLL %1...")).arg(bars);
		color = CountInColor;
		break;
	}
	}
	if (m_state->text() != text)
	{
		m_state->setText(text);
		m_state->setStyleSheet(QString{"color: %1;"}.arg(color.name()));
	}
	const auto border = state == Transport::State::Stopped || state == Transport::State::Paused ? QColor{} : color;
	m_position->setStateColor(border);
	m_time->setStateColor(QColor{});
}

} // namespace lmms::gui
