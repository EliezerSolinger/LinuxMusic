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
#include <numbers>

#include "EqFilter.h"
#include "OversamplingHelpers.h"
#include "SampleFrame.h"
#include "TubeStage.h"
#include "lmms_math.h"

namespace lmms
{

namespace
{

using Model = GainStage::Model;

constexpr int OversamplingStages = 2; // 4 times
constexpr int Oversampling = 1 << OversamplingStages;
//! Volts per unit of the signal: an op-amp's rail of +24 dBFS is a swing of 13 V on a +-15 V supply
constexpr float VoltsPerUnit = 13.f / 16.f;
constexpr float DcBlockerFrequency = 5.f;
//! The CLIP LED lights this close to the limit
constexpr float ClipWarning = 0.708f; // -3 dB
//! The SAT LED of a tube spans this range, around where it starts to clip
constexpr float TubeColorRange = 12.f; // dB

/**
 * The tubes' stages, in the common cathode stage of TubeCircuit. Koren's parameters (mu, ex, kg1, kp, kvb) and the
 * datasheets' capacitances (grid to plate, grid to cathode); usual plate and cathode resistors for each tube.
 * The 12AX7 V and H are the 12AX7 biased cold (a larger cathode resistor: it clips first towards cutoff, rich in
 * even harmonics) and hot on a higher supply (a smaller cathode resistor: both sides clip alike, stronger in odd
 * harmonics).
 */
auto tubeModel(Model model) -> const TubeModel&
{
	constexpr auto ax7 = Triode{100.0, 1.4, 1060.0, 600.0, 300.0, 1.7e-12, 1.6e-12};
	constexpr auto at7 = Triode{60.0, 1.35, 460.0, 300.0, 300.0, 1.5e-12, 2.2e-12};
	constexpr auto au7 = Triode{21.5, 1.3, 1180.0, 84.0, 300.0, 1.5e-12, 1.6e-12};
	static const auto s_ax7 = TubeModel{TubeCircuit{ax7, 250.0, 100e3, 1.5e3, 22e-6}};
	static const auto s_ax7Cold = TubeModel{TubeCircuit{ax7, 250.0, 100e3, 2.7e3, 22e-6}};
	static const auto s_ax7Hot = TubeModel{TubeCircuit{ax7, 330.0, 100e3, 1.0e3, 22e-6}};
	static const auto s_at7 = TubeModel{TubeCircuit{at7, 250.0, 47e3, 820.0, 22e-6}};
	static const auto s_au7 = TubeModel{TubeCircuit{au7, 250.0, 47e3, 1.5e3, 22e-6}};
	switch (model)
	{
		case Model::Tube12AX7Vintage: return s_ax7Cold;
		case Model::Tube12AX7Hot: return s_ax7Hot;
		case Model::Tube12AT7: return s_at7;
		case Model::Tube12AU7: return s_au7;
		default: return s_ax7;
	}
}

/**
 * Volts at the grid per unit of the signal with the GAIN pot in the middle: the 12AX7 starts to clip at 0 dBFS, so
 * it colors from -6 dBFS; the other tubes, at their own levels
 */
auto voltsPerUnit() -> float
{
	static const auto s_volts = static_cast<float>(tubeModel(Model::Tube12AX7).headroom);
	return s_volts;
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
	bool realistic = false;
	std::array<TubeStage, 2> tubes;

	void setUp(sample_rate_t rate)
	{
		sampleRate = rate;
		model = Model::Flat; // the tubes are set up when next used
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
		if (isTube(model)) { for (auto& tube : tubes) { tube.reset(); } }
		oversampling = false;
	}

	void use(Model next, bool nextRealistic)
	{
		if (next == model && nextRealistic == realistic) { return; }
		model = next;
		realistic = nextRealistic;
		if (isTube(model))
		{
			for (auto& tube : tubes) { tube.setUp(&tubeModel(model), static_cast<double>(sampleRate) * Oversampling); }
		}
		clear();
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
	// Unity gain for small signals, and the same polarity: a tube stage inverts
	const auto& stage = tubeModel(model);
	const float volts = voltsPerUnit();
	return stage.at(sample * volts).plate / static_cast<float>(stage.gain * volts);
}




auto GainStage::tubeClipLevel(Model model) -> float
{
	return static_cast<float>(tubeModel(model).headroom) / voltsPerUnit();
}




void GainStage::process(std::span<SampleFrame> frames, Model model, bool realistic, sample_rate_t sampleRate)
{
	auto& state = *m_state;
	if (state.sampleRate != sampleRate) { state.setUp(sampleRate); }
	state.use(model, realistic && isTube(model));

	float peak = 0.f;
	for (const auto& frame : frames) { peak = std::max({peak, std::abs(frame[0]), std::abs(frame[1])}); }
	measure(peak, model);
	if (model == Model::Flat) { return; }

	auto oversampled = std::array<float, Oversampling>{};
	if (isTube(model))
	{
		// In volts at the grid, and back to unity gain and the same polarity
		const auto& stage = tubeModel(model);
		const float volts = voltsPerUnit();
		const float back = 1.f / static_cast<float>(stage.gain * volts);
		for (auto& frame : frames)
		{
			for (ch_cnt_t ch = 0; ch < 2; ++ch)
			{
				auto& tube = state.tubes[ch];
				state.upsamplers[ch].processSample(oversampled.data(), frame[ch]);
				if (state.realistic)
				{
					// The circuit's coupling capacitors take the offset away
					for (auto& sample : oversampled) { sample = tube.processCircuit(sample * volts) * back; }
					frame[ch] = state.downsamplers[ch].processSample(oversampled.data());
				}
				else
				{
					for (auto& sample : oversampled) { sample = tube.processLight(sample * volts) * back; }
					frame[ch] = state.dcBlocker.update(state.downsamplers[ch].processSample(oversampled.data()), ch, 1.f);
				}
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
		// From 6 dB below where it starts to clip to 6 dB above, where it clips hard
		const float clip = tubeClipLevel(model);
		const float level = peak > 0.f ? ampToDbfs(peak / clip) : -TubeColorRange;
		saturation = std::clamp((level + TubeColorRange / 2.f) / TubeColorRange, 0.f, 1.f);
		limit = clip * 2.f;
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
