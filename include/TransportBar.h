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

#include <QWidget>

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


//! The transport in the middle of the main window's toolbar, after the LMMS redesign concept (LMMS#1911):
//! metronome, Part/Song mode, play/record/stop, clock, tempo, time signature, master volume and pitch,
//! and the CPU and output meters.
class TransportBar : public QWidget
{
	Q_OBJECT
public:
	explicit TransportBar(QWidget* parent);

private slots:
	void play();
	void stop();
	void record();
	void recordWhilePlaying();
	void updateState();

private:
	QToolButton* addButton(const char* icon, const QString& toolTip);

	QToolButton* m_metronomeButton;
	QToolButton* m_partButton;
	QToolButton* m_songButton;
	QToolButton* m_playButton;
	QToolButton* m_recordButton;
	QToolButton* m_recordWhilePlayingButton;
	QToolButton* m_stopButton;
};

} // namespace lmms::gui

#endif // LMMS_GUI_TRANSPORT_BAR_H
