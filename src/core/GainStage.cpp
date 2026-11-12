/*
 * GainStage.cpp - the gain stage of an analog desk: flat, an op-amp or a tube
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

#include "GainStage.h"

#include <algorithm>
#include <cmath>
#include <functional>
#include <numbers>

#include "EqFilter.h"
#include "OversamplingHelpers.h"
#include "SampleFrame.h"
#include "lmms_math.h"

namespace lmms
{

namespace
{

#include "r12ax7.h"

using Model = GainStage::Model;

constexpr int OversamplingStages = 2; // 4 times
constexpr int Oversampling = 1 << OversamplingStages;
//! Volts per unit of the signal: an op-amp's rail of +24 dBFS is a swing of 13 V on a +-15 V supply
constexpr float VoltsPerUnit = 13.f / 16.f;
constexpr float DcBlockerFrequency = 5.f;
//! The CLIP LED lights this close to the limit
constexpr float ClipWarning = 0.708f; // -3 dB
//! The SAT LED of a tube starts this far below where it clips
constexpr float TubeColorRange = 12.f; // dB

// The 12AX7 approximations of CAPS (dsp/TwelveAX7.h, by Tim Goetze, GPL): the stage's output voltage for input a,
// 1 being where it clips on the positive side

//! The transfer curve sampled from a SPICE simulation (TwelveAX7_3), interpolated
auto tubeSpice(float a) -> float
{
	constexpr auto zero = static_cast<float>(r12AX7::Zero);
	constexpr auto samples = static_cast<float>(r12AX7::Samples);
	a = zero + a * (samples - zero);
	if (a <= 0.f) { return r12AX7::v2v[0]; }
	if (a >= samples - 1.f) { return r12AX7::v2v[r12AX7::Samples - 1]; }
	const auto i = static_cast<int>(a);
	const float fraction = a - static_cast<float>(i);
	return r12AX7::v2v[i] * (1.f - fraction) + r12AX7::v2v[i + 1] * fraction;
}

//! A cubic polynomial clipped where its slope turns to 0 (TwelveAX7 and TwelveAX7_2)
struct TubePolynomial
{
	double b, c, d;
	double low, high; //!< where it clips
	auto operator()(double a) const -> float
	{
		a = std::clamp(a, low, high);
		return static_cast<float>(a * (b + a * (c + a * d)));
	}
};

//! TwelveAX7: CAPS' original model, "it took a good deal of fiddling to get the coefficients right"; it clips where
//! its slope turns to 0
auto vintagePolynomial() -> TubePolynomial
{
	constexpr double b = -0.79618574210627535, c = -0.21108555430962023, d = 0.38944033523200522;
	const double root = std::sqrt(4 * c * c - 12 * d * b);
	return {b, c, d, (-2 * c - root) / (6 * d), (-2 * c + root) / (6 * d)};
}

//! TwelveAX7_2: fitted to the curve, clipping at -0.52 and +0.98
constexpr auto hotPolynomial = TubePolynomial{-1.08150605597883, -0.262760944760536, 0.445770802765903, -0.52, 0.98};

/**
 * Norman Koren's triode model ("Improved VT models for SPICE simulations", 1996): the plate current for the grid and
 * plate voltages to the cathode, with his published parameters per tube.
 */
struct KorenTriode
{
	double mu, ex, kg1, kp, kvb;

	auto plateCurrent(double vgk, double vpk) const -> double
	{
		if (vpk <= 0.0) { return 0.0; }
		const double x = kp * (1.0 / mu + vgk / std::sqrt(kvb + vpk * vpk));
		const double softplus = x > 30.0 ? x : std::log1p(std::exp(x));
		const double e1 = vpk / kp * softplus;
		return e1 > 0.0 ? 2.0 * std::pow(e1, ex) / kg1 : 0.0;
	}
};

/**
 * A common cathode stage with a bypassed cathode resistor: its plate voltage for an input on the grid, sampled over
 * the input range in units of the input that cuts the tube off (a = -1), as a table.
 */
