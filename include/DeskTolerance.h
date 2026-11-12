/*
 * DeskTolerance.h - what makes an analog desk's channels differ from each other, and its output transformer
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

#ifndef LMMS_DESK_TOLERANCE_H
#define LMMS_DESK_TOLERANCE_H

#include <array>

#include "DeskFilters.h"
#include "LmmsTypes.h"
#include "lmms_export.h"

namespace lmms
{

/**
 * The tolerances of a channel's components: no two channels of an analog desk are quite alike, which is part of
 * how a desk sounds when they are summed. Each channel's are fixed, drawn from its number, as a physical desk's
 * channel 3 is always the same channel 3: within the usual tolerances of resistors, capacitors and pots (uniform):
 * +-0.1 dB of gain, +-1% on the filters' and the EQ's frequencies, +-2% on the EQ's gains, +-3% on where the gain
 * stage saturates.
 */
struct LMMS_EXPORT ChannelTolerance
{
	float gain = 0.f;							//!< dB
	float lowCut = 1.f, highPass = 1.f, lowPass = 1.f;	//!< frequency factors
	std::array<double, 4> eqFrequency{1.0, 1.0, 1.0, 1.0};
	std::array<double, 4> eqGain{1.0, 1.0, 1.0, 1.0};
	float drive = 1.f;							//!< factor on the gain stage's input, undone at its output

	//! None: an ideal channel
	static auto none() -> ChannelTolerance { return {}; }
	//! Channel @p channel's
	static auto forChannel(int channel) -> ChannelTolerance;
};

/**
 * The output transformer of a Neve desk's master, after its fader: the source driving the magnetizing inductance
 * of the core, whose flux (the integral of the voltage) saturates, so that it thickens the lows and does nothing to
 * the mids:
 * - the magnetizing current is linear plus a cubic term, (phi + p^3 / phi_s^2) / Lm, where p follows the flux
 *   with a dead band (a play operator): the core's hysteresis, lagging the saturation behind the flux;
 * - the source resistance over the inductance puts the low corner at 3 Hz; the saturation, about 0.5% at 30 Hz
 *   near full scale, falls with the square of the frequency, nothing at 1 kHz;
 * - the leakage inductance and capacitances roll the highs off: -0.5 dB at 20 kHz.
 * Solved implicitly at each sample (Newton's method).
 */
class LMMS_EXPORT OutputTransformer
{
public:
	static constexpr double LowCorner = 3.0;		//!< Hz
	static constexpr double SaturationFlux = 0.008;	//!< full scale times seconds
	static constexpr double Hysteresis = 0.02;		//!< of the saturation flux, the dead band's half width (its coercivity)
	static constexpr double HighCorner = 60000.0;	//!< Hz, first order

	void setUp(sample_rate_t sampleRate);
	void reset();
	auto process(float input, int channel) -> float;

private:
	double m_period = 0.0;
	double m_corner = 0.0;		//!< 2 pi LowCorner, the source resistance over the inductance
	std::array<double, 2> m_flux{}, m_lag{};
	RampedBiquad m_highs;
	sample_rate_t m_sampleRate = 0;
};

} // namespace lmms

#endif // LMMS_DESK_TOLERANCE_H
