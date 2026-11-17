/*
 * monodelay.cpp - defination of MonoDelay class.
 *
 * Copyright (c) 2014 David French <dave/dot/french3/at/googlemail/dot/com>
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

#include "MonoDelay.h"

#include <algorithm>

namespace lmms
{


MonoDelay::MonoDelay( int maxTime , int sampleRate )
	: m_maxTime{static_cast<float>(maxTime)}
{
	setSampleRate( sampleRate );
	m_length = static_cast<float>(m_maxLength);
}




void MonoDelay::tick( sample_t* sample )
{
	m_writeIndex = ( m_writeIndex + 1 ) % m_maxLength;
	int readIndex = m_writeIndex - static_cast<int>(m_length);
	if (readIndex < 0 ) { readIndex += m_maxLength; }
	float out = m_buffer[ readIndex ];
	m_buffer[ m_writeIndex ] = *sample + ( out * m_feedback );
	*sample = out;
}




void MonoDelay::setSampleRate( int sampleRate )
{
	// The longest delay in frames at this rate (one frame at least), and what was in it, gone
	m_maxLength = std::max(1, static_cast<int>(static_cast<float>(sampleRate) * m_maxTime));
	m_buffer.assign(m_maxLength, 0.f);
	m_writeIndex = 0;
	m_length = std::min(m_length, static_cast<float>(m_maxLength));
}


} // namespace lmms
