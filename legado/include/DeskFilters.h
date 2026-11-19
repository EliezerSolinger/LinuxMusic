/*
 * DeskFilters.h - the filters and the EQ of an analog desk's channel, matched to their analog responses
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

#ifndef LMMS_DESK_FILTERS_H
#define LMMS_DESK_FILTERS_H

#include <array>

#include "lmms_export.h"

namespace lmms
{

/**
 * The coefficients of a biquad, y = b0 x + b1 x1 + b2 x2 - a1 y1 - a2 y2, designed to match an analog prototype
 * up to Nyquist, unlike the bilinear transform, which cramps responses near it:
 * - second order: Martin Vicanek, "Matched Second Order Digital Filters" (2016): the poles by impulse invariance,
 *   the zeros so that the magnitude matches at DC, at the center frequency and at Nyquist;
 * - first order: the magnitude matches exactly at DC, at Nyquist and where the response changes most.
 * Frequencies in Hz.
 */
struct LMMS_EXPORT Biquad
{
	double b0 = 1.0, b1 = 0.0, b2 = 0.0, a1 = 0.0, a2 = 0.0;

	static auto lowPass(double frequency, double q, double sampleRate) -> Biquad;
	static auto highPass(double frequency, double q, double sampleRate) -> Biquad;
	static auto highPass1(double frequency, double sampleRate) -> Biquad;
	//! First order low-pass; @p frequency may be above Nyquist
	static auto lowPass1(double frequency, double sampleRate) -> Biquad;
	//! (s^2 + s G/Q + 1) / (s^2 + s/Q + 1): G at the center frequency; a boost (G >= 1)
	static auto peak(double frequency, double q, double gain, double sampleRate) -> Biquad;
	//! First order shelves, as a Baxandall tone control's: G at the far end, 3 dB short of it at the corner; boosts
	static auto lowShelf1(double corner, double gain, double sampleRate) -> Biquad;
	static auto highShelf1(double corner, double gain, double sampleRate) -> Biquad;

	auto isIdentity() const -> bool { return b0 == 1.0 && b1 == 0.0 && b2 == 0.0 && a1 == 0.0 && a2 == 0.0; }
	auto operator==(const Biquad&) const -> bool = default;

	//! The magnitude at @p frequency
	auto magnitude(double frequency, double sampleRate) const -> double;
};

/**
 * A stereo biquad (transposed direct form II) whose coefficients move linearly from those of the last period to
 * the new ones over a period, so that changing them does not click.
 */
class LMMS_EXPORT RampedBiquad
{
public:
	//! The coefficients to reach by the end of this period [audio thread]
	void setTarget(const Biquad& target) { m_from = m_to; m_to = target; }
	//! Ends the period: the target is reached
	void finish() { m_from = m_to; }
	void reset() { m_z1 = {}; m_z2 = {}; }
	auto isIdentity() const -> bool { return m_from.isIdentity() && m_to.isIdentity(); }
	auto target() const -> const Biquad& { return m_to; }

	auto process(double x, int channel, double progress) -> double
	{
		const auto c = at(progress);
		const double y = c.b0 * x + m_z1[channel];
		advance(c, x, y, channel);
		return y;
	}

	// For filters in a delay-free loop: the output is b0 times the input plus what the state gives
	auto at(double progress) const -> Biquad
	{
		if (progress >= 1.0 || m_from == m_to) { return m_to; }
		auto mix = [progress](double a, double b) { return a + (b - a) * progress; };
		return {mix(m_from.b0, m_to.b0), mix(m_from.b1, m_to.b1), mix(m_from.b2, m_to.b2), mix(m_from.a1, m_to.a1),
			mix(m_from.a2, m_to.a2)};
	}
	auto state(int channel) const -> double { return m_z1[channel]; }
	void advance(const Biquad& c, double x, double y, int channel)
	{
		m_z1[channel] = c.b1 * x - c.a1 * y + m_z2[channel];
		m_z2[channel] = c.b2 * x - c.a2 * y;
	}

private:
	Biquad m_from, m_to;
	std::array<double, 2> m_z1{}, m_z2{};
};

/**
 * The 4-band EQ of a Soundcraft or Mackie desk's channel: first order shelves at 80 Hz and 12 kHz and peaks at
 * 400 Hz and 2.5 kHz, +-15 dB, as in their active circuits:
 * - the peaks narrow as their gain grows (proportional Q): 2.5 octaves wide at +-3 dB, one at +-15 dB;
 * - the boosts act in parallel and the cuts in the amplifier's feedback, H = (1 + sum of boosts) / (1 + sum of
 *   cuts): alone, a band boosts or cuts as it says, while neighbouring bands interact, overlapping boosts or cuts
 *   adding up to less than their sum in dB, as on the desk. (The feedback is a delay-free loop, solved exactly.)
 * A band that changes from boost to cut goes through flat over a period, so that it does not click.
 */
class LMMS_EXPORT DeskEq
{
public:
	enum Band { Low, LowMid, HighMid, High, Bands };
	static constexpr double LowFrequency = 80.0;
	static constexpr double LowMidFrequency = 400.0;
	static constexpr double HighMidFrequency = 2500.0;
	static constexpr double HighFrequency = 12000.0;
	static constexpr double MaxGain = 15.0; // dB

	//! The peaks' bandwidth in octaves, at half their gain in dB, for a gain in dB
	static auto peakBandwidth(double gain) -> double;
	//! The peaks' Q for a gain in dB, giving that bandwidth
	static auto peakQ(double gain) -> double;

	/**
	 * Sets this period's gains in dB [audio thread]
	 * @param frequencies, gainFactors the channel's tolerances on the bands' frequencies and gains
	 */
	void begin(const std::array<float, Bands>& gains, double sampleRate,
		const std::array<double, Bands>& frequencies = {1.0, 1.0, 1.0, 1.0},
		const std::array<double, Bands>& gainFactors = {1.0, 1.0, 1.0, 1.0});
	//! Whether it does anything this period
	auto isActive() const -> bool;
	auto process(double x, int channel, double progress) -> double;
	//! Ends the period
	void finish();
	void reset();

	//! The EQ's magnitude at @p frequency with its current settings (for tests)
	auto magnitude(double frequency, double sampleRate) const -> double;

private:
	auto design(int band, double gain, double frequencyFactor, double sampleRate) const -> Biquad;

	std::array<RampedBiquad, Bands> m_filters;
	std::array<bool, Bands> m_cut{};	//!< whether each band is in the feedback, cutting
};

} // namespace lmms

#endif // LMMS_DESK_FILTERS_H
