/*
 * KickerOsc.h - alternative sweeping oscillator
 *
 * Copyright (c) 2006-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
 * Copyright (c) 2014 grejppi <grejppi/at/gmail.com>
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

#ifndef KICKER_OSC_H
#define KICKER_OSC_H

#include <cmath>

#include "DspEffectLibrary.h"
#include "Oscillator.h"

#include "lmms_math.h"

namespace lmms
{


template<class FX = DspEffectLibrary::StereoBypass>
class KickerOsc
{
public:
	KickerOsc( const FX & fx, const float start, const float end, const float noise, const float offset, 
		const float slope, const float env, const float diststart, const float distend, const float length,
		const float attack ) :
		m_phase( offset ),
		m_startFreq( start ),
		m_endFreq( end ),
		m_noise( noise ),
		m_slope( slope ),
		m_env( env ),
		m_distStart( diststart ),
		m_distEnd( distend ),
		m_hasDistEnv( diststart != distend ),
		m_length( length ),
		m_attack(std::max(attack, 1.f)),
		m_FX( fx ),
		m_counter( 0 ),
		m_freq( start )
	{
	}

	virtual ~KickerOsc() = default;

	void update( SampleFrame* buf, const f_cnt_t frames, const float sampleRate )
	{
		for( f_cnt_t frame = 0; frame < frames; ++frame )
		{
			const double gain = 1 - fastPow((m_counter < m_length) ? m_counter / m_length : 1, m_env);
			const sample_t s = ( Oscillator::sinSample( m_phase ) * ( 1 - m_noise ) ) + ( Oscillator::noiseSample( 0 ) * gain * gain * m_noise );
			buf[frame][0] = s * gain;
			buf[frame][1] = s * gain;
			
			// update distortion envelope if necessary
			if( m_hasDistEnv && m_counter < m_length )
			{
				float thres = std::lerp(m_distStart, m_distEnd, m_counter / m_length);
				m_FX.leftFX().setThreshold( thres );
				m_FX.rightFX().setThreshold( thres );
			}
			
			m_FX.nextSample( buf[frame][0], buf[frame][1] );

			// The attack: from silence, never a jump to the first sample (the "Click" starts the sine off zero)
			if (m_counter < m_attack)
			{
				const auto rise = static_cast<float>(0.5 - 0.5 * std::cos(std::numbers::pi * m_counter / m_attack));
				buf[frame][0] *= rise;
				buf[frame][1] *= rise;
			}
			m_phase += m_freq / sampleRate;

			const double change = (m_counter < m_length) ? ((m_startFreq - m_endFreq) * (1 - fastPow(m_counter / m_length, m_slope))) : 0;
			m_freq = m_endFreq + change;
			++m_counter;
		}
	}


private:
	float m_phase;
	const float m_startFreq;
	const float m_endFreq;
	const float m_noise;
	const float m_slope;
	const float m_env;
	const float m_distStart;
	const float m_distEnd;
	const bool m_hasDistEnv;
	const float m_length;
	const float m_attack;	//!< frames
	FX m_FX;

	unsigned long m_counter;
	double m_freq;

};


} // namespace lmms

#endif
