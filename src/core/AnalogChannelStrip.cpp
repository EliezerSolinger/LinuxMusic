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

#include "OversamplingHelpers.h"
#include "SampleFrame.h"
#include "lmms_math.h"

namespace lmms
{

namespace
{

#include "r12ax7.h"

// The gain stage
constexpr int OversamplingStages = 2; // 4 times
constexpr int Oversampling = 1 << OversamplingStages;
//! Volts per unit of the signal: the op-amp's rail (OpAmpRail) is a swing of 13 V on a +-15 V supply
constexpr float VoltsPerUnit = 13.f / AnalogChannelStrip::OpAmpRail;
constexpr float OpAmpSlewRate = 1.7e6f / VoltsPerUnit; // 1.7 V/us, the 4558's, in units per second
//! Tube input per unit of the signal: the tube starts to clip at +12 dBFS
constexpr float TubeDrive = (float(r12AX7::Zero) / float(r12AX7::Samples - r12AX7::Zero)) / 4.f;
constexpr float DcBlockerFrequency = 5.f;

//! The 12AX7 stage's output voltage for input @p a (1 = where it clips on the positive side), interpolated from the
//! sampled SPICE simulation, as in CAPS
auto tubeTransfer(float a) -> float
{
	a = r12AX7::Zero + a * (r12AX7::Samples - r12AX7::Zero);
	if (a <= 0.f) { return r12AX7::v2v[0]; }
	if (a >= r12AX7::Samples - 1) { return r12AX7::v2v[r12AX7::Samples - 1]; }
	const auto i = static_cast<int>(a);
	const float fraction = a - static_cast<float>(i);
	return r12AX7::v2v[i] * (1.f - fraction) + r12AX7::v2v[i + 1] * fraction;
}

// Fixed parts of the strip
constexpr float LowCutFrequency = 80.f;
constexpr float FilterQ = std::numbers::sqrt2_v<float> / 2.f;
constexpr float EqLowFrequency = 80.f;
constexpr float EqHighFrequency = 12000.f;
constexpr float EqHighMidFrequency = 2500.f;
constexpr float EqLowMidFrequency = 400.f;
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
	: m_tubeModel(false, parent)
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
	m_drive = std::make_unique<DriveStage>();
}




struct AnalogChannelStrip::DriveStage
{
	std::array<Upsampler<OversamplingStages>, 2> upsamplers;
	std::array<Downsampler<OversamplingStages>, 2> downsamplers;
	std::array<float, 2> slewed{}; //!< the op-amp's output, which the slew rate limits
	EqHp12Filter dcBlocker;
	sample_rate_t sampleRate = 0;
	bool oversampling = false;	//!< whether the oversamplers carry signal
	bool tube = false;			//!< what the oversamplers last carried

	void setUp(sample_rate_t rate)
	{
		sampleRate = rate;
		for (auto& upsampler : upsamplers) { upsampler.setup(OversamplingStages, static_cast<float>(rate)); }
		for (auto& downsampler : downsamplers) { downsampler.setup(OversamplingStages, static_cast<float>(rate)); }
		dcBlocker.setParameters(static_cast<float>(rate), DcBlockerFrequency, std::numbers::sqrt2_v<float> / 2.f, 1.f);
		oversampling = false;
	}

