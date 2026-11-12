/*
 * DeskDynamics.cpp - the compressor and the gate/expander of an analog desk's channel
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

#include "DeskDynamics.h"

#include <algorithm>
#include <cmath>
#include <limits>
#include <numbers>

#include <QCoreApplication>

#include "DeskFilters.h"
#include "DeskTolerance.h"
#include "OversamplingHelpers.h"
#include "lmms_math.h"

namespace lmms
{

namespace
{

using Model = CompressorModel;
using Speed = CompressorSpeed;

constexpr float Silence = -200.f;	//!< dB
constexpr float NominalRms = 4.f + DbuToDbfs;							//!< dBFS RMS, 0 VU
const float NominalPeak = dbfsToAmp(NominalRms) * std::numbers::sqrt2_v<float>;	//!< of a sine at 0 VU
constexpr float Ln10Over20 = 0.115129255f;

auto levelOf(float amplitude) -> float { return amplitude > 1e-10f ? ampToDbfs(amplitude) : Silence; }
auto tr(const char* text) -> QString { return QCoreApplication::translate("ChannelCompressor", text); }

//! A position 0 to 1 on a logarithmic scale through @p low, @p middle (at 0.5) and @p high
auto logScale(float position, float low, float middle, float high) -> float
{
	position = std::clamp(position, 0.f, 1.f);
	return position < 0.5f ? low * std::pow(middle / low, position * 2.f)
		: middle * std::pow(high / middle, (position - 0.5f) * 2.f);
}

auto logPosition(float value, float low, float middle, float high) -> float
{
	value = std::clamp(value, low, high);
	return value < middle ? std::log(value / low) / std::log(middle / low) / 2.f
		: 0.5f + std::log(value / middle) / std::log(high / middle) / 2.f;
}

//! A soft knee of width @p width around 0: 0 below, @p over above, quadratic between
auto knee(float over, float width) -> float
{
	if (width <= 0.f) { return std::max(over, 0.f); }
	if (over <= -width / 2.f) { return 0.f; }
	if (over >= width / 2.f) { return over; }
	const float into = over + width / 2.f;
	return into * into / (2.f * width);
}

// The steps of the units with stepped ratios
constexpr auto ApiRatios = std::array{1.5f, 2.f, 3.f, 4.f, 6.f, 10.f, std::numeric_limits<float>::infinity()};
constexpr auto UreiRatios = std::array{4.f, 8.f, 12.f, 20.f, 0.f}; // 0: all buttons in
//! The 1176's fixed thresholds per ratio, dBu (late UREI manual, +-2 dB)
constexpr auto UreiThresholds = std::array{-30.f, -26.f, -25.f, -24.f, -24.f};

template<std::size_t N>
auto step(float ratio, const std::array<float, N>& steps) -> std::size_t
{
	return std::min(static_cast<std::size_t>(std::clamp(ratio, 0.f, 1.f) * N), N - 1);
}

//! The Fairchild 670's time constant positions (1959 manual; RC values after Raffensperger, DAFx-12)
struct TimeConstant
{
	float attack;			//!< s
	float capacitor;		//!< uF, of the main capacitor
	float resistor;			//!< kOhm, its discharge
	float branches[2][2];	//!< series branches: kOhm, uF (0 for none)
	float documented;		//!< s, the release the manual gives
};
constexpr auto FairchildTimeConstants = std::array<TimeConstant, 6>{{
	{0.0002f, 2.f, 51.9f, {{0, 0}, {0, 0}}, 0.3f},
	{0.0002f, 2.f, 149.9f, {{0, 0}, {0, 0}}, 0.8f},
	{0.0004f, 4.f, 220.f, {{0, 0}, {0, 0}}, 2.f},
	{0.0004f, 8.f, 220.f, {{0, 0}, {0, 0}}, 5.f},
	{0.0004f, 4.f, 220.f, {{100.f, 8.f}, {0, 0}}, 2.f},
	{0.0002f, 2.f, 220.f, {{100.f, 8.f}, {100.f, 20.f}}, 0.3f},
}};

auto fairchildPosition(float release) -> int
{
	// 0.5 is position 3, the middle of the switch
	return std::clamp(static_cast<int>(std::floor(std::clamp(release, 0.f, 1.f) * 6.f - 0.5f)), 0, 5);
}

/**
 * How each model works. The engines and their parameters, from the research (docs/pesquisa-compressores.md); what
 * the research does not give is ours and marked there.
 */
enum class Engine { FeedForward, LogFeedback, VariMu, Optical };

struct Coloration
{
	float second, third;			//!< polynomial coefficients at no gain reduction, relative to the reference level
	float secondPer10, thirdPer10;	//!< added per 10 dB of gain reduction
	bool atThreshold;				//!< the reference level is the threshold (the tubes' operating level), else 0 VU
};

struct Path
{
	float highPass;		//!< Hz, second order, 0 for none
	float lowPass;		//!< Hz, first order, 0 for none
	float lowPassAt20;	//!< Hz, the low-pass at 20 dB of gain reduction (the RCA's response moves), 0 for fixed
	OutputTransformer::Core transformer;
	bool hasTransformer;
};

auto engineOf(Model model) -> Engine
{
	switch (model)
	{
		case Model::Ssl: case Model::Dbx160: return Engine::FeedForward;
		case Model::Api2500: case Model::Urei1176: return Engine::LogFeedback;
		case Model::La2a: return Engine::Optical;
		default: return Engine::VariMu;
	}
}

auto colorationOf(Model model) -> Coloration
{
	switch (model)
	{
		// The VCAs are clean (SSL: THD+N < 0.05% at +20 dBu); the API a little more, its discrete stages
		case Model::Ssl: return {0.f, 0.f, 0.f, 0.f, false};
		case Model::Api2500: return {0.0004f, 0.0002f, 0.f, 0.f, false};
		// The dbx 160's comes from its RMS detector's ripple modulating the VCA, by itself
		case Model::Dbx160: return {0.f, 0.f, 0.f, 0.f, false};
		// The 1176's FET: 0.07% at 1 kHz with ~8 dB of reduction (Moore); 2nd and 3rd, the 2nd growing with it
		case Model::Urei1176: return {0.00005f, 0.00005f, 0.0003f, 0.00028f, false};
		// The LA-2A's 12AX7 output stage: < 0.35% at +10 dBm, single ended, so the 2nd
		case Model::La2a: return {0.0025f, 0.0003f, 0.0004f, 0.0002f, false};
		// Push-pull variable-mu: the 3rd, growing with the reduction. The output sits at the nominal level (the
		// make-up), which the specifications' output levels are compared with
		case Model::Fairchild670: return {0.0001f, 0.0004f, 0.0001f, 0.0012f, false};	// < 1% at 10 dB, +12 dBm
		case Model::GatesStaLevel: return {0.0003f, 0.0003f, 0.0002f, 0.0005f, false};	// <= 1% to 30 dB, +10 dBm
		case Model::Altec436: return {0.001f, 0.0005f, 0.002f, 0.0006f, false};			// < 1.5% at 25 dB
		default: return {0.0003f, 0.0027f, 0.0002f, 0.0014f, false};					// RCA: < 0.6%, < 1% at 12 dB
	}
}

