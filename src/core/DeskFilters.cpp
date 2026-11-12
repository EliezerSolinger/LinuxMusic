/*
 * DeskFilters.cpp - the filters and the EQ of an analog desk's channel, matched to their analog responses
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

#include "DeskFilters.h"

#include <algorithm>
#include <cmath>
#include <complex>
#include <numbers>

namespace lmms
{

namespace
{

using std::numbers::pi;

//! The poles of a second order section, by impulse invariance, and Vicanek's terms for the magnitude matching
struct Matched
{
	double a1, a2;
	double a0Term, a1Term, a2Term;	// A0, A1, A2
	double phi0, phi1, phi2;

	Matched(double frequency, double q, double sampleRate)
	{
		const double w0 = 2.0 * pi * std::min(frequency, 0.4999 * sampleRate) / sampleRate;
		const double zeta = 1.0 / (2.0 * q);
		const double decay = std::exp(-zeta * w0);
		a1 = zeta <= 1.0 ? -2.0 * decay * std::cos(w0 * std::sqrt(1.0 - zeta * zeta))
			: -2.0 * decay * std::cosh(w0 * std::sqrt(zeta * zeta - 1.0));
		a2 = std::exp(-2.0 * zeta * w0);
		a0Term = (1.0 + a1 + a2) * (1.0 + a1 + a2);
		a1Term = (1.0 - a1 + a2) * (1.0 - a1 + a2);
		a2Term = -4.0 * a2;
		phi1 = std::sin(w0 / 2.0) * std::sin(w0 / 2.0);
		phi0 = 1.0 - phi1;
		phi2 = 4.0 * phi0 * phi1;
	}

	auto sum() const -> double { return a0Term * phi0 + a1Term * phi1 + a2Term * phi2; }
};

/**
 * A first order section matching an analog magnitude exactly at DC, at Nyquist and at @p frequency, where it
 * changes most (three conditions for its three coefficients): with A = (1 + a1)^2, B = (1 - a1)^2, its squared
 * magnitude is (dc^2 A cos^2(w/2) + nyquist^2 B sin^2(w/2)) / (A cos^2(w/2) + B sin^2(w/2))
 */
auto firstOrder(double dc, double nyquist, double frequency, double magnitude, double sampleRate) -> Biquad
{
	const double w = 2.0 * pi * frequency / sampleRate;
	const double phi1 = std::sin(w / 2.0) * std::sin(w / 2.0);
	const double phi0 = 1.0 - phi1;
	const double m2 = magnitude * magnitude;
	const double ratio = phi1 * (m2 - nyquist * nyquist) / (phi0 * (dc * dc - m2)); // A / B
	const double t = std::sqrt(std::max(ratio, 1e-12));
	const double a1 = (t - 1.0) / (t + 1.0);
	const double sum = dc * (1.0 + a1);
	const double difference = nyquist * (1.0 - a1);
	return {(sum + difference) / 2.0, (sum - difference) / 2.0, 0.0, a1, 0.0};
}

//! The magnitude of (s + wz) / (s + wp) at w, all in Hz
auto firstOrderMagnitude(double f, double zero, double pole) -> double
{
	return std::hypot(f, zero) / std::hypot(f, pole);
}

} // namespace




auto Biquad::lowPass(double frequency, double q, double sampleRate) -> Biquad
{
	const auto m = Matched{frequency, q, sampleRate};
	const double r1 = m.sum() * q * q;
	const double b0Term = m.a0Term;
	const double b1Term = std::max((r1 - b0Term * m.phi0) / m.phi1, 0.0);
	const double b0 = 0.5 * (std::sqrt(b0Term) + std::sqrt(b1Term));
	return {b0, std::sqrt(b0Term) - b0, 0.0, m.a1, m.a2};
}




