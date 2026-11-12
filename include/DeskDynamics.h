/*
 * DeskDynamics.h - the compressor and the gate/expander of an analog desk's channel
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

#include <array>
#include <memory>

#include <QString>

#include "LmmsTypes.h"
#include "lmms_export.h"

namespace lmms
{

/**
 * Levels in dBu, as on the desk, and in dBFS: 0 VU, the desk's nominal +4 dBu, is -18 dBFS RMS, the usual alignment
 * (see VuMeter)
 */
constexpr float DbuToDbfs = -22.f;

//! The compressors the analog strip models (see docs/pesquisa-compressores.md). Saved by number: new ones go last.
enum class CompressorModel
{
	Ssl, Api2500, Dbx160, Urei1176, La2a, Fairchild670, GatesStaLevel, Altec436, RcaBa6a, Count
};

//! The attack, within what the model allows
enum class CompressorSpeed { Fast, Norm, Slow, Count };

/**
 * The channel compressor, of a choice of models of real units, stereo-linked, with automatic make-up gain (a signal
 * at the nominal level comes out where it went in). The strip's controls are positions, which each model reads as
 * its unit does (docs/plano-compressores.md):
 * - amount (COMP), 0 to 1, how much it compresses: a threshold, or the input driving a fixed threshold;
 * - ratio (RATIO), 0 (out) to 1: a ratio, its steps, COMP/LIMIT, or the shape of a variable-mu curve;
 * - speed: FAST, NORM, SLOW, the attack within what the unit allows;
 * - release, 0 to 1: the release, or the time constant switch; 0.5 is the unit's factory setting.
 * The engines, from the research:
 * - feed-forward (SSL, dbx 160): an RMS detector, a gain computer with a knee, ballistics;
 * - feedback in the log domain (API 2500 in OLD mode, 1176): the detector reads the output; the loop is solved at
 *   each sample, so that ratio and knee come out of it as on the unit;
 * - variable-mu (Fairchild 670, Gates Sta-Level, Altec 436, RCA BA-6A): a rectifier charging the unit's RC network,
 *   whose control voltage sets the gain through the tube's law; the loop solved at each sample;
 * - optical (LA-2A): a cell with fast, slow and memory states, in the loop.
 * Then the gain element's distortion (oversampled), and the unit's frequency response and transformer.
 */
class LMMS_EXPORT ChannelCompressor
{
public:
	using Model = CompressorModel;
	using Speed = CompressorSpeed;

	struct Settings
	{
		Model model = Model::Ssl;
		float amount = 0.f;			//!< COMP, 0 to 1
		float ratio = 0.f;			//!< RATIO, 0 (out) to 1
		Speed speed = Speed::Norm;
		float release = 0.5f;		//!< 0 to 1; 0.5 is the unit's factory setting
	};

	//! The order the models are offered in: the VCAs, the FET, the optical, the variable-mu
	static constexpr auto DisplayOrder = std::array{Model::Ssl, Model::Api2500, Model::Dbx160, Model::Urei1176,
		Model::La2a, Model::Fairchild670, Model::GatesStaLevel, Model::Altec436, Model::RcaBa6a};

	enum class Family { Vca, Fet, Optical, VariMu };
	static auto family(Model model) -> Family;
	static auto name(Model model) -> QString;
	static auto description(Model model) -> QString;
	//! What the controls do on @p model, for their tooltips
	static auto amountName(Model model) -> QString;
	static auto ratioName(Model model) -> QString;
	static auto releaseName(Model model) -> QString;
	//! The controls' settings in the model's terms: "-12 dBu", "4:1", "ALL", "LIMIT", "TC 3 (2 s)"...
	static auto amountText(Model model, float amount) -> QString;
	static auto ratioText(Model model, float ratio) -> QString;
	static auto releaseText(Model model, float release) -> QString;
	static auto speedName(Speed speed) -> QString;
	//! Whether the model has a release to set (the dbx 160's comes out of its detector)
	static auto hasRelease(Model model) -> bool;

