/*
 * FreshVoicePool.h
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

#ifndef LMMS_FRESH_VOICE_POOL_H
#define LMMS_FRESH_VOICE_POOL_H

#include <atomic>
#include <cstddef>
#include <vector>

namespace lmms
{

/**
 * New voices made in advance on the main thread, each used by one note and then deleted on the main thread: for
 * voices a library builds with allocations and file reads, and which cannot be reset to how a new one is (an STK
 * instrument keeps its waves' phases), so that each note sounds as with a voice of its own.
 *
 * take() and retire() may be called from any thread and allocate nothing; refill() makes and deletes, on the main
 * thread (a timer, for instance).
 */
template<class T>
class FreshVoicePool
{
public:
	FreshVoicePool() = default;
	FreshVoicePool(const FreshVoicePool&) = delete;
	auto operator=(const FreshVoicePool&) -> FreshVoicePool& = delete;
	~FreshVoicePool()
	{
		// No note uses one any more
		for (auto* voice : m_fresh) { delete voice; }
		for (auto* voice : m_retired) { delete voice; }
	}

	/**
	 * Deletes the voices given back, and makes new ones with @p make (returning a T*, or none) until @p count are
	 * ready. With @p discardFresh, the ready ones are deleted first (what they were made with changed). [main thread]
	 */
	template<class Make>
	void refill(std::size_t count, Make&& make, bool discardFresh = false)
	{
		auto dropped = std::vector<T*>{};
		std::size_t ready = 0;
		{
			auto lock = Lock{m_lock};
			// Copied out, so that the list keeps its room for the voices given back meanwhile
			dropped.assign(m_retired.begin(), m_retired.end());
			m_retired.clear();
			if (discardFresh)
			{
				dropped.insert(dropped.end(), m_fresh.begin(), m_fresh.end());
				m_fresh.clear();
			}
			ready = m_fresh.size();
		}
		// Made and deleted outside the lock, which the audio thread may be waiting for
		for (auto* voice : dropped) { delete voice; }
		auto made = std::vector<T*>{};
		for (; ready + made.size() < count;)
		{
			auto* voice = make();
			if (!voice) { break; }
			made.push_back(voice);
		}

		auto lock = Lock{m_lock};
		m_fresh.insert(m_fresh.end(), made.begin(), made.end());
		// Room to take back every voice out (and those ready), so that retire() never allocates
		m_retired.reserve(m_out + m_fresh.size() + count);
		m_fresh.reserve(m_fresh.size() + count);
	}

	//! A voice never used, or none when all were taken
	auto take() -> T*
	{
		auto lock = Lock{m_lock};
		if (m_fresh.empty()) { return nullptr; }
		auto* voice = m_fresh.back();
		m_fresh.pop_back();
		++m_out;
		return voice;
	}

	/**
	 * Gives back a voice from take(), used: deleted by the next refill(). @returns false when there was no room for
	 * it (then the caller deletes it)
	 */
	auto retire(T* voice) -> bool
	{
		auto lock = Lock{m_lock};
		if (m_out > 0) { --m_out; }
		if (m_retired.size() == m_retired.capacity()) { return false; }
		m_retired.push_back(voice);
		return true;
	}

	auto ready() -> std::size_t
	{
		auto lock = Lock{m_lock};
		return m_fresh.size();
	}

private:
	//! Held for a few steps by any thread: spinning, without asking the system to sleep or wake
	class Lock
	{
	public:
		explicit Lock(std::atomic_flag& flag) : m_flag{flag}
		{
			while (m_flag.test_and_set(std::memory_order_acquire))
			{
				while (m_flag.test(std::memory_order_relaxed)) {}
			}
		}
		~Lock() { m_flag.clear(std::memory_order_release); }
		Lock(const Lock&) = delete;
		auto operator=(const Lock&) -> Lock& = delete;

	private:
		std::atomic_flag& m_flag;
	};

	std::atomic_flag m_lock = ATOMIC_FLAG_INIT;
	std::vector<T*> m_fresh;
	std::vector<T*> m_retired;
	std::size_t m_out = 0; //!< voices taken and not given back
};

} // namespace lmms

#endif // LMMS_FRESH_VOICE_POOL_H
