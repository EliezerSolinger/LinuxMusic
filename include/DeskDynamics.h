/*
 * DeskDynamics.h - the compressor and the gate/expander of an analog desk's channel, after the SSL E series
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

#ifndef LMMS_DESK_DYNAMICS_H
#define LMMS_DESK_DYNAMICS_H

#include "LmmsTypes.h"
#include "lmms_export.h"

namespace lmms
{

/**
 * Levels in dBu, as on the desk, and in dBFS: 0 VU, the desk's nominal +4 dBu, is -18 dBFS RMS, the usual alignment
 * (see VuMeter)
 */
constexpr float DbuToDbfs = -22.f;

/**
 * The channel compressor of an SSL E series desk (after SSL's E Series Dynamics user guide), stereo-linked:
 * - a true RMS detector (or a peak one);
 * - RATIO from 1:1 (out) to infinity (a limiter), with an "over-easy" soft knee, or a hard one;
 * - THRESHOLD from +4 to -26 dBu, with automatic make-up gain, so that the output stays at the same level however
 *   much it compresses: a signal at the nominal level gets back what it loses;
 * - attack: 30 ms for 20 dB of gain reduction, 3 ms with FAST (program dependent: a slope in dB);
 * - RELEASE from 0.1 to 4 s: a time constant in dB (logarithmic), or, with LIN REL, a constant slope in dB (the same
 *   slope as the logarithmic curve's from 20 dB), which also raises the threshold by 6 dB.
 */
class LMMS_EXPORT ChannelCompressor
{
public:
	struct Settings
	{
		float threshold = 0.f;	//!< dBu
		float ratio = 1.f;		//!< 1 is out, Infinity a limiter
		bool fast = false;
		float release = 0.3f;	//!< s
		bool peak = false;
		bool hardKnee = false;
		bool linearRelease = false;
	};
	static constexpr float KneeWidth = 10.f;	//!< dB of the over-easy knee
	static constexpr float RmsTime = 0.005f;	//!< s, of the true RMS detector

	//! How much a level in dBFS is reduced, before the attack and release, in dB
	static auto staticReduction(const Settings& settings, float level) -> float;
	//! The make-up gain in dB: what a signal at the nominal level loses
	static auto makeup(const Settings& settings) -> float;

	void begin(const Settings& settings, sample_rate_t sampleRate);
	auto isActive() const -> bool { return m_settings.ratio > 1.f; }
	//! The gain for a frame, linear [audio thread]
	auto gain(float left, float right) -> float;
	void reset();
	//! The gain reduction now, in dB
	auto reduction() const -> float { return m_reduction; }

private:
	Settings m_settings;
	float m_makeup = 0.f;
	float m_rmsCoefficient = 0.f;
	float m_attackStep = 0.f;		//!< dB per frame
	float m_releaseCoefficient = 0.f;
	float m_releaseStep = 0.f;		//!< dB per frame, linear release
	float m_meanSquare = 0.f;
	float m_reduction = 0.f;		//!< dB
};

/**
 * The gate/expander of an SSL E series channel (after SSL's E Series Dynamics user guide), stereo-linked:
 * - a 20:1 gate, or a 2:1 expander (EXP);
 * - RANGE from 0 (out) to 40 dB, how far it closes;
 * - THRESHOLD from -30 to +10 dBu, with hysteresis growing as the threshold is lowered (2 dB at +10 dBu to 8 dB at
 *   -30 dBu), so that instruments can decay below the threshold that opened it;
 * - attack (opening): a controlled linear 1.5 ms per 40 dB, or 100 us per 40 dB with FAST;
 * - HOLD from 0 to 4 s before it starts to close; RELEASE from 0.1 to 4 s, a time constant in dB.
 */
class LMMS_EXPORT ChannelGate
{
public:
	struct Settings
	{
		float threshold = -10.f;	//!< dBu
		float range = 0.f;			//!< dB, 0 is out
		bool expander = false;
		bool fast = false;
		float release = 0.3f;		//!< s
		float hold = 0.f;			//!< s
	};

	//! The hysteresis at a threshold in dBu
	static auto hysteresis(float threshold) -> float;

	void begin(const Settings& settings, sample_rate_t sampleRate);
	auto isActive() const -> bool { return m_settings.range > 0.f; }
	//! The gain for a frame, linear [audio thread]
	auto gain(float left, float right) -> float;
	void reset();
	auto reduction() const -> float { return m_reduction; }

private:
	Settings m_settings;
	float m_attackStep = 0.f;		//!< dB per frame
	float m_releaseCoefficient = 0.f;
	f_cnt_t m_holdFrames = 0;
	float m_keyRelease = 0.f;
	float m_key = 0.f;				//!< the key's peak envelope
	bool m_open = false;
	f_cnt_t m_holding = 0;
	float m_reduction = 0.f;		//!< dB
};

} // namespace lmms

#endif // LMMS_DESK_DYNAMICS_H
