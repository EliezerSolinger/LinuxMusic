/*
 * MasterSection.h - the master section of the mixer: bus compressor, monitoring and meters
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

#ifndef LMMS_MASTER_SECTION_H
#define LMMS_MASTER_SECTION_H

#include <array>
#include <atomic>
#include <span>
#include <vector>

#include "AutomatableModel.h"
#include "ChannelMeter.h"
#include "EqFilter.h"
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
 * The master section of a recording studio's desk, in both of the mixer's modes. On the master, in signal order:
 * - the PRE faders, left and right: the mix's level into the master's processing (its effects, the stereo section
 *   and the bus compressor), before them;
 * - in analog mode, the master's gain stage, after the PRE faders, which drive it: the summing amplifier of an analog
 *   desk, of the same models as the channels' (see GainStage), with no gain pot of its own;
 * - the stereo section, in mid/side (M = (L+R)/2, S = (L-R)/2), before the bus compressor (see processStereo());
 * - a bus compressor, after the SSL G-series one (off by default);
 * - then the master's own fader, POST.
 * These are part of the mix and of the export. Besides:
 * - the monitoring: what is heard from the speakers, without changing the mix or the export: monitor level, DIM
 *   (-20 dB), CUT, MONO (the mid) and SIDE. These are listening states, so they are neither saved with the project
 *   nor undoable;
 * - what the meters show: the correlation of the master's left and right channels, the points of the goniometer,
 *   and the bus compressor's gain reduction. (The VU meters show the master's RMS level, see ChannelMeter.)
 */
class LMMS_EXPORT MasterSection
{
public:
	explicit MasterSection(Model* parent);

	/**
	 * The PRE faders, one per side, linear like the channels' faders. While linked (the default) they move
	 * together: moving one moves the other to the same level, and linking puts the right one where the left is.
	 */
	FloatModel m_preLeftModel;
	FloatModel m_preRightModel;
	BoolModel m_preLinkModel;

	//! The PRE faders, on the mix before the master's effects [audio thread]
	void processPreFaders(std::span<SampleFrame> frames, sample_rate_t sampleRate);

	//! The level after the PRE faders, which their meters show
	ChannelMeter m_preMeter;

	//! The gain stage's model, a GainStage::Model (analog mode)
	IntModel m_driveModel;
	BoolModel m_realisticModel;		//!< for a tube: its circuit solved at each sample

	//! The gain stage, after the PRE faders, in analog mode [audio thread]
	void processDrive(std::span<SampleFrame> frames, sample_rate_t sampleRate);
	//! Forgets the gain stage's state, out of analog mode [audio thread]
	void resetDrive() { m_gainStage.reset(); }
	//! The gain stage, which also measures its saturation for its LEDs
	auto gainStage() -> GainStage& { return m_gainStage; }

	//! Analog mode: the output transformer after the master's fader, Neve style (see OutputTransformer); off
	BoolModel m_transformerModel;
	void processTransformer(std::span<SampleFrame> frames, sample_rate_t sampleRate);
	//! While it is out: it starts afresh when it is in again
	void bypassTransformer() { m_transformerWasOn = false; }

	// Stereo section (the width is the master channel's own width, see processStereo())
	FloatModel m_monoBassModel;			//!< % mono below the mono bass frequency: 0 is off, 100 all mono
	FloatModel m_bassFrequencyModel;		//!< Hz
	FloatModel m_airModel;					//!< dB of the side's high shelf
	static constexpr float AirFrequency = 12000.f;

	// Bus compressor
	BoolModel m_compressorModel;			//!< in or out
	FloatModel m_thresholdModel;			//!< dB
	IntModel m_ratioModel;					//!< a Ratio
	FloatModel m_attackModel;				//!< ms
	FloatModel m_releaseModel;				//!< s
	BoolModel m_autoReleaseModel;
	FloatModel m_makeupModel;				//!< dB

	enum class Ratio { Two, Four, Ten };
	static auto ratio(Ratio ratio) -> float;

	// Monitoring
	FloatModel m_monitorLevelModel;			//!< dB
	BoolModel m_dimModel;
	BoolModel m_cutModel;
	BoolModel m_monoModel;					//!< listen to the mid (L+R)/2
	BoolModel m_sideModel;					//!< listen to the side (L-R)/2; MONO and SIDE exclude each other
	static constexpr float DimLevel = -20.f; // dB

	/**
	 * The stereo section, on the master after its effects [audio thread]:
	 * - WIDTH (@p widthPercent, the master channel's width): the side's level, ramped over the period;
	 * - MONO BASS: below its frequency, the side is narrowed to its width (0 = mono), see MonoBass;
	 * - AIR: a high shelf on the side only, opening the mix without brightening its center.
	 * Each part at its neutral setting is skipped.
	 */
	void processStereo(std::span<SampleFrame> frames, sample_rate_t sampleRate, float widthPercent);

	//! The bus compressor, on the master's output before its fader [audio thread]
	void processCompressor(std::span<SampleFrame> frames, sample_rate_t sampleRate);

	//! Measures the master's output for the meters [audio thread]
	void measure(std::span<const SampleFrame> frames, sample_rate_t sampleRate);

	//! The master is silent: nothing to measure [audio thread]
	void measureSilence();

	//! The monitoring, on what goes to the speakers (not when exporting) [audio thread]
	void processMonitoring(std::span<SampleFrame> frames);

	//! The bus compressor's deepest gain reduction since the last call, in dB (0 or less) [GUI thread]
	auto takeGainReduction() -> float { return m_gainReduction.exchange(0.f, std::memory_order_relaxed); }

	//! The correlation of the master's left and right channels over the last ~300 ms: 1 mono, 0 unrelated,
	//! -1 cancelling each other [any thread]
	auto correlation() const -> float { return m_correlation.load(std::memory_order_relaxed); }

	//! The latest points of the goniometer, (side, mid) of the master's output, oldest first [GUI thread]
	static constexpr int ScopePoints = 512;
	void scopePoints(std::vector<std::array<float, 2>>& points) const;

	//! Puts the bus compressor back to its defaults (off)
	void setDefaults();

	void saveSettings(QDomDocument& doc, QDomElement& mixerElement);
	void loadSettings(const QDomElement& mixerElement);

private:
	GainStage m_gainStage;
	OutputTransformer m_transformer;
	bool m_transformerWasOn = false;

	// Stereo section
	MonoBass m_monoBass;
	float m_width = 1.f;			//!< of the last frame, ramped
	EqHighShelfFilter m_air;
	bool m_airWasOn = false;

	// Bus compressor
	float m_reduction = 0.f;		//!< dB, smoothed
	float m_sustain = 0.f;			//!< dB of reduction averaged over a second, for the auto release
	std::atomic<float> m_gainReduction = 0.f;

	// Correlation
	float m_sumLR = 0.f, m_sumLL = 0.f, m_sumRR = 0.f;
	std::atomic<float> m_correlation = 0.f;

	// Goniometer: a ring of (side, mid) points
	std::array<std::atomic<float>, 2 * ScopePoints> m_scope{};
	std::atomic<int> m_scopeWrite = 0;
	int m_scopeDecimation = 0;
	bool m_scopeSilent = false;

	// Monitoring
	float m_monitorGain = 1.f;		//!< the gain of the last frame, ramped to avoid clicks

	// PRE faders: the gains of the last frame, ramped
	std::array<float, 2> m_preGain = {1.f, 1.f};
};

} // namespace lmms

#endif // LMMS_MASTER_SECTION_H