class TriodeStage
{
public:
	TriodeStage(KorenTriode tube, double supply, double plateResistor, double cathodeResistor)
		: m_tube{tube}, m_supply{supply}, m_plateResistor{plateResistor}
	{
		// The operating point: the cathode resistor biases the grid by the current through it
		double low = 0.0, high = supply / (plateResistor + cathodeResistor);
		for (int i = 0; i < 60; ++i)
		{
			const double current = (low + high) / 2.0;
			const double flows = tube.plateCurrent(-current * cathodeResistor,
				supply - current * (plateResistor + cathodeResistor));
			(current > flows ? high : low) = current;
		}
		m_cathode = low * cathodeResistor;
		const double idle = low;

		// The input cutting the tube off: where the current falls to 2% of the idle current
		double off = -100.0 * m_cathode, on = 0.0;
		for (int i = 0; i < 60; ++i)
		{
			const double input = (off + on) / 2.0;
			(current(input) < 0.02 * idle ? off : on) = input;
		}
		m_cutoff = -off;

		for (int i = 0; i < TableSize; ++i)
		{
			const double a = TableLow + (TableHigh - TableLow) * i / (TableSize - 1);
			m_table[i] = static_cast<float>(plate(a * m_cutoff));
		}
	}

	//! Where the grid starts to conduct, as an input a: the stage's first clipping point, before the cutoff (a = -1)
	auto gridConduction() const -> float { return static_cast<float>(m_cathode / m_cutoff); }

	auto operator()(float a) const -> float
	{
		const float position = (a - TableLow) / (TableHigh - TableLow) * (TableSize - 1);
		if (position <= 0.f) { return m_table.front(); }
		if (position >= TableSize - 1) { return m_table.back(); }
		const auto i = static_cast<int>(position);
		const float fraction = position - static_cast<float>(i);
		return m_table[i] * (1.f - fraction) + m_table[i + 1] * fraction;
	}

private:
	static constexpr int TableSize = 4097;
	static constexpr float TableLow = -2.f;
	static constexpr float TableHigh = 4.f;

	//! The grid voltage to the cathode for an input; above 0 V the grid conducts and holds it near 0.6 V
	auto gridToCathode(double input) const -> double
	{
		const double vgk = input - m_cathode;
		return vgk > 0.0 ? 0.6 * std::tanh(vgk / 0.6) : vgk;
	}

	//! The plate voltage for an input: where the plate resistor's drop matches the tube's current
	auto plate(double input) const -> double
	{
		const double vgk = gridToCathode(input);
		double low = m_cathode, high = m_supply;
		for (int i = 0; i < 60; ++i)
		{
			const double vp = (low + high) / 2.0;
			const double drop = m_plateResistor * m_tube.plateCurrent(vgk, vp - m_cathode);
			(vp < m_supply - drop ? low : high) = vp;
		}
		return (low + high) / 2.0;
	}

	auto current(double input) const -> double { return (m_supply - plate(input)) / m_plateResistor; }

	KorenTriode m_tube;
	double m_supply, m_plateResistor;
	double m_cathode = 0.0;	//!< the cathode's voltage, the grid's bias
	double m_cutoff = 1.0;
	std::array<float, TableSize> m_table{};
};

//! A tube model's curve, normalized: the input at which it starts to clip on the negative side, its offset and its
//! slope for small signals
struct TubeCurve
{
	std::function<float(float)> transfer;
	float scale;
	float offset;
	float slope;
};

auto makeTubeCurve(std::function<float(float)> transfer, float scale) -> TubeCurve
{
	const float offset = transfer(0.f);
	const float slope = (transfer(1e-3f) - transfer(-1e-3f)) / 2e-3f;
	return {std::move(transfer), scale, offset, slope};
}

