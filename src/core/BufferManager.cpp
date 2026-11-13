/*
 * BufferManager.cpp - A buffer caching/memory management system
 *
 * Copyright (c) 2017 Lukas W <lukaswhl/at/gmail.com>
 * Copyright (c) 2014 Vesa Kivimäki <contact/dot/diizy/at/nbl/dot/fi>
 * Copyright (c) 2006-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
 *
 * This file is part of LMMS - https://lmms.io
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

#include "BufferManager.h"

#include <atomic>
#include <memory>
#include <utility>
#include <vector>

namespace lmms
{

namespace
{

//! Buffers made when the engine starts: notes, samples and previews playing at once
constexpr std::size_t InitialBuffers = 512;

struct Pool
{
	std::atomic_flag lock = ATOMIC_FLAG_INIT;
	f_cnt_t frames = 0;
	//! All the memory ever made for buffers; kept until the program ends, so a buffer is never freed under its user
	std::vector<std::unique_ptr<SampleFrame[]>> chunks;
	//! The buffers not in use; its capacity holds every buffer made, so giving one back never allocates
	std::vector<SampleFrame*> available;
	std::size_t made = 0;
	long overflows = 0;
};

auto pool() -> Pool&
{
	// Never destroyed: a play handle may give its buffer back while the program ends
	static auto* s_pool = new Pool{};
	return *s_pool;
}

//! Held for a few instructions at a time, by any thread
class PoolLock
{
public:
	PoolLock() : m_pool{pool()}
	{
		// Held for a few steps: spinning, without asking the system to sleep or wake (which the audio thread avoids)
		while (m_pool.lock.test_and_set(std::memory_order_acquire))
		{
			while (m_pool.lock.test(std::memory_order_relaxed)) {}
		}
	}
	~PoolLock()
	{
		m_pool.lock.clear(std::memory_order_release);
	}
	PoolLock(const PoolLock&) = delete;
	auto operator=(const PoolLock&) -> PoolLock& = delete;

	auto operator->() -> Pool* { return &m_pool; }

private:
	Pool& m_pool;
};

//! Makes @p count buffers of the pool's size, available [pool locked]
void grow(PoolLock& locked, std::size_t count)
{
	auto chunk = std::make_unique<SampleFrame[]>(count * locked->frames);
	locked->available.reserve(locked->made + count);
	for (std::size_t index = 0; index < count; ++index)
	{
		locked->available.push_back(chunk.get() + index * locked->frames);
	}
	locked->made += count;
	locked->chunks.push_back(std::move(chunk));
}

} // namespace


BufferManager::Buffer::Buffer(Buffer&& other) noexcept :
	m_data{std::exchange(other.m_data, nullptr)},
	m_frames{std::exchange(other.m_frames, 0)}
{
}

auto BufferManager::Buffer::operator=(Buffer&& other) noexcept -> Buffer&
{
	if (this != &other)
	{
		if (m_data) { release(m_data, m_frames); }
		m_data = std::exchange(other.m_data, nullptr);
		m_frames = std::exchange(other.m_frames, 0);
	}
	return *this;
}

BufferManager::Buffer::~Buffer()
{
	if (m_data) { release(m_data, m_frames); }
}


void BufferManager::init(f_cnt_t fpp)
{
	auto locked = PoolLock{};
	if (locked->frames == fpp && locked->made > 0) { return; }
	// Buffers of another size still in use keep their memory (the chunks stay); they are dropped when given back
	locked->frames = fpp;
	locked->available.clear();
	locked->made = 0;
	grow(locked, InitialBuffers);
}


auto BufferManager::acquire() -> Buffer
{
	auto locked = PoolLock{};
	if (locked->available.empty())
	{
		// More in use at once than the pool was made with: one more buffer, allocated where it should not be
		++locked->overflows;
		grow(locked, 1);
	}
	auto* data = locked->available.back();
	locked->available.pop_back();
	return Buffer{data, locked->frames};
}


void BufferManager::release(SampleFrame* data, f_cnt_t frames)
{
	auto locked = PoolLock{};
	if (frames == locked->frames) { locked->available.push_back(data); }
}


auto BufferManager::overflows() -> long
{
	auto locked = PoolLock{};
	return locked->overflows;
}

} // namespace lmms
