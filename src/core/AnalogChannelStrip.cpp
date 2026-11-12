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
constexpr double LowCutFrequency = 100.0;
constexpr double ButterworthQ = std::numbers::sqrt2 / 2.0;
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
	: m_gainModel(0.5f, 0.f, 1.f, 0.001f, parent)
	, m_driveModel(static_cast<int>(Drive::OpAmp4558), 0, static_cast<int>(Drive::Count) - 1, parent)
	, m_realisticModel(false, parent)
	, m_lowCutModel(false, parent)
	, m_highPassModel(20.f, 20.f, 1000.f, 1.f, parent)
	, m_lowPassModel(20000.f, 1000.f, 20000.f, 1.f, parent)
	, m_monoBassWidthModel(100.f, 0.f, 100.f, 1.f, parent)
	, m_monoBassFrequencyModel(120.f, 20.f, 300.f, 1.f, parent)
	, m_gateModel(-80.f, -80.f, 0.f, 0.1f, parent)
	, m_compressorModel(0.f, -40.f, 0.f, 0.1f, parent)
	, m_eqHighModel(0.f, -15.f, 15.f, 0.1f, parent)
	, m_eqHighMidModel(0.f, -15.f, 15.f, 0.1f, parent)
	, m_eqLowMidModel(0.f, -15.f, 15.f, 0.1f, parent)
	, m_eqLowModel(0.f, -15.f, 15.f, 0.1f, parent)
{
	m_highPassModel.setScaleLogarithmic(true);
	m_lowPassModel.setScaleLogarithmic(true);
	m_monoBassFrequencyModel.setScaleLogarithmic(true);
	m_monoBassWidthModel.setCenterValue(100.f);
	// Knobs draw their arc from these: from the neutral setting
	for (auto model : {&m_eqHighModel, &m_eqHighMidModel, &m_eqLowMidModel, &m_eqLowModel}) { model->setCenterValue(0.f); }
	m_highPassModel.setCenterValue(m_highPassModel.minValue());
	m_lowPassModel.setCenterValue(m_lowPassModel.maxValue());
	m_gateModel.setCenterValue(m_gateModel.minValue());
	m_compressorModel.setCenterValue(0.f);
	m_gainModel.setCenterValue(0.5f);
}




auto AnalogChannelStrip::potGain(float position) -> float
{
	if (position <= 0.f) { return 0.f; }
	if (position <= 0.5f)
	{
		const float x = 2.f * position;
		return x * x * x;
	}
	return dbfsToAmp((position - 0.5f) * 2.f * MaxGain);
}




