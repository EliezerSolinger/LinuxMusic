/*
 * Handoff.h
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

#ifndef LMMS_HANDOFF_H
#define LMMS_HANDOFF_H

#include <atomic>
#include <memory>

namespace lmms
{

/**
 * Hands something made off the audio thread to it without locks: offer() (any other thread) leaves it pending;
 * take() (the audio thread) swaps it in when the one it replaces has been given back, which the next offer() (or
 * the destructor) frees: the audio thread never frees. An empty offer removes what it has.
 */
template<typename T>
class Handoff
{
public:
	~Handoff()
	{
		delete m_pending.exchange(nullptr);
		delete m_retired.exchange(nullptr);
	}

	void offer(std::unique_ptr<T> next)
	{
		delete m_retired.exchange(nullptr);
		delete m_pending.exchange(new Slot{std::move(next)});
	}

	//! [audio thread] What it has now
	auto take() -> T*
	{
		if (m_retired.load() == nullptr)
		{
			if (auto* next = m_pending.exchange(nullptr))
			{
				m_retired.store(m_current.release());
				m_current.reset(next);
			}
		}
		return m_current ? m_current->value.get() : nullptr;
	}

private:
	struct Slot
	{
		std::unique_ptr<T> value;
	};
	std::unique_ptr<Slot> m_current;
	std::atomic<Slot*> m_pending = nullptr;
	std::atomic<Slot*> m_retired = nullptr;
};

} // namespace lmms

#endif // LMMS_HANDOFF_H
