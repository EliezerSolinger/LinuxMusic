/*
 * SharedMemoryPointer.h - pointers that stay valid across processes sharing memory
 *
 * Copyright (c) 2026 Dalton Messmer <messmer.dalton/at/gmail.com>
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

#ifndef LMMS_SHARED_MEMORY_POINTER_H
#define LMMS_SHARED_MEMORY_POINTER_H

#include <cstdint>

namespace lmms
{

/**
 * A simplified, free-function version of boost::interprocess::offset_ptr for use in shared memory.
 * See https://www.boost.org/doc/libs/latest/doc/html/interprocess/offset_ptr.html
 *
 * The offset is stored relative to its own address, so a pointer into the same shared memory block
 * stays valid in every process that maps the block, wherever it is mapped.
 *
 * AudioBufferData stores its channel mapping this way at the start of a shared memory allocation, so the
 * client side of a remote plugin can find each channel buffer (see RemotePluginClient).
 */
using PointerOffset = std::uint64_t;

template<typename T>
void assignPointer(PointerOffset& self, T* p) noexcept
{
	self = p != nullptr
		? reinterpret_cast<char*>(p) - reinterpret_cast<char*>(&self) + 1
		: 1;
}

template<typename T>
auto toPointer(PointerOffset& self) noexcept -> T*
{
	return self != 1
		? reinterpret_cast<T*>(reinterpret_cast<char*>(&self) + self - 1)
		: nullptr;
}

} // namespace lmms

#endif // LMMS_SHARED_MEMORY_POINTER_H
