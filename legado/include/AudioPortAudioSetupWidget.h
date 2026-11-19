/*
 * AudioPortAudioSetupWidget.h - the settings of the PortAudio sound driver
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
#ifndef LMMS_GUI_AUDIO_PORT_AUDIO_SETUP_WIDGET_H
#define LMMS_GUI_AUDIO_PORT_AUDIO_SETUP_WIDGET_H

#include "lmmsconfig.h"

#ifdef LMMS_HAVE_PORTAUDIO

#include "AudioDeviceSetupWidget.h"

class QComboBox;

namespace lmms::gui
{

class AudioPortAudioSetupWidget : public AudioDeviceSetupWidget
{
public:
	AudioPortAudioSetupWidget(QWidget* parent);

	void show() override;
	void saveSettings() override;

private:
	class DeviceSelectorWidget;
	QComboBox* m_backendComboBox = nullptr;
	DeviceSelectorWidget* m_inputDevice = nullptr;
	DeviceSelectorWidget* m_outputDevice = nullptr;
};

} // namespace lmms::gui

#endif // LMMS_HAVE_PORTAUDIO

#endif // LMMS_GUI_AUDIO_PORT_AUDIO_SETUP_WIDGET_H
