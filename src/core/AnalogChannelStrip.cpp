/*
 * AnalogChannelStrip.cpp - the built-in processing of a mixer channel in analog mode
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

#include "AnalogChannelStrip.h"

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

// Fixed parts of the strip
constexpr float LowCutFrequency = 80.f;
constexpr float FilterQ = std::numbers::sqrt2_v<float> / 2.f;
constexpr float EqLowFrequency = 80.f;
constexpr float EqHighFrequency = 12000.f;
constexpr float EqMidBandwidth = 1.4f; // octaves
constexpr float GateAttack = 0.001f;	// s
constexpr float GateHold = 0.02f;		// s
constexpr float GateRelease = 0.1f;		// s
constexpr float CompressorRatio = 4.f;
constexpr float CompressorKnee = 6.f;	// dB
constexpr float CompressorAttack = 0.005f;	// s
constexpr float CompressorRelease = 0.12f;	// s
constexpr float FilterTail = 0.05f;		// s, how long the filters may ring after the input

//! The one-pole smoothing coefficient for time constant @p seconds
auto smoothing(float seconds, sample_rate_t rate) -> float
{
	return 1.f - std::exp(-1.f / (seconds * static_cast<float>(rate)));
}

} // namespace




AnalogChannelStrip::AnalogChannelStrip(Model* parent)
	: m_lowCutModel(false, parent)
	, m_highPassModel(20.f, 20.f, 1000.f, 1.f, parent)
	, m_lowPassModel(20000.f, 1000.f, 20000.f, 1.f, parent)
	, m_gateModel(-80.f, -80.f, 0.f, 0.1f, parent)
	, m_compressorModel(0.f, -40.f, 0.f, 0.1f, parent)
	, m_eqHighModel(0.f, -15.f, 15.f, 0.1f, parent)
	, m_eqMidModel(0.f, -15.f, 15.f, 0.1f, parent)
	, m_eqMidFrequencyModel(1000.f, 200.f, 6000.f, 1.f, parent)
	, m_eqLowModel(0.f, -15.f, 15.f, 0.1f, parent)
{
	m_highPassModel.setScaleLogarithmic(true);
	m_lowPassModel.setScaleLogarithmic(true);
	m_eqMidFrequencyModel.setScaleLogarithmic(true);
	// Knobs draw their arc from these: from the neutral setting
	for (auto model : {&m_eqHighModel, &m_eqMidModel, &m_eqLowModel}) { model->setCenterValue(0.f); }
	m_highPassModel.setCenterValue(m_highPassModel.minValue());
	m_lowPassModel.setCenterValue(m_lowPassModel.maxValue());
	m_gateModel.setCenterValue(m_gateModel.minValue());
	m_compressorModel.setCenterValue(0.f);
	m_eqMidFrequencyModel.setCenterValue(1000.f);
}




auto AnalogChannelStrip::saturate(float sample) -> float
{
	const float magnitude = std::abs(sample);
	if (magnitude <= SaturationKnee) { return sample; }
	constexpr float span = SaturationRail - SaturationKnee;
	return std::copysign(SaturationKnee + span * std::tanh((magnitude - SaturationKnee) / span), sample);
}




void AnalogChannelStrip::process(std::span<SampleFrame> frames, sample_rate_t sampleRate, bool hadInput)
{
	// Sections left at their neutral setting are skipped
	const bool lowCut = m_lowCutModel.value();
	const bool highPassOn = m_highPassModel.value() > m_highPassModel.minValue();
	const bool lowPassOn = m_lowPassModel.value() < m_lowPassModel.maxValue();
	const bool gateOn = m_gateModel.value() > m_gateModel.minValue();
	const bool compressorOn = m_compressorModel.value() < 0.f;
	const bool eqLowOn = m_eqLowModel.value() != 0.f;
	const bool eqMidOn = m_eqMidModel.value() != 0.f;
	const bool eqHighOn = m_eqHighModel.value() != 0.f;

	// A section switched off starts afresh when switched on again
	if (!lowCut) { m_lowCut[0].clearHistory(); m_lowCut[1].clearHistory(); }
	if (!highPassOn) { m_highPass.clearHistory(); }
	if (!lowPassOn) { m_lowPass.clearHistory(); }
	if (!eqLowOn) { m_eqLow.clearHistory(); }
	if (!eqMidOn) { m_eqMid.clearHistory(); }
	if (!eqHighOn) { m_eqHigh.clearHistory(); }
	if (!gateOn) { m_gateGain = 1.f; m_gateHold = 0; }
	if (!compressorOn) { m_compressorEnvelope = 0.f; }

	const auto rate = static_cast<float>(sampleRate);
	for (auto& filter : m_lowCut) { filter.setParameters(rate, LowCutFrequency, FilterQ, 1.f); }
	m_highPass.setParameters(rate, m_highPassModel.value(), FilterQ, 1.f);
	m_lowPass.setParameters(rate, m_lowPassModel.value(), FilterQ, 1.f);
	m_eqLow.setParameters(rate, EqLowFrequency, FilterQ, m_eqLowModel.value());
	m_eqMid.setParameters(rate, m_eqMidFrequencyModel.value(), EqMidBandwidth, m_eqMidModel.value());
	m_eqHigh.setParameters(rate, EqHighFrequency, FilterQ, m_eqHighModel.value());

	// After the input falls silent, the filters ring out for a while
	const bool anyFilter = lowCut || highPassOn || lowPassOn || eqLowOn || eqMidOn || eqHighOn;
	if (!anyFilter) { m_tailFrames = 0; }
	else if (hadInput) { m_tailFrames = static_cast<f_cnt_t>(FilterTail * rate); }
	else { m_tailFrames = m_tailFrames > frames.size() ? m_tailFrames - frames.size() : 0; }

	const float gateThreshold = dbfsToAmp(m_gateModel.value());
	const float gateOpen = smoothing(GateAttack, sampleRate);
	const float gateClose = smoothing(GateRelease, sampleRate);
	const auto gateHold = static_cast<f_cnt_t>(GateHold * rate);

	// Gain computer with a soft knee, as in the Compressor plugin; the make-up gain is half of what the compressor
	// takes off a signal at 0 dBFS
	const float threshold = m_compressorModel.value();
	const float slope = 1.f / CompressorRatio - 1.f;
	const float makeup = slope * threshold / 2.f;
	const float attack = smoothing(CompressorAttack, sampleRate);
	const float release = smoothing(CompressorRelease, sampleRate);

	const auto count = frames.size();
	for (auto f = std::size_t{0}; f < count; ++f)
	{
		auto& frame = frames[f];
		const float progress = count > 1 ? static_cast<float>(f) / static_cast<float>(count - 1) : 1.f;
		for (ch_cnt_t ch = 0; ch < 2; ++ch)
		{
			float x = saturate(frame[ch]);
			if (lowCut) { x = m_lowCut[1].update(m_lowCut[0].update(x, ch, progress), ch, progress); }
			if (highPassOn) { x = m_highPass.update(x, ch, progress); }
			if (lowPassOn) { x = m_lowPass.update(x, ch, progress); }
			frame[ch] = x;
		}

		// Stereo-linked dynamics
		const float level = std::max(std::abs(frame[0]), std::abs(frame[1]));
		float gain = 1.f;
		if (gateOn)
		{
			if (level >= gateThreshold) { m_gateHold = gateHold; }
			const bool open = m_gateHold > 0;
			if (m_gateHold > 0) { --m_gateHold; }
			m_gateGain += ((open ? 1.f : 0.f) - m_gateGain) * (open ? gateOpen : gateClose);
			gain *= m_gateGain;
		}
		if (compressorOn)
		{
			m_compressorEnvelope += (level - m_compressorEnvelope) * (level > m_compressorEnvelope ? attack : release);
			const float over = ampToDbfs(std::max(m_compressorEnvelope, 1e-9f)) - threshold;
			float reduction = 0.f;
			if (over >= CompressorKnee / 2.f) { reduction = slope * over; }
			else if (over > -CompressorKnee / 2.f)
			{
				const float into = over + CompressorKnee / 2.f;
				reduction = slope * into * into / (2.f * CompressorKnee);
			}
			gain *= dbfsToAmp(reduction + makeup);
		}

		for (ch_cnt_t ch = 0; ch < 2; ++ch)
		{
			float x = frame[ch] * gain;
			if (eqLowOn) { x = m_eqLow.update(x, ch, progress); }
			if (eqMidOn) { x = m_eqMid.update(x, ch, progress); }
			if (eqHighOn) { x = m_eqHigh.update(x, ch, progress); }
			frame[ch] = x;
		}
	}
}




auto AnalogChannelStrip::holdsSignal() const -> bool
{
	return m_tailFrames > 0;
}




void AnalogChannelStrip::reset()
{
	for (auto filter : std::initializer_list<EqFilter*>{&m_lowCut[0], &m_lowCut[1], &m_highPass, &m_lowPass, &m_eqLow,
		&m_eqMid, &m_eqHigh})
	{
		filter->clearHistory();
	}
	m_tailFrames = 0;
	m_gateGain = 1.f;
	m_gateHold = 0;
	m_compressorEnvelope = 0.f;
}




void AnalogChannelStrip::setNeutral()
{
	m_lowCutModel.setValue(false);
	m_highPassModel.setValue(m_highPassModel.minValue());
	m_lowPassModel.setValue(m_lowPassModel.maxValue());
	m_gateModel.setValue(m_gateModel.minValue());
	m_compressorModel.setValue(0.f);
	m_eqHighModel.setValue(0.f);
	m_eqMidModel.setValue(0.f);
	m_eqMidFrequencyModel.setValue(1000.f);
	m_eqLowModel.setValue(0.f);
}




void AnalogChannelStrip::setDisplayNames(const QString& channelName)
{
	const auto prefix = channelName + ">";
	m_lowCutModel.setDisplayName(prefix + QObject::tr("Low cut"));
	m_highPassModel.setDisplayName(prefix + QObject::tr("High-pass"));
	m_lowPassModel.setDisplayName(prefix + QObject::tr("Low-pass"));
	m_gateModel.setDisplayName(prefix + QObject::tr("Gate"));
	m_compressorModel.setDisplayName(prefix + QObject::tr("Compressor"));
	m_eqHighModel.setDisplayName(prefix + QObject::tr("EQ high"));
	m_eqMidModel.setDisplayName(prefix + QObject::tr("EQ mid"));
	m_eqMidFrequencyModel.setDisplayName(prefix + QObject::tr("EQ mid frequency"));
	m_eqLowModel.setDisplayName(prefix + QObject::tr("EQ low"));
}




void AnalogChannelStrip::saveSettings(QDomDocument& doc, QDomElement& channelElement)
{
	auto strip = doc.createElement("analogstrip");
	m_lowCutModel.saveSettings(doc, strip, "lowcut");
	m_highPassModel.saveSettings(doc, strip, "highpass");
	m_lowPassModel.saveSettings(doc, strip, "lowpass");
	m_gateModel.saveSettings(doc, strip, "gate");
	m_compressorModel.saveSettings(doc, strip, "compressor");
	m_eqHighModel.saveSettings(doc, strip, "eqhigh");
	m_eqMidModel.saveSettings(doc, strip, "eqmid");
	m_eqMidFrequencyModel.saveSettings(doc, strip, "eqmidfreq");
	m_eqLowModel.saveSettings(doc, strip, "eqlow");
	channelElement.appendChild(strip);
}




void AnalogChannelStrip::loadSettings(const QDomElement& channelElement)
{
	setNeutral();
	const auto strip = channelElement.firstChildElement("analogstrip");
	if (strip.isNull()) { return; }
	m_lowCutModel.loadSettings(strip, "lowcut");
	m_highPassModel.loadSettings(strip, "highpass");
	m_lowPassModel.loadSettings(strip, "lowpass");
	m_gateModel.loadSettings(strip, "gate");
	m_compressorModel.loadSettings(strip, "compressor");
	m_eqHighModel.loadSettings(strip, "eqhigh");
	m_eqMidModel.loadSettings(strip, "eqmid");
	m_eqMidFrequencyModel.loadSettings(strip, "eqmidfreq");
	m_eqLowModel.loadSettings(strip, "eqlow");
}

} // namespace lmms