auto Biquad::highPass(double frequency, double q, double sampleRate) -> Biquad
{
	const auto m = Matched{frequency, q, sampleRate};
	const double b0 = q * std::sqrt(std::max(m.sum(), 0.0)) / (4.0 * m.phi1);
	return {b0, -2.0 * b0, b0, m.a1, m.a2};
}




auto Biquad::highPass1(double frequency, double sampleRate) -> Biquad
{
	// s / (s + wp): matched at the corner
	const double nyquist = sampleRate / 2.0;
	const double f = std::min(frequency, 0.45 * sampleRate);
	return firstOrder(0.0, nyquist / std::hypot(nyquist, f), f, std::sqrt(0.5), sampleRate);
}




auto Biquad::peak(double frequency, double q, double gain, double sampleRate) -> Biquad
{
	if (gain == 1.0) { return {}; }
	const auto m = Matched{frequency, q, sampleRate};
	const double g2 = gain * gain;
	const double r1 = m.sum() * g2;
	const double r2 = (-m.a0Term + m.a1Term + 4.0 * (m.phi0 - m.phi1) * m.a2Term) * g2;
	const double b0Term = m.a0Term;
	const double b2Term = (r1 - r2 * m.phi1 - b0Term) / (4.0 * m.phi1 * m.phi1);
	const double b1Term = std::max(r2 + b0Term + 4.0 * (m.phi1 - m.phi0) * b2Term, 0.0);
	const double w = 0.5 * (std::sqrt(b0Term) + std::sqrt(b1Term));
	const double b0 = 0.5 * (w + std::sqrt(std::max(w * w + b2Term, 0.0)));
	return {b0, 0.5 * (std::sqrt(b0Term) - std::sqrt(b1Term)), -b2Term / (4.0 * b0), m.a1, m.a2};
}




auto Biquad::lowShelf1(double corner, double gain, double sampleRate) -> Biquad
{
	// (s + wz) / (s + wp), the pole at the corner, the zero G times higher: G at DC; matched where it changes most
	if (gain == 1.0) { return {}; }
	const double nyquist = sampleRate / 2.0;
	const double zero = corner * gain;
	const double middle = std::min(std::sqrt(corner * zero), 0.45 * sampleRate);
	return firstOrder(gain, firstOrderMagnitude(nyquist, zero, corner), middle,
		firstOrderMagnitude(middle, zero, corner), sampleRate);
}




auto Biquad::highShelf1(double corner, double gain, double sampleRate) -> Biquad
{
	// G (s + wz) / (s + wp), the pole at the corner, the zero G times lower: G far above it
	if (gain == 1.0) { return {}; }
	const double nyquist = sampleRate / 2.0;
	const double zero = corner / gain;
	const double middle = std::min(std::sqrt(corner * zero), 0.45 * sampleRate);
	return firstOrder(1.0, gain * firstOrderMagnitude(nyquist, zero, corner), middle,
		gain * firstOrderMagnitude(middle, zero, corner), sampleRate);
}




auto Biquad::magnitude(double frequency, double sampleRate) const -> double
{
	const auto z = std::polar(1.0, -2.0 * pi * frequency / sampleRate); // z^-1
	return std::abs((b0 + z * (b1 + z * b2)) / (1.0 + z * (a1 + z * a2)));
}




auto DeskEq::peakBandwidth(double gain) -> double
{
	// 2.5 octaves for gentle settings, narrowing to one when pushed all the way
	const double g = std::clamp(std::abs(gain), 3.0, MaxGain);
	return 2.5 - 1.5 * (g - 3.0) / (MaxGain - 3.0);
}

auto DeskEq::peakQ(double gain) -> double
{
	// The bandwidth at half the gain in dB, where |H|^2 = G: there (1 - w^2) Q = sqrt(G) w, with w = 2^(-bw/2)
	const double g = std::pow(10.0, std::abs(gain) / 20.0);
	const double w = std::pow(2.0, -peakBandwidth(gain) / 2.0);
	return std::sqrt(g) * w / (1.0 - w * w);
}




