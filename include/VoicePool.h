/*
 * VoicePool.h
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

#ifndef LMMS_VOICE_POOL_H
#define LMMS_VOICE_POOL_H

#include <algorithm>
#include <atomic>
#include <cstddef>
#include <memory>
#include <utility>
#include <vector>

namespace lmms
{

/**
 * Voices an instrument makes in advance, for its notes to take on the audio thread without allocating: for voices a
 * library builds with allocations (and file reads) of its own, such as an emulated chip's buffers or an STK
 * instrument.
 *
 * rebuild() makes a new set of voices (for a new sample rate, or other settings voices are built with) on the main
 * thread; a voice of an earlier set in use goes on until its note gives it back, and is deleted by a later rebuild.
 * take() and give() may be called from any thread, and allocate nothing.
 */
template<class T>
class VoicePool
{
public:
	VoicePool() = default;
	VoicePool(const VoicePool&) = delete;
	auto operator=(const VoicePool&) -> VoicePool& = delete;

	/**
	 * Makes @p count voices with @p make (returning std::unique_ptr<T>), which become the ones take() gives; deletes
	 * the voices of earlier sets that are not in use. [main thread]
	 */
	template<class Make>
	void rebuild(std::size_t count, Make&& make)
	{
		// Made and deleted outside the lock, which the audio thread may be waiting for
		auto made = std::vector<std::unique_ptr<T>>{};
		made.reserve(count);
		for (std::size_t index = 0; index < count; ++index)
		{
			if (auto voice = make()) { made.push_back(std::move(voice)); }
		}

		auto dropped = std::vector<std::unique_ptr<T>>{};
		{
			auto lock = Lock{m_lock};
			++m_generation;
			// Of the earlier sets, those not in use go; those in use come back to the retired list
			for (auto& slot : m_slots)
			{
				const bool free = std::find(m_free.begin(), m_free.end(), slot.voice.get()) != m_free.end();
				const bool retired = std::find(m_retired.begin(), m_retired.end(), slot.voice.get()) != m_retired.end();
				if (free || retired) { dropped.push_back(std::move(slot.voice)); }
			}
			std::erase_if(m_slots, [](const Slot& slot) { return !slot.voice; });
			m_free.clear();
			m_retired.clear();
			for (auto& voice : made)
			{
				m_free.push_back(voice.get());
				m_slots.push_back(Slot{std::move(voice), m_generation});
			}
			// Room for every voice in either list, so that giving back never allocates
			m_free.reserve(m_slots.size());
			m_retired.reserve(m_slots.size());
		}
	}

	//! A voice not in use (of the current set), or none when all are taken
	auto take() -> T*
	{
		auto lock = Lock{m_lock};
		if (m_free.empty()) { return nullptr; }
		auto* voice = m_free.back();
		m_free.pop_back();
		return voice;
	}

	//! Gives back a voice from take(): it is taken again, or, from an earlier set, deleted by the next rebuild
	void give(T* voice)
	{
		auto lock = Lock{m_lock};
		const auto slot = std::find_if(m_slots.begin(), m_slots.end(), [voice](const Slot& s) {
			return s.voice.get() == voice;
		});
		if (slot == m_slots.end()) { return; }
		if (slot->generation == m_generation) { m_free.push_back(voice); }
		else { m_retired.push_back(voice); }
	}

	//! The voices not in use
	auto available() -> std::size_t
	{
		auto lock = Lock{m_lock};
		return m_free.size();
	}

private:
	struct Slot
	{
		std::unique_ptr<T> voice;
		unsigned generation;
	};

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
	std::vector<Slot> m_slots; //!< every voice, of every set still around
	std::vector<T*> m_free;
	std::vector<T*> m_retired;
	unsigned m_generation = 0;
};

} // namespace lmms

#endif // LMMS_VOICE_POOL_H
