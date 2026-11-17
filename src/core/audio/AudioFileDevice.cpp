/*
 * AudioFileDevice.cpp - base-class for audio-device-classes which write
 *                       their output into a file
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

#include "AudioFileDevice.h"

#include <QCoreApplication>

#include "UserNotice.h"

namespace lmms
{

namespace
{
//! The messages keep the translations they had when the export dialog showed them
class Text
{
	Q_DECLARE_TR_FUNCTIONS(lmms::gui::ExportProjectDialog)
};
} // namespace

AudioFileDevice::AudioFileDevice( OutputSettings const & outputSettings,
					const ch_cnt_t _channels,
					const QString & _file,
					AudioEngine*  _audioEngine ) :
	AudioDevice( _channels, _audioEngine ),
	m_outputFile( _file ),
	m_outputSettings(outputSettings)
{
	setSampleRate( outputSettings.getSampleRate() );

	if( m_outputFile.open( QFile::WriteOnly | QFile::Truncate ) == false )
	{
		// The device is not ready (outputFileOpened() is false): whoever made it goes on without it
		UserNotice::error(Text::tr("Could not open file"),
			Text::tr("Could not open file %1 "
				"for writing.\nPlease make "
				"sure you have write "
				"permission to the file and "
				"the directory containing the "
				"file and try again!").arg(_file));
	}
}




AudioFileDevice::~AudioFileDevice()
{
	m_outputFile.close();
}




int AudioFileDevice::writeData( const void* data, int len )
{
	if( m_outputFile.isOpen() )
	{
		return m_outputFile.write( (const char *) data, len );
	}

	return -1;
}

} // namespace lmms
