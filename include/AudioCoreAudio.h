/*
 * AudioCoreAudio.h - native CoreAudio driver for macOS: duplex, multichannel, with latencies
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

#ifndef LMMS_AUDIO_CORE_AUDIO_H
#define LMMS_AUDIO_CORE_AUDIO_H

#include "lmmsconfig.h"

#ifdef LMMS_BUILD_APPLE

#include <CoreAudio/CoreAudio.h>

#include <atomic>
#include <vector>

#include <QString>

#include "AudioDevice.h"


namespace lmms
{

/**
 * The macOS driver (docs/plano-gravacao.md, "Drivers"): an IOProc on the device hands the engine the input and takes
 * the output in the same callback, so the input is on the output's clock. When input and output are different
 * devices (e.g. the built-in microphone and speakers), a private aggregate device joins them, with the input's drift
 * compensated to the output's clock. The latencies are the devices' own, their safety offsets, their streams' and
 * the IO buffer.
 *
 * The input is opened only while capture is enabled (AudioEngine::setCaptureEnabled()): opening it asks the user for
 * the microphone, which should happen when they choose an input, not when the program starts. Its channels and
 * their names are known before, from the device's properties.
 */
class AudioCoreAudio : public AudioDevice
{
public:
	AudioCoreAudio(bool& successful, AudioEngine* engine);
	~AudioCoreAudio() override;

	AudioCoreAudio(const AudioCoreAudio&) = delete;
	auto operator=(const AudioCoreAudio&) -> AudioCoreAudio& = delete;

	static auto name() -> QString { return QT_TRANSLATE_NOOP("AudioDeviceSetupWidget", "CoreAudio"); }

	//! Where the settings keep the devices
	static constexpr auto ConfigSection = "audiocoreaudio";
	static constexpr auto ConfigOutputDevice = "outputdevice"; //!< the output device's UID; empty: the system's default
	static constexpr auto ConfigInputDevice = "inputdevice"; //!< the input device's UID; empty: the system's default
	static constexpr auto NoInput = "none"; //!< the input device's value for no input

	auto inputChannelName(ch_cnt_t channel) const -> QString override;
	auto capturesOnDemand() const -> bool override { return true; }
	void setCaptureEnabled(bool enabled) override;
	auto inputLatency() const -> f_cnt_t override { return m_inputLatency; }
	auto outputLatency() const -> f_cnt_t override { return m_outputLatency; }

	//! A device of the system, as the setup lists it
	struct DeviceInfo
	{
		AudioObjectID id = kAudioObjectUnknown;
		QString uid;
		QString name;
		int inputChannels = 0;
		int outputChannels = 0;
	};

	//! @returns the devices of the system
	static auto devices() -> std::vector<DeviceInfo>;

private:
	//! Where one channel of the device sits in the IOProc's buffer lists
	struct ChannelSlot
	{
		UInt32 buffer = 0; //!< which buffer of the list
		UInt32 offset = 0; //!< which channel in the buffer's interleaved frames
		UInt32 stride = 0; //!< the buffer's channels per frame
	};

	void startProcessingImpl() override;
	void stopProcessingImpl() override;

	auto open(AudioObjectID output, AudioObjectID input) -> bool;
	void close();
	auto createAggregate(AudioObjectID output, AudioObjectID input) -> AudioObjectID;

	static auto ioProc(AudioObjectID device, const AudioTimeStamp* now, const AudioBufferList* inputData,
		const AudioTimeStamp* inputTime, AudioBufferList* outputData, const AudioTimeStamp* outputTime,
		void* clientData) -> OSStatus;
	void process(const AudioBufferList* inputData, AudioBufferList* outputData);

	static auto propertyListener(AudioObjectID object, UInt32 count, const AudioObjectPropertyAddress* addresses,
		void* clientData) -> OSStatus;

	AudioObjectID m_output = kAudioObjectUnknown; //!< the output device
	AudioObjectID m_input = kAudioObjectUnknown; //!< the input device, opened only while capturing
	bool m_capturing = false; //!< whether the input is open
	UInt32 m_bufferFrames = 0;
	AudioObjectID m_device = kAudioObjectUnknown; //!< the device the IOProc runs on (maybe the aggregate)
	AudioObjectID m_aggregate = kAudioObjectUnknown; //!< the private aggregate, if one was made
	AudioDeviceIOProcID m_ioProcId = nullptr;

	std::vector<ChannelSlot> m_inputSlots; //!< the device's input channels that the engine takes, in order
	std::vector<ChannelSlot> m_outputSlots; //!< all the device's output channels
	std::vector<QString> m_inputNames;

	f_cnt_t m_inputLatency = 0;
	f_cnt_t m_outputLatency = 0;

	//! Planar scratch for the engine, allocated when the device opens
	f_cnt_t m_capacity = 0;
	std::vector<float> m_inputStorage;
	std::vector<float> m_outputStorage;
	std::vector<float*> m_inputPointers;
	std::vector<float*> m_outputPointers;

	std::atomic<bool> m_alive = true; //!< false once the device went away
};

} // namespace lmms

#endif // LMMS_BUILD_APPLE

#endif // LMMS_AUDIO_CORE_AUDIO_H
