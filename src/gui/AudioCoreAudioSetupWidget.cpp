/*
 * AudioCoreAudioSetupWidget.cpp - the settings of the CoreAudio sound driver
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
#include "AudioCoreAudioSetupWidget.h"

#ifdef LMMS_BUILD_APPLE

#include <QComboBox>
#include <QFormLayout>

#include "AudioCoreAudio.h"
#include "ConfigManager.h"

namespace lmms::gui
{

AudioCoreAudioSetupWidget::AudioCoreAudioSetupWidget(QWidget* parent)
	: AudioDeviceSetupWidget{AudioCoreAudio::name(), parent}
	, m_outputDevice{new QComboBox{this}}
	, m_inputDevice{new QComboBox{this}}
{
	auto* form = new QFormLayout{this};
	form->addRow(tr("Output device"), m_outputDevice);
	form->addRow(tr("Input device"), m_inputDevice);

	m_outputDevice->addItem(tr("[System Default]"), QString{});
	m_inputDevice->addItem(tr("[None]"), QString{AudioCoreAudio::NoInput});
	m_inputDevice->addItem(tr("[System Default]"), QString{});
	for (const auto& device : AudioCoreAudio::devices())
	{
		if (device.outputChannels > 0)
		{
			m_outputDevice->addItem(QString{"%1 (%2)"}.arg(device.name).arg(device.outputChannels), device.uid);
		}
		if (device.inputChannels > 0)
		{
			m_inputDevice->addItem(QString{"%1 (%2)"}.arg(device.name).arg(device.inputChannels), device.uid);
		}
	}

	const auto select = [](QComboBox* box, const QString& uid) {
		const auto index = box->findData(uid);
		box->setCurrentIndex(index >= 0 ? index : box->findData(QString{}));
	};
	select(m_outputDevice, ConfigManager::inst()->value(AudioCoreAudio::ConfigSection, AudioCoreAudio::ConfigOutputDevice));
	select(m_inputDevice, ConfigManager::inst()->value(AudioCoreAudio::ConfigSection, AudioCoreAudio::ConfigInputDevice));
}




void AudioCoreAudioSetupWidget::saveSettings()
{
	ConfigManager::inst()->setValue(AudioCoreAudio::ConfigSection, AudioCoreAudio::ConfigOutputDevice, m_outputDevice->currentData().toString());
	ConfigManager::inst()->setValue(AudioCoreAudio::ConfigSection, AudioCoreAudio::ConfigInputDevice, m_inputDevice->currentData().toString());
}

} // namespace lmms::gui

#endif // LMMS_BUILD_APPLE
