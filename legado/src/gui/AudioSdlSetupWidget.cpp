/*
 * AudioSdlSetupWidget.cpp - the settings of the Sdl sound driver
 *
 * Copyright (c) 2007-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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
#include "AudioSdlSetupWidget.h"

#ifdef LMMS_HAVE_SDL

#include <QComboBox>
#include <QFormLayout>
#include <SDL.h>

#include "ConfigManager.h"

namespace lmms::gui
{

auto AudioSdlSetupWidget::systemDefaultDevice() -> QString
{
	return AudioDeviceSetupWidget::tr("[System Default]");
}


AudioSdlSetupWidget::AudioSdlSetupWidget( QWidget * _parent ) :
	AudioDeviceSetupWidget( AudioSdl::name(), _parent )
{
	QFormLayout * form = new QFormLayout(this);
	form->setRowWrapPolicy(QFormLayout::WrapLongRows);

	m_playbackDeviceComboBox = new QComboBox(this);

	populatePlaybackDeviceComboBox();

	form->addRow(tr("Playback device"), m_playbackDeviceComboBox);
}


void AudioSdlSetupWidget::saveSettings()
{
	const auto currentPlaybackDevice = m_playbackDeviceComboBox->currentText();
	if (currentPlaybackDevice == systemDefaultDevice())
	{
		// Represent the default playback device with an empty string
		ConfigManager::inst()->setValue(AudioSdl::ConfigSection, AudioSdl::ConfigDevice, "");
	}
	else if (!currentPlaybackDevice.isEmpty())
	{
		ConfigManager::inst()->setValue(AudioSdl::ConfigSection, AudioSdl::ConfigDevice, currentPlaybackDevice);
	}
}


void AudioSdlSetupWidget::populatePlaybackDeviceComboBox()
{
	m_playbackDeviceComboBox->addItem(systemDefaultDevice());

	QStringList playbackDevices;
	const int numberOfPlaybackDevices = SDL_GetNumAudioDevices(0);
	for (int i = 0; i < numberOfPlaybackDevices; ++i)
	{
		const QString deviceName = SDL_GetAudioDeviceName(i, 0);
		playbackDevices.append(deviceName);
	}

	playbackDevices.sort();

	m_playbackDeviceComboBox->addItems(playbackDevices);

	const auto playbackDevice = ConfigManager::inst()->value(AudioSdl::ConfigSection, AudioSdl::ConfigDevice);
	m_playbackDeviceComboBox->setCurrentText(playbackDevice.isEmpty() ? systemDefaultDevice() : playbackDevice);
}

} // namespace lmms::gui

#endif // LMMS_HAVE_SDL