auto pathOf(Model model) -> Path
{
	// The units' responses, within their specifications: second order high-pass and first order low-pass placed for
	// the tolerance at the band's edges (ours)
	switch (model)
	{
		case Model::Urei1176: return {14.3f, 39000.f, 0.f, {2.0, 0.015, 60000.0}, true};		// 20 Hz-20 kHz +-1 dB
		case Model::La2a: return {0.f, 0.f, 0.f, {3.0, 0.012, 80000.0}, true};				// +-0.1 dB
		case Model::Fairchild670: return {28.5f, 29500.f, 0.f, {5.0, 0.006, 40000.0}, true};	// 40 Hz-15 kHz +-1 dB
		case Model::GatesStaLevel: return {21.4f, 29500.f, 0.f, {5.0, 0.007, 40000.0}, true};	// 30 Hz-15 kHz +-1 dB
		case Model::Altec436: return {24.f, 23400.f, 0.f, {6.0, 0.006, 35000.0}, true};		// 30 Hz-15 kHz +-1.5 dB
		case Model::RcaBa6a: return {21.4f, 29500.f, 19600.f, {6.0, 0.005, 30000.0}, true};	// -2 dB at 20 dB GR
		default: return {0.f, 0.f, 0.f, {}, false};
	}
}

//! A variable-mu tube's law: gain reduction in dB for a control voltage
struct TubeLaw
{
	float a, c0, b;		//!< GR = a ln(1 + v / c0) + b v
	auto operator()(float v) const -> float { return v > 0.f ? a * std::log1p(v / c0) + b * v : 0.f; }
};

} // namespace




// ----------------------------------------------------------------------------------------------------------------
// What the controls mean on each model

auto ChannelCompressor::family(Model model) -> Family
{
	switch (model)
	{
		case Model::Urei1176: return Family::Fet;
		case Model::La2a: return Family::Optical;
		case Model::Fairchild670: case Model::GatesStaLevel: case Model::Altec436: case Model::RcaBa6a:
			return Family::VariMu;
		default: return Family::Vca;
	}
}

auto ChannelCompressor::name(Model model) -> QString
{
	switch (model)
	{
		case Model::Ssl: return "SSL";
		case Model::Api2500: return "API";
		case Model::Dbx160: return "DBX160";
		case Model::Urei1176: return "1176";
		case Model::La2a: return "LA2A";
		case Model::Fairchild670: return "670";
		case Model::GatesStaLevel: return "GATES";
		case Model::Altec436: return "436";
		case Model::RcaBa6a: return "BA6A";
		default: return {};
	}
}

auto ChannelCompressor::description(Model model) -> QString
{
	switch (model)
	{
		case Model::Ssl: return tr("SSL E channel: VCA, RMS, soft knee; clean and precise");
		case Model::Api2500: return tr("API 2500: VCA in feedback (OLD), THRUST on the detector; punchy, keeps the lows");
		case Model::Dbx160: return tr("dbx 160: VCA, true RMS, hard knee; attack and release out of its detector");
		case Model::Urei1176: return tr("1176: FET in feedback, ultra fast; aggressive, ALL is all buttons in");
		case Model::La2a: return tr("LA-2A: optical cell; slow and smooth, two stage release with memory");
		case Model::Fairchild670: return tr("Fairchild 670: variable-mu tubes; the ratio grows with the level, time "
			"constant switch");
		case Model::GatesStaLevel: return tr("Gates Sta-Level: variable-mu, 1956; slow and dense, very long release");
		case Model::Altec436: return tr("Altec 436: variable-mu; the most colored");
		case Model::RcaBa6a: return tr("RCA BA-6A: variable-mu, 1950; about 10:1, single or dual time constants");
		default: return {};
	}
}

auto ChannelCompressor::amountName(Model model) -> QString
{
	switch (model)
	{
		case Model::Ssl: case Model::Api2500: case Model::Dbx160: return tr("Threshold");
		case Model::La2a: return tr("Peak reduction");
		default: return tr("Input, into the unit's threshold");
	}
}

auto ChannelCompressor::ratioName(Model model) -> QString
{
	switch (model)
	{
		case Model::Ssl: case Model::Dbx160: return tr("Ratio");
		case Model::Api2500: return tr("Ratio, in steps");
		case Model::Urei1176: return tr("Ratio buttons (ALL: all buttons in)");
		case Model::La2a: return tr("Compress or limit");
		case Model::Fairchild670: return tr("DC threshold: the shape of the curve");
		case Model::GatesStaLevel: return tr("Ratio (the unit's internal setting)");
		case Model::Altec436: return tr("Threshold, which raises the ratio from 2:1 to 4:1");
		default: return tr("Bias: the shape of the curve");
	}
}

auto ChannelCompressor::releaseName(Model model) -> QString
{
	switch (model)
	{
		case Model::Dbx160: return tr("None: the dbx 160's release comes out of its detector");
		case Model::La2a: return tr("Slow stage of the cell's release (the unit has no release control)");
		case Model::Fairchild670: return tr("Time constant switch");
		case Model::GatesStaLevel: return tr("Recovery (the unit's internal setting)");
		case Model::RcaBa6a: return tr("Release of the fast path (the unit has no release control)");
		default: return tr("Release");
	}
}

