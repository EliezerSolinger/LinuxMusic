/*
 * AudioAlsaSetupWidget.cpp - Implements a setup widget for ALSA-PCM-output
 *
 * Copyright (c) 2004-2015 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include <QComboBox>
#include <QFormLayout>

#include "AudioAlsaSetupWidget.h"

#ifdef LMMS_HAVE_ALSA

#include "ConfigManager.h"

namespace lmms::gui
{

AudioAlsaSetupWidget::AudioAlsaSetupWidget(QWidget* parent)
	: AudioDeviceSetupWidget(AudioAlsa::name(), parent)
	, m_deviceComboBox{new QComboBox{this}}
	, m_inputComboBox{new QComboBox{this}}
{
	auto form = new QFormLayout(this);

	// Each list holds the devices for its direction; the hardware ones (hw:) are exclusive and on the card's own
	// format, the others (default, PipeWire, PulseAudio) are shared with other programs
	m_inputComboBox->addItem(tr("[None]"), QString{AudioAlsa::NoCapture});
	m_inputComboBox->addItem(tr("[Same as the output]"), QString{});
	for (const auto& device : AudioAlsa::getAvailableDevices())
	{
		auto description = device.getDeviceDescription();
		description.replace('\n', " - ");
		if (device.canPlay())
		{
			m_deviceComboBox->addItem(device.getDeviceName(), device.getDeviceName());
			m_deviceComboBox->setItemData(m_deviceComboBox->count() - 1, description, Qt::ToolTipRole);
		}
		if (device.canCapture())
		{
			m_inputComboBox->addItem(device.getDeviceName(), device.getDeviceName());
			m_inputComboBox->setItemData(m_inputComboBox->count() - 1, description, Qt::ToolTipRole);
		}
	}

	// A device set by hand, not in the lists, stays selectable
	const auto select = [](QComboBox* box, const QString& value) {
		auto index = box->findData(value);
		if (index < 0)
		{
			box->addItem(value, value);
			index = box->count() - 1;
		}
		box->setCurrentIndex(index);
	};
	const auto device = ConfigManager::inst()->value("audioalsa", "device");
	select(m_deviceComboBox, device.isEmpty() ? QString{"default"} : device);
	select(m_inputComboBox, ConfigManager::inst()->value("audioalsa", "inputdevice"));
	m_deviceComboBox->setEditable(true);
	m_inputComboBox->setToolTip(tr("Where the tracks record from. A hardware device (hw:) on the same card as the "
		"output runs on the card's clock, in step with it."));

	form->addRow(tr("Output device"), m_deviceComboBox);
	form->addRow(tr("Input device"), m_inputComboBox);
}




void AudioAlsaSetupWidget::saveSettings()
{
	const auto device = m_deviceComboBox->currentText() == m_deviceComboBox->itemText(m_deviceComboBox->currentIndex())
		? m_deviceComboBox->currentData().toString()
		: m_deviceComboBox->currentText();
	ConfigManager::inst()->setValue("audioalsa", "device", device);
	ConfigManager::inst()->setValue("audioalsa", "inputdevice", m_inputComboBox->currentData().toString());
}


} // namespace lmms::gui

#endif // LMMS_HAVE_ALSA
