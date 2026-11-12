/*
 * DeskTolerance.cpp - what makes an analog desk's channels differ from each other, and its output transformer
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

#include "DeskTolerance.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <numbers>

namespace lmms
{

namespace
{

//! SplitMix64: a small, well mixed generator, the same on every platform
class Draw
{
public:
	explicit Draw(std::uint64_t seed) : m_state{seed} {}

	//! Uniform in [-1, 1]
	auto next() -> double
	{
		m_state += 0x9e3779b97f4a7c15ULL;
		std::uint64_t z = m_state;
		z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
		z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
		z ^= z >> 31;
		return static_cast<double>(z >> 11) / static_cast<double>(1ULL << 53) * 2.0 - 1.0;
	}

private:
	std::uint64_t m_state;
};

} // namespace




auto ChannelTolerance::forChannel(int channel) -> ChannelTolerance
{
	auto draw = Draw{0x4c696e75784d7573ULL ^ static_cast<std::uint64_t>(channel) * 0x100000001b3ULL};
	auto tolerance = ChannelTolerance{};
	tolerance.gain = static_cast<float>(0.1 * draw.next());
	tolerance.lowCut = static_cast<float>(1.0 + 0.01 * draw.next());
	tolerance.highPass = static_cast<float>(1.0 + 0.01 * draw.next());
	tolerance.lowPass = static_cast<float>(1.0 + 0.01 * draw.next());
	for (auto& factor : tolerance.eqFrequency) { factor = 1.0 + 0.01 * draw.next(); }
	for (auto& factor : tolerance.eqGain) { factor = 1.0 + 0.02 * draw.next(); }
	tolerance.drive = static_cast<float>(1.0 + 0.03 * draw.next());
	return tolerance;
}




void OutputTransformer::setUp(sample_rate_t sampleRate)
{
	if (sampleRate == m_sampleRate) { return; }
	m_sampleRate = sampleRate;
	m_period = 1.0 / sampleRate;
	m_corner = 2.0 * std::numbers::pi * LowCorner;
	m_highs.setTarget(Biquad::lowPass1(HighCorner, sampleRate));
	m_highs.finish();
	reset();
}




void OutputTransformer::reset()
{
	m_flux = {};
	m_lag = {};
	m_highs.reset();
}




auto OutputTransformer::process(float input, int channel) -> float
{
	// The voltage across the core, v = input - corner (phi + p^3 / phi_s^2), with phi = flux + v T (backward
	// Euler), solved for v
	const double source = input;
	const double band = Hysteresis * SaturationFlux;
	const double s2 = SaturationFlux * SaturationFlux;
	double v = source;
	double flux = 0.0, lag = 0.0;
	for (int iteration = 0; iteration < 8; ++iteration)
	{
		flux = m_flux[channel] + v * m_period;
		lag = std::clamp(m_lag[channel], flux - band, flux + band);
		const bool moving = lag != m_lag[channel];
		const double f = v - source + m_corner * (flux + lag * lag * lag / s2);
		const double slope = 1.0 + m_corner * m_period * (1.0 + (moving ? 3.0 * lag * lag / s2 : 0.0));
		const double step = f / slope;
		v -= step;
		if (std::abs(step) < 1e-12) { break; }
	}
	flux = m_flux[channel] + v * m_period;
	m_lag[channel] = std::clamp(m_lag[channel], flux - band, flux + band);
	m_flux[channel] = flux;
	return static_cast<float>(m_highs.process(v, channel, 1.0));
}

} // namespace lmms