	//! Empties the oversamplers, so that they start afresh
	void clear()
	{
		for (auto& upsampler : upsamplers) { upsampler.reset(); }
		for (auto& downsampler : downsamplers) { downsampler.reset(); }
		dcBlocker.clearHistory();
		oversampling = false;
	}
};




AnalogChannelStrip::~AnalogChannelStrip() = default;




auto AnalogChannelStrip::opAmp(float sample) -> float
{
	const float scale = sample < 0.f ? OpAmpNegativeRailRatio : 1.f;
	const float knee = OpAmpKnee * scale;
	const float magnitude = std::abs(sample);
	if (magnitude <= knee) { return sample; }
	const float span = (OpAmpRail - OpAmpKnee) * scale;
	return std::copysign(knee + span * std::tanh((magnitude - knee) / span), sample);
}




auto AnalogChannelStrip::tube(float sample) -> float
{
	// Unity gain for small signals, and the same polarity: a tube stage inverts, and sits on an offset
	static const float offset = tubeTransfer(0.f);
	static const float smallSignalGain = (tubeTransfer(1e-3f) - tubeTransfer(-1e-3f)) / 2e-3f;
	return (tubeTransfer(sample * TubeDrive) - offset) / (smallSignalGain * TubeDrive);
}




void AnalogChannelStrip::processDriveStage(std::span<SampleFrame> frames, sample_rate_t sampleRate)
{
	auto& drive = *m_drive;
	if (drive.sampleRate != sampleRate) { drive.setUp(sampleRate); }
	const bool tubeOn = m_tubeModel.value();
	if (tubeOn != drive.tube)
	{
		drive.clear();
		drive.tube = tubeOn;
	}

	auto oversampled = std::array<float, Oversampling>{};
	if (tubeOn)
	{
		for (auto& frame : frames)
		{
			for (ch_cnt_t ch = 0; ch < 2; ++ch)
			{
				drive.upsamplers[ch].processSample(oversampled.data(), frame[ch]);
				for (auto& sample : oversampled) { sample = tube(sample); }
				frame[ch] = drive.dcBlocker.update(drive.downsamplers[ch].processSample(oversampled.data()), ch, 1.f);
			}
		}
		drive.oversampling = true;
		return;
	}

	// Op-amp: far below the knee, nothing to do
	float peak = 0.f;
	for (const auto& frame : frames) { peak = std::max({peak, std::abs(frame[0]), std::abs(frame[1])}); }
	if (peak < OpAmpKnee / 4.f)
	{
		if (drive.oversampling) { drive.clear(); }
		for (const auto& frame : frames) { drive.slewed = {frame[0], frame[1]}; }
		return;
	}

	const float maxStep = OpAmpSlewRate / (static_cast<float>(sampleRate) * Oversampling);
	for (auto& frame : frames)
	{
		for (ch_cnt_t ch = 0; ch < 2; ++ch)
		{
			// What the op-amp changes, at the oversampled rate
			drive.upsamplers[ch].processSample(oversampled.data(), frame[ch]);
			for (auto& sample : oversampled)
			{
				const float target = opAmp(sample);
				drive.slewed[ch] += std::clamp(target - drive.slewed[ch], -maxStep, maxStep);
				sample = drive.slewed[ch] - sample;
			}
			frame[ch] += drive.downsamplers[ch].processSample(oversampled.data());
		}
	}
	drive.oversampling = true;
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
	const bool eqLowOn = m_eqLowModel.value() != 0.f;
	const bool eqHighMidOn = m_eqHighMidModel.value() != 0.f;
	const bool eqLowMidOn = m_eqLowMidModel.value() != 0.f;
	const bool eqHighOn = m_eqHighModel.value() != 0.f;

	// A section switched off starts afresh when switched on again
	if (!lowCut) { m_lowCut[0].clearHistory(); m_lowCut[1].clearHistory(); }
	if (!highPassOn) { m_highPass.clearHistory(); }
	if (!lowPassOn) { m_lowPass.clearHistory(); }
	if (!eqLowOn) { m_eqLow.clearHistory(); }
	if (!eqHighMidOn) { m_eqHighMid.clearHistory(); }
	if (!eqLowMidOn) { m_eqLowMid.clearHistory(); }
	if (!eqHighOn) { m_eqHigh.clearHistory(); }
	if (!gateOn) { m_gateGain = 1.f; m_gateHold = 0; }
	if (!compressorOn) { m_compressorEnvelope = 0.f; }

	const auto rate = static_cast<float>(sampleRate);
	for (auto& filter : m_lowCut) { filter.setParameters(rate, LowCutFrequency, FilterQ, 1.f); }
	m_highPass.setParameters(rate, m_highPassModel.value(), FilterQ, 1.f);
	m_lowPass.setParameters(rate, m_lowPassModel.value(), FilterQ, 1.f);
	m_eqLow.setParameters(rate, EqLowFrequency, FilterQ, m_eqLowModel.value());
	m_eqHighMid.setParameters(rate, EqHighMidFrequency, EqMidBandwidth, m_eqHighMidModel.value());
	m_eqLowMid.setParameters(rate, EqLowMidFrequency, EqMidBandwidth, m_eqLowMidModel.value());
	m_eqHigh.setParameters(rate, EqHighFrequency, FilterQ, m_eqHighModel.value());

	const auto count = frames.size();
	auto progressAt = [count](std::size_t f) {
		return count > 1 ? static_cast<float>(f) / static_cast<float>(count - 1) : 1.f;
	};

	// The low cut comes before the gain stage, so that the op-amp or the tube gets no rumble to saturate on
	if (lowCut)
	{
		for (auto f = std::size_t{0}; f < count; ++f)
		{
			for (ch_cnt_t ch = 0; ch < 2; ++ch)
			{
				frames[f][ch] = m_lowCut[1].update(m_lowCut[0].update(frames[f][ch], ch, progressAt(f)), ch,
					progressAt(f));
			}
		}
	}

	processDriveStage(frames, sampleRate);

	// After the input falls silent, the filters ring out for a while
	const bool anyFilter = lowCut || highPassOn || lowPassOn || monoBassOn || eqLowOn || eqLowMidOn || eqHighMidOn
		|| eqHighOn || m_tubeModel.value(); // the tube's DC blocker
	if (!anyFilter) { m_tailFrames = 0; }
	else if (hadInput) { m_tailFrames = static_cast<f_cnt_t>(FilterTail * rate); }
	else { m_tailFrames = m_tailFrames > frames.size() ? m_tailFrames - frames.size() : 0; }

	// Nothing after the gain stage to do
	if (!highPassOn && !lowPassOn && !monoBassOn && !gateOn && !compressorOn && !eqLowOn && !eqLowMidOn
		&& !eqHighMidOn && !eqHighOn)
	{
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
			if (highPassOn) { x = m_highPass.update(x, ch, progress); }
			if (lowPassOn) { x = m_lowPass.update(x, ch, progress); }
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
			if (eqLowOn) { x = m_eqLow.update(x, ch, progress); }
			if (eqLowMidOn) { x = m_eqLowMid.update(x, ch, progress); }
			if (eqHighMidOn) { x = m_eqHighMid.update(x, ch, progress); }
			if (eqHighOn) { x = m_eqHigh.update(x, ch, progress); }
			frame[ch] = x;
		}
	}

	if (monoBassOn) { m_monoBass.end(); }
}




auto AnalogChannelStrip::holdsSignal() const -> bool
{
	return m_tailFrames > 0;
}




void AnalogChannelStrip::reset()
{
	for (auto filter : std::initializer_list<EqFilter*>{&m_lowCut[0], &m_lowCut[1], &m_highPass, &m_lowPass, &m_eqLow,
		&m_eqLowMid, &m_eqHighMid, &m_eqHigh})
	{
		filter->clearHistory();
	}
	m_tailFrames = 0;
	m_monoBass.reset();
	m_drive->clear();
	m_drive->slewed = {};
	m_gateGain = 1.f;
	m_gateHold = 0;
	m_compressorEnvelope = 0.f;
}




void AnalogChannelStrip::setNeutral()
{
	m_tubeModel.setValue(false);
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
	m_tubeModel.setDisplayName(prefix + QObject::tr("Tube"));
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
	m_tubeModel.saveSettings(doc, strip, "tube");
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
	m_tubeModel.loadSettings(strip, "tube");
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
