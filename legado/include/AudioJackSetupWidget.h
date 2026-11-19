/*
 * AudioJackSetupWidget.h - the settings of the Jack sound driver
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
#ifndef LMMS_GUI_AUDIO_JACK_SETUP_WIDGET_H
#define LMMS_GUI_AUDIO_JACK_SETUP_WIDGET_H

#include "lmmsconfig.h"

#ifdef LMMS_HAVE_JACK

#include "AudioDeviceSetupWidget.h"
#include "AudioJack.h"

class QLineEdit;
class QMenu;
class QSpinBox;
class QToolButton;

namespace lmms::gui
{

class AudioJackSetupWidget : public AudioDeviceSetupWidget
{
public:
	AudioJackSetupWidget(QWidget* parent);
	void saveSettings() override;

private:
	std::vector<std::string> getAudioPortNames(JackPortFlags portFlags) const;
	std::vector<std::string> getAudioInputNames() const;
	std::vector<std::string> getAudioOutputNames() const;
	static QMenu* buildMenu(QToolButton* toolButton, const std::vector<std::string>& names, const QString& filteredLMMSClientName);

private:
	QLineEdit* m_clientName;
	// Because we do not have access to a JackAudio driver instance we have to be our own client to display inputs and outputs...
	jack_client_t* m_client;

	std::vector<QToolButton*> m_outputDevices;
	std::vector<QToolButton*> m_inputDevices;
	QSpinBox* m_inputCount;
};

} // namespace lmms::gui

#endif // LMMS_HAVE_JACK

#endif // LMMS_GUI_AUDIO_JACK_SETUP_WIDGET_H
