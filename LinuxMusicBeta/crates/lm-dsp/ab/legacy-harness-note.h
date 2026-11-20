// A note's counters, which only its own play() moves, set by the A/B harness that plays an instrument in its place
// (see legacy-harness.cpp): the standard's way to name a private member (an explicit instantiation may)
#pragma once

#include <atomic>

#include "NotePlayHandle.h"

namespace harness
{

template<typename Tag, typename Tag::type Member>
struct Rob
{
	friend typename Tag::type get(Tag) { return Member; }
};
struct NoteFramesPlayed
{
	using type = std::atomic<lmms::f_cnt_t> lmms::NotePlayHandle::*;
	friend type get(NoteFramesPlayed);
};
struct NoteReleased
{
	using type = std::atomic<bool> lmms::NotePlayHandle::*;
	friend type get(NoteReleased);
};
struct NoteBeforeRelease
{
	using type = lmms::f_cnt_t lmms::NotePlayHandle::*;
	friend type get(NoteBeforeRelease);
};

template struct Rob<NoteFramesPlayed, &lmms::NotePlayHandle::m_totalFramesPlayed>;
template struct Rob<NoteReleased, &lmms::NotePlayHandle::m_released>;
template struct Rob<NoteBeforeRelease, &lmms::NotePlayHandle::m_framesBeforeRelease>;

} // namespace harness
