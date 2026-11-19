/*
 * RmsHelper.h - helper class for calculating RMS
 *
 * Copyright (c) 2014 Vesa Kivimäki <contact/dot/diizy/at/nbl/dot/fi>
 * Copyright (c) 2008 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_RMS_HELPER_H
#define LMMS_RMS_HELPER_H

#include <algorithm>
#include <vector>

#include "lmms_math.h"


namespace lmms
{

//! The RMS of the last size() values given to update()
class RmsHelper
{
public:
	explicit RmsHelper(std::size_t size)
	{
		setSize( size );
	}
	virtual ~RmsHelper() = default;

	//! Allocates only when it grows past what it had
	void setSize(std::size_t size)
	{
		m_size = std::max<std::size_t>(size, 1);
		m_buffer.assign(m_size, 0.f);
		reset();
	}

	inline void reset()
	{
		m_sizef = 1.0f / (float) m_size;
		m_pos = 0;
		m_sum = 0.0f;
		std::fill(m_buffer.begin(), m_buffer.end(), 0.f);
	}

	inline float update( const float in )
	{
		m_sum -= m_buffer[ m_pos ];
		m_sum += m_buffer[ m_pos ] = in * in;
		++m_pos %= m_size;
		return std::sqrt(m_sum * m_sizef);
	}

private:
	std::vector<float> m_buffer;
	float m_sum = 0.f;
	std::size_t m_pos = 0;
	std::size_t m_size = 1;
	float m_sizef = 1.f;
};


} // namespace lmms

#endif // LMMS_RMS_HELPER_H
