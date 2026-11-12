/*
 * AnalogChannelStrip.h - the built-in processing of a mixer channel in analog mode
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

#ifndef LMMS_ANALOG_CHANNEL_STRIP_H
#define LMMS_ANALOG_CHANNEL_STRIP_H

#include <array>
#include <span>

#include "AutomatableModel.h"
#include "DeskDynamics.h"
#include "DeskFilters.h"
#include "DeskTolerance.h"
#include "GainStage.h"
#include "LmmsTypes.h"
#include "MonoBass.h"

class QDomDocument;
class QDomElement;

namespace lmms
{

class SampleFrame;

/**
 * The channel strip of an analog mixing desk, built into each mixer channel (but the master) while the mixer is in
 * analog mode (see Mixer::isAnalog()). In signal order, after the channel's trim:
 * - the GAIN pot, muted to +30 dB;
 * - the low cut button (100 Hz, 18 dB/oct, as on Soundcraft desks), before the gain stage so that it gets no rumble
 *   to saturate on;
 * - the gain stage: flat, an op-amp or a tube, of a choice of models (see GainStage);
 * - sweepable high-pass and low-pass filters (12 dB/oct), and mono bass (see MonoBass);
 * - the dynamics of an SSL E series channel: a gate/expander and a compressor (see ChannelGate, ChannelCompressor);
 * - the 4-band EQ of a Soundcraft or Mackie desk (see DeskEq): shelves at 80 Hz and 12 kHz, peaks at 400 Hz and
 *   2.5 kHz, with proportional Q and interacting bands.
 * Each section left at its neutral setting is skipped, so a neutral strip leaves the audio untouched. The filters
 * match their analog responses up to Nyquist (see Biquad), and ramp their changes over a period.
 */
class LMMS_EXPORT AnalogChannelStrip
{
public:
	explicit AnalogChannelStrip(Model* parent);

	/**
	 * The GAIN pot, as on a desk: muted at the far left, 0 dB in the middle, +30 dB at the far right, where the gain
	 * stage saturates. Its position, 0 to 1 (see potGain()).
	 */
	FloatModel m_gainModel;
	static constexpr float MaxGain = 30.f; // dB
	//! The gain, linear, at pot position @p position: up to the middle, (2 position)^3 (-18 dB at a quarter), then
	//! linear in dB up to MaxGain
	static auto potGain(float position) -> float;

	using Drive = GainStage::Model;
	IntModel m_driveModel;			//!< the gain stage's model, a GainStage::Model
	BoolModel m_realisticModel;		//!< for a tube: its circuit solved at each sample (see GainStage::process())

	//! The gain stage, which also measures its saturation for the strip's LEDs
	auto gainStage() -> GainStage& { return m_gainStage; }

	BoolModel m_lowCutModel;
	FloatModel m_highPassModel;		//!< Hz; off at the lowest setting
	FloatModel m_lowPassModel;		//!< Hz; off at the highest setting
	FloatModel m_monoBassWidthModel;		//!< % of the side kept below the mono bass frequency; off at 100
	FloatModel m_monoBassFrequencyModel;	//!< Hz
	// The SSL E series' dynamics (see ChannelCompressor and ChannelGate): on the strip, COMP, RATIO and FAST, GATE
	// and RANGE; the rest in the channel's panel
	FloatModel m_compressorModel;			//!< threshold, dBu
	FloatModel m_compressorRatioModel;		//!< 1 (out) to MaxRatio (infinity)
	BoolModel m_compressorFastModel;
	FloatModel m_compressorReleaseModel;	//!< s
	BoolModel m_compressorPeakModel;
	BoolModel m_compressorHardKneeModel;
	BoolModel m_compressorLinearReleaseModel;
	FloatModel m_gateModel;					//!< threshold, dBu
	FloatModel m_gateRangeModel;			//!< dB, 0 (out) to 40
	BoolModel m_gateExpanderModel;
	BoolModel m_gateFastModel;
	FloatModel m_gateReleaseModel;			//!< s
	FloatModel m_gateHoldModel;				//!< s
	static constexpr float MaxRatio = 20.f;	//!< the ratio knob's end, which is infinity
	//! The compressor's ratio at a ratio knob setting
	static auto ratioAt(float setting) -> float;
	FloatModel m_eqHighModel;		//!< dB
	FloatModel m_eqHighMidModel;	//!< dB
	FloatModel m_eqLowMidModel;		//!< dB
	FloatModel m_eqLowModel;		//!< dB

	/**
	 * Processes the channel's main pair in place; @p hadInput tells whether it had signal [audio thread]
	 * @param tolerance its components' (see ChannelTolerance)
	 */
	void process(std::span<SampleFrame> frames, sample_rate_t sampleRate, bool hadInput,
		const ChannelTolerance& tolerance = ChannelTolerance::none());

	//! Whether the filters may still ring after the input fell silent, so the channel must be processed
	//! [audio thread]
	auto holdsSignal() const -> bool;

	//! Forgets the filters' and the dynamics' state, e.g. while the channel is muted [audio thread]
	void reset();

	//! Puts every control back to its neutral setting
	void setNeutral();

	void setDisplayNames(const QString& channelName);
	void saveSettings(QDomDocument& doc, QDomElement& channelElement);
	void loadSettings(const QDomElement& channelElement);


private:
	void finishFilters();

	GainStage m_gainStage;
	float m_gain = 1.f; //!< of the last frame, ramped

	RampedBiquad m_lowCut2, m_lowCut1;	// third order Butterworth: 18 dB/oct
	RampedBiquad m_highPass;
	RampedBiquad m_lowPass;
	DeskEq m_eq;
	MonoBass m_monoBass;

	//! Frames the filters may still ring for after the input fell silent
	f_cnt_t m_tailFrames = 0;

	ChannelCompressor m_compressor;
	ChannelGate m_gate;
};

} // namespace lmms

#endif // LMMS_ANALOG_CHANNEL_STRIP_H