namespace
{

//! The threshold an amount means, in dBu: from the unit's top (not compressing) down
auto thresholdFor(Model model, float amount)
{
	amount = std::clamp(amount, 0.f, 1.f);
	switch (model)
	{
		case Model::Ssl: return 4.f - 30.f * amount;		// +4 to -26
		case Model::Api2500: return 10.f - 30.f * amount;	// +10 to -20
		case Model::Dbx160: return 12.f - 50.f * amount;	// +12 to -38
		default: return 10.f - 40.f * amount;				// the input driving the unit: ours
	}
}

auto speedIndex(Speed speed) { return static_cast<int>(speed); }

//! Seconds of release a position means
auto releaseFor(Model model, float release)
{
	switch (model)
	{
		case Model::Ssl: return logScale(release, 0.1f, 0.3f, 4.f);
		case Model::Api2500: return logScale(release, 0.05f, 0.3f, 3.f);
		case Model::Urei1176: return logScale(release, 0.05f, 0.25f, 1.1f);
		case Model::La2a: return logScale(release, 0.3f, 1.f, 3.f);		// a factor on the slow stage
		case Model::Fairchild670: return FairchildTimeConstants[fairchildPosition(release)].documented;
		case Model::GatesStaLevel: return logScale(release, 0.32f, 1.f, 1.6f);	// a factor on the factory recovery
		case Model::Altec436: return logScale(release, 0.3f, 0.7f, 1.3f);
		case Model::RcaBa6a: return logScale(release, 0.15f, 0.33f, 0.7f);
		default: return 0.f;
	}
}

} // namespace

auto ChannelCompressor::amountText(Model model, float amount) -> QString
{
	switch (model)
	{
		case Model::Ssl: case Model::Api2500: case Model::Dbx160:
			return QString{"%1 dBu"}.arg(thresholdFor(model, amount), 0, 'f', 1);
		case Model::La2a: return QString{"PR %1"}.arg(std::round(amount * 100.f));
		default: return QString{"IN +%1 dB"}.arg(40.f * std::clamp(amount, 0.f, 1.f), 0, 'f', 1);
	}
}

auto ChannelCompressor::ratioText(Model model, float ratio) -> QString
{
	if (ratio <= 0.f) { return tr("OFF"); }
	auto ratioString = [](float r) {
		return std::isinf(r) ? QString{"%1:1"}.arg(QChar{0x221E}) : QString{"%1:1"}.arg(r, 0, 'g', 3);
	};
	switch (model)
	{
		case Model::Ssl: case Model::Dbx160:
		{
			const float r = std::pow(20.f, ratio);
			return r >= 19.99f ? ratioString(std::numeric_limits<float>::infinity()) : ratioString(r);
		}
		case Model::Api2500: return ratioString(ApiRatios[step(ratio, ApiRatios)]);
		case Model::Urei1176:
		{
			const auto r = UreiRatios[step(ratio, UreiRatios)];
			return r == 0.f ? tr("ALL") : ratioString(r);
		}
		case Model::La2a: return ratio < 0.5f ? tr("COMP") : tr("LIMIT");
		case Model::GatesStaLevel: return ratioString(1.6f + 4.4f * ratio);
		case Model::Altec436: return QString{"+%1 dBm, %2"}.arg(16.f * ratio, 0, 'f', 0).arg(ratioString(2.f + 2.f * ratio));
		default: return QString{"%1%"}.arg(std::round(ratio * 100.f));
	}
}

auto ChannelCompressor::releaseText(Model model, float release) -> QString
{
	switch (model)
	{
		case Model::Dbx160: return tr("auto");
		case Model::La2a: case Model::GatesStaLevel: return QString{"x%1"}.arg(releaseFor(model, release), 0, 'f', 2);
		case Model::Fairchild670:
		{
			const int position = fairchildPosition(release);
			const auto& tc = FairchildTimeConstants[position];
			return position < 4 ? QString{"TC %1 (%2 s)"}.arg(position + 1).arg(tc.documented)
				: QString{"TC %1 (auto)"}.arg(position + 1);
		}
		default:
		{
			const float seconds = releaseFor(model, release);
			return seconds < 1.f ? QString{"%1 ms"}.arg(std::round(seconds * 1000.f))
				: QString{"%1 s"}.arg(seconds, 0, 'f', 2);
		}
	}
}

auto ChannelCompressor::speedName(Speed speed) -> QString
{
	switch (speed)
	{
		case Speed::Fast: return tr("FAST");
		case Speed::Slow: return tr("SLOW");
		default: return tr("NORM");
	}
}

auto ChannelCompressor::hasRelease(Model model) -> bool { return model != Model::Dbx160; }

auto ChannelCompressor::amountForSslThreshold(float dbu) -> float { return std::clamp((4.f - dbu) / 30.f, 0.f, 1.f); }

auto ChannelCompressor::ratioForSslRatio(float ratio) -> float
{
	return ratio <= 1.f ? 0.f : std::clamp(std::log(ratio) / std::log(20.f), 0.f, 1.f);
}

auto ChannelCompressor::releaseForSslRelease(float seconds) -> float
{
	return logPosition(seconds, 0.1f, 0.3f, 4.f);
}




// ----------------------------------------------------------------------------------------------------------------
// The engines

struct ChannelCompressor::State
{
	// What begin() works out
	Engine engine = Engine::FeedForward;
	float threshold = 0.f;		//!< dBFS: RMS for the RMS detectors, peak for the others
	float slope = 0.f;			//!< 1 - 1 / ratio (feed-forward) or the loop gain k = ratio - 1 (feedback)
	float kneeWidth = 0.f;
	float makeup = 0.f;			//!< dB
	float rmsCoefficient = 0.f;
	float attackStep = 0.f;		//!< dB per frame (slope attacks)
	float attackCoefficient = 0.f;
	float releaseCoefficient = 0.f;
	float fastReleaseCoefficient = 0.f;
	float chargeCoefficient = 0.f;
	bool allButtons = false;
	bool thrust = false;
	bool dualRelease = false;	//!< the SSL's fixed fast part

	// Variable-mu: the rectifier, the RC network and the tube's law
	TubeLaw law{};
	float sidechain = 1.f;		//!< V per unit of the output's amplitude
	float bias = 0.f;			//!< V the rectifier needs before it conducts
	float period = 0.f;
	struct Network
	{
		float attack, release;				//!< s: charging through the rectifier, discharging
		float branchTau[2], branchRatio[2];	//!< series branches: R C of the branch, their capacitor over the main one
		int branches;
	};
	std::array<Network, 2> networks{};		//!< two for the RCA's dual mode
	int networkCount = 1;

	// Optical: the cell
	float cellGain = 1.f, cellOffset = 0.f, cellExponent = 1.f;
	float cellEnvelope = 0.f;	//!< the panel's drive, averaged (the panel and the cell integrate)
	float envelopeCoefficient = 0.f;
	float cellAttack = 0.f, cellFall = 0.f, cellCharge = 0.f, cellSlowFall = 0.f, cellMemoryRise = 0.f,
		cellMemoryFall = 0.f;
	float limitFeedForward = 0.f;
	float thrustNormal = 1.f;	//!< the THRUST filter's gain at 1 kHz, divided out

