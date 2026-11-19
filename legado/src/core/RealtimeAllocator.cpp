/*
 * RealtimeAllocator.cpp
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

#include "RealtimeAllocator.h"

#include <algorithm>
#include <array>
#include <atomic>
#include <bit>
#include <cstdint>
#include <cstdlib>
#include <cstring>

namespace lmms::RealtimeAllocator
{

namespace
{

// The TLSF's constants: blocks are 16 aligned; each power of two is split in 16 classes; below 256 bytes the classes
// are 16 apart
constexpr std::size_t Align = 16;
constexpr int SlLog2 = 4;
constexpr int SlCount = 1 << SlLog2;
constexpr int FlShift = SlLog2 + 4; // log2(Align)
constexpr std::size_t SmallBlock = std::size_t{1} << FlShift;
constexpr int FlCount = 40 - FlShift + 2; // blocks below 2^40 bytes

/**
 * A block of the pool: its header, then its payload (where a free block keeps its free list's links). The blocks lie
 * one after the other; the last one, of no size and never free, ends the pool.
 */
struct Block
{
	Block* previousInPool;
	std::size_t sizeAndFree; //!< the payload's size (a multiple of 16) | 1 when free
	// The payload; while free:
	Block* nextFree;
	Block* previousFree;
};
constexpr std::size_t HeaderSize = offsetof(Block, nextFree);
static_assert(HeaderSize == Align);
constexpr std::size_t MinPayload = sizeof(Block) - HeaderSize;

auto sizeOf(const Block* block) -> std::size_t { return block->sizeAndFree & ~std::size_t{1}; }
auto isFree(const Block* block) -> bool { return block->sizeAndFree & 1; }
void setSize(Block* block, std::size_t size) { block->sizeAndFree = size | (block->sizeAndFree & 1); }
void setFree(Block* block, bool free) { block->sizeAndFree = sizeOf(block) | (free ? 1 : 0); }
auto payloadOf(Block* block) -> void* { return reinterpret_cast<std::byte*>(block) + HeaderSize; }
auto blockOf(void* payload) -> Block* { return reinterpret_cast<Block*>(static_cast<std::byte*>(payload) - HeaderSize); }
auto nextInPool(Block* block) -> Block*
{
	return reinterpret_cast<Block*>(reinterpret_cast<std::byte*>(block) + HeaderSize + sizeOf(block));
}

auto mostSignificantBit(std::size_t value) -> int { return std::bit_width(value) - 1; }

//! The list a block of @p size bytes goes in
void mappingInsert(std::size_t size, int& fl, int& sl)
{
	if (size < SmallBlock)
	{
		fl = 0;
		sl = static_cast<int>(size / (SmallBlock / SlCount));
	}
	else
	{
		const auto msb = mostSignificantBit(size);
		sl = static_cast<int>(size >> (msb - SlLog2)) ^ SlCount;
		fl = msb - (FlShift - 1);
	}
}

//! The first list whose blocks all hold @p size bytes
void mappingSearch(std::size_t size, int& fl, int& sl)
{
	if (size >= SmallBlock) { size += (std::size_t{1} << (mostSignificantBit(size) - SlLog2)) - 1; }
	mappingInsert(size, fl, sl);
}

struct Pool
{
	std::atomic_flag lock = ATOMIC_FLAG_INIT;
	std::byte* begin = nullptr;
	std::byte* end = nullptr;
	std::uint64_t flBitmap = 0;
	std::array<std::uint32_t, FlCount> slBitmap = {};
	std::array<std::array<Block*, SlCount>, FlCount> free = {};
	std::size_t used = 0;
	std::size_t peak = 0;
	long fallbacks = 0;
	std::atomic<bool> initialized = false;
};

auto pool() -> Pool&
{
	// Never destroyed: memory from the pool may be given back while the program ends
	static auto* s_pool = new Pool{};
	return *s_pool;
}

class Locked
{
public:
	Locked() : m_pool{pool()}
	{
		while (m_pool.lock.test_and_set(std::memory_order_acquire))
		{
			while (m_pool.lock.test(std::memory_order_relaxed)) {}
		}
	}
	~Locked() { m_pool.lock.clear(std::memory_order_release); }
	Locked(const Locked&) = delete;
	auto operator=(const Locked&) -> Locked& = delete;
	auto operator->() -> Pool* { return &m_pool; }

private:
	Pool& m_pool;
};

