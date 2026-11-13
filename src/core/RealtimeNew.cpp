/*
 * RealtimeNew.cpp - the program's operator new and delete: the audio threads' memory from the realtime pool
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

// Part of the program itself, not of its core library: the tests replace these operators with their own (to count
// what the audio thread allocates). They replace the C++ library's for the whole program, the plugins included (not
// on Windows, where each library keeps its own).

#ifndef _WIN32 // see above

#include <algorithm>
#include <cstdlib>
#include <new>

#include "PluginChecks.h"
#include "RealtimeAllocator.h"

// Under the address sanitizer, every allocation goes to the system, which the sanitizer watches (the pool is one
// block to it: overflows inside it would go unseen)
#if defined(__SANITIZE_ADDRESS__)
#define LMMS_REALTIME_POOL 0
#elif defined(__has_feature)
#if __has_feature(address_sanitizer) || __has_feature(memory_sanitizer)
#define LMMS_REALTIME_POOL 0
#endif
#endif
#ifndef LMMS_REALTIME_POOL
#define LMMS_REALTIME_POOL 1
#endif

namespace
{

namespace ThreadCheck = lmms::ThreadCheck;

auto fromSystem(std::size_t size) -> void*
{
	if (auto* memory = std::malloc(size ? size : 1)) { return memory; }
	throw std::bad_alloc{};
}

auto fromSystemAligned(std::size_t size, std::size_t alignment) -> void*
{
	void* memory = nullptr;
	if (posix_memalign(&memory, std::max(alignment, sizeof(void*)), size ? size : 1) == 0) { return memory; }
	throw std::bad_alloc{};
}

auto newMemory(std::size_t size) -> void*
{
	// The audio threads take from the pool; past it, and on other threads, the system
	if (LMMS_REALTIME_POOL && ThreadCheck::isAudioThread())
	{
		if (auto* memory = lmms::RealtimeAllocator::allocate(size)) { return memory; }
	}
	return fromSystem(size);
}

auto newAligned(std::size_t size, std::size_t alignment) -> void*
{
	if (LMMS_REALTIME_POOL && ThreadCheck::isAudioThread())
	{
		if (auto* memory = lmms::RealtimeAllocator::allocateAligned(size, alignment)) { return memory; }
	}
	return fromSystemAligned(size, alignment);
}

void deleteMemory(void* memory, bool aligned) noexcept
{
	if (!memory) { return; }
	if (lmms::RealtimeAllocator::owns(memory))
	{
		if (aligned) { lmms::RealtimeAllocator::releaseAligned(memory); }
		else { lmms::RealtimeAllocator::release(memory); }
		return;
	}
	// The system's memory, freed later by another thread when the audio thread lets it go
	if (LMMS_REALTIME_POOL && ThreadCheck::isAudioThread())
	{
		if (lmms::RealtimeAllocator::deferFree(memory)) { return; }
	}
	else { lmms::RealtimeAllocator::drainDeferredIfAny(); }
	std::free(memory);
}

} // namespace

void* operator new(std::size_t size) { return newMemory(size); }
void* operator new[](std::size_t size) { return newMemory(size); }
void* operator new(std::size_t size, const std::nothrow_t&) noexcept
{
	try { return newMemory(size); } catch (...) { return nullptr; }
}
void* operator new[](std::size_t size, const std::nothrow_t&) noexcept
{
	try { return newMemory(size); } catch (...) { return nullptr; }
}
void* operator new(std::size_t size, std::align_val_t alignment)
{
	return newAligned(size, static_cast<std::size_t>(alignment));
}
void* operator new[](std::size_t size, std::align_val_t alignment)
{
	return newAligned(size, static_cast<std::size_t>(alignment));
}
void* operator new(std::size_t size, std::align_val_t alignment, const std::nothrow_t&) noexcept
{
	try { return newAligned(size, static_cast<std::size_t>(alignment)); } catch (...) { return nullptr; }
}
void* operator new[](std::size_t size, std::align_val_t alignment, const std::nothrow_t&) noexcept
{
	try { return newAligned(size, static_cast<std::size_t>(alignment)); } catch (...) { return nullptr; }
}

void operator delete(void* memory) noexcept { deleteMemory(memory, false); }
void operator delete[](void* memory) noexcept { deleteMemory(memory, false); }
void operator delete(void* memory, std::size_t) noexcept { deleteMemory(memory, false); }
void operator delete[](void* memory, std::size_t) noexcept { deleteMemory(memory, false); }
void operator delete(void* memory, const std::nothrow_t&) noexcept { deleteMemory(memory, false); }
void operator delete[](void* memory, const std::nothrow_t&) noexcept { deleteMemory(memory, false); }
void operator delete(void* memory, std::align_val_t) noexcept { deleteMemory(memory, true); }
void operator delete[](void* memory, std::align_val_t) noexcept { deleteMemory(memory, true); }
void operator delete(void* memory, std::size_t, std::align_val_t) noexcept { deleteMemory(memory, true); }
void operator delete[](void* memory, std::size_t, std::align_val_t) noexcept { deleteMemory(memory, true); }
void operator delete(void* memory, std::align_val_t, const std::nothrow_t&) noexcept { deleteMemory(memory, true); }
void operator delete[](void* memory, std::align_val_t, const std::nothrow_t&) noexcept { deleteMemory(memory, true); }

#endif // _WIN32