	Coloration coloration{};
	Path path{};
	float reference = 1.f;		//!< amplitude of the coloration's reference level
	bool colored = false;

	// Running state
	float meanSquare = 0.f;
	float reduction = 0.f;		//!< dB
	float slowReduction = 0.f;	//!< dB, the SSL's slow release part
	std::array<std::array<float, 3>, 2> voltages{};	//!< per network: main node, branches
	float cellFast = 0.f, cellSlow = 0.f, cellMemory = 0.f;
	std::array<float, 2> dcSecond{};
	std::array<Upsampler<2>, 2> upsamplers;
	std::array<Downsampler<2>, 2> downsamplers;
	RampedBiquad thrustFilter;
	RampedBiquad thrustFilter2;
	RampedBiquad highPass, lowPass;
	std::unique_ptr<OutputTransformer> transformer;
	sample_rate_t sampleRate = 0;
	Model model = Model::Count;
};




ChannelCompressor::ChannelCompressor()
	: m_state{std::make_unique<State>()}
{
}

ChannelCompressor::~ChannelCompressor() = default;




namespace
{

auto coefficient(float seconds, float rate) -> float
{
	return seconds > 0.f ? 1.f - std::exp(-1.f / (seconds * rate)) : 1.f;
}

//! The variable-mu units: their tube laws and sidechains, calibrated so that the ratio the loop gives is the
//! unit's (asymptotically 1 + a ln10 / 20 for a ln law; the linear term makes the Fairchild's grow)
void setUpVariMu(ChannelCompressor::State& s, Model model, const ChannelCompressor::Settings& settings, float rate)
{
	const float ratio = std::clamp(settings.ratio, 0.f, 1.f);
	const int speed = speedIndex(settings.speed);
	float threshold = s.threshold;	// dBFS of the peak where the rectifier starts to conduct
	float bias = 0.f;				// V; by default where the law's knee falls
	s.networkCount = 1;
	auto& net = s.networks[0];
	net.branches = 0;
	switch (model)
	{
		case Model::Fairchild670:
		{
			// The DC threshold shapes the curve: from about 2:1 to about 30:1 (manual); the linear term makes the
			// ratio grow with the level
			// The 6386 is a remote cutoff tube: its gain falls exponentially with the bias, so the reduction in dB
			// grows linearly with the control voltage. In the loop the slope is then ln10/20 (GR + beta), beta being
			// the rectifier's bias: the ratio grows with the reduction, and the DC threshold sets where it starts
			// (from about 1.6:1 to about 7:1, growing to 20:1 and more when pushed)
			s.law = {0.f, 1.f, 1.f};
			bias = 5.f + 50.f * ratio;
			const auto& tc = FairchildTimeConstants[fairchildPosition(settings.release)];
			const float attack = speed == 0 ? 0.0002f : speed == 2 ? 0.0008f : tc.attack;
			const float c = tc.capacitor;
			net.attack = attack;
			// The documented release is ~2.3 to 2.9 times R C (der): the law's slope near the operating point
			net.release = tc.resistor * c * 1e-3f;
			for (int b = 0; b < 2; ++b)
			{
				if (tc.branches[b][0] <= 0.f) { continue; }
				net.branchTau[net.branches] = tc.branches[b][0] * tc.branches[b][1] * 1e-3f;
				net.branchRatio[net.branches] = tc.branches[b][1] / c;
				++net.branches;
			}
			break;
		}
		case Model::GatesStaLevel:
		{
			// The manual's table of bias against reduction: GR = 16.6 ln(1 + V / 8.1); the ratio resistor scales it
			// (1.6:1 to 6:1, 3.3:1 from the factory)
			const float target = 1.6f + 4.4f * ratio;
			s.law = {(target - 1.f) / Ln10Over20, 8.1f, 0.f};
			const float factor = releaseFor(model, settings.release);
			if (speed == 0)
			{
				// Double: 25 ms, a fast recovery (2.35 s from the factory) and a slow one after sustained peaks
				net.attack = 0.025f;
				net.release = 2.35f / 2.5f * factor;
				net.branchTau[0] = 3.1f * factor;
				net.branchRatio[0] = 2.f;
				net.branches = 1;
			}
			else
			{
				// Single: 75 ms (150 ms slow, ours), 7.05 s to 63%
				net.attack = speed == 2 ? 0.15f : 0.075f;
				net.release = 7.05f / 2.5f * factor;
			}
			break;
		}
		case Model::Altec436:
		{
			// The threshold, 0 to +16 dBm, raises the ratio from 2:1 to 4:1
			const float target = 2.f + 2.f * ratio;
			s.law = {(target - 1.f) / Ln10Over20, 6.f, 0.f};
			threshold += 16.f * ratio;
			net.attack = speed == 0 ? 0.025f : speed == 2 ? 0.1f : 0.05f;
			net.release = releaseFor(model, settings.release) / 2.5f;
			break;
		}
		default:
		{
			// RCA BA-6A: about 10:1 with a hard knee; the bias shapes it from 4:1
			const float target = 4.f + 6.f * ratio;
			s.law = {(target - 1.f) / Ln10Over20, 1.5f, 0.f};
			const float fastRelease = releaseFor(model, settings.release) / 2.5f;
			// Single: the fast path; dual: both; slow: the slow path only
			if (speed == 2) { net.attack = 0.9f; net.release = 2.f / 2.5f; }
			else { net.attack = 0.0006f; net.release = fastRelease; }
			if (speed == 1)
			{
				s.networks[1] = {0.9f, 2.f / 2.5f, {}, {}, 0};
				s.networkCount = 2;
			}
			break;
		}
	}
	// The rectifier's bias: conduction from the threshold up; the sidechain scaled so that the law's knee falls there
	s.bias = bias > 0.f ? bias : s.law.c0 * 2.f;
	s.sidechain = s.bias / dbfsToAmp(threshold);
	s.period = 1.f / rate;
}

} // namespace




