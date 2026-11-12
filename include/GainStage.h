/*
 * GainStage.h - the gain stage of an analog desk: flat, an op-amp or a tube
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

#ifndef LMMS_GAIN_STAGE_H
#define LMMS_GAIN_STAGE_H

#include <array>
#include <atomic>
#include <memory>
#include <span>

#include "LmmsTypes.h"
#include "lmms_export.h"

namespace lmms
{

class SampleFrame;

/**
 * The gain stage of an analog desk, of a choice of models, used by the channels' analog strips and the master.
 * It works at 4 times the sample rate, so that its harmonics do not alias:
 * - op-amps: only the part the op-amp changes (its output minus its input, which is 0 below the knee) goes through
 *   the oversampling, then is added to the signal, so below the knee the signal is untouched, bit for bit; blocks far
 *   below the knee skip the oversampling. The op-amp's slew rate limits very loud, very high frequencies.
 * - tubes: the whole signal goes through the tube, then a DC blocker (its asymmetry adds an offset).
 * It measures how far it saturates and when it nears its limit, for the SAT and CLIP LEDs.
 */
class LMMS_EXPORT GainStage
{
public:
	/**
	 * The models. Their numbers are saved with projects, so new ones go at the end.
	 * - FLAT: nothing, the gain is clean.
	 * - Op-amps, saturating softly from their knee towards their rails: the headroom of a desk above its nominal
	 *   level, so they are clean at usual levels and saturate when the gain is pushed (datasheet slew rates; knee and
	 *   rails approximate each chip's output swing):
	 *   - 4558, of many Soundcraft and Mackie desks: knee at +18 dBFS, the negative rail 5% lower (even harmonics),
	 *     slow (1.7 V/us);
	 *   - TL072, JFET input, of Mackie desks and much studio gear: knee at +19 dBFS, rails a little lower and
	 *     slightly asymmetric, 13 V/us;
	 *   - NE5532, the dual NE5534, the op-amp of most clean desks (SSL, Neve, Amek, Mackie Onyx): knee at +21 dBFS,
	 *     symmetric rails, 9 V/us;
	 *   - NE5534, the single one: as the NE5532, 13 V/us.
	 * - Tubes, preamp triodes in a common cathode stage, which color a little at usual levels and start to clip at
	 *   +12 dBFS:
	 *   - 12AX7: the curve Tim Goetze sampled from a SPICE simulation for CAPS;
	 *   - 12AX7 V (vintage): CAPS' original polynomial, tuned by ear, richest in even harmonics;
	 *   - 12AX7 H (hot): CAPS' polynomial fitted to the curve, stronger in odd harmonics;
	 *   - 12AT7 and 12AU7: Norman Koren's triode model with his published parameters, the stage's transfer curve
	 *     solved numerically, normalized at whichever side clips first (the grid conducting): less gain than the
	 *     12AX7, so more headroom past where they start to clip.
	 */
	enum class Model
	{
		Flat, OpAmp4558, OpAmp5532, Tube12AX7, Tube12AX7Vintage, Tube12AX7Hot, OpAmp5534, OpAmpTL072, Tube12AT7,
		Tube12AU7, Count
	};

	//! The order the models are offered in: flat, the op-amps, the tubes
	static constexpr auto DisplayOrder = std::array{Model::Flat, Model::OpAmp4558, Model::OpAmpTL072, Model::OpAmp5532,
		Model::OpAmp5534, Model::Tube12AX7, Model::Tube12AX7Vintage, Model::Tube12AX7Hot, Model::Tube12AT7,
		Model::Tube12AU7};

	static auto isTube(Model model) -> bool;
	static auto isOpAmp(Model model) -> bool;

	//! What an op-amp model is like
	struct OpAmpSpec
	{
		float knee;					//!< where it starts to saturate, linear
		float rail;					//!< where it saturates towards, linear
		float negativeRailRatio;	//!< the negative side's knee and rail relative to the positive one's
		float slewRate;				//!< V/us
	};
	static auto opAmpSpec(Model model) -> OpAmpSpec;

	//! The op-amp's transfer function: linear up to the knee, then softly towards the rail
	static auto opAmp(Model model, float sample) -> float;

	//! The tube's transfer function, with unity gain and the same polarity for small signals, and no offset
	static auto tube(Model model, float sample) -> float;

	//! Where the tubes start to clip, linear (+12 dBFS)
	static constexpr float TubeClipLevel = 4.f;

	//! Where FLAT would clip on a desk, for its CLIP LED (+24 dBFS, an op-amp's rail)
	static constexpr float FlatClipLevel = 16.f;

	GainStage();
	~GainStage();

	//! Processes both channels of @p frames with @p model, and measures them for the LEDs [audio thread]
	void process(std::span<SampleFrame> frames, Model model, sample_rate_t sampleRate);

	//! Forgets the oversamplers' and the DC blocker's state [audio thread]
	void reset();

	/**
	 * How far the stage saturated since the last call, 0 (clean) to 1 (fully), for the SAT LED: for an op-amp, from
	 * its knee to its rail; for a tube, from 12 dB below where it starts to clip to there [GUI thread]
	 */
	auto takeSaturation() -> float { return m_saturation.exchange(0.f, std::memory_order_relaxed); }

	//! Whether the stage came within 3 dB of clipping since the last call, for the CLIP LED [GUI thread]
	auto takeClip() -> bool { return m_clip.exchange(false, std::memory_order_relaxed); }

private:
	void measure(float peak, Model model);

	struct State;
	std::unique_ptr<State> m_state; //!< the oversamplers, kept out of this header, which the plugins see
	std::atomic<float> m_saturation = 0.f;
	std::atomic<bool> m_clip = false;
};

} // namespace lmms

#endif // LMMS_GAIN_STAGE_H