void AnalogChannelStrip::process(std::span<SampleFrame> frames, sample_rate_t sampleRate, bool hadInput)
{
	// Sections left at their neutral setting are skipped
	const bool lowCut = m_lowCutModel.value();
	const bool highPassOn = m_highPassModel.value() > m_highPassModel.minValue();
	const bool lowPassOn = m_lowPassModel.value() < m_lowPassModel.maxValue();
	const bool monoBassOn = m_monoBass.begin(m_monoBassFrequencyModel.value(), m_monoBassWidthModel.value() / 100.f,
		sampleRate);
	const bool gateOn = m_gateModel.value() > m_gateModel.minValue();
	const bool compressorOn = m_compressorModel.value() < 0.f;
	const double rate = sampleRate;
	m_eq.begin({m_eqLowModel.value(), m_eqLowMidModel.value(), m_eqHighMidModel.value(), m_eqHighModel.value()}, rate);
	const bool eqOn = m_eq.isActive();

	// A section switched off starts afresh when switched on again
	if (!lowCut) { m_lowCut2.reset(); m_lowCut1.reset(); }
	if (!highPassOn) { m_highPass.reset(); }
	if (!lowPassOn) { m_lowPass.reset(); }
	if (!gateOn) { m_gateGain = 1.f; m_gateHold = 0; }
	if (!compressorOn) { m_compressorEnvelope = 0.f; }

	m_lowCut2.setTarget(Biquad::highPass(LowCutFrequency, 1.0, rate));
	m_lowCut1.setTarget(Biquad::highPass1(LowCutFrequency, rate));
	m_highPass.setTarget(Biquad::highPass(m_highPassModel.value(), ButterworthQ, rate));
	m_lowPass.setTarget(Biquad::lowPass(m_lowPassModel.value(), ButterworthQ, rate));

	const auto count = frames.size();
	auto progressAt = [count](std::size_t f) {
		return count > 1 ? static_cast<float>(f) / static_cast<float>(count - 1) : 1.f;
	};

	// The GAIN pot, ramped over the period so that turning it does not click
	const float gain = potGain(m_gainModel.value());
	if (gain != 1.f || m_gain != 1.f)
	{
		for (auto f = std::size_t{0}; f < count; ++f)
		{
			const float g = m_gain + (gain - m_gain) * progressAt(f);
			frames[f][0] *= g;
			frames[f][1] *= g;
		}
	}
	m_gain = gain;

	// The low cut comes before the gain stage, so that the op-amp or the tube gets no rumble to saturate on
	if (lowCut)
	{
		for (auto f = std::size_t{0}; f < count; ++f)
		{
			for (ch_cnt_t ch = 0; ch < 2; ++ch)
			{
				const double progress = progressAt(f);
				frames[f][ch] = static_cast<float>(m_lowCut1.process(m_lowCut2.process(frames[f][ch], ch, progress), ch,
					progress));
			}
		}
	}

	const auto drive = static_cast<Drive>(m_driveModel.value());
	m_gainStage.process(frames, drive, m_realisticModel.value(), sampleRate);

	// After the input falls silent, the filters ring out for a while
	const bool anyFilter = lowCut || highPassOn || lowPassOn || monoBassOn || eqOn
		|| GainStage::isTube(drive); // the tube's coupling
	if (!anyFilter) { m_tailFrames = 0; }
	else if (hadInput) { m_tailFrames = static_cast<f_cnt_t>(FilterTail * rate); }
	else { m_tailFrames = m_tailFrames > frames.size() ? m_tailFrames - frames.size() : 0; }

	// Nothing after the gain stage to do
	if (!highPassOn && !lowPassOn && !monoBassOn && !gateOn && !compressorOn && !eqOn)
	{
		finishFilters();
		return;
	}

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

	for (auto f = std::size_t{0}; f < count; ++f)
	{
		auto& frame = frames[f];
		const float progress = progressAt(f);
		for (ch_cnt_t ch = 0; ch < 2; ++ch)
		{
			float x = frame[ch];
			if (highPassOn) { x = static_cast<float>(m_highPass.process(x, ch, progress)); }
			if (lowPassOn) { x = static_cast<float>(m_lowPass.process(x, ch, progress)); }
			frame[ch] = x;
		}
		if (monoBassOn)
		{
			float mid = (frame[0] + frame[1]) * 0.5f;
			float side = (frame[0] - frame[1]) * 0.5f;
			m_monoBass.process(mid, side, progress);
			frame[0] = mid + side;
			frame[1] = mid - side;
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
			if (eqOn) { x = static_cast<float>(m_eq.process(x, ch, progress)); }
			frame[ch] = x;
		}
	}

	if (monoBassOn) { m_monoBass.end(); }
	finishFilters();
}




void AnalogChannelStrip::finishFilters()
{
	for (auto filter : {&m_lowCut2, &m_lowCut1, &m_highPass, &m_lowPass}) { filter->finish(); }
	m_eq.finish();
}




auto AnalogChannelStrip::holdsSignal() const -> bool
{
	return m_tailFrames > 0;
}




void AnalogChannelStrip::reset()
{
	for (auto filter : {&m_lowCut2, &m_lowCut1, &m_highPass, &m_lowPass}) { filter->reset(); }
	m_eq.reset();
	m_tailFrames = 0;
	m_monoBass.reset();
	m_gainStage.reset();
	m_gateGain = 1.f;
	m_gateHold = 0;
	m_compressorEnvelope = 0.f;
}




void AnalogChannelStrip::setNeutral()
{
	m_gainModel.setValue(0.5f);
	m_driveModel.setValue(static_cast<int>(Drive::OpAmp4558));
	m_realisticModel.setValue(false);
	m_lowCutModel.setValue(false);
	m_highPassModel.setValue(m_highPassModel.minValue());
	m_lowPassModel.setValue(m_lowPassModel.maxValue());
	m_monoBassWidthModel.setValue(100.f);
	m_monoBassFrequencyModel.setValue(120.f);
	m_gateModel.setValue(m_gateModel.minValue());
	m_compressorModel.setValue(0.f);
	m_eqHighModel.setValue(0.f);
	m_eqHighMidModel.setValue(0.f);
	m_eqLowMidModel.setValue(0.f);
	m_eqLowModel.setValue(0.f);
}