auto tubeCurve(Model model) -> const TubeCurve&
{
	static const auto s_spice = makeTubeCurve(tubeSpice,
		static_cast<float>(r12AX7::Zero) / static_cast<float>(r12AX7::Samples - r12AX7::Zero));
	static const auto s_vintage = [] {
		const auto polynomial = vintagePolynomial();
		return makeTubeCurve([polynomial](float a) { return polynomial(a); }, static_cast<float>(-polynomial.low));
	}();
	static const auto s_hot = makeTubeCurve([](float a) { return hotPolynomial(a); }, static_cast<float>(-hotPolynomial.low));
	// Koren's parameters (mu, ex, kg1, kp, kvb), in usual stages: +250 V, plate and cathode resistors
	static const auto s_at7 = [] {
		const auto stage = std::make_shared<TriodeStage>(KorenTriode{60.0, 1.35, 460.0, 300.0, 300.0}, 250.0,
			47e3, 820.0);
		return makeTubeCurve([stage](float a) { return (*stage)(a); }, std::min(1.f, stage->gridConduction()));
	}();
	static const auto s_au7 = [] {
		const auto stage = std::make_shared<TriodeStage>(KorenTriode{21.5, 1.3, 1180.0, 84.0, 300.0}, 250.0,
			47e3, 1500.0);
		return makeTubeCurve([stage](float a) { return (*stage)(a); }, std::min(1.f, stage->gridConduction()));
	}();
	switch (model)
	{
		case Model::Tube12AX7Vintage: return s_vintage;
		case Model::Tube12AX7Hot: return s_hot;
		case Model::Tube12AT7: return s_at7;
		case Model::Tube12AU7: return s_au7;
		default: return s_spice;
	}
}

} // namespace




struct GainStage::State
{
	std::array<Upsampler<OversamplingStages>, 2> upsamplers;
	std::array<Downsampler<OversamplingStages>, 2> downsamplers;
	std::array<float, 2> slewed{}; //!< the op-amp's output, which the slew rate limits
	EqHp12Filter dcBlocker;
	sample_rate_t sampleRate = 0;
	bool oversampling = false;	//!< whether the oversamplers carry signal
	Model model = Model::Flat;	//!< what they last carried

	void setUp(sample_rate_t rate)
	{
		sampleRate = rate;
		for (auto& upsampler : upsamplers) { upsampler.setup(OversamplingStages, static_cast<float>(rate)); }
		for (auto& downsampler : downsamplers) { downsampler.setup(OversamplingStages, static_cast<float>(rate)); }
		dcBlocker.setParameters(static_cast<float>(rate), DcBlockerFrequency, std::numbers::sqrt2_v<float> / 2.f, 1.f);
		oversampling = false;
	}

	void clear()
	{
		for (auto& upsampler : upsamplers) { upsampler.reset(); }
		for (auto& downsampler : downsamplers) { downsampler.reset(); }
		dcBlocker.clearHistory();
		oversampling = false;
	}
};




GainStage::GainStage()
	: m_state{std::make_unique<State>()}
{
	// The tubes' curves are computed on first use; done here, on the main thread, rather than on the audio thread
	for (const auto model : DisplayOrder)
	{
		if (isTube(model)) { tube(model, 0.f); }
	}
}

GainStage::~GainStage() = default;




auto GainStage::isTube(Model model) -> bool
{
	return model == Model::Tube12AX7 || model == Model::Tube12AX7Vintage || model == Model::Tube12AX7Hot
		|| model == Model::Tube12AT7 || model == Model::Tube12AU7;
}




auto GainStage::isOpAmp(Model model) -> bool
{
	return model == Model::OpAmp4558 || model == Model::OpAmp5532 || model == Model::OpAmp5534
		|| model == Model::OpAmpTL072;
}




auto GainStage::opAmpSpec(Model model) -> OpAmpSpec
{
	switch (model)
	{
		case Model::OpAmp5532: return {11.22f, 16.f, 1.f, 9.f};		// knee at +21 dBFS
		case Model::OpAmp5534: return {11.22f, 16.f, 1.f, 13.f};
		case Model::OpAmpTL072: return {8.91f, 14.8f, 0.97f, 13.f};	// knee at +19 dBFS, rail at +23.4 dBFS
		default: return {8.f, 16.f, 0.95f, 1.7f};						// 4558: knee at +18 dBFS
	}
}