	// For projects from before the models, all SSL: the positions of a threshold in dBu, a ratio (1 to 20, 20 being
	// infinity) and a release in seconds
	static auto amountForSslThreshold(float dbu) -> float;
	static auto ratioForSslRatio(float ratio) -> float;
	static auto releaseForSslRelease(float seconds) -> float;

	ChannelCompressor();
	~ChannelCompressor();

	void begin(const Settings& settings, sample_rate_t sampleRate);
	auto isActive() const -> bool { return m_settings.ratio > 0.f; }
	//! Compresses a frame in place [audio thread]
	void process(float& left, float& right);
	void reset();
	//! The gain reduction now, in dB
	auto reduction() const -> float;
	//! The make-up gain, in dB
	auto makeup() const -> float;

	struct State;

private:
	Settings m_settings;
	std::unique_ptr<State> m_state;
};

//! The gate's operating modes: what has no knob (see docs/pesquisa-compressores.md, part 2). Saved by number.
enum class GateMode { Std, Kick, Snare, Toms, Vox, Guitar, Bass, Noise, Expander, Count };

/**
 * The gate/expander of an SSL E series channel, stereo-linked: THRESHOLD (-30 to +10 dBu) and RANGE (0, out, to
 * 40 dB) on knobs; the mode sets the rest:
 * - a 20:1 gate or a 2:1 expander;
 * - the attack, a slope in dB: 40 dB in 1.5 ms (normal) or 100 us (fast);
 * - hold, release (a time constant in dB, so closing further takes longer), hysteresis (the SSL's, growing as the
 *   threshold is lowered, or the mode's own);
 * - the key's filters, so that e.g. the kick's gate does not open on the hi-hat.
 */
class LMMS_EXPORT ChannelGate
{
public:
	struct Settings
	{
		float threshold = -10.f;	//!< dBu
		float range = 0.f;			//!< dB, 0 is out
		GateMode mode = GateMode::Std;
	};

	static constexpr auto DisplayOrder = std::array{GateMode::Std, GateMode::Kick, GateMode::Snare, GateMode::Toms,
		GateMode::Vox, GateMode::Guitar, GateMode::Bass, GateMode::Noise, GateMode::Expander};

	//! What a mode sets
	struct ModeSpec
	{
		bool expander;
		bool fast;
		float hold;			//!< s
		float release;		//!< s
		float hysteresis;	//!< dB; 0 for the SSL's, which depends on the threshold
		float keyHighPass;	//!< Hz, 0 for none
		float keyLowPass;	//!< Hz, 0 for none
	};
	static auto modeSpec(GateMode mode) -> ModeSpec;
	static auto modeName(GateMode mode) -> QString;
	static auto modeDescription(GateMode mode) -> QString;

	//! The SSL's hysteresis at a threshold in dBu
	static auto hysteresis(float threshold) -> float;

	ChannelGate();
	~ChannelGate();

	void begin(const Settings& settings, sample_rate_t sampleRate);
	auto isActive() const -> bool { return m_settings.range > 0.f; }
	//! The gain for a frame, linear [audio thread]
	auto gain(float left, float right) -> float;
	void reset();
	auto reduction() const -> float { return m_reduction; }

private:
	struct Key;

	Settings m_settings;
	ModeSpec m_spec{};
	std::unique_ptr<Key> m_key;
	float m_attackStep = 0.f;		//!< dB per frame
	float m_releaseCoefficient = 0.f;
	f_cnt_t m_holdFrames = 0;
	float m_keyRelease = 0.f;
	float m_envelope = 0.f;			//!< the key's peak envelope
	bool m_open = false;
	f_cnt_t m_holding = 0;
	float m_reduction = 0.f;		//!< dB
};

} // namespace lmms

#endif // LMMS_DESK_DYNAMICS_H