auto DeskEq::design(int band, double gain, double sampleRate) const -> Biquad
{
	// Each band as a boost; a cut is the inverse of the boost of the same amount, in the feedback
	const double boost = std::pow(10.0, std::abs(gain) / 20.0);
	switch (band)
	{
		case Low: return Biquad::lowShelf1(LowFrequency, boost, sampleRate);
		case LowMid: return Biquad::peak(LowMidFrequency, peakQ(gain), boost, sampleRate);
		case HighMid: return Biquad::peak(HighMidFrequency, peakQ(gain), boost, sampleRate);
		default: return Biquad::highShelf1(HighFrequency, boost, sampleRate);
	}
}




void DeskEq::begin(const std::array<float, Bands>& gains, double sampleRate)
{
	for (int band = 0; band < Bands; ++band)
	{
		auto& filter = m_filters[band];
		const double gain = gains[band];
		const bool cut = gain < 0.0;
		// Changing sides: flat first, then the other side from the next period on
		if (cut != m_cut[band] && gain != 0.0)
		{
			if (!filter.target().isIdentity())
			{
				filter.setTarget({});
				continue;
			}
			m_cut[band] = cut;
		}
		filter.setTarget(design(band, gain, sampleRate));
	}
}




auto DeskEq::isActive() const -> bool
{
	return std::any_of(m_filters.begin(), m_filters.end(), [](const auto& f) { return !f.isIdentity(); });
}




auto DeskEq::process(double x, int channel, double progress) -> double
{
	// The boosts, in parallel: 1 + sum of (H - 1)
	double boosted = x;
	for (int band = 0; band < Bands; ++band)
	{
		auto& filter = m_filters[band];
		if (m_cut[band] || filter.isIdentity()) { continue; }
		boosted += filter.process(x, channel, progress) - x;
	}

	// The cuts, in the feedback: y = boosted - sum of (H - 1)(y), with H(y) = b0 y + state, solved for y
	double states = 0.0;
	double slope = 1.0;
	auto coefficients = std::array<Biquad, Bands>{};
	for (int band = 0; band < Bands; ++band)
	{
		const auto& filter = m_filters[band];
		if (!m_cut[band] || filter.isIdentity()) { continue; }
		coefficients[band] = filter.at(progress);
		states += filter.state(channel);
		slope += coefficients[band].b0 - 1.0;
	}
	if (slope == 1.0 && states == 0.0) { return boosted; }
	const double y = (boosted - states) / slope;
	for (int band = 0; band < Bands; ++band)
	{
		auto& filter = m_filters[band];
		if (!m_cut[band] || filter.isIdentity()) { continue; }
		const auto& c = coefficients[band];
		filter.advance(c, y, c.b0 * y + filter.state(channel), channel);
	}
	return y;
}




void DeskEq::finish()
{
	for (auto& filter : m_filters)
	{
		filter.finish();
		// A flat band starts afresh when it is used again
		if (filter.isIdentity()) { filter.reset(); }
	}
}




void DeskEq::reset()
{
	for (auto& filter : m_filters) { filter.reset(); }
}




auto DeskEq::magnitude(double frequency, double sampleRate) const -> double
{
	const auto z = std::polar(1.0, -2.0 * pi * frequency / sampleRate);
	auto response = [z](const Biquad& c) {
		return (c.b0 + z * (c.b1 + z * c.b2)) / (1.0 + z * (c.a1 + z * c.a2));
	};
	auto boosts = std::complex<double>{1.0};
	auto cuts = std::complex<double>{1.0};
	for (int band = 0; band < Bands; ++band)
	{
		const auto& c = m_filters[band].target();
		if (c.isIdentity()) { continue; }
		(m_cut[band] ? cuts : boosts) += response(c) - 1.0;
	}
	return std::abs(boosts / cuts);
}

} // namespace lmms
