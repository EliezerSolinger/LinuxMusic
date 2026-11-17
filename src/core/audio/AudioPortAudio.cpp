/*
 * AudioPortAudio.cpp - device-class that performs PCM-output via PortAudio
 *
 * Copyright (c) 2008 Csaba Hruska <csaba.hruska/at/gmail.com>
 * Copyright (c) 2010 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include <iostream>

#include "lmmsconfig.h"

#ifdef LMMS_HAVE_PORTAUDIO

#include <algorithm>
#include <cmath>

#include "AudioEngine.h"
#include "AudioPortAudio.h"
#include "ConfigManager.h"

using namespace lmms::PortAudioSettings;

namespace lmms {
AudioPortAudio::AudioPortAudio(bool& successful, AudioEngine* engine)
	: AudioDevice(DEFAULT_CHANNELS, engine)
{
	const auto numDevices = Pa_GetDeviceCount();
	if (numDevices < 0)
	{
		std::cerr << "Pa_GetDeviceCount() failed: " << Pa_GetErrorText(numDevices) << '\n';
		successful = false;
		return;
	}

	const auto backend = ConfigManager::inst()->value(tag(), backendAttribute());
	
	const auto inputDeviceName = ConfigManager::inst()->value(tag(), deviceNameAttribute(Direction::Input));
	const auto inputDeviceChannels = ConfigManager::inst()->value(tag(), channelsAttribute(Direction::Input)).toInt();

	const auto outputDeviceName = ConfigManager::inst()->value(tag(), deviceNameAttribute(Direction::Output));
	const auto outputDeviceChannels = ConfigManager::inst()->value(tag(), channelsAttribute(Direction::Output)).toInt();

	auto inputDeviceIndex = paNoDevice;
	auto outputDeviceIndex = paNoDevice;

	for (auto i = 0; i < numDevices && (inputDeviceIndex == paNoDevice || outputDeviceIndex == paNoDevice); ++i)
	{
		const auto deviceInfo = Pa_GetDeviceInfo(i);
		const auto hostApiInfo = Pa_GetHostApiInfo(deviceInfo->hostApi);
		if (deviceInfo->name == inputDeviceName && hostApiInfo->name == backend) { inputDeviceIndex = i; }
		if (deviceInfo->name == outputDeviceName && hostApiInfo->name == backend) { outputDeviceIndex = i; }
	}

	const auto sampleRate = engine->baseSampleRate();
	const auto framesPerBuffer = engine->framesPerPeriod();
	const auto inputLatency
		= inputDeviceIndex == paNoDevice ? 0. : Pa_GetDeviceInfo(inputDeviceIndex)->defaultLowInputLatency;
	const auto outputLatency
		= outputDeviceIndex == paNoDevice ? 0. : Pa_GetDeviceInfo(outputDeviceIndex)->defaultLowOutputLatency;

	const auto inputParameters = PaStreamParameters {
		.device = inputDeviceIndex,
		.channelCount = inputDeviceChannels,
		.sampleFormat = paFloat32,
		.suggestedLatency = inputLatency,
		.hostApiSpecificStreamInfo = nullptr
	};
	
	const auto outputParameters = PaStreamParameters {
		.device = outputDeviceIndex,
		.channelCount = outputDeviceChannels,
		.sampleFormat = paFloat32,
		.suggestedLatency = outputLatency,
		.hostApiSpecificStreamInfo = nullptr
	};

	const auto inputParametersPtr = inputDeviceIndex == paNoDevice ? nullptr : &inputParameters; 
	const auto outputParametersPtr = outputDeviceIndex == paNoDevice ? nullptr : &outputParameters;
	auto err = Pa_IsFormatSupported(inputParametersPtr, outputParametersPtr, sampleRate);

	if (err != paFormatIsSupported)
	{
		std::cerr << "Pa_IsFormatSupported() failed: " << Pa_GetErrorText(err) << '\n';
		successful = false;
		return;
	}

	err = Pa_OpenStream(&m_paStream, inputParametersPtr, outputParametersPtr, sampleRate, framesPerBuffer, paNoFlag,
		&AudioPortAudio::processCallback, this);
	
	if (err != paNoError)
	{
		std::cerr << "Pa_OpenStream() failed: " << Pa_GetErrorText(err) << '\n';
		successful = false;
		return;
	}

	successful = true;
	setSampleRate(sampleRate);
	setChannels(outputDeviceChannels);

	// The input comes in the same callback as the output: the engine takes it with the output (planar)
	const auto inputChannels = inputParametersPtr ? inputDeviceChannels : 0;
	setInputChannels(static_cast<ch_cnt_t>(inputChannels));
	m_inputStorage.assign(static_cast<std::size_t>(inputChannels) * framesPerBuffer, 0.f);
	m_inputPointers.resize(inputChannels);
	for (auto channel = 0; channel < inputChannels; ++channel)
	{
		m_inputPointers[channel] = m_inputStorage.data() + static_cast<std::size_t>(channel) * framesPerBuffer;
	}
	m_inputCapacity = framesPerBuffer;

	if (const auto info = Pa_GetStreamInfo(m_paStream))
	{
		m_inputLatency = static_cast<f_cnt_t>(std::lround(info->inputLatency * info->sampleRate));
		m_outputLatency = static_cast<f_cnt_t>(std::lround(info->outputLatency * info->sampleRate));
	}
}

AudioPortAudio::~AudioPortAudio()
{
	stopProcessing();
	Pa_CloseStream(m_paStream);
}

void AudioPortAudio::startProcessingImpl()
{
	Pa_StartStream(m_paStream);
}

void AudioPortAudio::stopProcessingImpl()
{
	Pa_StopStream(m_paStream);
}

int AudioPortAudio::processCallback(const void* input, void* output, unsigned long frameCount,
	const PaStreamCallbackTimeInfo*, PaStreamCallbackFlags statusFlags, void* userData)
{
	const auto device = static_cast<AudioPortAudio*>(userData);
	const auto channels = device->channels();
	const auto outputBuffer = reinterpret_cast<float*>(output);

	if (!device->isRunning())
	{
		std::fill_n(outputBuffer, frameCount * channels, 0.f);
		return paComplete;
	}

	if (statusFlags & (paInputUnderflow | paInputOverflow | paOutputUnderflow | paOutputOverflow))
	{
		device->reportXrun();
	}

	// The stream was opened with framesPerPeriod() frames per buffer; a host that hands more is taken in parts
	const auto inputChannels = device->inputChannels();
	const auto inputBuffer = static_cast<const float*>(input);
	unsigned long done = 0;
	while (done < frameCount)
	{
		const auto frames = std::min<unsigned long>(frameCount - done, device->m_inputCapacity);
		for (ch_cnt_t channel = 0; channel < inputChannels; ++channel)
		{
			auto* planar = device->m_inputPointers[channel];
			for (unsigned long frame = 0; frame < frames; ++frame)
			{
				// A stream without input data (e.g. paInputUnderflow) hands no buffer: silence
				planar[frame] = inputBuffer ? inputBuffer[(done + frame) * inputChannels + channel] : 0.f;
			}
		}
		const auto inputView = PlanarBufferView<const float>{
			const_cast<const float* const*>(device->m_inputPointers.data()), inputChannels, frames};
		device->audioEngine()->renderNextBuffer(inputView,
			InterleavedBufferView<float>{outputBuffer + done * channels, channels, frames});
		done += frames;
	}
	return paContinue;
}
} // namespace lmms

#endif // LMMS_HAVE_PORTAUDIO
