/*
 * MonoBass.h - narrows the stereo image of the lows of a mid/side signal
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

#ifndef LMMS_MONO_BASS_H
#define LMMS_MONO_BASS_H

#include <array>

#include "LmmsTypes.h"
#include "lmms_export.h"

namespace lmms
{

/**
 * Mono bass: below a frequency, narrows the side of a mid/side signal (M = (L+R)/2, S = (L-R)/2) to a width, 0 being
 * mono. Used by the master's stereo section and by the channels' analog strips.
 *
 * A Linkwitz-Riley crossover (24 dB/oct) splits the side, S' = HP(S) + width LP(S), and the mid goes through the same
 * crossover, LP(M) + HP(M), an allpass of flat gain: mid and side get the same phase, so the image above the
 * frequency stays where it was. (Subtracting a low-pass from the side would not do: the low-pass lags in phase, so the
 * lows would not cancel.) The crossover is built from state variable filters (topology-preserving transform), whose
 * frequency glides sample by sample without clicks; the width is ramped over each period, and switching on or off
 * crossfades between the crossover and the dry signal over a period.
 *
 * Each period: begin(), then process() for every frame if begin() returned true, then end(). [audio thread]
 */
class LMMS_EXPORT MonoBass
{
public:
	/**
	 * Starts a period with @p frequency (Hz) and @p width (of the side below it: 1 = off, 0 = mono).
	 * @returns whether the period needs processing: on, or fading out
	 */
	auto begin(float frequency, float width, sample_rate_t sampleRate) -> bool;

	//! One frame, in mid/side, @p progress of the way through the period (up to 1 at its last frame)
	void process(float& mid, float& side, float progress);

	void end();

	//! Forgets the crossover's state, e.g. while the channel is muted
	void reset();

private:
	//! A state variable filter, Butterworth, giving its low-pass and high-pass outputs
	struct Svf
	{
		float ic1 = 0.f, ic2 = 0.f;
		struct Outputs { float low; float high; };
		auto process(float in, float g) -> Outputs;
	};

	//! A 4th-order Linkwitz-Riley crossover: two Butterworth sections in a row on each side of the split
	struct Crossover
	{
		Svf low1, low2, high1, high2;
		struct Outputs { float low; float high; };
		auto process(float in, float g) -> Outputs;
	};

	std::array<Crossover, 2> m_crossover;	//!< for the mid and the side
	float m_g = 0.f;			//!< the frequency warped for the filters, at the end of the last period; 0 = afresh
	float m_targetG = 0.f;
	float m_width = 1.f;		//!< at the end of the last period
	float m_targetWidth = 1.f;
	float m_mix = 0.f;			//!< of the crossover against the dry signal, at the end of the last period
	float m_targetMix = 0.f;
};

} // namespace lmms

#endif // LMMS_MONO_BASS_H
