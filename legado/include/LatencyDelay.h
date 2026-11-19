/*
 * LatencyDelay.h - delays audio by a whole number of frames, for latency compensation
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

#ifndef LMMS_LATENCY_DELAY_H
#define LMMS_LATENCY_DELAY_H

#include <algorithm>
#include <vector>

#include "LmmsTypes.h"

namespace lmms
{

/**
 * Delays a few planar channels by a whole number of frames. Each period, process() every channel, then advance().
 * With a delay of 0, process() copies.
 */
class LatencyDelay
{
public:
	auto delay() const -> f_cnt_t { return m_delay; }

	//! Sets the delay and the channel count, dropping what was delayed. Allocates when either changes.
	void setDelay(f_cnt_t frames, ch_cnt_t channels)
	{
		if (frames == m_delay && channels == m_lines.size()) { return; }
		m_delay = frames;
		m_lines.assign(channels, std::vector<float>(frames, 0.f));
		m_position = 0;
		m_pending = 0;
	}

	//! Drops what was delayed
	void clear()
	{
		for (auto& line : m_lines) { std::fill(line.begin(), line.end(), 0.f); }
		m_pending = 0;
	}

	//! Whether delayed audio is still to come out, so the channels must be processed even without input
	auto holdsSignal() const -> bool { return m_pending > 0; }

	//! Delays @p frames frames of channel @p channel from @p in to @p out, which may be the same buffer
	void process(ch_cnt_t channel, const float* in, float* out, f_cnt_t frames)
	{
		if (m_delay == 0 || channel >= m_lines.size())
		{
			if (in != out) { std::copy_n(in, frames, out); }
			return;
		}
		auto& line = m_lines[channel];
		auto position = m_position;
		for (f_cnt_t frame = 0; frame < frames; ++frame)
		{
			const float delayed = line[position];
			line[position] = in[frame];
			out[frame] = delayed;
			if (++position == m_delay) { position = 0; }
		}
	}

	//! Ends a period of @p frames frames; @p hadInput tells whether its input had signal
	void advance(f_cnt_t frames, bool hadInput)
	{
		if (m_delay == 0) { return; }
		m_position = (m_position + frames) % m_delay;
		m_pending = hadInput ? m_delay : (m_pending > frames ? m_pending - frames : 0);
	}

private:
	f_cnt_t m_delay = 0;
	std::vector<std::vector<float>> m_lines;
	f_cnt_t m_position = 0;
	f_cnt_t m_pending = 0; //!< frames of delayed input still in the lines
};

} // namespace lmms

#endif // LMMS_LATENCY_DELAY_H
