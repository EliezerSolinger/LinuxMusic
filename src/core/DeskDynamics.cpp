/*
 * DeskDynamics.cpp - the compressor and the gate/expander of an analog desk's channel, after the SSL E series
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

#include "lmms_math.h"

namespace lmms
{

namespace
{

constexpr float NominalLevel = 4.f + DbuToDbfs;		//!< dBFS RMS, 0 VU
constexpr float LinearReleaseThreshold = 6.f;		//!< dB LIN REL raises the threshold by
constexpr float CompressorAttack = 0.030f;			//!< s for 20 dB of gain reduction
constexpr float CompressorFastAttack = 0.003f;
constexpr float GateAttack = 0.0015f;				//!< s for 40 dB
constexpr float GateFastAttack = 0.0001f;
constexpr float GateRatio = 20.f;
constexpr float ExpanderRatio = 2.f;
constexpr float GateKeyRelease = 0.01f;				//!< s, of the key's peak envelope, so it does not chatter
constexpr float Silence = -200.f;					//!< dB

auto levelOf(float amplitude) -> float { return amplitude > 1e-10f ? ampToDbfs(amplitude) : Silence; }

} // namespace




auto ChannelCompressor::staticReduction(const Settings& settings, float level) -> float
{
	if (settings.ratio <= 1.f) { return 0.f; }
	const float threshold = settings.threshold + DbuToDbfs + (settings.linearRelease ? LinearReleaseThreshold : 0.f);
	const float slope = std::isinf(settings.ratio) ? 1.f : 1.f - 1.f / settings.ratio;
	const float over = level - threshold;
	if (settings.hardKnee) { return std::max(over, 0.f) * slope; }
	if (over <= -KneeWidth / 2.f) { return 0.f; }
	if (over >= KneeWidth / 2.f) { return over * slope; }
	const float into = over + KneeWidth / 2.f;
	return slope * into * into / (2.f * KneeWidth);
}




auto ChannelCompressor::makeup(const Settings& settings) -> float
{
	return staticReduction(settings, NominalLevel);
}




void ChannelCompressor::begin(const Settings& settings, sample_rate_t sampleRate)
{
	m_settings = settings;
	const auto rate = static_cast<float>(sampleRate);
	m_makeup = makeup(settings);
	m_rmsCoefficient = 1.f - std::exp(-1.f / (RmsTime * rate));
	m_attackStep = 20.f / ((settings.fast ? CompressorFastAttack : CompressorAttack) * rate);
	m_releaseCoefficient = 1.f - std::exp(-1.f / (settings.release * rate));
	m_releaseStep = 20.f / (settings.release * rate);
}




auto ChannelCompressor::gain(float left, float right) -> float
{
	float level;
	if (m_settings.peak) { level = levelOf(std::max(std::abs(left), std::abs(right))); }
	else
	{
		m_meanSquare += (std::max(left * left, right * right) - m_meanSquare) * m_rmsCoefficient;
		level = levelOf(std::sqrt(m_meanSquare));
	}
	const float target = staticReduction(m_settings, level);
	if (target > m_reduction) { m_reduction = std::min(target, m_reduction + m_attackStep); }
	else if (m_settings.linearRelease) { m_reduction = std::max(target, m_reduction - m_releaseStep); }
	else { m_reduction += (target - m_reduction) * m_releaseCoefficient; }
	return dbfsToAmp(m_makeup - m_reduction);
}




void ChannelCompressor::reset()
{
	m_meanSquare = 0.f;
	m_reduction = 0.f;
}




auto ChannelGate::hysteresis(float threshold) -> float
{
	return 2.f + 6.f * std::clamp((10.f - threshold) / 40.f, 0.f, 1.f);
}




void ChannelGate::begin(const Settings& settings, sample_rate_t sampleRate)
{
	m_settings = settings;
	const auto rate = static_cast<float>(sampleRate);
	m_attackStep = 40.f / ((settings.fast ? GateFastAttack : GateAttack) * rate);
	m_releaseCoefficient = 1.f - std::exp(-1.f / (settings.release * rate));
	m_holdFrames = static_cast<f_cnt_t>(settings.hold * rate);
	m_keyRelease = std::exp(-1.f / (GateKeyRelease * rate));
}




auto ChannelGate::gain(float left, float right) -> float
{
	// The key: a peak envelope, quick to rise and briefly held, so that zero crossings do not close the gate
	m_key = std::max(std::max(std::abs(left), std::abs(right)), m_key * m_keyRelease);
	const float level = levelOf(m_key);
	const float threshold = m_settings.threshold + DbuToDbfs;

	if (level >= threshold)
	{
		m_open = true;
		m_holding = m_holdFrames;
	}
	else if (level < threshold - hysteresis(m_settings.threshold))
	{
		if (m_holding > 0) { --m_holding; }
		else { m_open = false; }
	}

	const float ratio = m_settings.expander ? ExpanderRatio : GateRatio;
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
	m_key = 0.f;
	m_reduction = 0.f;
}

} // namespace lmms