void insertFree(Pool& p, Block* block)
{
	int fl = 0, sl = 0;
	mappingInsert(sizeOf(block), fl, sl);
	auto*& head = p.free[fl][sl];
	block->nextFree = head;
	block->previousFree = nullptr;
	if (head) { head->previousFree = block; }
	head = block;
	p.flBitmap |= std::uint64_t{1} << fl;
	p.slBitmap[fl] |= 1u << sl;
}

void removeFree(Pool& p, Block* block)
{
	int fl = 0, sl = 0;
	mappingInsert(sizeOf(block), fl, sl);
	if (block->nextFree) { block->nextFree->previousFree = block->previousFree; }
	if (block->previousFree) { block->previousFree->nextFree = block->nextFree; }
	else
	{
		p.free[fl][sl] = block->nextFree;
		if (!p.free[fl][sl])
		{
			p.slBitmap[fl] &= ~(1u << sl);
			if (!p.slBitmap[fl]) { p.flBitmap &= ~(std::uint64_t{1} << fl); }
		}
	}
}

auto findFree(Pool& p, std::size_t size) -> Block*
{
	int fl = 0, sl = 0;
	mappingSearch(size, fl, sl);
	if (fl >= FlCount) { return nullptr; }
	auto slMap = p.slBitmap[fl] & (~0u << sl);
	if (!slMap)
	{
		const auto flMap = fl + 1 < 64 ? p.flBitmap & (~std::uint64_t{0} << (fl + 1)) : 0;
		if (!flMap) { return nullptr; }
		fl = std::countr_zero(flMap);
		slMap = p.slBitmap[fl];
	}
	sl = std::countr_zero(slMap);
	return p.free[fl][sl];
}

auto allocateLocked(Pool& p, std::size_t size) -> void*
{
	// Larger than any pool (and than rounding up can hold)
	if (size > (std::size_t{1} << 39))
	{
		++p.fallbacks;
		return nullptr;
	}
	size = std::max(MinPayload, (size + Align - 1) & ~(Align - 1));
	Block* block = findFree(p, size);
	if (!block)
	{
		++p.fallbacks;
		return nullptr;
	}
	removeFree(p, block);

	// What is left past the payload becomes a free block, when it can hold one
	const auto blockSize = sizeOf(block);
	if (blockSize >= size + HeaderSize + MinPayload)
	{
		auto* rest = reinterpret_cast<Block*>(reinterpret_cast<std::byte*>(payloadOf(block)) + size);
		rest->sizeAndFree = (blockSize - size - HeaderSize) | 1;
		rest->previousInPool = block;
		nextInPool(rest)->previousInPool = rest;
		setSize(block, size);
		insertFree(p, rest);
	}
	setFree(block, false);

	p.used += sizeOf(block);
	p.peak = std::max(p.peak, p.used);
	return payloadOf(block);
}

void releaseLocked(Pool& p, void* memory)
{
	auto* block = blockOf(memory);
	p.used -= sizeOf(block);
	setFree(block, true);

	// Merged with the free blocks beside it
	if (auto* previous = block->previousInPool; previous && isFree(previous))
	{
		removeFree(p, previous);
		setSize(previous, sizeOf(previous) + HeaderSize + sizeOf(block));
		nextInPool(previous)->previousInPool = previous;
		block = previous;
	}
	if (auto* next = nextInPool(block); isFree(next))
	{
		removeFree(p, next);
		setSize(block, sizeOf(block) + HeaderSize + sizeOf(next));
		nextInPool(block)->previousInPool = block;
	}
	insertFree(p, block);
}

// The frees the audio thread hands over (see deferFree())
constexpr std::size_t DeferredSlots = 8192;
std::array<std::atomic<void*>, DeferredSlots> s_deferred = {};
std::atomic<std::size_t> s_deferredCursor = 0;
std::atomic<long> s_deferredCount = 0;
std::atomic<long> s_deferredPending = 0;
std::atomic_flag s_draining = ATOMIC_FLAG_INIT;

} // namespace


