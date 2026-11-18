/*
 * TransportBar.h - the main window's central transport bar
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

#ifndef LMMS_GUI_TRANSPORT_BAR_H
#define LMMS_GUI_TRANSPORT_BAR_H

#include <cstdint>
#include <vector>

#include <QColor>
#include <QElapsedTimer>
#include <QWidget>

class QLabel;
class QLineEdit;
class QToolButton;

namespace lmms::gui
{

//! CPU load and master output level, drawn as two flat bars
class TransportMeters : public QWidget
{
	Q_OBJECT
public:
	explicit TransportMeters(QWidget* parent);

	QSize sizeHint() const override;

public slots:
	void updateLevels();

protected:
	void paintEvent(QPaintEvent* event) override;

private:
	float m_cpu = 0.f; //!< 0..1
	float m_out = 0.f; //!< 0..1, peak of the master channel
};


/**
 * Where the song is, in a format chosen with a click on it (design 03): bars and beats, minutes and seconds,
 * samples or timecode. Its border takes the transport's colour (playing, recording, counting in).
 */
class TransportClock : public QWidget
{
	Q_OBJECT
	Q_PROPERTY(QColor textColor MEMBER m_textColor)
	Q_PROPERTY(QColor backgroundColor MEMBER m_backgroundColor)
	Q_PROPERTY(QColor borderColor MEMBER m_borderColor)
public:
	enum class Format
	{
		BarsBeats,
		MinutesSeconds,
		Samples,
		Timecode
	};

	//! @param large the main one; @param configKey where its format is kept
	TransportClock(bool large, const QString& configKey, Format fallback, QWidget* parent);

	auto format() const -> Format { return m_format; }
	void setFormat(Format format);
	//! The position as this clock shows it now
	auto text() const -> QString;
	//! The border's colour (the transport's state); invalid: the theme's
	void setStateColor(const QColor& color);

	static auto formatName(Format format) -> QString;
	//! @p ms milliseconds of the song (and @p frames, its frames) in @p format
	static auto formatted(Format format, int ms, std::int64_t frames) -> QString;

	QSize sizeHint() const override;

protected:
	void paintEvent(QPaintEvent* event) override;
	void mousePressEvent(QMouseEvent* event) override;

private:
	bool m_large;
	QString m_configKey;
	Format m_format;
	QColor m_stateColor;
	QColor m_textColor = QColor{43, 212, 120};
	QColor m_backgroundColor = QColor{10, 12, 14};
	QColor m_borderColor = QColor{44, 50, 56};
};


/**
 * The tempo, edited in place (design 03): dragged up or down, with the wheel, typed (a double click), tapped, and a
 * way to the tempo map. A click opens those choices.
 */
class TransportTempo : public QWidget
{
	Q_OBJECT
public:
	explicit TransportTempo(QWidget* parent);

	//! A tap: after two, the tempo is the one tapped (the last few taps' average)
	void tap();
	//! Whether it is counting taps (each click is one)
	auto tapping() const -> bool { return m_tapping; }
	void startTapping();
	void startTyping();

	QSize sizeHint() const override;

protected:
	void paintEvent(QPaintEvent* event) override;
	void mousePressEvent(QMouseEvent* event) override;
	void mouseMoveEvent(QMouseEvent* event) override;
	void mouseReleaseEvent(QMouseEvent* event) override;
	void mouseDoubleClickEvent(QMouseEvent* event) override;
	void wheelEvent(QWheelEvent* event) override;

private:
	void setTempo(int bpm);
	void showMenu();
	void stopTapping();

	QLineEdit* m_editor = nullptr;
	int m_pressY = 0;
	int m_pressTempo = 0;
	bool m_dragged = false;
	bool m_tapping = false;
	QElapsedTimer m_sinceTap;
	std::vector<qint64> m_taps; //!< ms between the taps
};


//! The transport in the middle of the main window's toolbar (design 03, approved on 17/11): the buttons, the
//! position in two clocks, the state in a word, the tempo, the time signature, the metronome and the pre-roll, the
//! master's mute and the meters.
class TransportBar : public QWidget
{
	Q_OBJECT
public:
	explicit TransportBar(QWidget* parent);

	//! Starts recording, or ends it (as the record button does) [R]
	void record();
	//! Switches the cycle on or off [C]
	void toggleCycle();

	//! The word saying what the transport does (for the tests and the screenshots)
	auto stateText() const -> QString;

private slots:
	void play();
	void stop();
	void toStart();
	void recordWhilePlaying();
	void updateState();
	void updatePreRollButton();

private:
	QToolButton* addButton(const char* icon, const QString& toolTip, const char* name);
	static auto recordToolTip() -> QString;

	QToolButton* m_toStartButton;
	QToolButton* m_metronomeButton;
	QToolButton* m_playButton;
	QToolButton* m_recordButton;
	QToolButton* m_stopButton;
	QToolButton* m_cycleButton;
	QToolButton* m_preRollButton;
	QToolButton* m_timeSigButton;
	QToolButton* m_masterMuteButton;
	TransportClock* m_position;
	TransportClock* m_time;
	QLabel* m_state;
};

} // namespace lmms::gui

#endif // LMMS_GUI_TRANSPORT_BAR_H
