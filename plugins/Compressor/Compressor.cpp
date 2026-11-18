/*
 * Compressor.cpp
 *
 * Copyright (c) 2020 Lost Robot <r94231@gmail.com>
 *
 * This file is part of LMMS - https://lmms.io
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

#include "Compressor.h"

#include <cmath>
#include <numbers>

#include "embed.h"
#include "lmms_math.h"
#include "plugin_export.h"

namespace lmms
{


extern "C"
{

Plugin::Descriptor PLUGIN_EXPORT compressor_plugin_descriptor =
{
	LMMS_STRINGIFY(PLUGIN_NAME),
	"Compressor",
	QT_TRANSLATE_NOOP("PluginBrowser", "A dynamic range compressor."),
	"Lost Robot <r94231@gmail.com>",
	0x0100,
	Plugin::Type::Effect,
	new PixmapLoader("lmms-plugin-logo"),
	nullptr,
	nullptr,
} ;

}


CompressorEffect::CompressorEffect(Model* parent, const Descriptor::SubPluginFeatures::Key* key) :
	LegacyEffect(&compressor_plugin_descriptor, parent, key),
	m_compressorControls(this)
{
	m_sampleRate = Engine::audioEngine()->outputSampleRate();

	m_yL[0] = m_yL[1] = COMP_NOISE_FLOOR;

	// 200 ms
	m_crestTimeConst = std::exp(-1.f / (0.2f * m_sampleRate));

	// The knobs reach the processing on the audio thread, at the start of process() (applyKnobChanges())

	connect(Engine::audioEngine(), SIGNAL(sampleRateChanged()), this, SLOT(changeSampleRate()));
	changeSampleRate();
}




float CompressorEffect::msToCoeff(float ms)
{
	// Convert time in milliseconds to applicable lowpass coefficient
	return std::exp(m_coeffPrecalc / ms);
}



void CompressorEffect::calcAutoMakeup()
{
	// Formulas using the compressor's Threshold, Ratio, and Knee values to estimate a good makeup gain value

	const float thresholdVal = m_thresholdVal.load(std::memory_order_relaxed);
	const float kneeVal = m_kneeVal.load(std::memory_order_relaxed);
	const float ratioVal = m_ratioVal.load(std::memory_order_relaxed);
	float tempGainResult;
	if (-thresholdVal < kneeVal)
	{
		const float temp = -thresholdVal + kneeVal;
		tempGainResult = ((m_compressorControls.m_limiterModel.value() ? 0 : ratioVal) - 1) * temp * temp / (4 * kneeVal);
	}
	else// Above knee
	{
		tempGainResult = m_compressorControls.m_limiterModel.value()
			? thresholdVal
			: thresholdVal - thresholdVal * ratioVal;
	}

	m_autoMakeupVal = 1.f / dbfsToAmp(tempGainResult);
}



void CompressorEffect::calcAttack()
{
	m_attCoeff = msToCoeff(m_compressorControls.m_attackModel.value());
}

void CompressorEffect::calcRelease()
{
	m_relCoeff = msToCoeff(m_compressorControls.m_releaseModel.value());
}

void CompressorEffect::calcAutoAttack()
{
	m_autoAttVal = m_compressorControls.m_autoAttackModel.value() * 0.01f;
}

void CompressorEffect::calcAutoRelease()
{
	m_autoRelVal = m_compressorControls.m_autoReleaseModel.value() * 0.01f;
}

void CompressorEffect::calcHold()
{
	m_holdLength = m_compressorControls.m_holdModel.value() * 0.001f * m_sampleRate;
	m_holdTimer[0] = 0;
	m_holdTimer[1] = 0;
}

void CompressorEffect::calcOutGain()
{
	// 0.999 is needed to keep the values from crossing the threshold all the time
	// (most commonly for limiters specifically), and is kept across all modes for consistency.
	m_outGainVal = dbfsToAmp(m_compressorControls.m_outGainModel.value()) * 0.999;
}

void CompressorEffect::calcRatio()
{
	m_ratioVal.store(1.f / m_compressorControls.m_ratioModel.value(), std::memory_order_relaxed);
	m_redrawKnee = true;
}

void CompressorEffect::calcRange()
{
	// Range is inactive when turned all the way down
	m_rangeVal = (m_compressorControls.m_rangeModel.value() > m_compressorControls.m_rangeModel.minValue())
			? dbfsToAmp(m_compressorControls.m_rangeModel.value())
			: 0;
}

void CompressorEffect::resizeRMS()
{
	const float rmsValue = m_compressorControls.m_rmsModel.value();
	m_rmsTimeConst = (rmsValue > 0) ? std::exp(-1.f / (rmsValue * 0.001f * m_sampleRate)) : 0;
}

void CompressorEffect::calcLookaheadLength()
{
	m_lookaheadLength = std::ceil((m_compressorControls.m_lookaheadLengthModel.value() / 1000.f) * m_sampleRate);
}

void CompressorEffect::calcThreshold()
{
	const auto threshold = m_compressorControls.m_thresholdModel.value();
	m_thresholdVal.store(threshold, std::memory_order_relaxed);
	m_thresholdAmpVal = dbfsToAmp(threshold);
	m_redrawKnee = true;
	m_redrawThreshold = true;
}

void CompressorEffect::calcKnee()
{
	m_kneeVal.store(m_compressorControls.m_kneeModel.value() * 0.5f, std::memory_order_relaxed);
	m_redrawKnee = true;
}

void CompressorEffect::calcInGain()
{
	m_inGainVal = dbfsToAmp(m_compressorControls.m_inGainModel.value());
}

void CompressorEffect::redrawKnee()
{
	m_redrawKnee = true;
}

void CompressorEffect::calcTiltCoeffs()
{
	using namespace std::numbers;
	m_tiltVal = m_compressorControls.m_tiltModel.value();

	constexpr float amp = 6.f / ln2_v<float>;

	constexpr float gfactor = 5;
	const float g1 = m_tiltVal > 0 ? -gfactor * m_tiltVal : -m_tiltVal;
	const float g2 = m_tiltVal > 0 ? m_tiltVal : gfactor * m_tiltVal;

	m_lgain = std::exp(g1 / amp) - 1;
	m_hgain = std::exp(g2 / amp) - 1;

	const float omega = 2 * pi_v<float> * m_compressorControls.m_tiltFreqModel.value();
	const float n = 1 / (m_sampleRate * 3 + omega);
	m_a0 = 2 * omega * n;
	m_b1 = (m_sampleRate * 3 - omega) * n;
}

void CompressorEffect::calcMix()
{
	m_mixVal = m_compressorControls.m_mixModel.value() * 0.01;
}



void CompressorEffect::applyKnobChanges()
{
	// (every flag taken: | rather than ||)
	auto& c = m_compressorControls;
	if (c.m_attackModel.isValueChanged()) { calcAttack(); }
	if (c.m_releaseModel.isValueChanged()) { calcRelease(); }
	if (c.m_holdModel.isValueChanged()) { calcHold(); }
	const auto threshold = c.m_thresholdModel.isValueChanged();
	const auto ratio = c.m_ratioModel.isValueChanged();
	const auto knee = c.m_kneeModel.isValueChanged();
	const auto limiter = c.m_limiterModel.isValueChanged();
	if (threshold) { calcThreshold(); }
	if (ratio) { calcRatio(); }
	if (knee) { calcKnee(); }
	if (c.m_rangeModel.isValueChanged()) { calcRange(); }
	if (c.m_rmsModel.isValueChanged()) { resizeRMS(); }
	if (c.m_lookaheadLengthModel.isValueChanged()) { calcLookaheadLength(); }
	if (c.m_outGainModel.isValueChanged()) { calcOutGain(); }
	if (c.m_inGainModel.isValueChanged()) { calcInGain(); }
	if (c.m_tiltModel.isValueChanged() | c.m_tiltFreqModel.isValueChanged()) { calcTiltCoeffs(); }
	if (limiter) { redrawKnee(); }
	if (c.m_mixModel.isValueChanged()) { calcMix(); }
	if (c.m_autoAttackModel.isValueChanged()) { calcAutoAttack(); }
	if (c.m_autoReleaseModel.isValueChanged()) { calcAutoRelease(); }
	if (threshold | ratio | knee | limiter | c.m_autoMakeupModel.isValueChanged()) { calcAutoMakeup(); }
}


auto CompressorEffect::process(ProcessContext& context) -> ProcessStatus
{
	auto inOut = context.inOut().interleavedBuffer();
	m_cleanedBuffers = false;
	applyKnobChanges();

	const float d = dryLevel();
	const float w = wetLevel();

	float lOutPeak = 0.0;
	float rOutPeak = 0.0;
	float lInPeak = 0.0;
	float rInPeak = 0.0;

	const bool midside = m_compressorControls.m_midsideModel.value();
	const bool peakmode = m_compressorControls.m_peakmodeModel.value();
	const float inBalance = m_compressorControls.m_inBalanceModel.value();
	const float outBalance = m_compressorControls.m_outBalanceModel.value();
	const bool limiter = m_compressorControls.m_limiterModel.value();
	const float blend = m_compressorControls.m_blendModel.value();
	const float stereoBalance = m_compressorControls.m_stereoBalanceModel.value();
	const bool autoMakeup = m_compressorControls.m_autoMakeupModel.value();
	const int stereoLink = m_compressorControls.m_stereoLinkModel.value();
	const bool audition = m_compressorControls.m_auditionModel.value();
	const bool feedback = m_compressorControls.m_feedbackModel.value();
	// (once per period: they may change on another thread meanwhile)
	const float thresholdVal = m_thresholdVal.load(std::memory_order_relaxed);
	const float kneeVal = m_kneeVal.load(std::memory_order_relaxed);
	const float ratioVal = m_ratioVal.load(std::memory_order_relaxed);
	// The visualizer's highest envelope and gain over this period
	float displayPeak[2] = {COMP_NOISE_FLOOR, COMP_NOISE_FLOOR};
	float displayGain[2] = {COMP_NOISE_FLOOR, COMP_NOISE_FLOOR};
	const bool lookahead = m_compressorControls.m_lookaheadModel.value();

	for (float* frame : inOut.framesView())
	{
		auto drySignal = std::array{frame[0], frame[1]};
		auto s = std::array{drySignal[0] * m_inGainVal, drySignal[1] * m_inGainVal};

		// Calculate tilt filters, to bias the sidechain to the low or high frequencies
		if (m_tiltVal)
		{
			calcTiltFilter(s[0], s[0], 0);
			calcTiltFilter(s[1], s[1], 1);
		}

		if (midside)// Convert left/right to mid/side
		{
			const float temp = s[0];
			s[0] = (temp + s[1]) * 0.5;
			s[1] = temp - s[1];
		}

		s[0] *= inBalance > 0 ? 1 - inBalance : 1;
		s[1] *= inBalance < 0 ? 1 + inBalance : 1;

		m_gainResult[0] = 0;
		m_gainResult[1] = 0;

		for (int i = 0; i < 2; i++)
		{
			float inputValue = (feedback && !lookahead) ? m_prevOut[i] : s[i];

			// Calculate the crest factor of the audio by diving the peak by the RMS
			m_crestPeakVal[i] = qMax(qMax(COMP_NOISE_FLOOR, inputValue * inputValue), m_crestTimeConst * m_crestPeakVal[i] + (1 - m_crestTimeConst) * (inputValue * inputValue));
			m_crestRmsVal[i] = qMax(COMP_NOISE_FLOOR, m_crestTimeConst * m_crestRmsVal[i] + ((1 - m_crestTimeConst) * (inputValue * inputValue)));
			m_crestFactorVal[i] = m_crestPeakVal[i] / m_crestRmsVal[i];

			m_rmsVal[i] = m_rmsTimeConst * m_rmsVal[i] + ((1 - m_rmsTimeConst) * (inputValue * inputValue));

			// Grab the peak or RMS value
			inputValue = qMax(COMP_NOISE_FLOOR, peakmode ? std::abs(inputValue) : std::sqrt(m_rmsVal[i]));

			float t = inputValue;

			if (t > m_yL[i])// Attack phase
			{
				// We want the "resting value" of our crest factor to be with a sine wave,
				// which with this variable has a value of 2.
				// So, we pull this value down to 0, and multiply it by the percentage of
				// automatic attack control that is applied.  We then add 2 back to it.
				float crestFactorValTemp = ((m_crestFactorVal[i] - 2.f) * m_autoAttVal) + 2.f;

				// Calculate attack value depending on crest factor
				const float att = m_autoAttVal
					? msToCoeff(2.f * m_compressorControls.m_attackModel.value() / (crestFactorValTemp))
					: m_attCoeff;

				m_yL[i] = m_yL[i] * att + (1 - att) * t;
				m_holdTimer[i] = m_holdLength;// Reset hold timer
			}
			else// Release phase
			{
				float crestFactorValTemp = ((m_crestFactorVal[i] - 2.f) * m_autoRelVal) + 2.f;

				const float rel = m_autoRelVal
					? msToCoeff(2.f * m_compressorControls.m_releaseModel.value() / (crestFactorValTemp))
					: m_relCoeff;

				if (m_holdTimer[i])// Don't change peak if hold is being applied
				{
					--m_holdTimer[i];
				}
				else
				{
					m_yL[i] = m_yL[i] * rel + (1 - rel) * t;
				}
			}

			// Keep it above the noise floor
			m_yL[i] = qMax(COMP_NOISE_FLOOR, m_yL[i]);
			
			float scVal = m_yL[i];
			
			if (lookahead)
			{
				const float temp = scVal;
				// Lookahead is calculated by picking the largest value between
				// the current sidechain signal and the delayed sidechain signal.
				scVal = std::max(m_scLookBuf[i][m_lookWrite], m_scLookBuf[i][(m_lookWrite + m_lookBufLength - m_lookaheadLength) % m_lookBufLength]);
				m_scLookBuf[i][m_lookWrite] = temp;
			}

			// For the visualizer
			displayPeak[i] = qMax(scVal, displayPeak[i]);

			const float currentPeakDbfs = ampToDbfs(scVal);

			// Now find the gain change that should be applied,
			// depending on the measured input value.
			if (currentPeakDbfs - thresholdVal < -kneeVal)// Below knee
			{
				m_gainResult[i] = currentPeakDbfs;
			}
			else if (currentPeakDbfs - thresholdVal < kneeVal)// Within knee
			{
				const float temp = currentPeakDbfs - thresholdVal + kneeVal;
				m_gainResult[i] = currentPeakDbfs + ((limiter ? 0 : ratioVal) - 1) * temp * temp / (4 * kneeVal);
			}
			else// Above knee
			{
				m_gainResult[i] = limiter
					? thresholdVal
					: thresholdVal + (currentPeakDbfs - thresholdVal) * ratioVal;
			}

			m_gainResult[i] = dbfsToAmp(m_gainResult[i]) / scVal;
			m_gainResult[i] = qMax(m_rangeVal, m_gainResult[i]);
		}

		switch (static_cast<StereoLinkMode>(stereoLink))
		{
			case StereoLinkMode::Unlinked:
			{
				break;
			}
			case StereoLinkMode::Maximum:
			{
				m_gainResult[0] = m_gainResult[1] = qMin(m_gainResult[0], m_gainResult[1]);
				break;
			}
			case StereoLinkMode::Average:
			{
				m_gainResult[0] = m_gainResult[1] = (m_gainResult[0] + m_gainResult[1]) * 0.5f;
				break;
			}
			case StereoLinkMode::Minimum:
			{
				m_gainResult[0] = m_gainResult[1] = qMax(m_gainResult[0], m_gainResult[1]);
				break;
			}
			case StereoLinkMode::Blend:
			{
				if (blend > 0)// 0 is unlinked
				{
					if (blend <= 1)// Blend to minimum volume
					{
						const float temp1 = qMin(m_gainResult[0], m_gainResult[1]);
						m_gainResult[0] = std::lerp(m_gainResult[0], temp1, blend);
						m_gainResult[1] = std::lerp(m_gainResult[1], temp1, blend);
					}
					else if (blend <= 2)// Blend to average volume
					{
						const float temp1 = qMin(m_gainResult[0], m_gainResult[1]);
						const float temp2 = (m_gainResult[0] + m_gainResult[1]) * 0.5f;
						m_gainResult[0] = std::lerp(temp1, temp2, blend - 1);
						m_gainResult[1] = m_gainResult[0];
					}
					else// Blend to maximum volume
					{
						const float temp1 = (m_gainResult[0] + m_gainResult[1]) * 0.5f;
						const float temp2 = qMax(m_gainResult[0], m_gainResult[1]);
						m_gainResult[0] = std::lerp(temp1, temp2, blend - 2);
						m_gainResult[1] = m_gainResult[0];
					}
				}
				break;
			}
		}

		// Bias compression to the left or right (or mid or side)
		if (stereoBalance != 0)
		{
			m_gainResult[0] = 1 - ((1 - m_gainResult[0]) * (stereoBalance > 0 ? 1 - stereoBalance : 1));
			m_gainResult[1] = 1 - ((1 - m_gainResult[1]) * (stereoBalance < 0 ? 1 + stereoBalance : 1));
		}

		// For visualizer
		displayGain[0] = qMax(m_gainResult[0], displayGain[0]);
		displayGain[1] = qMax(m_gainResult[1], displayGain[1]);

		// Delay the signal by 20 ms via ring buffer if lookahead is enabled
		if (lookahead)
		{
			s[0] = m_inLookBuf[0][m_lookWrite];
			s[1] = m_inLookBuf[1][m_lookWrite];
			m_inLookBuf[0][m_lookWrite] = drySignal[0];
			m_inLookBuf[1][m_lookWrite] = drySignal[1];
		}
		else
		{
			s[0] = drySignal[0];
			s[1] = drySignal[1];
		}

		auto delayedDrySignal = std::array{s[0], s[1]};

		if (midside)// Convert left/right to mid/side
		{
			const float temp = s[0];
			s[0] = (temp + s[1]) * 0.5;
			s[1] = temp - s[1];
		}

		s[0] *= inBalance > 0 ? 1 - inBalance : 1;
		s[1] *= inBalance < 0 ? 1 + inBalance : 1;

		s[0] *= m_gainResult[0] * m_inGainVal * m_outGainVal * (outBalance > 0 ? 1 - outBalance : 1);
		s[1] *= m_gainResult[1] * m_inGainVal * m_outGainVal * (outBalance < 0 ? 1 + outBalance : 1);

		if (midside)// Convert mid/side back to left/right
		{
			const float temp1 = s[0];
			const float temp2 = s[1] * 0.5;
			s[0] = temp1 + temp2;
			s[1] = temp1 - temp2;
		}

		m_prevOut[0] = s[0];
		m_prevOut[1] = s[1];

		// Negate wet signal from dry signal
		if (audition)
		{
			s[0] = (-s[0] + delayedDrySignal[0] * m_outGainVal * m_inGainVal);
			s[1] = (-s[1] + delayedDrySignal[1] * m_outGainVal * m_inGainVal);
		}
		else if (autoMakeup)
		{
			s[0] *= m_autoMakeupVal;
			s[1] *= m_autoMakeupVal;
		}

		// Calculate wet/dry value results
		const float temp1 = delayedDrySignal[0];
		const float temp2 = delayedDrySignal[1];
		frame[0] = d * temp1 + w * s[0];
		frame[1] = d * temp2 + w * s[1];
		frame[0] = (1 - m_mixVal) * temp1 + m_mixVal * frame[0];
		frame[1] = (1 - m_mixVal) * temp2 + m_mixVal * frame[1];

		if (--m_lookWrite < 0) { m_lookWrite = m_lookBufLength - 1; }

		lInPeak = drySignal[0] > lInPeak ? drySignal[0] : lInPeak;
		rInPeak = drySignal[1] > rInPeak ? drySignal[1] : rInPeak;
		lOutPeak = s[0] > lOutPeak ? s[0] : lOutPeak;
		rOutPeak = s[1] > rOutPeak ? s[1] : rOutPeak;
	}

	for (int i = 0; i < 2; ++i)
	{
		m_displayPeak[i].raise(displayPeak[i]);
		m_displayGain[i].raise(displayGain[i]);
		m_currentPeak[i].set(m_yL[i]);
		m_currentGain[i].set(m_gainResult[i]);
	}

	m_compressorControls.m_outPeakL.set(lOutPeak);
	m_compressorControls.m_outPeakR.set(rOutPeak);
	m_compressorControls.m_inPeakL.set(lInPeak);
	m_compressorControls.m_inPeakR.set(rInPeak);

	return ProcessStatus::ContinueIfNotQuiet;
}

void CompressorEffect::processBypassedImpl()
{
	// Clear lookahead buffers and other values when needed
	if (!m_cleanedBuffers)
	{
		m_yL[0] = m_yL[1] = COMP_NOISE_FLOOR;
		m_gainResult[0] = m_gainResult[1] = 1;
		for (int i = 0; i < 2; ++i)
		{
			m_displayPeak[i].set(COMP_NOISE_FLOOR);
			m_displayGain[i].set(COMP_NOISE_FLOOR);
			m_currentPeak[i].set(COMP_NOISE_FLOOR);
			m_currentGain[i].set(1);
		}
		std::fill(std::begin(m_scLookBuf[0]), std::end(m_scLookBuf[0]), COMP_NOISE_FLOOR);
		std::fill(std::begin(m_scLookBuf[1]), std::end(m_scLookBuf[1]), COMP_NOISE_FLOOR);
		std::fill(std::begin(m_inLookBuf[0]), std::end(m_inLookBuf[0]), 0);
		std::fill(std::begin(m_inLookBuf[1]), std::end(m_inLookBuf[1]), 0);
		m_cleanedBuffers = true;
	}
}

inline void CompressorEffect::calcTiltFilter(sample_t inputSample, sample_t &outputSample, int filtNum)
{
	m_tiltOut[filtNum] = m_a0 * inputSample + m_b1 * m_tiltOut[filtNum];
	outputSample = inputSample + m_lgain * m_tiltOut[filtNum] + m_hgain * (inputSample - m_tiltOut[filtNum]);
}



void CompressorEffect::changeSampleRate()
{
	m_sampleRate = Engine::audioEngine()->outputSampleRate();

	m_coeffPrecalc = COMP_LOG / (m_sampleRate * 0.001f);

	// 200 ms
	m_crestTimeConst = std::exp(-1.f / (0.2f * m_sampleRate));

	m_lookBufLength = std::ceil((20.f / 1000.f) * m_sampleRate) + 2;
	for (int i = 0; i < 2; ++i)
	{
		m_inLookBuf[i].resize(m_lookBufLength);
		m_scLookBuf[i].resize(m_lookBufLength, COMP_NOISE_FLOOR);
	}
	m_lookWrite = 0;

	calcThreshold();
	calcKnee();
	calcRatio();
	calcAutoMakeup();// This should be after Threshold, Knee, and Ratio

	calcAttack();
	calcRelease();
	calcRange();
	calcLookaheadLength();
	calcHold();
	resizeRMS();
	calcOutGain();
	calcInGain();
	calcTiltCoeffs();
	calcMix();

	calcAutoAttack();
	calcAutoRelease();
}



extern "C"
{

// necessary for getting instance out of shared lib
PLUGIN_EXPORT Plugin * lmms_plugin_main(Model* parent, void* data)
{
	return new CompressorEffect(parent, static_cast<const Plugin::Descriptor::SubPluginFeatures::Key *>(data));
}

}


} // namespace lmms