void ChannelCompressor::begin(const Settings& settings, sample_rate_t sampleRate)
{
	auto& s = *m_state;
	const bool changed = settings.model != m_settings.model || settings.amount != m_settings.amount
		|| settings.ratio != m_settings.ratio || settings.speed != m_settings.speed
		|| settings.release != m_settings.release || sampleRate != s.sampleRate;
	m_settings = settings;
	if (!changed && s.model == settings.model) { return; }
	const auto rate = static_cast<float>(sampleRate);
	const auto model = settings.model;
	const int speed = speedIndex(settings.speed);

	if (model != s.model || sampleRate != s.sampleRate)
	{
		s.model = model;
		s.sampleRate = sampleRate;
		s.engine = engineOf(model);
		s.coloration = colorationOf(model);
		s.path = pathOf(model);
		s.colored = s.coloration.second != 0.f || s.coloration.third != 0.f || s.coloration.secondPer10 != 0.f
			|| s.coloration.thirdPer10 != 0.f;
		for (auto& u : s.upsamplers) { u.setup(2, rate); }
		for (auto& d : s.downsamplers) { d.setup(2, rate); }
		if (s.path.highPass > 0.f) { s.highPass.setTarget(Biquad::highPass(s.path.highPass, std::numbers::sqrt2 / 2.0, rate)); }
		if (s.path.lowPass > 0.f) { s.lowPass.setTarget(Biquad::lowPass1(s.path.lowPass, rate)); }
		s.highPass.finish();
		s.lowPass.finish();
		s.transformer = s.path.hasTransformer ? std::make_unique<OutputTransformer>(s.path.transformer) : nullptr;
		if (s.transformer) { s.transformer->setUp(sampleRate); }
		// API's THRUST (LOUD): +10 dB per decade on the detector, as two first order shelves (ours), 0 dB at 1 kHz
		s.thrust = model == Model::Api2500;
		s.thrustFilter.setTarget(Biquad::highShelf1(100.0, dbfsToAmp(15.f), rate));
		s.thrustFilter2.setTarget(Biquad::highShelf1(2000.0, dbfsToAmp(15.f), rate));
		s.thrustFilter.finish();
		s.thrustFilter2.finish();
		s.thrustNormal = static_cast<float>(s.thrustFilter.target().magnitude(1000.0, rate)
			* s.thrustFilter2.target().magnitude(1000.0, rate));
		reset();
	}

	s.threshold = thresholdFor(model, settings.amount) + DbuToDbfs;
	s.allButtons = false;
	s.dualRelease = false;
	const float ratio = std::clamp(settings.ratio, 0.f, 1.f);

	switch (s.engine)
	{
		case Engine::FeedForward:
		{
			if (model == Model::Ssl)
			{
				const float r = std::pow(20.f, ratio);
				s.slope = r >= 19.99f ? 1.f : 1.f - 1.f / r;
				s.kneeWidth = 10.f;
				s.rmsCoefficient = coefficient(0.005f, rate);
				// 20 dB in 3 / 30 / 60 ms; the release's fixed fast part (40 ms, ours) and the knob's slow one
				s.attackStep = 20.f / (std::array{0.003f, 0.03f, 0.06f}[speed] * rate);
				s.releaseCoefficient = coefficient(releaseFor(model, settings.release), rate);
				s.fastReleaseCoefficient = coefficient(0.04f, rate);
				s.chargeCoefficient = coefficient(0.15f, rate);
				s.dualRelease = true;
			}
			else
			{
				// dbx 160: one true RMS detector (36 ms gives the manual's attacks and 120 dB/s), a hard knee,
				// infinity at about 120:1
				const float r = std::pow(20.f, ratio);
				s.slope = r >= 19.99f ? 1.f - 1.f / 120.f : 1.f - 1.f / r;
				s.kneeWidth = 0.f;
				s.rmsCoefficient = coefficient(std::array{0.018f, 0.036f, 0.072f}[speed], rate);
			}
			s.makeup = knee(NominalRms - s.threshold, s.kneeWidth) * s.slope;
			break;
		}
		case Engine::LogFeedback:
		{
			float r;
			if (model == Model::Api2500)
			{
				r = ApiRatios[step(ratio, ApiRatios)];
				s.kneeWidth = 6.f;	// MED (ours)
				s.rmsCoefficient = coefficient(0.01f, rate);
				s.attackCoefficient = coefficient(std::array{0.0003f, 0.003f, 0.03f}[speed], rate);
				s.slope = std::isinf(r) ? 50.f : r - 1.f;	// feedback cannot reach infinity
			}
			else
			{
				const auto index = step(ratio, UreiRatios);
				r = UreiRatios[index];
				s.allButtons = r == 0.f;
				// The fixed threshold of the ratio, moved by the input (ours: COMP is the input)
				s.threshold = thresholdFor(model, settings.amount) + (UreiThresholds[index] + 30.f) + DbuToDbfs;
				s.kneeWidth = s.allButtons ? 1.f : std::array{6.f, 4.f, 3.f, 2.f}[index];
				s.slope = s.allButtons ? 30.f : r - 1.f;
				// The attack resistor: 20 / 200 / 800 us; all buttons in, the attack lags on the transients
				const float attack = std::array{0.00002f, 0.0002f, 0.0008f}[speed];
				s.attackCoefficient = coefficient(s.allButtons ? attack * 3.f : attack, rate);
			}
			s.releaseCoefficient = coefficient(releaseFor(model, settings.release), rate);
			// Make-up: the reduction a signal at the nominal level settles at, solving g = k knee(L - g - T)
			const float level = model == Model::Api2500 ? NominalRms : levelOf(NominalPeak);
			float low = 0.f, high = 120.f;
			for (int i = 0; i < 60; ++i)
			{
				const float g = (low + high) / 2.f;
				(s.slope * knee(level - g - s.threshold, s.kneeWidth) > g ? low : high) = g;
			}
			s.makeup = (low + high) / 2.f;
			break;
		}
		case Engine::VariMu:
		{
			setUpVariMu(s, model, settings, rate);
			// Make-up: the reduction of a sine at the nominal level, its peak charging the network
			float low = 0.f, high = 120.f;
			for (int i = 0; i < 60; ++i)
			{
				const float g = (low + high) / 2.f;
				const float voltage = std::max(s.sidechain * NominalPeak * dbfsToAmp(-g) - s.bias, 0.f);
				(s.law(voltage) > g ? low : high) = g;
			}
			s.makeup = (low + high) / 2.f;
			break;
		}
		case Engine::Optical:
		{
			// The cell: light from the panel, which grows faster than the drive (EL panels), and a photoresistor
			// dividing the signal, GR = 20 log(1 + L^gamma); COMP ~4:1, LIMIT ~infinity (UA), as the loop's slope
			const bool limit = ratio >= 0.5f;
			s.cellExponent = limit ? 14.f : 4.3f;
			s.cellGain = 1.f / dbfsToAmp(s.threshold);
			s.cellOffset = 0.5f;
			// (A partly feed-forward LIMIT is forum lore, and it runs away: LIMIT comes out of the loop's slope)
			s.limitFeedForward = 0.f;
			s.envelopeCoefficient = coefficient(0.002f, rate);
			s.cellAttack = coefficient(std::array{0.005f, 0.01f, 0.02f}[speed], rate);
			s.cellFall = coefficient(0.05f, rate);			// half the reduction gone in about 60 ms
			s.cellCharge = coefficient(2.f, rate);			// the slow state builds up while it is lit
			s.cellSlowFall = coefficient(1.5f * releaseFor(model, settings.release), rate);
			s.cellMemoryRise = coefficient(5.f, rate);
			s.cellMemoryFall = coefficient(30.f, rate);
			float low = 0.f, high = 120.f;
			for (int i = 0; i < 60; ++i)
			{
				const float g = (low + high) / 2.f;
				// The average of a rectified sine is 2 / pi of its peak
				const float drive = std::max(s.cellGain * NominalPeak * 2.f / std::numbers::pi_v<float>
					* (dbfsToAmp(-g) + s.limitFeedForward) - s.cellOffset, 0.f);
				const float light = std::pow(drive, s.cellExponent * 0.7f);
				(20.f * std::log10(1.f + light) > g ? low : high) = g;
			}
			s.makeup = (low + high) / 2.f;
			break;
		}
	}
	s.reference = s.coloration.atThreshold ? dbfsToAmp(s.threshold) : NominalPeak;
}