void init(std::size_t bytes)
{
	auto locked = Locked{};
	if (locked->initialized.load(std::memory_order_relaxed)) { return; }

	bytes = std::max(bytes & ~(Align - 1), 4 * (HeaderSize + MinPayload));
#ifdef _WIN32
	void* memory = _aligned_malloc(bytes, Align); // never freed
	if (!memory) { return; }
#else
	void* memory = nullptr;
	if (posix_memalign(&memory, Align, bytes) != 0) { return; }
#endif
	// Touched now, so the audio thread never waits for the system to give a page
	std::memset(memory, 0, bytes);

	locked->begin = static_cast<std::byte*>(memory);
	locked->end = locked->begin + bytes;
	auto* first = reinterpret_cast<Block*>(locked->begin);
	first->previousInPool = nullptr;
	first->sizeAndFree = (bytes - 2 * HeaderSize) | 1;
	auto* last = nextInPool(first);
	last->previousInPool = first;
	last->sizeAndFree = 0; // ends the pool: no size, never free
	insertFree(*locked.operator->(), first);
	locked->initialized.store(true, std::memory_order_release);
}


auto isInitialized() -> bool
{
	return pool().initialized.load(std::memory_order_acquire);
}


auto allocate(std::size_t size) -> void*
{
	if (!isInitialized()) { return nullptr; }
	auto locked = Locked{};
	return allocateLocked(*locked.operator->(), size);
}


auto allocateAligned(std::size_t size, std::size_t alignment) -> void*
{
	if (alignment <= Align) { return allocate(size); }
	// Room to move the start to the alignment, keeping where the block starts just before it
	auto* memory = static_cast<std::byte*>(allocate(size + alignment));
	if (!memory) { return nullptr; }
	const auto start = reinterpret_cast<std::uintptr_t>(memory + sizeof(void*));
	auto* aligned = reinterpret_cast<std::byte*>((start + alignment - 1) & ~(std::uintptr_t{alignment} - 1));
	std::memcpy(aligned - sizeof(void*), &memory, sizeof(void*));
	return aligned;
}


auto owns(const void* memory) -> bool
{
	const auto& p = pool();
	if (!p.initialized.load(std::memory_order_acquire)) { return false; }
	const auto* byte = static_cast<const std::byte*>(memory);
	return byte >= p.begin && byte < p.end;
}


void release(void* memory)
{
	auto locked = Locked{};
	releaseLocked(*locked.operator->(), memory);
}


void releaseAligned(void* memory)
{
	void* start = nullptr;
	std::memcpy(&start, static_cast<std::byte*>(memory) - sizeof(void*), sizeof(void*));
	release(start);
}


auto deferFree(void* memory) -> bool
{
	for (std::size_t tries = 0; tries < DeferredSlots; ++tries)
	{
		auto& slot = s_deferred[s_deferredCursor.fetch_add(1, std::memory_order_relaxed) % DeferredSlots];
		void* empty = nullptr;
		if (slot.compare_exchange_strong(empty, memory, std::memory_order_release, std::memory_order_relaxed))
		{
			s_deferredCount.fetch_add(1, std::memory_order_relaxed);
			s_deferredPending.fetch_add(1, std::memory_order_release);
			return true;
		}
	}
	return false;
}


void drainDeferred()
{
	for (auto& slot : s_deferred)
	{
		if (slot.load(std::memory_order_relaxed) == nullptr) { continue; }
		if (void* memory = slot.exchange(nullptr, std::memory_order_acquire))
		{
			s_deferredPending.fetch_sub(1, std::memory_order_relaxed);
			std::free(memory);
		}
	}
}


void drainDeferredIfAny()
{
	if (s_deferredPending.load(std::memory_order_acquire) <= 0) { return; }
	if (s_draining.test_and_set(std::memory_order_acquire)) { return; }
	drainDeferred();
	s_draining.clear(std::memory_order_release);
}


auto stats() -> Stats
{
	auto locked = Locked{};
	return Stats{
		static_cast<std::size_t>(locked->end - locked->begin),
		locked->used,
		locked->peak,
		locked->fallbacks,
		s_deferredCount.load(std::memory_order_relaxed)
	};
}


auto isWhole() -> bool
{
	auto locked = Locked{};
	if (!locked->begin) { return true; }
	auto* first = reinterpret_cast<Block*>(locked->begin);
	return isFree(first) && nextInPool(first) == reinterpret_cast<Block*>(locked->end - HeaderSize);
}

} // namespace lmms::RealtimeAllocator
