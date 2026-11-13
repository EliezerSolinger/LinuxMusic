/*
 * AudioResampler.cpp
 *
 * Copyright (c) 2025 saker <sakertooth@gmail.com>
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

#include "AudioResampler.h"

#include <algorithm>
#include <atomic>
#include <samplerate.h>
#include <stdexcept>
#include <vector>

namespace lmms {

namespace {

constexpr auto converterType(AudioResampler::Mode mode) -> int
{
	switch (mode)
	{
	case AudioResampler::Mode::ZOH:
		return SRC_ZERO_ORDER_HOLD;
	case AudioResampler::Mode::Linear:
		return SRC_LINEAR;
	case AudioResampler::Mode::SincFastest:
		return SRC_SINC_FASTEST;
	case AudioResampler::Mode::SincMedium:
		return SRC_SINC_MEDIUM_QUALITY;
	case AudioResampler::Mode::SincBest:
		return SRC_SINC_BEST_QUALITY;
	default:
		throw std::invalid_argument{"Invalid interpolation mode"};
	}
}
//! The states made in advance for each mode, mono and stereo (see AudioResampler::reserve())
constexpr std::size_t ModeCount = 5;
constexpr ch_cnt_t PooledChannels = 2;

struct StatePool
{
	std::vector<SRC_STATE*> states; //!< its room holds every state made for it, so giving one back never allocates
	std::size_t target = 0;
	std::size_t made = 0;
};

struct Pools
{
	std::atomic_flag lock = ATOMIC_FLAG_INIT;
	StatePool pools[ModeCount][PooledChannels];
	std::atomic<bool> wantsRefill = false;
	std::atomic<long> misses = 0;
};

auto pools() -> Pools&
{
	static auto* s_pools = new Pools{}; // never destroyed: a resampler may go while the program ends
	return *s_pools;
}

class PoolLock
{
public:
	PoolLock() : m_pools{pools()}
	{
		// Held for a few steps: spinning, without asking the system to sleep or wake (which the audio thread avoids)
		while (m_pools.lock.test_and_set(std::memory_order_acquire))
		{
			while (m_pools.lock.test(std::memory_order_relaxed)) {}
		}
	}
	~PoolLock()
	{
		m_pools.lock.clear(std::memory_order_release);
	}
	PoolLock(const PoolLock&) = delete;
	auto operator=(const PoolLock&) -> PoolLock& = delete;

private:
	Pools& m_pools;
};

auto poolOf(AudioResampler::Mode mode, ch_cnt_t channels) -> StatePool*
{
	const auto index = static_cast<std::size_t>(mode);
	if (index >= ModeCount || channels < 1 || channels > PooledChannels) { return nullptr; }
	return &pools().pools[index][channels - 1];
}

//! Makes states until @p pool holds @p count [pool locked]
void fill(StatePool& pool, AudioResampler::Mode mode, ch_cnt_t channels, std::size_t count)
{
	while (pool.states.size() < count)
	{
		int error = 0;
		auto* state = src_new(converterType(mode), channels, &error);
		if (!state) { return; }
		++pool.made;
		pool.states.reserve(pool.made);
		pool.states.push_back(state);
	}
}

} // namespace

AudioResampler::AudioResampler(Mode mode, ch_cnt_t channels)
	: m_state{nullptr, StateDeleter{mode, channels}}
	, m_mode{mode}
	, m_channels{channels}
{
	if (channels <= 0) { throw std::logic_error{"Invalid channel count"}; }

	// A state from the pool, as new; else one made now
	if (auto* pool = poolOf(mode, channels))
	{
		auto lock = PoolLock{};
		if (!pool->states.empty())
		{
			m_state.reset(pool->states.back());
			pool->states.pop_back();
			if (pool->states.size() < pool->target / 2) { pools().wantsRefill.store(true, std::memory_order_relaxed); }
		}
		else if (pool->target > 0)
		{
			pools().misses.fetch_add(1, std::memory_order_relaxed);
			pools().wantsRefill.store(true, std::memory_order_relaxed);
		}
	}
	if (m_state) { src_reset(static_cast<SRC_STATE*>(m_state.get())); }
	else
	{
		m_state.reset(src_new(converterType(mode), channels, &m_error));
		if (!m_state) { throw std::runtime_error{src_strerror(m_error)}; }
	}
}

void AudioResampler::reserve(Mode mode, ch_cnt_t channels, std::size_t count)
{
	auto* pool = poolOf(mode, channels);
	if (!pool) { return; }
	auto lock = PoolLock{};
	pool->target = std::max(pool->target, count);
	fill(*pool, mode, channels, pool->target);
}

void AudioResampler::refillPools()
{
	if (!pools().wantsRefill.exchange(false, std::memory_order_relaxed)) { return; }
	auto lock = PoolLock{};
	for (std::size_t mode = 0; mode < ModeCount; ++mode)
	{
		for (ch_cnt_t channels = 1; channels <= PooledChannels; ++channels)
		{
			auto& pool = pools().pools[mode][channels - 1];
			fill(pool, static_cast<Mode>(mode), channels, pool.target);
		}
	}
}

auto AudioResampler::poolMisses() -> long
{
	return pools().misses.load(std::memory_order_relaxed);
}

auto AudioResampler::process(InterleavedBufferView<const float> input, InterleavedBufferView<float> output) -> Result
{
	if (input.channels() != m_channels || output.channels() != m_channels)
	{
		throw std::invalid_argument{"Invalid channel count"};
	}

	auto data = SRC_DATA{};

	data.data_in = input.data();
	data.input_frames = input.frames();

	data.data_out = output.data();
	data.output_frames = output.frames();

	data.src_ratio = m_ratio;
	data.end_of_input = 0;

	if ((m_error = src_process(static_cast<SRC_STATE*>(m_state.get()), &data)))
	{
		throw std::runtime_error{src_strerror(m_error)};
	}

	return {static_cast<f_cnt_t>(data.input_frames_used), static_cast<f_cnt_t>(data.output_frames_gen)};
}

void AudioResampler::reset()
{
	if ((m_error = src_reset(static_cast<SRC_STATE*>(m_state.get()))))
	{
		throw std::runtime_error{src_strerror(m_error)};
	}
}

void AudioResampler::StateDeleter::operator()(void* state)
{
	// Back to its pool, which has room for it; one made past the pool goes too (it has room for all it made)
	if (auto* pool = poolOf(mode, channels))
	{
		auto lock = PoolLock{};
		if (pool->states.size() < pool->states.capacity())
		{
			pool->states.push_back(static_cast<SRC_STATE*>(state));
			return;
		}
	}
	src_delete(static_cast<SRC_STATE*>(state));
}

} // namespace lmms
