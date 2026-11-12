/*
 * SampleTrack.h - class SampleTrack, a track which provides arrangement of samples
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

#ifndef LMMS_SAMPLE_TRACK_H
#define LMMS_SAMPLE_TRACK_H

#include "AudioBusHandle.h"
#include "AudioInput.h"
#include "MixerChannelLcdModel.h"
#include "Track.h"


namespace lmms
{

namespace gui
{

class SampleTrackView;
class SampleTrackWindow;

} // namespace gui


class SampleTrack : public Track
{
	Q_OBJECT
public:
	SampleTrack( TrackContainer* tc );
	~SampleTrack() override;

	bool play( const TimePos & _start, const f_cnt_t _frames,
						const f_cnt_t _frame_base, int _clip_num = -1 ) override;
	gui::TrackView * createView( gui::TrackContainerView* tcv ) override;
	Clip* createClip(const TimePos & pos) override;


	void saveTrackSpecificSettings(QDomDocument& doc, QDomElement& parent, bool presetMode) override;
	void loadTrackSpecificSettings( const QDomElement & _this ) override;

	inline IntModel * mixerChannelModel()
	{
		return &m_mixerChannelModel;
	}

	FloatModel* volumeModel() { return &m_volumeModel; }
	FloatModel* panningModel() { return &m_panningModel; }

	/**
	 * Recording (stage 3, docs/etapa-3-gravacao.md): audio input, arming and monitoring are the tracks', as in
	 * REAPER, not the mixer channels'. Where the track records from, whether it records, and when it plays its
	 * input back.
	 */
	enum class MonitorMode
	{
		Off,	//!< never play the input
		On,		//!< always play the input
		Auto	//!< play the input while armed for recording
	};
	auto input() const -> const AudioInput& { return m_input; }
	void setInput(const AudioInput& input);
	BoolModel* recordArmModel() { return &m_recordArmModel; }
	IntModel* monitorModel() { return &m_monitorModel; }	//!< a MonitorMode

	inline AudioBusHandle* audioBusHandle()
	{
		return &m_audioBusHandle;
	}

	QString nodeName() const override
	{
		return "sampletrack";
	}

	bool isPlaying()
	{
		return m_isPlaying;
	}

	void setPlaying(bool playing)
	{
		if (m_isPlaying != playing) { emit playingChanged(); }
		m_isPlaying = playing;
	}

signals:
	void playingChanged();
	void inputChanged();

public slots:
	void updateClips();
	void setPlayingClips( bool isPlaying );
	void updateMixerChannel();

private:
	FloatModel m_volumeModel;
	FloatModel m_panningModel;
	MixerChannelLcdModel m_mixerChannelModel;
	AudioInput m_input;
	BoolModel m_recordArmModel;
	IntModel m_monitorModel;
	AudioBusHandle m_audioBusHandle;
	bool m_isPlaying;



	friend class gui::SampleTrackView;
	friend class gui::SampleTrackWindow;

} ;


} // namespace lmms

#endif // LMMS_SAMPLE_TRACK_H
