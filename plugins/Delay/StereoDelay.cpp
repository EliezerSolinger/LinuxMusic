/*
 * stereodelay.cpp - defination of StereoDelay class.
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

#include <algorithm>
#include "StereoDelay.h"

#include "SampleFrame.h"

namespace lmms
{


StereoDelay::StereoDelay( int maxTime, int sampleRate )
{
	m_buffer = 0;
	m_maxTime = static_cast<float>(maxTime);
	m_maxLength = maxTime * sampleRate;
	m_length = static_cast<float>(m_maxLength);

	m_writeIndex = 0;
	m_feedback = 0.0f;
	setSampleRate( sampleRate );
}




StereoDelay::~StereoDelay()
{
	if( m_buffer )
	{
		delete[] m_buffer;
	}
}




void StereoDelay::tick( SampleFrame& frame )
{
	m_writeIndex = ( m_writeIndex + 1 ) % ( int )m_maxLength;
	int readIndex = m_writeIndex - static_cast<int>(m_length);
	if (readIndex < 0 ) { readIndex += m_maxLength; }
	float lOut = m_buffer[ readIndex ][ 0 ];
	float rOut = m_buffer[ readIndex ] [1 ];
	m_buffer[ m_writeIndex ][ 0 ] = frame[ 0 ] + ( lOut * m_feedback );
	m_buffer[ m_writeIndex ][ 1 ] = frame[ 1 ] + ( rOut * m_feedback );
	frame[ 0 ] = lOut;
	frame[ 1 ] = rOut;
}






void StereoDelay::setSampleRate( int sampleRate )
{
	if( m_buffer )
	{
		delete[] m_buffer;
	}

	// The longest delay at this rate: the buffer's length is the length tick() wraps at (it kept the length of
	// the rate it was made at, and at a lower rate it wrote past the end of the buffer)
	const int bufferSize = std::max(1, static_cast<int>(sampleRate * m_maxTime));
	m_buffer = new SampleFrame[bufferSize];
	m_maxLength = bufferSize;
	m_length = std::min(m_length, static_cast<float>(m_maxLength));
	clear();
}


void StereoDelay::clear()
{
	for (int i = 0; i < m_maxLength; ++i)
	{
		m_buffer[i][0] = 0.0f;
		m_buffer[i][1] = 0.0f;
	}
	m_writeIndex = 0;
}


} // namespace lmms
