/*
 * BufferManager.h - A buffer caching/memory management system
 *
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

#ifndef LMMS_BUFFER_MANAGER_H
#define LMMS_BUFFER_MANAGER_H

#include <span>

#include "SampleFrame.h"
#include "lmms_export.h"
#include "LmmsTypes.h"

namespace lmms
{

/**
 * The play handles' period buffers, from a pool made when the engine starts: a note started on the audio thread
 * takes one without allocating, and gives it back when it ends.
 *
 * The pool grows (allocating, which the audio thread should not) only when more buffers are in use at once than it
 * was made with; it never shrinks. Taking and giving back are safe from any thread.
 */
class LMMS_EXPORT BufferManager
{
public:
	//! A buffer taken from the pool, given back when this goes. Holds none when default made or moved from.
	class LMMS_EXPORT Buffer
	{
	public:
		Buffer() = default;
		Buffer(Buffer&& other) noexcept;
		auto operator=(Buffer&& other) noexcept -> Buffer&;
		Buffer(const Buffer&) = delete;
		auto operator=(const Buffer&) -> Buffer& = delete;
		~Buffer();

		auto data() const -> SampleFrame* { return m_data; }
		auto size() const -> f_cnt_t { return m_frames; }
		auto span() const -> std::span<SampleFrame> { return {m_data, m_frames}; }

	private:
		friend class BufferManager;
		Buffer(SampleFrame* data, f_cnt_t frames) : m_data{data}, m_frames{frames} {}

		SampleFrame* m_data = nullptr;
		f_cnt_t m_frames = 0;
	};

	//! Makes the pool, with buffers of @p fpp frames. Again with another size, buffers of the old one still in use
	//! stay valid, and are dropped when given back.
	static void init(f_cnt_t fpp);
	static auto acquire() -> Buffer;

	//! How many buffers had to be allocated because the pool was empty (each an allocation where there should be none)
	static auto overflows() -> long;

private:
	static void release(SampleFrame* data, f_cnt_t frames);
};


} // namespace lmms

#endif // LMMS_BUFFER_MANAGER_H
