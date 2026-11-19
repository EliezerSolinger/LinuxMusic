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

/**
 * The tolerances are a contract with the people who mix on the desk: channel 3 has the same character in every
 * project and in every version of the program, so that they can learn their desk ("channel 3 is a little
 * brighter"). So each tolerance is drawn from its own stream, keyed by the channel's number and the tolerance's
 * own number below, by integer arithmetic only (the same on every platform). Never change the seed, the mixing or
 * these numbers; a new tolerance takes a new number, which leaves the others as they are. (MixerTest's
 * channelTolerancesAreFrozen() holds them.)
 */
enum class Part : std::uint64_t
{
	Gain = 1, LowCut = 2, HighPass = 3, LowPass = 4,
	EqLowFrequency = 5, EqLowMidFrequency = 6, EqHighMidFrequency = 7, EqHighFrequency = 8,
	EqLowGain = 9, EqLowMidGain = 10, EqHighMidGain = 11, EqHighGain = 12,
	Drive = 13,
};

constexpr std::uint64_t Seed = 0x4c696e75784d7573ULL; // "LinuxMus"

//! SplitMix64's finalizer: a well mixed 64-bit hash
constexpr auto mix(std::uint64_t z) -> std::uint64_t
{
	z += 0x9e3779b97f4a7c15ULL;
	z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
	z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
	return z ^ (z >> 31);
}

//! Channel @p channel's draw for @p part, uniform in [-1, 1]
auto draw(int channel, Part part) -> double
{
	const auto z = mix(mix(Seed ^ static_cast<std::uint64_t>(channel)) ^ static_cast<std::uint64_t>(part));
	return static_cast<double>(z >> 11) / static_cast<double>(1ULL << 53) * 2.0 - 1.0;
}

} // namespace




auto ChannelTolerance::forChannel(int channel) -> ChannelTolerance
{
	auto tolerance = ChannelTolerance{};
	tolerance.gain = static_cast<float>(0.1 * draw(channel, Part::Gain));
	tolerance.lowCut = static_cast<float>(1.0 + 0.01 * draw(channel, Part::LowCut));
	tolerance.highPass = static_cast<float>(1.0 + 0.01 * draw(channel, Part::HighPass));
	tolerance.lowPass = static_cast<float>(1.0 + 0.01 * draw(channel, Part::LowPass));
	constexpr auto frequencies = std::array{Part::EqLowFrequency, Part::EqLowMidFrequency, Part::EqHighMidFrequency,
		Part::EqHighFrequency};
	constexpr auto gains = std::array{Part::EqLowGain, Part::EqLowMidGain, Part::EqHighMidGain, Part::EqHighGain};
	for (auto band = std::size_t{0}; band < 4; ++band)
	{
		tolerance.eqFrequency[band] = 1.0 + 0.01 * draw(channel, frequencies[band]);
		tolerance.eqGain[band] = 1.0 + 0.02 * draw(channel, gains[band]);
	}
	tolerance.drive = static_cast<float>(1.0 + 0.03 * draw(channel, Part::Drive));
	return tolerance;
}




void OutputTransformer::setUp(sample_rate_t sampleRate)
{
	if (sampleRate == m_sampleRate) { return; }
	m_sampleRate = sampleRate;
	m_period = 1.0 / sampleRate;
	m_corner = 2.0 * std::numbers::pi * m_core.lowCorner;
	m_highs.setTarget(Biquad::lowPass1(m_core.highCorner, sampleRate));
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
	const double band = Hysteresis * m_core.saturationFlux;
	const double s2 = m_core.saturationFlux * m_core.saturationFlux;
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