namespace
{

//! Solves h(x) = 0 for a decreasing h on [low, high] (regula falsi, Illinois)
template<class F>
auto solve(F h, float low, float high) -> float
{
	float hLow = h(low);
	if (hLow <= 0.f) { return low; }
	float hHigh = h(high);
	if (hHigh >= 0.f) { return high; }
	int side = 0;
	float x = low;
	for (int i = 0; i < 30; ++i)
	{
		x = (low * hHigh - high * hLow) / (hHigh - hLow);
		const float hx = h(x);
		if (std::abs(hx) < 1e-5f || high - low < 1e-6f) { break; }
		if (hx > 0.f)
		{
			low = x;
			hLow = hx;
			if (side == 1) { hHigh /= 2.f; }
			side = 1;
		}
		else
		{
			high = x;
			hHigh = hx;
			if (side == -1) { hLow /= 2.f; }
			side = -1;
		}
	}
	return x;
}

} // namespace




void ChannelCompressor::process(float& left, float& right)
{
	auto& s = *m_state;
	float gain = 1.f;
	switch (s.engine)
	{
		case Engine::FeedForward:
		{
			s.meanSquare += (std::max(left * left, right * right) - s.meanSquare) * s.rmsCoefficient;
			const float target = knee(levelOf(std::sqrt(s.meanSquare)) - s.threshold, s.kneeWidth) * s.slope;
			if (s.dualRelease)
			{
				// SSL: a slope attack; on release a fast part, and the slow part, which builds up while it compresses
				if (target > s.reduction) { s.reduction = std::min(target, s.reduction + s.attackStep); }
				else { s.reduction += (target - s.reduction) * s.fastReleaseCoefficient; }
				if (s.reduction > s.slowReduction)
				{
					s.slowReduction += (s.reduction - s.slowReduction) * s.chargeCoefficient;
				}
				else { s.slowReduction += (target - s.slowReduction) * s.releaseCoefficient; }
				s.slowReduction = std::max(s.slowReduction, 0.f);
				gain = dbfsToAmp(s.makeup - std::max(s.reduction, s.slowReduction));
			}
			else
			{
				// dbx 160: the detector is the ballistics
				s.reduction = target;
				gain = dbfsToAmp(s.makeup - s.reduction);
			}
			break;
		}
		case Engine::LogFeedback:
		{
			// The detector reads the output: solve for the reduction that the reduction leads to
			float keyLeft = left, keyRight = right;
			if (s.thrust)
			{
				keyLeft = static_cast<float>(s.thrustFilter2.process(s.thrustFilter.process(left, 0, 1.0), 0, 1.0))
					/ s.thrustNormal;
				keyRight = static_cast<float>(s.thrustFilter2.process(s.thrustFilter.process(right, 1, 1.0), 1, 1.0))
					/ s.thrustNormal;
			}
			const float keyPeak = std::max(std::abs(keyLeft), std::abs(keyRight));
			const float keySquare = std::max(keyLeft * keyLeft, keyRight * keyRight);
			const bool rms = s.rmsCoefficient > 0.f && s.model == Model::Api2500;
			auto next = [&s, keyPeak, keySquare, rms](float g, float* meanSquare) {
				const float through = dbfsToAmp(-g);
				float level;
				if (rms)
				{
					const float ms = s.meanSquare + (keySquare * through * through - s.meanSquare) * s.rmsCoefficient;
					if (meanSquare) { *meanSquare = ms; }
					level = levelOf(std::sqrt(ms));
				}
				else { level = levelOf(keyPeak * through); }
				const float target = s.slope * knee(level - s.threshold, s.kneeWidth);
				const float coeff = target > s.reduction ? s.attackCoefficient : s.releaseCoefficient;
				return s.reduction + (target - s.reduction) * coeff;
			};
			const float start = next(s.reduction, nullptr);
			const float g = solve([&next](float x) { return next(x, nullptr) - x; }, 0.f,
				std::max({start, s.reduction, 0.f}) + 60.f);
			float meanSquare = s.meanSquare;
			s.reduction = next(g, &meanSquare);
			s.meanSquare = meanSquare;
			gain = dbfsToAmp(s.makeup - s.reduction);
			break;
		}
		case Engine::VariMu:
		{
			const float peak = std::max(std::abs(left), std::abs(right));
			// Each network: charged through the rectifier from the output, discharged through its resistor, its
			// branches drawing from it; solve the main node for the gain it leads to
			auto charge = [&s, peak](int n, float v, float node) {
				const auto& net = s.networks[n];
				const auto& state = s.voltages[n];
				const float rectified = std::max(s.sidechain * peak * dbfsToAmp(-s.law(v)) - s.bias, 0.f);
				float dv = -state[0] / net.release;
				for (int b = 0; b < net.branches; ++b)
				{
					dv -= (state[0] - state[b + 1]) * net.branchRatio[b] / net.branchTau[b];
				}
				dv += std::max(rectified - node, 0.f) / net.attack;
				return state[0] + dv * s.period;
			};
			auto controlOf = [&s](const std::array<float, 2>& nodes) {
				return s.networkCount == 1 ? nodes[0] : std::max(nodes[0], nodes[1]);
			};
			auto nodes = std::array<float, 2>{};
			auto evaluate = [&](float v) {
				for (int n = 0; n < s.networkCount; ++n) { nodes[n] = charge(n, v, s.voltages[n][0]); }
				return controlOf(nodes);
			};
			const float high = std::max(evaluate(0.f), 0.f) + 1.f;
			const float v = solve([&evaluate](float x) { return evaluate(x) - x; }, 0.f, high);
			evaluate(v);
			for (int n = 0; n < s.networkCount; ++n)
			{
				auto& state = s.voltages[n];
				const auto& net = s.networks[n];
				for (int b = 0; b < net.branches; ++b)
				{
					state[b + 1] += (state[0] - state[b + 1]) / net.branchTau[b] * s.period;
				}
				state[0] = std::max(nodes[n], 0.f);
			}
			s.reduction = s.law(controlOf({s.voltages[0][0], s.voltages[1][0]}));
			gain = dbfsToAmp(s.makeup - s.reduction);
			break;
		}
		case Engine::Optical:
		{
			const float peak = std::max(std::abs(left), std::abs(right));
			// The cell in the loop: its light from the output (and, limiting, a part of the input)
			auto cellStep = [&s, peak](float g, float& fast, float& slow) {
				const float through = dbfsToAmp(-g);
				const float envelope = s.cellEnvelope + (peak * (through + s.limitFeedForward) - s.cellEnvelope)
					* s.envelopeCoefficient;
				const float drive = std::max(s.cellGain * envelope - s.cellOffset, 0.f);
				const float target = std::pow(drive, s.cellExponent * 0.7f);
				fast = s.cellFast + (target - s.cellFast) * (target > s.cellFast ? s.cellAttack : s.cellFall);
				const float memory = 1.f + 2.f * s.cellMemory;
				slow = s.cellSlow + (fast > s.cellSlow ? (fast - s.cellSlow) * s.cellCharge
					: (fast - s.cellSlow) * (s.cellSlowFall / memory));
				return 20.f * std::log10(1.f + std::max(fast, slow));
			};
			float fast = 0.f, slow = 0.f;
			const float high = cellStep(0.f, fast, slow) + 1.f;
			const float g = solve([&cellStep](float x) { float f, sl; return cellStep(x, f, sl) - x; }, 0.f, high);
			s.reduction = cellStep(g, fast, slow);
			s.cellEnvelope += (peak * (dbfsToAmp(-s.reduction) + s.limitFeedForward) - s.cellEnvelope)
				* s.envelopeCoefficient;
			s.cellFast = fast;
			s.cellSlow = slow;
			// The memory grows while the cell is lit and fades slowly
			const bool lit = fast > 0.1f;
			s.cellMemory += ((lit ? 1.f : 0.f) - s.cellMemory) * (lit ? s.cellMemoryRise : s.cellMemoryFall);
			gain = dbfsToAmp(s.makeup - s.reduction);
			break;
		}
	}

	left *= gain;
	right *= gain;

	// The gain element's distortion, oversampled: a 2nd (without its offset) and a 3rd, growing with the reduction
	if (s.colored)
	{
		const auto& c = s.coloration;
		const float per10 = std::max(s.reduction, 0.f) / 10.f;
		const float growth = s.model == Model::Fairchild670 ? per10 * std::sqrt(per10) : per10;
		float second = 2.f * (c.second + c.secondPer10 * growth);
		float third = 4.f * (c.third + c.thirdPer10 * growth);
		if (s.allButtons) { second *= 3.f; third *= 3.f; }
		const float reference = s.reference;
		auto color = [&](float& sample, int ch) {
			auto oversampled = std::array<float, 4>{};
			s.upsamplers[ch].processSample(oversampled.data(), sample);
			for (auto& x : oversampled)
			{
				const float u = 4.f * std::tanh(x / reference / 4.f);
				const float squared = u * u;
				s.dcSecond[ch] += (squared - s.dcSecond[ch]) * 0.0002f;
				x += reference * (second * (squared - s.dcSecond[ch]) + third * squared * u);
			}
			sample = s.downsamplers[ch].processSample(oversampled.data());
		};
		color(left, 0);
		color(right, 1);
	}

	// The unit's response and transformer
	if (s.path.lowPassAt20 > 0.f)
	{
		const float amount = std::clamp(s.reduction / 20.f, 0.f, 1.f);
		const float corner = s.path.lowPass + (s.path.lowPassAt20 - s.path.lowPass) * amount;
		s.lowPass.setTarget(Biquad::lowPass1(corner, static_cast<float>(s.sampleRate)));
		s.lowPass.finish();
	}
	if (s.path.highPass > 0.f)
	{
		left = static_cast<float>(s.highPass.process(left, 0, 1.0));
		right = static_cast<float>(s.highPass.process(right, 1, 1.0));
	}
	if (s.path.lowPass > 0.f)
	{
		left = static_cast<float>(s.lowPass.process(left, 0, 1.0));
		right = static_cast<float>(s.lowPass.process(right, 1, 1.0));
	}
	if (s.transformer)
	{
		left = s.transformer->process(left, 0);
		right = s.transformer->process(right, 1);
	}
}




