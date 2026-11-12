/*
 * MonoBass.cpp - narrows the stereo image of the lows of a mid/side signal
 *
 * This file is part of LinuxMusic, a fork of LMMS - https://lmms.io
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

#include "MonoBass.h"

#include <algorithm>
#include <cmath>
#include <numbers>

namespace lmms
{

auto MonoBass::Svf::process(float in, float g) -> Outputs
{
	// Zavalishin's topology-preserving transform; Butterworth: 1/Q = sqrt(2)
	constexpr float k = std::numbers::sqrt2_v<float>;
	const float a1 = 1.f / (1.f + g * (g + k));
	const float a2 = g * a1;
	const float a3 = g * a2;
	const float v3 = in - ic2;
	const float v1 = a1 * ic1 + a2 * v3;
	const float v2 = ic2 + a2 * ic1 + a3 * v3;
	ic1 = 2.f * v1 - ic1;
	ic2 = 2.f * v2 - ic2;
	return {v2, in - k * v1 - v2};
}




auto MonoBass::Crossover::process(float in, float g) -> Outputs
{
	const float low = low2.process(low1.process(in, g).low, g).low;
	const float high = high2.process(high1.process(in, g).high, g).high;
	return {low, high};
}




auto MonoBass::begin(float frequency, float width, sample_rate_t sampleRate) -> bool
{
	const auto rate = static_cast<float>(sampleRate);
	m_targetWidth = std::clamp(width, 0.f, 1.f);
	m_targetMix = m_targetWidth < 1.f ? 1.f : 0.f;
	if (m_targetMix == 0.f && m_mix == 0.f)
	{
		m_g = 0.f;
		m_width = 1.f;
		return false;
	}

	// The frequency warped for the filters (bilinear transform); it glides over the period
	m_targetG = std::tan(std::numbers::pi_v<float> * std::min(frequency, 0.45f * rate) / rate);
	if (m_g == 0.f)
	{
		// Starting afresh, faded in
		m_crossover = {};
		m_g = m_targetG;
	}
	return true;
}




void MonoBass::process(float& mid, float& side, float progress)
{
	const float g = m_g + (m_targetG - m_g) * progress;
	const float width = m_width + (m_targetWidth - m_width) * progress;
	const auto midBands = m_crossover[0].process(mid, g);
	const auto sideBands = m_crossover[1].process(side, g);
	const float mix = m_mix + (m_targetMix - m_mix) * progress;
	mid += (midBands.low + midBands.high - mid) * mix;
	side += (sideBands.high + width * sideBands.low - side) * mix;
}




void MonoBass::end()
{
	m_g = m_targetMix > 0.f ? m_targetG : 0.f;
	m_width = m_targetWidth;
	m_mix = m_targetMix;
}




void MonoBass::reset()
{
	m_crossover = {};
	m_g = 0.f;
	m_width = 1.f;
	m_mix = 0.f;
}

} // namespace lmms
