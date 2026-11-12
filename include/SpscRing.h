/*
 * SpscRing.h - a wait-free ring buffer for one writer thread and one reader thread
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

#ifndef LMMS_SPSC_RING_H
#define LMMS_SPSC_RING_H

#include <algorithm>
#include <atomic>
#include <bit>
#include <cstddef>
#include <vector>

namespace lmms
{

/**
 * A ring of values from one writer thread (e.g. the audio thread) to one reader thread (e.g. a disk thread). Neither
 * side waits, locks or allocates: the storage is allocated when the ring is made. The writer writes what fits and
 * learns how much did; the reader reads what is there.
 */
template<class T>
class SpscRing
{
public:
	//! A ring that holds at least @p capacity values
	explicit SpscRing(std::size_t capacity)
		: m_storage(std::bit_ceil(std::max<std::size_t>(capacity, 2)))
		, m_mask(m_storage.size() - 1)
	{
	}

	auto capacity() const -> std::size_t { return m_storage.size(); }

	//! Writer: how many values fit now
	auto writeSpace() const -> std::size_t
	{
		return capacity() - (m_written.load(std::memory_order_relaxed) - m_read.load(std::memory_order_acquire));
	}

	//! Reader: how many values are there to read
	auto readSpace() const -> std::size_t
	{
		return m_written.load(std::memory_order_acquire) - m_read.load(std::memory_order_relaxed);
	}

	//! Writer: writes up to @p count values from @p source (or @p count copies of @p fill if @p source is null)
	//! @returns how many were written
	auto write(const T* source, std::size_t count, const T& fill = T{}) -> std::size_t
	{
		const auto written = m_written.load(std::memory_order_relaxed);
		count = std::min(count, writeSpace());
		for (std::size_t index = 0; index < count; ++index)
		{
			m_storage[(written + index) & m_mask] = source ? source[index] : fill;
		}
		m_written.store(written + count, std::memory_order_release);
		return count;
	}

	//! Reader: reads up to @p count values into @p target
	//! @returns how many were read
	auto read(T* target, std::size_t count) -> std::size_t
	{
		const auto read = m_read.load(std::memory_order_relaxed);
		count = std::min(count, readSpace());
		for (std::size_t index = 0; index < count; ++index)
		{
			target[index] = m_storage[(read + index) & m_mask];
		}
		m_read.store(read + count, std::memory_order_release);
		return count;
	}

private:
	std::vector<T> m_storage;
	std::size_t m_mask;
	std::atomic<std::size_t> m_written = 0; //!< values written so far (only the writer changes it)
	std::atomic<std::size_t> m_read = 0; //!< values read so far (only the reader changes it)
};

} // namespace lmms

#endif // LMMS_SPSC_RING_H
