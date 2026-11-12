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
#include <memory>
#include <span>

#include "AutomatableModel.h"
#include "EqFilter.h"
#include "LmmsTypes.h"
#include "MonoBass.h"

class QDomDocument;
class QDomElement;

namespace lmms
{

class SampleFrame;

/**
 * The channel strip of an analog mixing desk, built into each mixer channel (but the master) while the mixer is in
 * analog mode (see Mixer::isAnalog()). In signal order, after the channel's gain (its trim):
 * - the low cut button (80 Hz, 24 dB/oct), before the gain stage so that it gets no rumble to saturate on;
 * - the gain stage: an op-amp, or with the TUBE button a 12AX7 tube stage (see processDriveStage());
 * - sweepable high-pass and low-pass filters (12 dB/oct), and mono bass (see MonoBass);
 * - a gate and a compressor, one knob each (their thresholds), with stereo-linked detection;
 * - a classic 4-band EQ with fixed frequencies: high shelf at 12 kHz, high mid peak at 2.5 kHz, low mid peak at
 *   400 Hz, low shelf at 80 Hz.
 * Each section left at its neutral setting is skipped, so a neutral strip leaves the audio untouched. The filters
 * are those of the Eq plugin (EqFilter).
 */
class LMMS_EXPORT AnalogChannelStrip
{
public:
	explicit AnalogChannelStrip(Model* parent);
	~AnalogChannelStrip();

	BoolModel m_tubeModel;			//!< the gain stage is a tube instead of an op-amp
	BoolModel m_lowCutModel;
	FloatModel m_highPassModel;		//!< Hz; off at the lowest setting
	FloatModel m_lowPassModel;		//!< Hz; off at the highest setting
	FloatModel m_monoBassWidthModel;		//!< % of the side kept below the mono bass frequency; off at 100
	FloatModel m_monoBassFrequencyModel;	//!< Hz
	FloatModel m_gateModel;			//!< threshold in dB; off at the lowest setting
	FloatModel m_compressorModel;	//!< threshold in dB; off at 0
	FloatModel m_eqHighModel;		//!< dB
	FloatModel m_eqHighMidModel;	//!< dB
	FloatModel m_eqLowMidModel;		//!< dB
	FloatModel m_eqLowModel;		//!< dB

	//! Processes the channel's main pair in place; @p hadInput tells whether it had signal [audio thread]
	void process(std::span<SampleFrame> frames, sample_rate_t sampleRate, bool hadInput);

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

	/**
	 * The op-amp's transfer function, after a 4558 (the op-amp of many Soundcraft and Mackie desks): linear up to the
	 * knee, then softly towards the rail. The headroom is that of a desk above its nominal level, so it is clean at
	 * any usual level and saturates when the gain is pushed. The negative rail is a little lower than the positive
	 * one, as in real op-amps, which adds even harmonics.
	 */
	static constexpr float OpAmpKnee = 8.f;		// +18 dBFS
	static constexpr float OpAmpRail = 16.f;	// +24 dBFS
	static constexpr float OpAmpNegativeRailRatio = 0.95f;
	static auto opAmp(float sample) -> float;

	/**
	 * The tube's transfer function: a 12AX7 preamp stage, from the SPICE simulation sampled by Tim Goetze for CAPS,
	 * with unity gain for small signals. It starts to clip at +12 dBFS, and colors a little below that.
	 */
	static auto tube(float sample) -> float;

private:
	/**
	 * The gain stage, at 4 times the sample rate so that its harmonics do not alias:
	 * - op-amp: only the part the op-amp changes (its output minus its input, which is 0 below the knee) goes through
	 *   the oversampling, then is added to the signal. Below the knee the signal is untouched, bit for bit. The
	 *   op-amp's slew rate (1.7 V/us, the 4558's) limits very loud, very high frequencies, as in the real chip. Blocks
	 *   far below the knee skip the oversampling.
	 * - tube: the whole signal goes through the tube, then a DC blocker (its asymmetry adds an offset).
	 */
	void processDriveStage(std::span<SampleFrame> frames, sample_rate_t sampleRate);

	//! The oversamplers and the state of the gain stage (kept out of the header, which the plugins see)
	struct DriveStage;
	std::unique_ptr<DriveStage> m_drive;

	// The filters of the Eq plugin, which crossfade changed settings over a period
	EqHp12Filter m_lowCut[2];	// 24 dB/oct
	EqHp12Filter m_highPass;
	EqLp12Filter m_lowPass;
	EqLowShelfFilter m_eqLow;
	EqPeakFilter m_eqHighMid;
	EqPeakFilter m_eqLowMid;
	EqHighShelfFilter m_eqHigh;
	MonoBass m_monoBass;

	//! Frames the filters may still ring for after the input fell silent
	f_cnt_t m_tailFrames = 0;

	// Dynamics
	float m_gateGain = 1.f;
	f_cnt_t m_gateHold = 0;
	float m_compressorEnvelope = 0.f;
};

} // namespace lmms

#endif // LMMS_ANALOG_CHANNEL_STRIP_H
