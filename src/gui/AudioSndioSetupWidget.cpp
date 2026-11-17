/*
 * AudioSndioSetupWidget.cpp - the settings of the Sndio sound driver
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
#include "AudioSndioSetupWidget.h"

#ifdef LMMS_HAVE_SNDIO

#include <QFormLayout>
#include <QLineEdit>

#include "ConfigManager.h"
#include "LcdSpinBox.h"
#include "lmms_constants.h"

namespace lmms::gui
{

AudioSndioSetupWidget::AudioSndioSetupWidget( QWidget * _parent ) :
	AudioDeviceSetupWidget( AudioSndio::name(), _parent )
{
	QFormLayout * form = new QFormLayout(this);

	m_device = new QLineEdit( "", this );
	form->addRow(tr("Device"), m_device);

	auto m = new LcdSpinBoxModel();
	m->setRange(DEFAULT_CHANNELS, DEFAULT_CHANNELS);
	m->setStep( 2 );
	m->setValue( ConfigManager::inst()->value( "audiosndio",
	    "channels" ).toInt() );

	m_channels = new LcdSpinBox( 1, this );
	m_channels->setModel( m );

	form->addRow(tr("Channels"), m_channels);
}


AudioSndioSetupWidget::~AudioSndioSetupWidget()
{
	delete m_channels->model();
}


void AudioSndioSetupWidget::saveSettings()
{
	ConfigManager::inst()->setValue( "audiosndio", "device",
	    m_device->text() );
	ConfigManager::inst()->setValue( "audiosndio", "channels",
	    QString::number( m_channels->value<int>() ) );
}

} // namespace lmms::gui

#endif // LMMS_HAVE_SNDIO