auto GainStage::opAmp(Model model, float sample) -> float
{
	const auto spec = opAmpSpec(model);
	const float scale = sample < 0.f ? spec.negativeRailRatio : 1.f;
	const float knee = spec.knee * scale;
	const float magnitude = std::abs(sample);
	if (magnitude <= knee) { return sample; }
	const float span = (spec.rail - spec.knee) * scale;
	return std::copysign(knee + span * std::tanh((magnitude - knee) / span), sample);
}




auto GainStage::tube(Model model, float sample) -> float
{
	// Unity gain for small signals, and the same polarity: a tube stage inverts, and sits on an offset
	const auto& curve = tubeCurve(model);
	const float input = curve.scale / TubeClipLevel;
	return (curve.transfer(sample * input) - curve.offset) / (curve.slope * input);
}




void GainStage::process(std::span<SampleFrame> frames, Model model, sample_rate_t sampleRate)
{
	auto& state = *m_state;
	if (state.sampleRate != sampleRate) { state.setUp(sampleRate); }
	if (model != state.model)
	{
		state.clear();
		state.model = model;
	}

	float peak = 0.f;
	for (const auto& frame : frames) { peak = std::max({peak, std::abs(frame[0]), std::abs(frame[1])}); }
	measure(peak, model);
	if (model == Model::Flat) { return; }

	auto oversampled = std::array<float, Oversampling>{};
	if (isTube(model))
	{
		for (auto& frame : frames)
		{
			for (ch_cnt_t ch = 0; ch < 2; ++ch)
			{
				state.upsamplers[ch].processSample(oversampled.data(), frame[ch]);
				for (auto& sample : oversampled) { sample = tube(model, sample); }
				frame[ch] = state.dcBlocker.update(state.downsamplers[ch].processSample(oversampled.data()), ch, 1.f);
			}
		}
		state.oversampling = true;
		return;
	}

	// Op-amp: far below the knee, nothing to do
	const auto spec = opAmpSpec(model);
	if (peak < spec.knee * spec.negativeRailRatio / 4.f)
	{
		if (state.oversampling) { state.clear(); }
		for (const auto& frame : frames) { state.slewed = {frame[0], frame[1]}; }
		return;
	}

	// The slew rate in units per oversampled frame
	const float maxStep = spec.slewRate * 1e6f / VoltsPerUnit / (static_cast<float>(sampleRate) * Oversampling);
	for (auto& frame : frames)
	{
		for (ch_cnt_t ch = 0; ch < 2; ++ch)
		{
			// What the op-amp changes, at the oversampled rate
			state.upsamplers[ch].processSample(oversampled.data(), frame[ch]);
			for (auto& sample : oversampled)
			{
				const float target = opAmp(model, sample);
				state.slewed[ch] += std::clamp(target - state.slewed[ch], -maxStep, maxStep);
				sample = state.slewed[ch] - sample;
			}
			frame[ch] += state.downsamplers[ch].processSample(oversampled.data());
		}
	}
	state.oversampling = true;
}




void GainStage::measure(float peak, Model model)
{
	float saturation = 0.f;
	float limit = FlatClipLevel;
	if (isOpAmp(model))
	{
		const auto spec = opAmpSpec(model);
		const float knee = spec.knee * spec.negativeRailRatio;
		const float rail = spec.rail * spec.negativeRailRatio;
		saturation = std::clamp((peak - knee) / (rail - knee), 0.f, 1.f);
		limit = rail;
	}
	else if (isTube(model))
	{
		const float level = peak > 0.f ? ampToDbfs(peak / TubeClipLevel) : -TubeColorRange;
		saturation = std::clamp((level + TubeColorRange) / TubeColorRange, 0.f, 1.f);
		limit = TubeClipLevel;
	}

	// The LEDs show the most since the GUI last looked
	auto shown = m_saturation.load(std::memory_order_relaxed);
	while (saturation > shown && !m_saturation.compare_exchange_weak(shown, saturation, std::memory_order_relaxed)) {}
	if (peak >= limit * ClipWarning) { m_clip.store(true, std::memory_order_relaxed); }
}




void GainStage::reset()
{
	m_state->clear();
	m_state->slewed = {};
}

} // namespace lmms
