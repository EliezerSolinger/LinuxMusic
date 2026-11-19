/*
 * LockFreeQueue.h - bounded multi-producer multi-consumer queue without locks
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

#ifndef LMMS_LOCK_FREE_QUEUE_H
#define LMMS_LOCK_FREE_QUEUE_H

#include <array>
#include <atomic>
#include <cstddef>
#include <optional>
#include <type_traits>

namespace lmms
{

/**
 * A bounded queue that any number of threads may push to and pop from without locks and without allocating,
 * so it is real-time safe (Dmitry Vyukov's bounded MPMC queue).
 *
 * @tparam T a trivially copyable type
 * @tparam Capacity a power of two
 */
template<typename T, std::size_t Capacity>
class LockFreeQueue
{
	static_assert(Capacity >= 2 && (Capacity & (Capacity - 1)) == 0, "Capacity must be a power of two");
	static_assert(std::is_trivially_copyable_v<T>);

public:
	LockFreeQueue()
	{
		for (std::size_t i = 0; i < Capacity; ++i) { m_cells[i].sequence.store(i, std::memory_order_relaxed); }
	}

	LockFreeQueue(const LockFreeQueue&) = delete;
	auto operator=(const LockFreeQueue&) -> LockFreeQueue& = delete;

	//! @returns false if the queue is full
	auto tryPush(const T& value) -> bool
	{
		auto pos = m_enqueuePos.load(std::memory_order_relaxed);
		for (;;)
		{
			auto& cell = m_cells[pos & Mask];
			const auto sequence = cell.sequence.load(std::memory_order_acquire);
			const auto diff = static_cast<std::ptrdiff_t>(sequence) - static_cast<std::ptrdiff_t>(pos);
			if (diff == 0)
			{
				if (m_enqueuePos.compare_exchange_weak(pos, pos + 1, std::memory_order_relaxed))
				{
					cell.value = value;
					cell.sequence.store(pos + 1, std::memory_order_release);
					return true;
				}
			}
			else if (diff < 0) { return false; }
			else { pos = m_enqueuePos.load(std::memory_order_relaxed); }
		}
	}

	//! @returns the oldest value, or nothing if the queue is empty
	auto tryPop() -> std::optional<T>
	{
		auto pos = m_dequeuePos.load(std::memory_order_relaxed);
		for (;;)
		{
			auto& cell = m_cells[pos & Mask];
			const auto sequence = cell.sequence.load(std::memory_order_acquire);
			const auto diff = static_cast<std::ptrdiff_t>(sequence) - static_cast<std::ptrdiff_t>(pos + 1);
			if (diff == 0)
			{
				if (m_dequeuePos.compare_exchange_weak(pos, pos + 1, std::memory_order_relaxed))
				{
					const T value = cell.value;
					cell.sequence.store(pos + Capacity, std::memory_order_release);
					return value;
				}
			}
			else if (diff < 0) { return std::nullopt; }
			else { pos = m_dequeuePos.load(std::memory_order_relaxed); }
		}
	}

	static constexpr auto capacity() -> std::size_t { return Capacity; }

private:
	static constexpr std::size_t Mask = Capacity - 1;

	struct Cell
	{
		std::atomic<std::size_t> sequence;
		T value;
	};

	// Separate cache lines for the producers' and consumers' positions
	alignas(64) std::array<Cell, Capacity> m_cells;
	alignas(64) std::atomic<std::size_t> m_enqueuePos = 0;
	alignas(64) std::atomic<std::size_t> m_dequeuePos = 0;
};

} // namespace lmms

#endif // LMMS_LOCK_FREE_QUEUE_H
