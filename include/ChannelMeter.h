/*
 * ChannelMeter.h - thread-safe peak and RMS levels of a stereo signal
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

#ifndef LMMS_CHANNEL_METER_H
#define LMMS_CHANNEL_METER_H

#include <array>
#include <atomic>
#include <optional>

#include "LmmsTypes.h"
#include "lmms_export.h"

namespace lmms
{

class SampleFrame;

//! Levels of a stereo signal, written by the audio thread and read by the GUI without locks.
//!
//! The audio thread calls measure() once per period. The GUI calls takePeaks() to get the highest
//! peak since its previous call, and rms() for the level averaged over the last ~300 ms.
class LMMS_EXPORT ChannelMeter
{
public:
	//! Audio thread: measures one period of output
	void measure(const SampleFrame* buffer, f_cnt_t frames, sample_rate_t sampleRate);

	//! Audio thread: reports silence (e.g. while muted)
	void measureSilence();

	//! GUI thread: the highest peak of each channel since the previous call, or nothing if no period was
	//! measured since then
	auto takePeaks() -> std::optional<std::array<float, 2>>;

	//! Any thread: RMS level of a channel (0 = left, 1 = right), linear
	auto rms(int channel) const -> float { return m_rms[channel].load(std::memory_order_relaxed); }

	//! Seconds over which the RMS level is averaged
	static constexpr float RmsWindow = 0.3f;

private:
	std::array<std::atomic<float>, 2> m_peak = {0.f, 0.f};
	std::array<std::atomic<float>, 2> m_rms = {0.f, 0.f};
	std::atomic<bool> m_fresh = false;

	// Audio thread only
	std::array<float, 2> m_meanSquare = {0.f, 0.f};
};

} // namespace lmms

#endif // LMMS_CHANNEL_METER_H
