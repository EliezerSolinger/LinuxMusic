/*
 * AudioJackSetupWidget.cpp - the settings of the Jack sound driver
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
#include "AudioJackSetupWidget.h"

#ifdef LMMS_HAVE_JACK

#include <QFormLayout>
#include <QLineEdit>
#include <QMenu>
#include <QSpinBox>
#include <QToolButton>

#include "ConfigManager.h"
#include "lmmsversion.h"

namespace lmms::gui
{

using namespace JackSettings;

AudioJackSetupWidget::AudioJackSetupWidget(QWidget* parent)
	: AudioDeviceSetupWidget(AudioJack::name(), parent)
{
	// TODO: Once backend can be changed without restarting LMMS, add a button to start/stop
	//       the JACK server and an indicator for the JACK server status.

	jack_status_t status;
	m_client = jack_client_open("LinuxMusic-Setup Dialog", JackNoStartServer, &status);
	if (!m_client)
	{
		// Failure is expected when not using the JACK backend
	}

	QFormLayout * form = new QFormLayout(this);

	// Set the field growth policy to allow fields to expand horizontally
	form->setFieldGrowthPolicy(QFormLayout::ExpandingFieldsGrow);

	const auto cm = ConfigManager::inst();
	QString cn = cm->value(audioJackClass, clientNameKey);
	if (cn.isEmpty()) { cn = LMMS_PROJECT_ID; }
	m_clientName = new QLineEdit(cn, this);

	form->addRow(tr("Client name"), m_clientName);

	auto buildToolButton = [](QWidget* parent, const QString& currentSelection, const std::vector<std::string>& names, const QString& filteredLMMSClientName)
	{
		auto toolButton = new QToolButton(parent);
		// Make sure that the tool button will fill out the available space in the form layout
		toolButton->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
		toolButton->setPopupMode(QToolButton::InstantPopup);
		toolButton->setText(currentSelection);
		auto menu = AudioJackSetupWidget::buildMenu(toolButton, names, filteredLMMSClientName);
		toolButton->setMenu(menu);

		return toolButton;
	};

	// Outputs
	const auto audioOutputNames = getAudioOutputNames();

	constexpr size_t numberOfOutputChannels = 2;
	for (size_t i = 0; i < numberOfOutputChannels; ++i)
	{
		const auto outputKey = getOutputKeyByChannel(i);
		const auto outputValue = cm->value(audioJackClass, outputKey);
		auto outputDevice = buildToolButton(this, outputValue, audioOutputNames, cn);
		form->addRow(buildOutputName(i) + ":", outputDevice);
		m_outputDevices.push_back(outputDevice);
	}

	// Inputs: as many as the tracks need to record from, each connected to a port of JACK
	m_inputCount = new QSpinBox(this);
	m_inputCount->setRange(0, MaxInputs);
	m_inputCount->setValue(inputCount());
	m_inputCount->setToolTip(tr("Input ports the tracks can record from"));
	form->addRow(tr("Inputs"), m_inputCount);

	auto inputsBox = new QWidget(this);
	auto inputsForm = new QFormLayout(inputsBox);
	inputsForm->setContentsMargins(0, 0, 0, 0);
	inputsForm->setFieldGrowthPolicy(QFormLayout::ExpandingFieldsGrow);
	form->addRow(inputsBox);

	const auto audioInputNames = getAudioInputNames();
	const auto showInputs = [this, inputsForm, audioInputNames, cn, buildToolButton](int count) {
		// Keep what was chosen for the inputs that stay
		auto chosen = QStringList{};
		for (auto* button : m_inputDevices) { chosen << button->text(); }
		while (inputsForm->rowCount() > 0) { inputsForm->removeRow(0); }
		m_inputDevices.clear();
		for (int i = 0; i < count; ++i)
		{
			const auto value = i < chosen.size()
				? chosen[i]
				: ConfigManager::inst()->value(audioJackClass, getInputKeyByChannel(i));
			auto inputDevice = buildToolButton(inputsForm->parentWidget(), value, audioInputNames, cn);
			inputsForm->addRow(buildInputName(static_cast<ch_cnt_t>(i)) + ":", inputDevice);
			m_inputDevices.push_back(inputDevice);
		}
	};
	showInputs(m_inputCount->value());
	connect(m_inputCount, qOverload<int>(&QSpinBox::valueChanged), this, showInputs);

	if (m_client != nullptr)
	{
		jack_deactivate(m_client);
		jack_client_close(m_client);
	}
}


void AudioJackSetupWidget::saveSettings()
{
	ConfigManager::inst()->setValue(audioJackClass, clientNameKey, m_clientName->text());

	for (size_t i = 0; i < m_outputDevices.size(); ++i)
	{
		ConfigManager::inst()->setValue(audioJackClass, getOutputKeyByChannel(i), m_outputDevices[i]->text());	
	}

	ConfigManager::inst()->setValue(audioJackClass, inputsKey, QString::number(m_inputCount->value()));
	for (size_t i = 0; i < m_inputDevices.size(); ++i)
	{
		ConfigManager::inst()->setValue(audioJackClass, getInputKeyByChannel(i), m_inputDevices[i]->text());	
	}
}


std::vector<std::string> AudioJackSetupWidget::getAudioPortNames(JackPortFlags portFlags) const
{
	std::vector<std::string> audioPorts;

	// We are using weak_libjack. If JACK is not installed this will result in the client being nullptr.
	// Because jack_get_ports in weak_libjack does not check for nullptr we have to do this here and fail gracefully,
	// i.e. with an empty list of audio ports.
	if (!m_client)
	{
		return audioPorts;
	}

	const char **inputAudioPorts = jack_get_ports(m_client, nullptr, JACK_DEFAULT_AUDIO_TYPE, portFlags);
	if (inputAudioPorts)
	{
		for (int i = 0; inputAudioPorts[i] != nullptr; ++i)
		{
			auto currentPortName = inputAudioPorts[i];

			audioPorts.push_back(currentPortName);
		}
		jack_free(inputAudioPorts); // Remember to free after use
	}

	return audioPorts;
}


std::vector<std::string> AudioJackSetupWidget::getAudioOutputNames() const
{
	return getAudioPortNames(JackPortIsInput);
}


std::vector<std::string> AudioJackSetupWidget::getAudioInputNames() const
{
	return getAudioPortNames(JackPortIsOutput);
}


QMenu* AudioJackSetupWidget::buildMenu(QToolButton* toolButton, const std::vector<std::string>& names, const QString& filteredLMMSClientName)
{
	auto menu = new QMenu(toolButton);
	QMap<QString, QMenu*> clientNameToSubMenuMap;
	QList<QAction*> topLevelActions;
	for (const auto& currentName : names)
	{
		const auto clientNameWithPortName = QString::fromStdString(currentName);

		auto actionLambda = [toolButton, clientNameWithPortName](bool checked)
		{
			toolButton->setText(clientNameWithPortName);
		};

		// Split into individual client name and port name
		const auto list = clientNameWithPortName.split(":");
		if (list.size() == 2)
		{
			const auto& clientName = list[0];
			const auto& portName = list[1];

			if (clientName == filteredLMMSClientName)
			{
				// Prevent loops by not adding port of the LMMS client to the menu
				continue;
			}

			QMenu* clientSubMenu = nullptr;

			auto it = clientNameToSubMenuMap.find(clientName);
			if (it == clientNameToSubMenuMap.end())
			{
				clientSubMenu = new QMenu(menu);
				clientSubMenu->setTitle(clientName);
				clientNameToSubMenuMap.insert(clientName, clientSubMenu);
			}
			else
			{
				clientSubMenu = *it;
			}

			auto action = new QAction(portName, clientSubMenu);
			connect(action, &QAction::triggered, actionLambda);
			clientSubMenu->addAction(action);
		}
		else
		{
			// We cannot split into client and port name. Add the whole thing to the top level menu
			auto action = new QAction(QString::fromStdString(currentName), menu);
			connect(action, &QAction::triggered, actionLambda);
			topLevelActions.append(action);
		}
	}

	// First add the sub menus. By iterating the map they will be sorted automatically
	for (auto it = clientNameToSubMenuMap.begin(); it != clientNameToSubMenuMap.end(); ++it)
	{
		menu->addMenu(it.value());
	}

	// Now add potential top level actions, i.e. the entries which cannot be split at exactly one ":"
	// They must be sorted explicitly
	std::sort(topLevelActions.begin(), topLevelActions.end(), [](QAction* a, QAction* b) { return a->text() < b->text(); });
	menu->addActions(topLevelActions);

	// Add the menu entry which represents the disconnected state at the very end
	auto disconnectedAction = new QAction(disconnectedRepresentation, menu);
	connect(disconnectedAction, &QAction::triggered, [toolButton](bool checked)	{ toolButton->setText(disconnectedRepresentation); });
	menu->addAction(disconnectedAction);

	return menu;
}

} // namespace lmms::gui

#endif // LMMS_HAVE_JACK
