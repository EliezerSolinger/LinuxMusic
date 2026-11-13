/*
 * AllocationObserver.c - the system allocator's calls, seen by a test (AudioThreadAllocationTest)
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

// Loaded before the test (DYLD_INSERT_LIBRARIES on macOS, LD_PRELOAD on Linux), it stands between every library and
// malloc(), free() and the like: Qt and C code call them themselves, not operator new. Each call is told to the
// observer the test sets, then goes on to the system.

#include <stddef.h>
#include <stdlib.h>

#if defined(__APPLE__)
#define OBSERVER_EXPORT __attribute__((visibility("default")))
#else
#define OBSERVER_EXPORT __attribute__((visibility("default")))
#endif

//! Set by the test; called on each call to the system's allocator (on any thread)
OBSERVER_EXPORT void (*lmmsAllocationObserver)(void) = NULL;

static void observe(void)
{
	void (*observer)(void) = __atomic_load_n(&lmmsAllocationObserver, __ATOMIC_RELAXED);
	if (observer) { observer(); }
}

#if defined(__APPLE__)

#include <malloc/malloc.h>

static void* observedMalloc(size_t size) { observe(); return malloc(size); }
static void* observedCalloc(size_t count, size_t size) { observe(); return calloc(count, size); }
static void* observedRealloc(void* memory, size_t size) { observe(); return realloc(memory, size); }
static void observedFree(void* memory) { if (memory) { observe(); } free(memory); }
static int observedPosixMemalign(void** memory, size_t alignment, size_t size)
{
	observe();
	return posix_memalign(memory, alignment, size);
}
static void* observedAlignedAlloc(size_t alignment, size_t size) { observe(); return aligned_alloc(alignment, size); }
static void* observedValloc(size_t size) { observe(); return valloc(size); }

#define INTERPOSE(replacement, original) \
	__attribute__((used)) static const struct { const void* r; const void* o; } interpose_##original \
	__attribute__((section("__DATA,__interpose"))) = { (const void*)(replacement), (const void*)(original) }

INTERPOSE(observedMalloc, malloc);
INTERPOSE(observedCalloc, calloc);
INTERPOSE(observedRealloc, realloc);
INTERPOSE(observedFree, free);
INTERPOSE(observedPosixMemalign, posix_memalign);
INTERPOSE(observedAlignedAlloc, aligned_alloc);
INTERPOSE(observedValloc, valloc);

#else // glibc

extern void* __libc_malloc(size_t size);
extern void* __libc_calloc(size_t count, size_t size);
extern void* __libc_realloc(void* memory, size_t size);
extern void __libc_free(void* memory);
extern void* __libc_memalign(size_t alignment, size_t size);

OBSERVER_EXPORT void* malloc(size_t size) { observe(); return __libc_malloc(size); }
OBSERVER_EXPORT void* calloc(size_t count, size_t size) { observe(); return __libc_calloc(count, size); }
OBSERVER_EXPORT void* realloc(void* memory, size_t size) { observe(); return __libc_realloc(memory, size); }
OBSERVER_EXPORT void free(void* memory) { if (memory) { observe(); } __libc_free(memory); }
OBSERVER_EXPORT int posix_memalign(void** memory, size_t alignment, size_t size)
{
	observe();
	*memory = __libc_memalign(alignment, size);
	return *memory ? 0 : 12; // ENOMEM
}
OBSERVER_EXPORT void* aligned_alloc(size_t alignment, size_t size) { observe(); return __libc_memalign(alignment, size); }

#endif