void ChannelCompressor::reset()
{
	auto& s = *m_state;
	s.meanSquare = 0.f;
	s.reduction = 0.f;
	s.slowReduction = 0.f;
	s.voltages = {};
	s.cellFast = s.cellSlow = s.cellMemory = s.cellEnvelope = 0.f;
	s.dcSecond = {};
	for (auto& u : s.upsamplers) { u.reset(); }
	for (auto& d : s.downsamplers) { d.reset(); }
	s.thrustFilter.reset();
	s.thrustFilter2.reset();
	s.highPass.reset();
	s.lowPass.reset();
	if (s.transformer) { s.transformer->reset(); }
}

auto ChannelCompressor::reduction() const -> float { return m_state->reduction; }
auto ChannelCompressor::makeup() const -> float { return m_state->makeup; }




// ----------------------------------------------------------------------------------------------------------------
// The gate

namespace
{

constexpr float GateAttack = 0.0015f;		//!< s for 40 dB
constexpr float GateFastAttack = 0.0001f;
constexpr float GateRatio = 20.f;
constexpr float ExpanderRatio = 2.f;
constexpr float GateKeyRelease = 0.01f;		//!< s, of the key's peak envelope, so it does not chatter

auto gateTr(const char* text) -> QString { return QCoreApplication::translate("ChannelGate", text); }

} // namespace

