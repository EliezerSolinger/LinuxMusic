/*
 * SampleTrackView.h
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

#ifndef LMMS_GUI_SAMPLE_TRACK_VIEW_H
#define LMMS_GUI_SAMPLE_TRACK_VIEW_H


#include "AudioInputTrack.h"
#include "TrackView.h"

class QMenu;
class QPushButton;

namespace lmms
{

namespace gui
{

class TrackChannelControls;
class VolumeKnob;

class Knob;
class AutomatableButton;
class MixerChannelLcdSpinBox;
class SampleTrackWindow;
class TrackLabelButton;


class SampleTrackView : public TrackView
{
	Q_OBJECT
public:
	SampleTrackView( AudioInputTrack* Track, TrackContainerView* tcv );
	~SampleTrackView() override;

	SampleTrackWindow * getSampleTrackWindow()
	{
		return m_window;
	}

	AudioInputTrack * model()
	{
		return castModel<AudioInputTrack>();
	}

	const AudioInputTrack * model() const
	{
		return castModel<AudioInputTrack>();
	}


	QMenu * createMixerMenu( QString title, QString newMixerLabel ) override;


public slots:
	void showEffects();
	void updateIndicator();


protected:
	void modelChanged() override;
	QString nodeName() const override
	{
		return "SampleTrackView";
	}

	void dragEnterEvent(QDragEnterEvent *dee) override;
	void dropEvent(QDropEvent *de) override;

private slots:
	//! @brief Assign a specific mixer Channel for this track.
	void assignMixerLine(int channelIndex);

	//! @brief Create and assign a new mixer Channel for this track.
	void createMixerLine();
	void corruptStateUpdate();

	void updateInputButton();


private:
	SampleTrackWindow * m_window;
	MixerChannelLcdSpinBox* m_mixerChannelNumber;
	VolumeKnob* m_volumeKnob;
	Knob * m_panningKnob;
	TrackChannelControls* m_channelControls = nullptr;
	FadeButton * m_activityIndicator;
	AutomatableButton* m_recordArmButton;
	QPushButton* m_inputButton;
	QWidget* m_inputMeter;

	//! Lists the audio device's inputs, mono and in stereo pairs, and the monitoring modes
	void fillInputMenu(QMenu* menu);

	TrackLabelButton * m_tlb;

	FadeButton * getActivityIndicator() override
	{
		return m_activityIndicator;
	}

	friend class SampleTrackWindow;
} ;


} // namespace gui

} // namespace lmms

#endif // LMMS_GUI_SAMPLE_TRACK_VIEW_H
