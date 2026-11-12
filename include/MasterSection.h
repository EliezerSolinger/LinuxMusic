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

#include <atomic>
#include <span>

#include "AutomatableModel.h"
#include "LmmsTypes.h"

class QDomDocument;
class QDomElement;

namespace lmms
{

class SampleFrame;

/**
 * The master section of a recording studio's desk, in both of the mixer's modes:
 * - a bus compressor on the master, after the SSL G-series one, before the master fader (part of the mix and of
 *   the export; off by default);
 * - the monitoring: what is heard from the speakers, without changing the mix or the export: monitor level, DIM
 *   (-20 dB), CUT and MONO. These are listening states, so they are neither saved with the project nor undoable;
 * - what the meters show: the correlation of the master's left and right channels, and the bus compressor's gain
 *   reduction. (The VU meters show the master's RMS level, see ChannelMeter.)
 */
class LMMS_EXPORT MasterSection
{
public:
	explicit MasterSection(Model* parent);

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
	BoolModel m_monoModel;
	static constexpr float DimLevel = -20.f; // dB

	//! The bus compressor, on the master's output before its fader [audio thread]
	void processCompressor(std::span<SampleFrame> frames, sample_rate_t sampleRate);

	//! Measures the master's output for the meters [audio thread]
	void measure(std::span<const SampleFrame> frames, sample_rate_t sampleRate);

	//! The monitoring, on what goes to the speakers (not when exporting) [audio thread]
	void processMonitoring(std::span<SampleFrame> frames);

	//! The bus compressor's deepest gain reduction since the last call, in dB (0 or less) [GUI thread]
	auto takeGainReduction() -> float { return m_gainReduction.exchange(0.f, std::memory_order_relaxed); }

	//! The correlation of the master's left and right channels over the last ~300 ms: 1 mono, 0 unrelated,
	//! -1 cancelling each other [any thread]
	auto correlation() const -> float { return m_correlation.load(std::memory_order_relaxed); }

	//! Puts the bus compressor back to its defaults (off)
	void setDefaults();

	void saveSettings(QDomDocument& doc, QDomElement& mixerElement);
	void loadSettings(const QDomElement& mixerElement);

private:
	// Bus compressor
	float m_reduction = 0.f;		//!< dB, smoothed
	float m_sustain = 0.f;			//!< dB of reduction averaged over a second, for the auto release
	std::atomic<float> m_gainReduction = 0.f;

	// Correlation
	float m_sumLR = 0.f, m_sumLL = 0.f, m_sumRR = 0.f;
	std::atomic<float> m_correlation = 0.f;

	// Monitoring
	float m_monitorGain = 1.f;		//!< the gain of the last frame, ramped to avoid clicks
};

} // namespace lmms

#endif // LMMS_MASTER_SECTION_H
