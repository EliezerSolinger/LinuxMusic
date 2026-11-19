/*
 * AudioPortAudio.h - device-class that performs PCM-output via PortAudio
 *
 * Copyright (c) 2008 Csaba Hruska <csaba.hruska/at/gmail.com>
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

#ifndef LMMS_AUDIO_PORTAUDIO_H
#define LMMS_AUDIO_PORTAUDIO_H

#include "lmmsconfig.h"

#ifdef LMMS_HAVE_PORTAUDIO

#include <QString>
#include <portaudio.h>
#include <vector>

#include "AudioDevice.h"

namespace lmms {

//! What PortAudio's settings are called (the driver reads them, its setup widget writes them)
namespace PortAudioSettings {
enum class Direction
{
	Input,
	Output
};

constexpr auto tag()
{
	return "audioportaudio";
}

constexpr auto backendAttribute()
{
	return "backend";
}

constexpr auto deviceNameAttribute(Direction direction)
{
	switch (direction)
	{
	case Direction::Input:
		return "inputdevice";
	case Direction::Output:
		return "outputdevice";
	}

	return "";
}

constexpr auto channelsAttribute(Direction direction)
{
	switch (direction)
	{
	case Direction::Input:
		return "inputchannels";
	case Direction::Output:
		return "outputchannels";
	}

	return "";
}

inline int maxChannels(const PaDeviceInfo* info, Direction direction)
{
	switch (direction)
	{
	case Direction::Input:
		return info->maxInputChannels;
	case Direction::Output:
		return info->maxOutputChannels;
	}

	return 0;
}
} // namespace PortAudioSettings

namespace detail {
class PortAudioInitializationGuard
{
public:
	PortAudioInitializationGuard()
		: m_error(Pa_Initialize())
	{
		if (m_error != paNoError) { throw std::runtime_error{"PortAudio: could not initialize"}; }
	}

	~PortAudioInitializationGuard()
	{
		if (m_error == paNoError) { Pa_Terminate(); }
	}

	PortAudioInitializationGuard(const PortAudioInitializationGuard&) = delete;
	PortAudioInitializationGuard(PortAudioInitializationGuard&&) = delete;
	PortAudioInitializationGuard& operator=(const PortAudioInitializationGuard&) = delete;
	PortAudioInitializationGuard& operator=(PortAudioInitializationGuard&&) = delete;

private:
	PaError m_error = paNoError;
};
} // namespace detail

class AudioPortAudio : public AudioDevice
{
public:
	AudioPortAudio(bool& successful, AudioEngine* engine);
	~AudioPortAudio() override;

	AudioPortAudio(const AudioPortAudio&) = delete;
	AudioPortAudio(AudioPortAudio&&) = delete;
	AudioPortAudio& operator=(const AudioPortAudio&) = delete;
	AudioPortAudio& operator=(AudioPortAudio&&) = delete;

	static auto name() -> QString { return QT_TRANSLATE_NOOP("AudioDeviceSetupWidget", "PortAudio"); }

	auto inputLatency() const -> f_cnt_t override { return m_inputLatency; }
	auto outputLatency() const -> f_cnt_t override { return m_outputLatency; }

private:
	void startProcessingImpl() override;
	void stopProcessingImpl() override;

	static int processCallback(const void* input, void* output, unsigned long frameCount,
		const PaStreamCallbackTimeInfo* timeInfo, PaStreamCallbackFlags statusFlags, void* userData);

	detail::PortAudioInitializationGuard m_initGuard;
	PaStream* m_paStream = nullptr;

	//! The input, made planar for the engine (allocated when the stream opens)
	std::vector<float> m_inputStorage;
	std::vector<float*> m_inputPointers;
	unsigned long m_inputCapacity = 0;

	f_cnt_t m_inputLatency = 0;
	f_cnt_t m_outputLatency = 0;
};
} // namespace lmms

#endif // LMMS_HAVE_PORTAUDIO

#endif // LMMS_AUDIO_PORTAUDIO_H