void AnalogChannelStrip::setDisplayNames(const QString& channelName)
{
	const auto prefix = channelName + ">";
	m_gainModel.setDisplayName(prefix + QObject::tr("Gain"));
	m_driveModel.setDisplayName(prefix + QObject::tr("Gain stage"));
	m_realisticModel.setDisplayName(prefix + QObject::tr("Realistic tube"));
	m_lowCutModel.setDisplayName(prefix + QObject::tr("Low cut"));
	m_highPassModel.setDisplayName(prefix + QObject::tr("High-pass"));
	m_lowPassModel.setDisplayName(prefix + QObject::tr("Low-pass"));
	m_monoBassWidthModel.setDisplayName(prefix + QObject::tr("Mono bass width"));
	m_monoBassFrequencyModel.setDisplayName(prefix + QObject::tr("Mono bass frequency"));
	m_gateModel.setDisplayName(prefix + QObject::tr("Gate"));
	m_compressorModel.setDisplayName(prefix + QObject::tr("Compressor"));
	m_eqHighModel.setDisplayName(prefix + QObject::tr("EQ high"));
	m_eqHighMidModel.setDisplayName(prefix + QObject::tr("EQ high mid"));
	m_eqLowMidModel.setDisplayName(prefix + QObject::tr("EQ low mid"));
	m_eqLowModel.setDisplayName(prefix + QObject::tr("EQ low"));
}




void AnalogChannelStrip::saveSettings(QDomDocument& doc, QDomElement& channelElement)
{
	auto strip = doc.createElement("analogstrip");
	m_gainModel.saveSettings(doc, strip, "gain");
	m_driveModel.saveSettings(doc, strip, "drive");
	m_realisticModel.saveSettings(doc, strip, "realistic");
	m_lowCutModel.saveSettings(doc, strip, "lowcut");
	m_highPassModel.saveSettings(doc, strip, "highpass");
	m_lowPassModel.saveSettings(doc, strip, "lowpass");
	m_monoBassWidthModel.saveSettings(doc, strip, "monobass");
	m_monoBassFrequencyModel.saveSettings(doc, strip, "monobassfreq");
	m_gateModel.saveSettings(doc, strip, "gate");
	m_compressorModel.saveSettings(doc, strip, "compressor");
	m_eqHighModel.saveSettings(doc, strip, "eqhigh");
	m_eqHighMidModel.saveSettings(doc, strip, "eqhighmid");
	m_eqLowMidModel.saveSettings(doc, strip, "eqlowmid");
	m_eqLowModel.saveSettings(doc, strip, "eqlow");
	channelElement.appendChild(strip);
}




void AnalogChannelStrip::loadSettings(const QDomElement& channelElement)
{
	setNeutral();
	const auto strip = channelElement.firstChildElement("analogstrip");
	if (strip.isNull()) { return; }
	if (strip.hasAttribute("drive") || !strip.firstChildElement("drive").isNull())
	{
		m_driveModel.loadSettings(strip, "drive");
	}
	// The TUBE button of before
	else if (strip.attribute("tube", "0").toInt() != 0) { m_driveModel.setValue(static_cast<int>(Drive::Tube12AX7)); }
	m_gainModel.loadSettings(strip, "gain");
	m_realisticModel.loadSettings(strip, "realistic");
	m_lowCutModel.loadSettings(strip, "lowcut");
	m_highPassModel.loadSettings(strip, "highpass");
	m_lowPassModel.loadSettings(strip, "lowpass");
	m_monoBassWidthModel.loadSettings(strip, "monobass");
	m_monoBassFrequencyModel.loadSettings(strip, "monobassfreq");
	m_gateModel.loadSettings(strip, "gate");
	m_compressorModel.loadSettings(strip, "compressor");
	m_eqHighModel.loadSettings(strip, "eqhigh");
	m_eqHighMidModel.loadSettings(strip, "eqhighmid");
	m_eqLowMidModel.loadSettings(strip, "eqlowmid");
	m_eqLowModel.loadSettings(strip, "eqlow");
}

} // namespace lmms
