/*
 * AudioInputTrack.h
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

#ifndef LMMS_AUDIO_INPUT_TRACK_H
#define LMMS_AUDIO_INPUT_TRACK_H

#include <atomic>

#include "AudioBusHandle.h"
#include "AudioInput.h"
#include "MixerChannelLcdModel.h"
#include "Track.h"

namespace lmms
{

/**
 * A track that plays audio into a mixer channel and records from an input (docs/etapa-3-gravacao.md): the sample
 * track and the audio track. Audio input, arming and monitoring are the track's, as in REAPER, not the mixer
 * channel's: where the track records from, whether it records, and when it plays its input back (through its effects
 * and the mixer, by an InputMonitorHandle).
 */
class LMMS_EXPORT AudioInputTrack : public Track
{
	Q_OBJECT
public:
	AudioInputTrack(Type type, TrackContainer* container, const QString& busName);
	~AudioInputTrack() override;

	IntModel* mixerChannelModel() { return &m_mixerChannelModel; }
	FloatModel* volumeModel() { return &m_volumeModel; }
	FloatModel* panningModel() { return &m_panningModel; }
	AudioBusHandle* audioBusHandle() { return &m_audioBusHandle; }

	enum class MonitorMode
	{
		Off,	//!< never play the input
		On,		//!< always play the input
		Auto	//!< play the input while armed for recording (tape auto: not while only playing)
	};

	auto input() const -> const AudioInput& { return m_input; }
	void setInput(const AudioInput& input);
	BoolModel* recordArmModel() { return &m_recordArmModel; }
	auto isRecordArmed() const -> bool { return m_recordArmModel.value(); }
	IntModel* monitorModel() { return &m_monitorModel; } //!< a MonitorMode
	auto monitorMode() const -> int { return m_monitorModel.value(); } //!< a MonitorMode

	//! Audio thread: the input's peak in a period, for the input meter
	void reportInputPeak(float peak)
	{
		auto current = m_inputPeak.load(std::memory_order_relaxed);
		while (peak > current && !m_inputPeak.compare_exchange_weak(current, peak, std::memory_order_relaxed)) {}
	}
	//! Interface: the input's peak since the last call
	auto takeInputPeak() -> float { return m_inputPeak.exchange(0.f, std::memory_order_relaxed); }

signals:
	void inputChanged();

public slots:
	void updateMixerChannel();

protected:
	//! The effects, volume, panning, mixer channel, arming, monitoring and input, in the track's element
	void saveAudioSettings(QDomDocument& doc, QDomElement& element);
	void loadAudioSettings(const QDomElement& element);

private:
	FloatModel m_volumeModel;
	FloatModel m_panningModel;
	MixerChannelLcdModel m_mixerChannelModel;
	AudioInput m_input;
	std::atomic<float> m_inputPeak = 0.f;
	BoolModel m_recordArmModel;
	IntModel m_monitorModel;
	AudioBusHandle m_audioBusHandle;
};

} // namespace lmms

#endif // LMMS_AUDIO_INPUT_TRACK_H
