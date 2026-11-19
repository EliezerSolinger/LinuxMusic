/*
 * ChannelMeter.cpp - thread-safe peak and RMS levels of a stereo signal
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

#include "ChannelMeter.h"

#include <cmath>

#include "SampleFrame.h"

namespace lmms
{

namespace
{

//! Raises @p target to @p value if it is higher, without locks
void storeMax(std::atomic<float>& target, float value)
{
	float current = target.load(std::memory_order_relaxed);
	while (value > current && !target.compare_exchange_weak(current, value, std::memory_order_relaxed)) {}
}

} // namespace




void ChannelMeter::measure(const SampleFrame* buffer, f_cnt_t frames, sample_rate_t sampleRate)
{
	if (frames == 0) { return; }

	auto peak = std::array{0.f, 0.f};
	auto sumOfSquares = std::array{0.f, 0.f};
	for (f_cnt_t f = 0; f < frames; ++f)
	{
		for (int ch = 0; ch < 2; ++ch)
		{
			const float sample = buffer[f][ch];
			peak[ch] = std::max(peak[ch], std::abs(sample));
			sumOfSquares[ch] += sample * sample;
		}
	}

	// Exponential average of the mean square over the RMS window
	const float keep = std::exp(-static_cast<float>(frames) / (RmsWindow * sampleRate));
	for (int ch = 0; ch < 2; ++ch)
	{
		m_meanSquare[ch] = m_meanSquare[ch] * keep + sumOfSquares[ch] / frames * (1.f - keep);
		m_rms[ch].store(std::sqrt(m_meanSquare[ch]), std::memory_order_relaxed);
		storeMax(m_peak[ch], peak[ch]);
	}
	m_fresh.store(true, std::memory_order_release);
}




void ChannelMeter::measureSilence()
{
	for (int ch = 0; ch < 2; ++ch)
	{
		m_meanSquare[ch] = 0.f;
		m_rms[ch].store(0.f, std::memory_order_relaxed);
	}
	m_fresh.store(true, std::memory_order_release);
}




auto ChannelMeter::takePeaks() -> std::optional<std::array<float, 2>>
{
	if (!m_fresh.exchange(false, std::memory_order_acquire)) { return std::nullopt; }
	return std::array{
		m_peak[0].exchange(0.f, std::memory_order_relaxed),
		m_peak[1].exchange(0.f, std::memory_order_relaxed)
	};
}

} // namespace lmms
