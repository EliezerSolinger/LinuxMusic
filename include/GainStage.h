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
 * - tubes: the whole signal goes through a tube stage (see TubeStage), in volts at its grid: the GAIN pot sets how
 *   hard it is driven, and its output is brought back to unity gain. Either light (its curve, with the Miller
 *   capacitance, blocking and sag approximated around it, then a DC blocker) or realistic (its circuit solved at each
 *   sample).
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
	 *     slow (1.7 V/us), its swing giving way a little into 600 ohms, and the crossover distortion of its class
	 *     AB output stage, which its feedback reduces less the higher the frequency (always there, very low);
	 *   - TL072, JFET input, of Mackie desks and much studio gear: knee at +19 dBFS, rails a little lower and
	 *     slightly asymmetric, 13 V/us, weak into 600 ohms, and its output flipping to the positive rail when its
	 *     input goes past its common mode range (+22.6 dBFS negative peaks): its ugly overload;
	 *   - NE5532, the dual NE5534, the op-amp of most clean desks (SSL, Neve, Amek, Mackie Onyx): knee at +21 dBFS,
	 *     symmetric rails, 9 V/us, made to drive 600 ohms;
	 *   - NE5534, the single one: as the NE5532, 13 V/us.
	 * - Tubes, preamp triodes in a common cathode stage (Norman Koren's model, see TubeStage), driven in volts: with
	 *   the GAIN pot in the middle, the 12AX7 starts to clip at 0 dBFS, so it colors from -6 dBFS; the others clip at
	 *   their own levels:
	 *   - 12AX7, the usual stage;
	 *   - 12AX7 V (vintage): biased cold, clipping first towards cutoff, rich in even harmonics;
	 *   - 12AX7 H (hot): biased hot on a higher supply, clipping alike on both sides, stronger in odd harmonics;
	 *   - 12AT7 and 12AU7: less gain, so more headroom.
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
		float rail;					//!< where it saturates towards with no load, linear
		float negativeRailRatio;	//!< the negative side's knee and rail relative to the positive one's
		float slewRate;				//!< V/us
		float outputCurrent;		//!< A it can drive before its swing gives way
		float gainBandwidth;		//!< Hz
		float crossover;			//!< V of its output stage's dead band (class AB), 0 for none worth modelling
		float commonModeLimit;		//!< below this input (linear) its output flips to the positive rail; 0 for none
	};
	static auto opAmpSpec(Model model) -> OpAmpSpec;

	//! The load the op-amps drive, as a desk's line driver does: 600 ohms
	static constexpr float Load = 600.f;
	//! Where an op-amp's output saturates towards, linear: its rail, or lower if it cannot drive the load that far
	static auto opAmpRail(Model model) -> float;

	/**
	 * The op-amp's transfer function: linear up to the knee, then softly towards the rail (see opAmpRail()); for
	 * the TL072, the output flips to the positive rail when the input goes past its common mode range
	 */
	static auto opAmp(Model model, float sample) -> float;

	//! The tube's static curve, with unity gain and the same polarity for small signals, and no offset
	static auto tube(Model model, float sample) -> float;

	//! Where a tube starts to clip, linear, with the GAIN pot in the middle (0 dBFS for the 12AX7)
	static auto tubeClipLevel(Model model) -> float;

	//! Where FLAT would clip on a desk, for its CLIP LED (+24 dBFS, an op-amp's rail)
	static constexpr float FlatClipLevel = 16.f;

	GainStage();
	~GainStage();

	/**
	 * Processes both channels of @p frames with @p model, and measures them for the LEDs [audio thread].
	 * @param realistic for a tube: its circuit solved at each sample, rather than approximated around its curve
	 * @param drive a factor on its input, undone at its output: where it saturates (a channel's tolerance)
	 */
	void process(std::span<SampleFrame> frames, Model model, bool realistic, sample_rate_t sampleRate,
		float drive = 1.f);

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
	//! The 4558's crossover distortion, at every level [audio thread]
	void processCrossover(std::span<SampleFrame> frames, const OpAmpSpec& spec, sample_rate_t sampleRate);

	struct State;
	std::unique_ptr<State> m_state; //!< the oversamplers, kept out of this header, which the plugins see
	std::atomic<float> m_saturation = 0.f;
	std::atomic<bool> m_clip = false;
};

} // namespace lmms

#endif // LMMS_GAIN_STAGE_H
