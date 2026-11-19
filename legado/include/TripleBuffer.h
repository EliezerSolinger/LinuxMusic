/*
 * TripleBuffer.h - hands the latest value from one thread to another, without locks or waits
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

#ifndef LMMS_TRIPLE_BUFFER_H
#define LMMS_TRIPLE_BUFFER_H

#include <array>
#include <atomic>
#include <cstdint>

namespace lmms
{

/**
 * Three slots that hand the latest value from one writer thread to one reader thread, wait-free on both sides: the
 * writer fills its slot and publishes it, the reader takes the latest published slot. Neither ever waits for the
 * other, and neither touches the slot the other is using. Values the reader did not take in time are skipped, which
 * is what displays of the audio (meters, scopes) want.
 *
 * The slots are constructed up front (outside the audio thread), so a writer that only copies into them never
 * allocates.
 */
template<class T>
class TripleBuffer
{
public:
	TripleBuffer() = default;

	//! Every slot starts as a copy of @p initial (e.g. a buffer of the right size)
	explicit TripleBuffer(const T& initial)
		: m_slots{initial, initial, initial}
	{
	}

	//! Sets every slot to @p initial and forgets what was published. Only while neither side is using it.
	void reset(const T& initial)
	{
		m_slots = {initial, initial, initial};
		m_write = 0;
		m_read = 1;
		m_middle.store(2, std::memory_order_release);
	}

	//! Writer: the slot to fill. It is the writer's alone until publish().
	auto writeSlot() -> T& { return m_slots[m_write]; }

	//! Writer: hands the filled slot over as the latest value
	void publish()
	{
		const auto previous = m_middle.exchange(static_cast<std::uint8_t>(m_write | Fresh), std::memory_order_acq_rel);
		m_write = previous & IndexMask;
	}

	/**
	 * Reader: takes the latest published value, if one was published since the last call
	 * @returns whether readSlot() changed
	 */
	auto update() -> bool
	{
		if (!(m_middle.load(std::memory_order_relaxed) & Fresh)) { return false; }
		const auto previous = m_middle.exchange(m_read, std::memory_order_acq_rel);
		m_read = previous & IndexMask;
		return true;
	}

	//! Reader: the latest value taken by update(). It is the reader's alone until the next update().
	auto readSlot() const -> const T& { return m_slots[m_read]; }
	auto readSlot() -> T& { return m_slots[m_read]; }

private:
	static constexpr std::uint8_t IndexMask = 0x3;
	static constexpr std::uint8_t Fresh = 0x4; //!< the middle slot holds a value the reader has not taken

	std::array<T, 3> m_slots{};
	std::uint8_t m_write = 0; //!< the writer's slot
	std::uint8_t m_read = 1; //!< the reader's slot
	std::atomic<std::uint8_t> m_middle = 2; //!< the slot between them, and whether it is fresh
};

} // namespace lmms

#endif // LMMS_TRIPLE_BUFFER_H
