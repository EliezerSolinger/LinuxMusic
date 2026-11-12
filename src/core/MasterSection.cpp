/*
 * MasterSection.cpp - the master section of the mixer: bus compressor, monitoring and meters
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

#include "MasterSection.h"

#include <algorithm>
#include <cmath>
#include <numbers>

#include <QDomElement>

#include "SampleFrame.h"
#include "lmms_math.h"

namespace lmms
{

namespace
{

constexpr float CompressorKnee = 2.f;			// dB
constexpr float FastRelease = 0.1f;				// s, the auto release on short transients
constexpr float SlowRelease = 1.2f;				// s, the auto release on sustained compression
constexpr float SustainTime = 1.f;				// s, over which the auto release judges the compression
constexpr float SustainForSlowRelease = 6.f;	// dB of sustained reduction for the slowest auto release
constexpr float CorrelationTime = 0.3f;			// s

auto smoothing(float seconds, sample_rate_t rate) -> float
{
	return 1.f - std::exp(-1.f / (seconds * static_cast<float>(rate)));
}

} // namespace




MasterSection::MasterSection(Model* parent)
	: m_preLeftModel(1.f, 0.f, 2.f, 0.001f, parent, QObject::tr("Pre master left"))
	, m_preRightModel(1.f, 0.f, 2.f, 0.001f, parent, QObject::tr("Pre master right"))
	, m_preLinkModel(true, parent, QObject::tr("Pre master link"))
	, m_bassWidthModel(100.f, 0.f, 100.f, 1.f, parent, QObject::tr("Mono bass width"))
	, m_bassFrequencyModel(120.f, 20.f, 300.f, 1.f, parent, QObject::tr("Mono bass frequency"))
	, m_airModel(0.f, -6.f, 6.f, 0.1f, parent, QObject::tr("Side air"))
	, m_compressorModel(false, parent, QObject::tr("Bus compressor"))
	, m_thresholdModel(-10.f, -30.f, 0.f, 0.1f, parent, QObject::tr("Bus compressor threshold"))
	, m_ratioModel(static_cast<int>(Ratio::Four), 0, 2, parent, QObject::tr("Bus compressor ratio"))
	, m_attackModel(10.f, 0.1f, 30.f, 0.1f, parent, QObject::tr("Bus compressor attack"))
	, m_releaseModel(0.3f, 0.1f, 1.2f, 0.01f, parent, QObject::tr("Bus compressor release"))
	, m_autoReleaseModel(true, parent, QObject::tr("Bus compressor auto release"))
	, m_makeupModel(0.f, 0.f, 15.f, 0.1f, parent, QObject::tr("Bus compressor make-up"))
	, m_monitorLevelModel(0.f, -60.f, 6.f, 0.1f, parent, QObject::tr("Monitor level"))
	, m_dimModel(false, parent, QObject::tr("Dim"))
	, m_cutModel(false, parent, QObject::tr("Cut"))
	, m_monoModel(false, parent, QObject::tr("Mono"))
	, m_sideModel(false, parent, QObject::tr("Side"))
{
	m_bassFrequencyModel.setScaleLogarithmic(true);
	m_bassWidthModel.setCenterValue(100.f);
	m_airModel.setCenterValue(0.f);

	// Linked PRE faders move together; the one that follows does not add an undo step of its own (undoing the one
	// moved brings the other along again)
	auto follow = [this](FloatModel& leader, FloatModel& follower) {
		if (!m_preLinkModel.value() || follower.value() == leader.value()) { return; }
		follower.saveJournallingState(false);
		follower.setValue(leader.value());
		follower.restoreJournallingState();
	};
	QObject::connect(&m_preLeftModel, &FloatModel::dataChanged, &m_preRightModel,
		[this, follow] { follow(m_preLeftModel, m_preRightModel); });
	QObject::connect(&m_preRightModel, &FloatModel::dataChanged, &m_preLeftModel,
		[this, follow] { follow(m_preRightModel, m_preLeftModel); });
	QObject::connect(&m_preLinkModel, &BoolModel::dataChanged, &m_preRightModel,
		[this, follow] { follow(m_preLeftModel, m_preRightModel); });

	// Listening to the mid (MONO) or to the side
	QObject::connect(&m_monoModel, &BoolModel::dataChanged, &m_sideModel, [this] {
		if (m_monoModel.value()) { m_sideModel.setValue(false); }
	});
	QObject::connect(&m_sideModel, &BoolModel::dataChanged, &m_monoModel, [this] {
		if (m_sideModel.value()) { m_monoModel.setValue(false); }
	});

	m_attackModel.setScaleLogarithmic(true);
	m_releaseModel.setScaleLogarithmic(true);
	m_monitorLevelModel.setCenterValue(0.f);

	// Listening states: not part of the project's history
	for (auto model : std::initializer_list<AutomatableModel*>{&m_monitorLevelModel, &m_dimModel, &m_cutModel,
		&m_monoModel, &m_sideModel})
	{
		model->setJournalling(false);
	}
}




auto MasterSection::ratio(Ratio ratio) -> float
{
	switch (ratio)
	{
		case Ratio::Two: return 2.f;
		case Ratio::Four: return 4.f;
		case Ratio::Ten: return 10.f;
	}
	return 4.f;
}




void MasterSection::processPreFaders(std::span<SampleFrame> frames, sample_rate_t sampleRate)
{
	const auto target = std::array{m_preLeftModel.value(), m_preRightModel.value()};
	if (target == m_preGain && target[0] == 1.f && target[1] == 1.f)
	{
		m_preMeter.measure(frames.data(), frames.size(), sampleRate);
		return;
	}

	// Ramped over the period
	const auto count = static_cast<float>(std::max<std::size_t>(frames.size(), 1));
	for (int ch = 0; ch < 2; ++ch)
	{
		const float step = (target[ch] - m_preGain[ch]) / count;
		float gain = m_preGain[ch];
		for (auto& frame : frames)
		{
			gain += step;
			frame[ch] *= gain;
		}
	}
	m_preGain = target;
	m_preMeter.measure(frames.data(), frames.size(), sampleRate);
}




auto MasterSection::Svf::process(float in, float g) -> Outputs
{
	// Butterworth: 1/Q = sqrt(2)
	constexpr float k = std::numbers::sqrt2_v<float>;
	const float a1 = 1.f / (1.f + g * (g + k));
	const float a2 = g * a1;
	const float a3 = g * a2;
	const float v3 = in - ic2;
	const float v1 = a1 * ic1 + a2 * v3;
	const float v2 = ic2 + a2 * ic1 + a3 * v3;
	ic1 = 2.f * v1 - ic1;
	ic2 = 2.f * v2 - ic2;
	return {v2, in - k * v1 - v2};
}




auto MasterSection::Crossover::process(float in, float g) -> Outputs
{
	const float low = low2.process(low1.process(in, g).low, g).low;
	const float high = high2.process(high1.process(in, g).high, g).high;
	return {low, high};
}




void MasterSection::processStereo(std::span<SampleFrame> frames, sample_rate_t sampleRate, float widthPercent)
{
	const float width = widthPercent / 100.f;
	const float bassWidth = m_bassWidthModel.value() / 100.f;
	const bool bassOn = bassWidth < 1.f;
	const bool bassActive = bassOn || m_bassMix > 0.f; // on, or fading out
	const float air = m_airModel.value();
	// One more period after the air is back at 0 dB, so that the shelf fades out
	const bool airActive = air != 0.f || m_airWasOn;
	m_airWasOn = air != 0.f;

	if (width == 1.f && m_width == 1.f && !bassActive && !airActive)
	{
		m_air.clearHistory();
		return;
	}

	const auto rate = static_cast<float>(sampleRate);
	// The crossover's frequency, warped for the filters (bilinear transform); it glides over the period
	const float targetG = std::tan(std::numbers::pi_v<float> * std::min(m_bassFrequencyModel.value(), 0.45f * rate)
		/ rate);
	if (bassActive && m_crossoverG == 0.f)
	{
		// Starting afresh (faded in, see below)
		m_crossover = {};
		m_crossoverG = targetG;
	}
	if (airActive) { m_air.setParameters(rate, AirFrequency, std::numbers::sqrt2_v<float> / 2.f, air); }

	const float bassMixTarget = bassOn ? 1.f : 0.f;
	const auto count = frames.size();
	for (auto f = std::size_t{0}; f < count; ++f)
	{
		auto& frame = frames[f];
		const float progress = static_cast<float>(f + 1) / static_cast<float>(count);
		float mid = (frame[0] + frame[1]) * 0.5f;
		float side = (frame[0] - frame[1]) * 0.5f;

		if (bassActive)
		{
			// Mid through the crossover (an allpass), side with its lows narrowed
			const float g = m_crossoverG + (targetG - m_crossoverG) * progress;
			const float keep = m_bassWidth + (bassWidth - m_bassWidth) * progress;
			const auto midBands = m_crossover[0].process(mid, g);
			const auto sideBands = m_crossover[1].process(side, g);
			const float mix = m_bassMix + (bassMixTarget - m_bassMix) * progress;
			mid += (midBands.low + midBands.high - mid) * mix;
			side += (sideBands.high + keep * sideBands.low - side) * mix;
		}

		if (airActive) { side = m_air.update(side, 0, progress); }
		side *= m_width + (width - m_width) * progress;

		frame[0] = mid + side;
		frame[1] = mid - side;
	}

	m_width = width;
	m_bassWidth = bassWidth;
	m_bassMix = bassMixTarget;
	m_crossoverG = bassActive && bassOn ? targetG : 0.f;
}




void MasterSection::processCompressor(std::span<SampleFrame> frames, sample_rate_t sampleRate)
{
	if (!m_compressorModel.value())
	{
		m_reduction = 0.f;
		m_sustain = 0.f;
		return;
	}

	const float threshold = m_thresholdModel.value();
	const float slope = 1.f / ratio(static_cast<Ratio>(m_ratioModel.value())) - 1.f;
	const float attack = smoothing(m_attackModel.value() / 1000.f, sampleRate);
	const float fixedRelease = smoothing(m_releaseModel.value(), sampleRate);
	const bool autoRelease = m_autoReleaseModel.value();
	const float sustainCoefficient = smoothing(SustainTime, sampleRate);
	const float makeup = m_makeupModel.value();

	float deepest = 0.f;
	for (auto& frame : frames)
	{
		// Stereo-linked, with a narrow soft knee
		const float level = std::max(std::abs(frame[0]), std::abs(frame[1]));
		const float over = ampToDbfs(std::max(level, 1e-9f)) - threshold;
		float target = 0.f;
		if (over >= CompressorKnee / 2.f) { target = slope * over; }
		else if (over > -CompressorKnee / 2.f)
		{
			const float into = over + CompressorKnee / 2.f;
			target = slope * into * into / (2.f * CompressorKnee);
		}

		// The auto release lengthens with sustained compression, so that it recovers fast from short peaks
		// without pumping on dense material
		float release = fixedRelease;
		if (autoRelease)
		{
			m_sustain += (-m_reduction - m_sustain) * sustainCoefficient;
			const float amount = std::clamp(m_sustain / SustainForSlowRelease, 0.f, 1.f);
			release = smoothing(FastRelease + (SlowRelease - FastRelease) * amount, sampleRate);
		}
		m_reduction += (target - m_reduction) * (target < m_reduction ? attack : release);

		const float gain = dbfsToAmp(m_reduction + makeup);
		frame[0] *= gain;
		frame[1] *= gain;
		deepest = std::min(deepest, m_reduction);
	}

	// The meter shows the deepest reduction until the GUI reads it
	auto shown = m_gainReduction.load(std::memory_order_relaxed);
	while (deepest < shown && !m_gainReduction.compare_exchange_weak(shown, deepest, std::memory_order_relaxed)) {}
}




void MasterSection::measure(std::span<const SampleFrame> frames, sample_rate_t sampleRate)
{
	const float coefficient = smoothing(CorrelationTime, sampleRate);
	for (const auto& frame : frames)
	{
		m_sumLR += (frame[0] * frame[1] - m_sumLR) * coefficient;
		m_sumLL += (frame[0] * frame[0] - m_sumLL) * coefficient;
		m_sumRR += (frame[1] * frame[1] - m_sumRR) * coefficient;
	}
	// Every other frame for the goniometer: (side, mid)
	int write = m_scopeWrite.load(std::memory_order_relaxed);
	for (const auto& frame : frames)
	{
		if (++m_scopeDecimation < 2) { continue; }
		m_scopeDecimation = 0;
		m_scope[2 * write].store((frame[0] - frame[1]) * 0.5f, std::memory_order_relaxed);
		m_scope[2 * write + 1].store((frame[0] + frame[1]) * 0.5f, std::memory_order_relaxed);
		write = (write + 1) % ScopePoints;
	}
	m_scopeWrite.store(write, std::memory_order_relaxed);
	m_scopeSilent = false;

	const float power = std::sqrt(m_sumLL * m_sumRR);
	m_correlation.store(power > 1e-12f ? std::clamp(m_sumLR / power, -1.f, 1.f) : 0.f, std::memory_order_relaxed);
}




void MasterSection::measureSilence()
{
	m_preMeter.measureSilence();
	m_sumLR = m_sumLL = m_sumRR = 0.f;
	m_correlation.store(0.f, std::memory_order_relaxed);
	if (m_scopeSilent) { return; }
	for (auto& value : m_scope) { value.store(0.f, std::memory_order_relaxed); }
	m_scopeSilent = true;
}




void MasterSection::scopePoints(std::vector<std::array<float, 2>>& points) const
{
	points.resize(ScopePoints);
	const int write = m_scopeWrite.load(std::memory_order_relaxed);
	for (int i = 0; i < ScopePoints; ++i)
	{
		const int at = (write + i) % ScopePoints;
		points[i] = {m_scope[2 * at].load(std::memory_order_relaxed), m_scope[2 * at + 1].load(std::memory_order_relaxed)};
	}
}




void MasterSection::processMonitoring(std::span<SampleFrame> frames)
{
	const float target = m_cutModel.value() ? 0.f
		: dbfsToAmp(m_monitorLevelModel.value() + (m_dimModel.value() ? DimLevel : 0.f));
	const bool mono = m_monoModel.value();
	const bool side = m_sideModel.value();
	if (target == 1.f && m_monitorGain == 1.f && !mono && !side) { return; }

	// Ramped over the period, so that pressing DIM or CUT does not click
	const float step = (target - m_monitorGain) / static_cast<float>(std::max<std::size_t>(frames.size(), 1));
	for (auto& frame : frames)
	{
		m_monitorGain += step;
		if (mono || side)
		{
			// The mid or the side on both speakers, in phase
			const float sum = (frame[0] + (side ? -frame[1] : frame[1])) * 0.5f;
			frame[0] = sum;
			frame[1] = sum;
		}
		frame[0] *= m_monitorGain;
		frame[1] *= m_monitorGain;
	}
	m_monitorGain = target;
}




void MasterSection::setDefaults()
{
	m_preLinkModel.setValue(true);
	m_preLeftModel.setValue(1.f);
	m_preRightModel.setValue(1.f);
	m_bassWidthModel.setValue(100.f);
	m_bassFrequencyModel.setValue(120.f);
	m_airModel.setValue(0.f);
	m_compressorModel.setValue(false);
	m_thresholdModel.setValue(-10.f);
	m_ratioModel.setValue(static_cast<int>(Ratio::Four));
	m_attackModel.setValue(10.f);
	m_releaseModel.setValue(0.3f);
	m_autoReleaseModel.setValue(true);
	m_makeupModel.setValue(0.f);
}




void MasterSection::saveSettings(QDomDocument& doc, QDomElement& mixerElement)
{
	auto master = doc.createElement("masterstrip");
	m_preLeftModel.saveSettings(doc, master, "preleft");
	m_preRightModel.saveSettings(doc, master, "preright");
	m_preLinkModel.saveSettings(doc, master, "prelink");
	m_bassWidthModel.saveSettings(doc, master, "basswidth");
	m_bassFrequencyModel.saveSettings(doc, master, "bassfreq");
	m_airModel.saveSettings(doc, master, "air");
	mixerElement.appendChild(master);

	auto element = doc.createElement("buscompressor");
	m_compressorModel.saveSettings(doc, element, "on");
	m_thresholdModel.saveSettings(doc, element, "threshold");
	m_ratioModel.saveSettings(doc, element, "ratio");
	m_attackModel.saveSettings(doc, element, "attack");
	m_releaseModel.saveSettings(doc, element, "release");
	m_autoReleaseModel.saveSettings(doc, element, "autorelease");
	m_makeupModel.saveSettings(doc, element, "makeup");
	mixerElement.appendChild(element);
}




void MasterSection::loadSettings(const QDomElement& mixerElement)
{
	setDefaults();
	if (const auto master = mixerElement.firstChildElement("masterstrip"); !master.isNull())
	{
		// The link first, so that loading one fader does not move the other
		m_preLinkModel.loadSettings(master, "prelink");
		const bool linked = m_preLinkModel.value();
		m_preLinkModel.setValue(false);
		m_preLeftModel.loadSettings(master, "preleft");
		m_preRightModel.loadSettings(master, "preright");
		m_preLinkModel.setValue(linked);
		m_bassWidthModel.loadSettings(master, "basswidth");
		m_bassFrequencyModel.loadSettings(master, "bassfreq");
		m_airModel.loadSettings(master, "air");
	}
	const auto element = mixerElement.firstChildElement("buscompressor");
	if (element.isNull()) { return; }
	m_compressorModel.loadSettings(element, "on");
	m_thresholdModel.loadSettings(element, "threshold");
	m_ratioModel.loadSettings(element, "ratio");
	m_attackModel.loadSettings(element, "attack");
	m_releaseModel.loadSettings(element, "release");
	m_autoReleaseModel.loadSettings(element, "autorelease");
	m_makeupModel.loadSettings(element, "makeup");
}

} // namespace lmms
