/*
 * AudioDevice.h - base-class for audio-devices, used by LMMS audio engine
 *
 * Copyright (c) 2004-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_AUDIO_DEVICE_H
#define LMMS_AUDIO_DEVICE_H

#include <QMutex>
#include <QString>
#include <atomic>
#include <samplerate.h>

#include "LmmsTypes.h"

class QThread;

namespace lmms
{

class AudioEngine;
class AudioBusHandle;
class SampleFrame;


class AudioDevice
{
public:
	AudioDevice( const ch_cnt_t _channels, AudioEngine* audioEngine );
	virtual ~AudioDevice();

	// if audio-driver supports ports, classes inheriting AudioBusHandle
	// (e.g. channel-tracks) can register themselves for making
	// audio-driver able to collect their individual output and provide
	// them at a specific port - currently only supported by JACK
	virtual void registerPort(AudioBusHandle* port);
	virtual void unregisterPort(AudioBusHandle* port);
	virtual void renamePort(AudioBusHandle* port);

	/**
	 * Audio input (docs/plano-gravacao.md, "Drivers"): a device that captures hands its input to the engine in the
	 * same callback as it takes the output (AudioEngine::renderNextBuffer(input, output)), so the input is on the
	 * same clock and in the same period as the output, with no ring buffer in between.
	 */
	//! @returns how many input channels the device captures (0 if it does not capture)
	auto inputChannels() const -> ch_cnt_t { return m_inputChannels; }

	//! @returns whether the device captures audio
	bool supportsCapture() const { return m_inputChannels > 0; }

	//! @returns the name of input channel @p channel (zero-based), as the interface shows it
	virtual auto inputChannelName(ch_cnt_t channel) const -> QString;

	//! @returns the frames between a sound reaching the input and the device handing it to the engine
	virtual auto inputLatency() const -> f_cnt_t { return 0; }

	//! @returns the frames between the engine handing a frame to the device and the frame leaving the output
	virtual auto outputLatency() const -> f_cnt_t { return 0; }

	//! @returns whether the device opens its input only while capture is enabled (see setCaptureEnabled())
	virtual auto capturesOnDemand() const -> bool { return false; }

	/**
	 * For devices that capture on demand: opens the input, or closes it while no track records or monitors (which
	 * spares the input's CPU, and the system's privacy prompt until the user chooses an input). Only while the
	 * device is not processing. While the input is closed, the device hands the engine silence on its inputChannels().
	 */
	virtual void setCaptureEnabled(bool) {}

	//! @returns how many over- or underruns the device reported since it opened (any thread)
	auto xruns() const -> unsigned { return m_xruns.load(std::memory_order_relaxed); }

	inline sample_rate_t sampleRate() const
	{
		return m_sampleRate;
	}

	void startProcessing();

	void stopProcessing();

	bool isRunning() const { return m_running.test(std::memory_order_acquire); }

protected:
	// convert a given audio-buffer to a buffer in signed 16-bit samples
	// returns num of bytes in outbuf
	int convertToS16(const SampleFrame* _ab, const f_cnt_t _frames, int_sample_t* _output_buffer,
		const bool _convert_endian = false);

	// clear given signed-int-16-buffer
	void clearS16Buffer(int_sample_t* _outbuf, const f_cnt_t _frames);

	ch_cnt_t channels() const { return m_channels; }

	AudioEngine* audioEngine() { return m_audioEngine; }

	void setSampleRate(const sample_rate_t _new_sr) { m_sampleRate = _new_sr; }
	void setChannels(const ch_cnt_t channels) { m_channels = channels; }

	//! Sets how many input channels the device captures; only while it is not processing
	void setInputChannels(ch_cnt_t channels) { m_inputChannels = channels; }

	//! Counts an over- or underrun (any thread, also the audio callback)
	void reportXrun() { m_xruns.fetch_add(1, std::memory_order_relaxed); }

	static void stopProcessingThread( QThread * thread );
private:
	virtual void startProcessingImpl() = 0;
	virtual void stopProcessingImpl() = 0;

	sample_rate_t m_sampleRate;
	ch_cnt_t m_channels;
	ch_cnt_t m_inputChannels = 0;
	std::atomic<unsigned> m_xruns = 0;
	AudioEngine* m_audioEngine = nullptr;
	std::atomic_flag m_running = ATOMIC_FLAG_INIT;
};

} // namespace lmms

#endif // LMMS_AUDIO_DEVICE_H
