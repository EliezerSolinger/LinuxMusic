/*
 * RealtimeAllocator.h
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

#ifndef LMMS_REALTIME_ALLOCATOR_H
#define LMMS_REALTIME_ALLOCATOR_H

#include <cstddef>

#include "lmms_export.h"

namespace lmms
{

/**
 * Memory for the audio threads without the system's allocator, which may lock or ask the system for pages: a pool
 * reserved (and touched) when the program starts, managed as a TLSF heap (two-level segregated fit: finding,
 * splitting and merging blocks takes the same few steps whatever the size or the state of the pool).
 *
 * Code the program does not control (the STK's instruments, ZynAddSubFX, Xpressive's expressions) makes a note's
 * voice with new on the audio thread; the program's operator new (RealtimeNew.cpp) gives memory from here to the
 * audio threads, and its delete gives it back from any thread. The program's own code allocates nothing on the
 * audio thread to begin with (AudioThreadAllocationTest).
 *
 * Thread-safe: a spin lock held for a few steps. Memory past the pool, or bigger than it can hold, comes from the
 * system instead (counted in fallbacks()).
 */
namespace RealtimeAllocator
{

//! Reserves the pool (once; again does nothing). Before it, allocate() returns nothing.
LMMS_EXPORT void init(std::size_t bytes);
LMMS_EXPORT auto isInitialized() -> bool;

//! @p size bytes, aligned to 16, from the pool; nothing when the pool cannot hold them (counted as a fallback)
LMMS_EXPORT auto allocate(std::size_t size) -> void*;
//! As allocate(), aligned to @p alignment (a power of two)
LMMS_EXPORT auto allocateAligned(std::size_t size, std::size_t alignment) -> void*;

//! Whether @p memory is in the pool (then it goes back with release() or releaseAligned())
LMMS_EXPORT auto owns(const void* memory) -> bool;
LMMS_EXPORT void release(void* memory);
//! Gives back memory from allocateAligned()
LMMS_EXPORT void releaseAligned(void* memory);

/**
 * Frees system memory later, off the audio thread: the audio thread hands over what it would free (freeing may
 * lock), and drainDeferred() frees it. A full hand-over frees at once. @returns whether it was handed over.
 */
LMMS_EXPORT auto deferFree(void* memory) -> bool;
//! Frees what deferFree() handed over [not on an audio thread]
LMMS_EXPORT void drainDeferred();
//! As drainDeferred(), only when something was handed over, and only if no other thread is draining: cheap enough
//! to call on every free [not on an audio thread]
LMMS_EXPORT void drainDeferredIfAny();

struct Stats
{
	std::size_t poolBytes = 0;
	std::size_t usedBytes = 0;
	std::size_t peakBytes = 0;
	long fallbacks = 0; //!< allocations the pool could not give
	long deferredFrees = 0; //!< frees handed over by the audio thread
};
LMMS_EXPORT auto stats() -> Stats;

//! Whether the pool is whole: every block free and merged back into one (for tests)
LMMS_EXPORT auto isWhole() -> bool;

} // namespace RealtimeAllocator

} // namespace lmms

#endif // LMMS_REALTIME_ALLOCATOR_H
