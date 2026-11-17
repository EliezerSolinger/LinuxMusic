/*
 * AtomicPeak.h
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

#ifndef LMMS_ATOMIC_PEAK_H
#define LMMS_ATOMIC_PEAK_H

#include <atomic>
#include <optional>

namespace lmms
{

/**
 * A level shared by the audio thread, which raises or sets it every period, and a meter in the GUI, which takes
 * it (reads and resets it) every frame. Lock-free, so the audio thread never waits; a NaN never raises it.
 */
class AtomicPeak
{
public:
	constexpr AtomicPeak(float initial = 0.f) noexcept : m_value{initial} {}

	AtomicPeak(const AtomicPeak&) = delete;
	auto operator=(const AtomicPeak&) -> AtomicPeak& = delete;

	//! Raises the peak to @p value if it is higher
	void raise(float value) noexcept
	{
		auto current = m_value.load(std::memory_order_relaxed);
		while (value > current && !m_value.compare_exchange_weak(current, value, std::memory_order_relaxed)) {}
	}

	void set(float value) noexcept { m_value.store(value, std::memory_order_relaxed); }

	auto value() const noexcept -> float { return m_value.load(std::memory_order_relaxed); }

	//! @returns the peak, which becomes @p reset
	auto take(float reset = 0.f) noexcept -> float { return m_value.exchange(reset, std::memory_order_relaxed); }

	//! @returns the peak if it is above @p threshold, which then becomes @p reset; nothing (and no change) else
	auto takeAbove(float threshold, float reset = 0.f) noexcept -> std::optional<float>
	{
		auto current = m_value.load(std::memory_order_relaxed);
		while (current > threshold)
		{
			if (m_value.compare_exchange_weak(current, reset, std::memory_order_relaxed)) { return current; }
		}
		return std::nullopt;
	}

private:
	std::atomic<float> m_value;
};

} // namespace lmms

#endif // LMMS_ATOMIC_PEAK_H
