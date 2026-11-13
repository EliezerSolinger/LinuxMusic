/*
 * AudioDummy.h - dummy audio-device
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

#ifndef LMMS_AUDIO_DUMMY_H
#define LMMS_AUDIO_DUMMY_H

#include <cmath>
#include <vector>
#include "AudioDevice.h"
#include "AudioDeviceSetupWidget.h"
#include "AudioEngine.h"
#include "MicroTimer.h"

namespace lmms
{

class AudioDummy : public QThread, public AudioDevice
{
	Q_OBJECT
public:
	//! @param withInputs two inputs, silent, so recording can be tried without a sound card
	//!        (LINUXMUSIC_DUMMY_INPUT_TONE=1 makes them play a tone instead, to see a recording's waveform while
	//!        developing); not when the engine only renders (exports, tests)
	AudioDummy(bool& _success_ful, AudioEngine* audioEngine, bool withInputs = true) :
		AudioDevice( DEFAULT_CHANNELS, audioEngine )
	{
		_success_ful = true;
		if (withInputs) { setInputChannels(2); }
	}

	~AudioDummy() override
	{
		stopProcessing();
	}

	inline static QString name()
	{
		return QT_TRANSLATE_NOOP( "AudioDeviceSetupWidget", "Dummy (no sound output)" );
	}


	class setupWidget : public gui::AudioDeviceSetupWidget
	{
	public:
		setupWidget( QWidget * _parent ) :
			gui::AudioDeviceSetupWidget( AudioDummy::name(), _parent )
		{
		}

		~setupWidget() override = default;

		void saveSettings() override
		{
		}

		void show() override
		{
			parentWidget()->hide();
			QWidget::show();
		}

	} ;


private:
	void startProcessingImpl() override
	{
		start();
	}

	void stopProcessingImpl() override
	{
		stopProcessingThread( this );
	}

	void run() override
	{
		const bool tone = qEnvironmentVariableIntValue("LINUXMUSIC_DUMMY_INPUT_TONE") == 1;
		const auto frames = static_cast<std::size_t>(audioEngine()->framesPerPeriod());
		auto left = std::vector<float>(frames, 0.f);
		auto right = std::vector<float>(frames, 0.f);
		auto output = std::vector<float>(2 * frames, 0.f);
		auto phase = 0.0;
		MicroTimer timer;
		while (AudioDevice::isRunning())
		{
			timer.reset();
			if (tone)
			{
				// A 220 Hz tone swelling and fading every two seconds
				const auto rate = static_cast<double>(audioEngine()->outputSampleRate());
				for (std::size_t frame = 0; frame < frames; ++frame, phase += 1.0)
				{
					const auto envelope = 0.5 * (1.0 - std::cos(2.0 * 3.14159265358979 * phase / (2.0 * rate)));
					left[frame] = right[frame] = static_cast<float>(0.8 * envelope * std::sin(2.0 * 3.14159265358979 * 220.0 * phase / rate));
				}
			}
			const float* inputs[] = {left.data(), right.data()};
			audioEngine()->renderNextBuffer(PlanarBufferView<const float>{inputs, inputChannels(), frames},
				InterleavedBufferView<float>{output.data(), 2, frames});

			const int microseconds = static_cast<int>( audioEngine()->framesPerPeriod() * 1000000.0f / audioEngine()->outputSampleRate() - timer.elapsed() );
			if( microseconds > 0 )
			{
				usleep( microseconds );
			}
		}
	}

} ;

} // namespace lmms

#endif // LMMS_AUDIO_DUMMY_H
