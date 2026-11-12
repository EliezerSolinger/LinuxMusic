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
#include "EqFilter.h"
#include "LmmsTypes.h"

class QDomDocument;
class QDomElement;

namespace lmms
{

class SampleFrame;

/**
 * The channel strip of an analog mixing desk, built into each mixer channel (but the master) while the mixer is in
 * analog mode (see Mixer::isAnalog()). In signal order, after the channel's input gain (its trim):
 * - the gain stage, an op-amp that is linear up to +18 dBFS and saturates softly towards its rail at +24 dBFS;
 * - filters: a low cut button (80 Hz, 24 dB/oct) and sweepable high-pass and low-pass filters (12 dB/oct);
 * - a gate and a compressor, one knob each (their thresholds), with stereo-linked detection;
 * - a 3-band EQ: low shelf at 80 Hz, sweepable mid peak, high shelf at 12 kHz.
 * Each section left at its neutral setting is skipped, so a neutral strip leaves the audio untouched. The filters
 * are those of the Eq plugin (EqFilter).
 */
class LMMS_EXPORT AnalogChannelStrip
{
public:
	explicit AnalogChannelStrip(Model* parent);

	BoolModel m_lowCutModel;
	FloatModel m_highPassModel;		//!< Hz; off at the lowest setting
	FloatModel m_lowPassModel;		//!< Hz; off at the highest setting
	FloatModel m_gateModel;			//!< threshold in dB; off at the lowest setting
	FloatModel m_compressorModel;	//!< threshold in dB; off at 0
	FloatModel m_eqHighModel;		//!< dB
	FloatModel m_eqMidModel;		//!< dB
	FloatModel m_eqMidFrequencyModel;	//!< Hz
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

	//! The gain stage: linear up to the knee, then softly towards the rail
	//! Like the headroom of a desk above its nominal level: clean at any usual level, saturating when the gain is
	//! pushed
	static constexpr float SaturationKnee = 8.f;	// +18 dBFS
	static constexpr float SaturationRail = 16.f;	// +24 dBFS
	static auto saturate(float sample) -> float;

private:
	// The filters of the Eq plugin, which crossfade changed settings over a period
	EqHp12Filter m_lowCut[2];	// 24 dB/oct
	EqHp12Filter m_highPass;
	EqLp12Filter m_lowPass;
	EqLowShelfFilter m_eqLow;
	EqPeakFilter m_eqMid;
	EqHighShelfFilter m_eqHigh;

	//! Frames the filters may still ring for after the input fell silent
	f_cnt_t m_tailFrames = 0;

	// Dynamics
	float m_gateGain = 1.f;
	f_cnt_t m_gateHold = 0;
	float m_compressorEnvelope = 0.f;
};

} // namespace lmms

#endif // LMMS_ANALOG_CHANNEL_STRIP_H
