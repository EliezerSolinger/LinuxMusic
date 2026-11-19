/*
 * IntrusiveList.h
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

#ifndef LMMS_INTRUSIVE_LIST_H
#define LMMS_INTRUSIVE_LIST_H

#include <cstddef>
#include <iterator>

namespace lmms
{

/**
 * A list of objects linked through a pointer of their own (@p Next), so that adding and removing allocates nothing:
 * for lists the audio thread changes, such as a note's chord notes. An object is in at most one list per link member.
 * Not thread-safe; removing during an iteration invalidates the iterator at the removed object only.
 */
template<class T, T* T::*Next>
class IntrusiveList
{
public:
	class iterator
	{
	public:
		using iterator_category = std::forward_iterator_tag;
		using value_type = T*;
		using difference_type = std::ptrdiff_t;
		using pointer = T* const*;
		using reference = T* const&;

		iterator() = default;
		explicit iterator(T* node) : m_node{node} {}

		auto operator*() const -> T* const& { return m_node; }
		auto operator++() -> iterator&
		{
			m_node = m_node->*Next;
			return *this;
		}
		auto operator++(int) -> iterator
		{
			auto previous = *this;
			++*this;
			return previous;
		}
		auto operator==(const iterator&) const -> bool = default;

	private:
		T* m_node = nullptr;
	};

	IntrusiveList() = default;
	// A copy would share the links
	IntrusiveList(const IntrusiveList&) = delete;
	auto operator=(const IntrusiveList&) -> IntrusiveList& = delete;

	auto begin() const -> iterator { return iterator{m_head}; }
	auto end() const -> iterator { return iterator{}; }
	auto isEmpty() const -> bool { return m_head == nullptr; }
	auto size() const -> std::size_t { return m_size; }
	auto first() const -> T* { return m_head; }

	void push_back(T* node)
	{
		node->*Next = nullptr;
		if (m_tail) { m_tail->*Next = node; }
		else { m_head = node; }
		m_tail = node;
		++m_size;
	}

	//! Takes @p node out, if it is in; @returns whether it was
	auto removeOne(T* node) -> bool
	{
		T* previous = nullptr;
		for (T* current = m_head; current; previous = current, current = current->*Next)
		{
			if (current != node) { continue; }
			if (previous) { previous->*Next = current->*Next; }
			else { m_head = current->*Next; }
			if (m_tail == current) { m_tail = previous; }
			current->*Next = nullptr;
			--m_size;
			return true;
		}
		return false;
	}

	//! Empties the list (the objects' links are left as they are)
	void clear()
	{
		m_head = nullptr;
		m_tail = nullptr;
		m_size = 0;
	}

	auto indexOf(const T* node) const -> int
	{
		int index = 0;
		for (T* current = m_head; current; current = current->*Next, ++index)
		{
			if (current == node) { return index; }
		}
		return -1;
	}

private:
	T* m_head = nullptr;
	T* m_tail = nullptr;
	std::size_t m_size = 0;
};

} // namespace lmms

#endif // LMMS_INTRUSIVE_LIST_H
