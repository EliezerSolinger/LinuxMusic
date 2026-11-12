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
	: m_compressorModel(false, parent, QObject::tr("Bus compressor"))
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
{
	m_attackModel.setScaleLogarithmic(true);
	m_releaseModel.setScaleLogarithmic(true);
	m_monitorLevelModel.setCenterValue(0.f);

	// Listening states: not part of the project's history
	for (auto model : std::initializer_list<AutomatableModel*>{&m_monitorLevelModel, &m_dimModel, &m_cutModel,
		&m_monoModel})
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
	const float power = std::sqrt(m_sumLL * m_sumRR);
	m_correlation.store(power > 1e-12f ? std::clamp(m_sumLR / power, -1.f, 1.f) : 0.f, std::memory_order_relaxed);
}




void MasterSection::processMonitoring(std::span<SampleFrame> frames)
{
	const float target = m_cutModel.value() ? 0.f
		: dbfsToAmp(m_monitorLevelModel.value() + (m_dimModel.value() ? DimLevel : 0.f));
	const bool mono = m_monoModel.value();
	if (target == 1.f && m_monitorGain == 1.f && !mono) { return; }

	// Ramped over the period, so that pressing DIM or CUT does not click
	const float step = (target - m_monitorGain) / static_cast<float>(std::max<std::size_t>(frames.size(), 1));
	for (auto& frame : frames)
	{
		m_monitorGain += step;
		if (mono)
		{
			const float sum = (frame[0] + frame[1]) * 0.5f;
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