struct ChannelGate::Key
{
	RampedBiquad highPass, lowPass;
	bool filtered = false;
};

ChannelGate::ChannelGate() : m_key{std::make_unique<Key>()} {}
ChannelGate::~ChannelGate() = default;

auto ChannelGate::modeSpec(GateMode mode) -> ModeSpec
{
	// docs/pesquisa-compressores.md, part 2
	switch (mode)
	{
		case GateMode::Kick: return {false, true, 0.05f, 0.1f, 3.f, 35.f, 250.f};
		case GateMode::Snare: return {false, true, 0.05f, 0.24f, 3.f, 1000.f, 2000.f};
		case GateMode::Toms: return {false, true, 0.1f, 0.75f, 6.f, 60.f, 3000.f};
		case GateMode::Vox: return {true, false, 0.f, 0.1f, 4.f, 150.f, 0.f};
		case GateMode::Guitar: return {false, false, 0.05f, 0.3f, 6.f, 80.f, 0.f};
		case GateMode::Bass: return {false, false, 0.2f, 0.2f, 3.f, 0.f, 0.f};
		case GateMode::Noise: return {true, false, 0.05f, 0.28f, 3.f, 150.f, 0.f};
		case GateMode::Expander: return {true, false, 0.f, 0.1f, 0.f, 0.f, 0.f};
		default: return {false, true, 0.f, 0.33f, 0.f, 0.f, 0.f};
	}
}

auto ChannelGate::modeName(GateMode mode) -> QString
{
	switch (mode)
	{
		case GateMode::Kick: return gateTr("KICK");
		case GateMode::Snare: return gateTr("SNARE");
		case GateMode::Toms: return gateTr("TOMS");
		case GateMode::Vox: return gateTr("VOX");
		case GateMode::Guitar: return gateTr("GTR");
		case GateMode::Bass: return gateTr("BASS");
		case GateMode::Noise: return gateTr("NOISE");
		case GateMode::Expander: return gateTr("EXP");
		default: return gateTr("STD");
	}
}

auto ChannelGate::modeDescription(GateMode mode) -> QString
{
	switch (mode)
	{
		case GateMode::Kick: return gateTr("Kick: fast, 50 ms hold, 0.1 s release, keyed on 35-250 Hz");
		case GateMode::Snare: return gateTr("Snare: fast, 50 ms hold, 0.24 s release, keyed on 1-2 kHz");
		case GateMode::Toms: return gateTr("Toms: fast, 100 ms hold, 0.75 s release, keyed on 60 Hz-3 kHz");
		case GateMode::Vox: return gateTr("Vocals: 2:1 expander, 0.1 s release, keyed above 150 Hz");
		case GateMode::Guitar: return gateTr("Guitar: 50 ms hold, 0.3 s release, keyed above 80 Hz");
		case GateMode::Bass: return gateTr("Bass: 0.2 s hold and release");
		case GateMode::Noise: return gateTr("Noise: gentle 2:1 expander, 0.28 s release, keyed above 150 Hz");
		case GateMode::Expander: return gateTr("Expander: the SSL's 2:1 expander");
		default: return gateTr("Standard: the SSL's gate, fast, 0.33 s release");
	}
}

auto ChannelGate::hysteresis(float threshold) -> float
{
	return 2.f + 6.f * std::clamp((10.f - threshold) / 40.f, 0.f, 1.f);
}

void ChannelGate::begin(const Settings& settings, sample_rate_t sampleRate)
{
	const bool modeChanged = settings.mode != m_settings.mode;
	m_settings = settings;
	m_spec = modeSpec(settings.mode);
	const auto rate = static_cast<float>(sampleRate);
	m_attackStep = 40.f / ((m_spec.fast ? GateFastAttack : GateAttack) * rate);
	m_releaseCoefficient = 1.f - std::exp(-1.f / (m_spec.release * rate));
	m_holdFrames = static_cast<f_cnt_t>(m_spec.hold * rate);
	m_keyRelease = std::exp(-1.f / (GateKeyRelease * rate));
	auto& key = *m_key;
	key.filtered = m_spec.keyHighPass > 0.f || m_spec.keyLowPass > 0.f;
	key.highPass.setTarget(m_spec.keyHighPass > 0.f ? Biquad::highPass(m_spec.keyHighPass, 0.7071, rate) : Biquad{});
	key.lowPass.setTarget(m_spec.keyLowPass > 0.f ? Biquad::lowPass(m_spec.keyLowPass, 0.7071, rate) : Biquad{});
	key.highPass.finish();
	key.lowPass.finish();
	if (modeChanged) { key.highPass.reset(); key.lowPass.reset(); }
}

auto ChannelGate::gain(float left, float right) -> float
{
	auto& key = *m_key;
	if (key.filtered)
	{
		left = static_cast<float>(key.lowPass.process(key.highPass.process(left, 0, 1.0), 0, 1.0));
		right = static_cast<float>(key.lowPass.process(key.highPass.process(right, 1, 1.0), 1, 1.0));
	}
	// The key: a peak envelope, quick to rise and briefly held, so that zero crossings do not close the gate
	m_envelope = std::max(std::max(std::abs(left), std::abs(right)), m_envelope * m_keyRelease);
	const float level = levelOf(m_envelope);
	const float threshold = m_settings.threshold + DbuToDbfs;
	const float hysteresisDb = m_spec.hysteresis > 0.f ? m_spec.hysteresis : hysteresis(m_settings.threshold);

	if (level >= threshold)
	{
		m_open = true;
		m_holding = m_holdFrames;
	}
	else if (level < threshold - hysteresisDb)
	{
		if (m_holding > 0) { --m_holding; }
		else { m_open = false; }
	}

	const float ratio = m_spec.expander ? ExpanderRatio : GateRatio;
	const float target = m_open ? 0.f
		: std::min(m_settings.range, std::max(threshold - level, 0.f) * (ratio - 1.f));
	if (target < m_reduction) { m_reduction = std::max(target, m_reduction - m_attackStep); }
	else { m_reduction += (target - m_reduction) * m_releaseCoefficient; }
	return dbfsToAmp(-m_reduction);
}

void ChannelGate::reset()
{
	m_open = false;
	m_holding = 0;
	m_envelope = 0.f;
	m_reduction = 0.f;
	m_key->highPass.reset();
	m_key->lowPass.reset();
}

} // namespace lmms
