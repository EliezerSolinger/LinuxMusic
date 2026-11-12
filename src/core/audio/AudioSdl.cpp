/*
 * AudioSdl.cpp - device-class that performs PCM-output via SDL
 *
 * Copyright (c) 2004-2009 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "AudioSdl.h"
#include "LmmsTypes.h"

#ifdef LMMS_HAVE_SDL

#include <QFormLayout>
#include <QComboBox>
#include <SDL.h>

#include "AudioEngine.h"
#include "ConfigManager.h"

namespace lmms
{

constexpr auto SectionSDL = "audiosdl";
constexpr auto PlaybackDeviceSDL = "device";

AudioSdl::AudioSdl(bool& _success_ful, AudioEngine* _audioEngine)
	: AudioDevice(DEFAULT_CHANNELS, _audioEngine)
{
	_success_ful = false;

	if( SDL_Init( SDL_INIT_AUDIO | SDL_INIT_NOPARACHUTE ) < 0 )
	{
		qCritical( "Couldn't initialize SDL: %s\n", SDL_GetError() );
		return;
	}

	m_audioHandle.freq = sampleRate();
	m_audioHandle.format = AUDIO_F32SYS;	// we want it in byte-order
						// of system, so we don't have
						// to convert the buffers

	m_audioHandle.channels = channels();
	m_audioHandle.samples = _audioEngine->framesPerAudioBuffer();

	m_audioHandle.callback = sdlAudioCallback;
	m_audioHandle.userdata = this;

  	SDL_AudioSpec actual; 

	const auto playbackDevice = ConfigManager::inst()->value(SectionSDL, PlaybackDeviceSDL).toStdString();
	const bool isDefaultPlayback = playbackDevice.empty();

	// Try with the configured device
	const auto playbackDeviceCStr = isDefaultPlayback ? nullptr : playbackDevice.c_str();
	m_outputDevice = SDL_OpenAudioDevice(playbackDeviceCStr, 0, &m_audioHandle, &actual, 0);

	// If we did not get a device ID try again with the default device if we did not try that before
	if (m_outputDevice == 0 && !isDefaultPlayback)
	{
		m_outputDevice = SDL_OpenAudioDevice(nullptr, 0, &m_audioHandle, &actual, 0);
	}

	if (m_outputDevice == 0) {
		qCritical( "Couldn't open SDL-audio: %s\n", SDL_GetError() );
		return;
	}

	_success_ful = true;

	// Workaround for a race condition that causes SDL to segfault
	SDL_Delay(50);

	// SDL captures through a callback of its own, which is not in step with the output's: it does not capture
	// here (see AudioDevice::inputChannels()).
}




AudioSdl::~AudioSdl()
{
	if (m_outputDevice != 0)
		SDL_CloseAudioDevice(m_outputDevice);

	SDL_Quit();
}




void AudioSdl::startProcessingImpl()
{
	SDL_PauseAudioDevice(m_outputDevice, 0);
}




void AudioSdl::stopProcessingImpl()
{
	if( SDL_GetAudioDeviceStatus(m_outputDevice) == SDL_AUDIO_PLAYING )
	{
		SDL_LockAudioDevice(m_outputDevice);
		SDL_PauseAudioDevice(m_outputDevice, 1);
		SDL_UnlockAudioDevice(m_outputDevice);
	}
}

void AudioSdl::sdlAudioCallback( void * _udata, Uint8 * _buf, int _len )
{
	auto _this = static_cast<AudioSdl*>(_udata);

	_this->sdlAudioCallback( _buf, _len );
}




void AudioSdl::sdlAudioCallback( Uint8 * _buf, int _len )
{
	if (!isRunning())
	{
		memset( _buf, 0, _len );
		return;
	}

	const auto frameCount = _len / sizeof(float) / channels();
	audioEngine()->renderNextBuffer({reinterpret_cast<float*>(_buf), channels(), frameCount});
}

QString AudioSdl::setupWidget::s_systemDefaultDevice = AudioDeviceSetupWidget::tr("[System Default]");

AudioSdl::setupWidget::setupWidget( QWidget * _parent ) :
	AudioDeviceSetupWidget( AudioSdl::name(), _parent )
{
	QFormLayout * form = new QFormLayout(this);
	form->setRowWrapPolicy(QFormLayout::WrapLongRows);

	m_playbackDeviceComboBox = new QComboBox(this);

	populatePlaybackDeviceComboBox();

	form->addRow(tr("Playback device"), m_playbackDeviceComboBox);
}




void AudioSdl::setupWidget::saveSettings()
{
	const auto currentPlaybackDevice = m_playbackDeviceComboBox->currentText();
	if (currentPlaybackDevice == s_systemDefaultDevice)
	{
		// Represent the default playback device with an empty string
		ConfigManager::inst()->setValue(SectionSDL, PlaybackDeviceSDL, "");
	}
	else if (!currentPlaybackDevice.isEmpty())
	{
		ConfigManager::inst()->setValue(SectionSDL, PlaybackDeviceSDL, currentPlaybackDevice);
	}
}

void AudioSdl::setupWidget::populatePlaybackDeviceComboBox()
{
	m_playbackDeviceComboBox->addItem(s_systemDefaultDevice);

	QStringList playbackDevices;
	const int numberOfPlaybackDevices = SDL_GetNumAudioDevices(0);
	for (int i = 0; i < numberOfPlaybackDevices; ++i)
	{
		const QString deviceName = SDL_GetAudioDeviceName(i, 0);
		playbackDevices.append(deviceName);
	}

	playbackDevices.sort();

	m_playbackDeviceComboBox->addItems(playbackDevices);

	const auto playbackDevice = ConfigManager::inst()->value(SectionSDL, PlaybackDeviceSDL);
	m_playbackDeviceComboBox->setCurrentText(playbackDevice.isEmpty() ? s_systemDefaultDevice : playbackDevice);
}


} // namespace lmms

#endif // LMMS_HAVE_SDL

