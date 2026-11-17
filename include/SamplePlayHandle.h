
/*
 * SamplePlayHandle.h - play-handle for playing a sample
 *
 * Copyright (c) 2005-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_SAMPLE_PLAY_HANDLE_H
#define LMMS_SAMPLE_PLAY_HANDLE_H

#include <memory>

#include "Sample.h"
#include "PlayHandle.h"
#include "VoicePool.h"

namespace lmms
{


class PatternTrack;
class SampleClip;
class Track;


class LMMS_EXPORT SamplePlayHandle : public PlayHandle
{
public:
	//! Plays a sample of its own, on a bus of its own (a preview, a click)
	explicit SamplePlayHandle(std::unique_ptr<Sample> sample);
	explicit SamplePlayHandle( const QString& sampleFile );
	SamplePlayHandle( SampleClip* clip );
	//! A handle for a sample track's pool: nothing to play until restart() gives it a clip
	SamplePlayHandle();
	~SamplePlayHandle() override;

	inline bool affinityMatters() const override
	{
		return true;
	}


	void play(std::span<SampleFrame> buffer) override;
	bool isFinished() const override;

	bool isFromTrack( const Track * _track ) const override;

	f_cnt_t totalFrames() const;
	inline f_cnt_t framesDone() const
	{
		return( m_frame );
	}
	void setDoneMayReturnTrue( bool _enable )
	{
		m_doneMayReturnTrue = _enable;
	}

	void setPatternTrack(PatternTrack* pt)
	{
		m_patternTrack = pt;
	}

	//! Plays @p clip from its start again, as a new handle for it would: for a handle from a sample track's pool
	//! (allocates nothing) [audio thread]
	void restart(SampleClip* clip, VoicePool<SamplePlayHandle>* pool);

	//! Back to its sample track's pool, or deleted
	void dispose() override;

private:
	VoicePool<SamplePlayHandle>* m_pool = nullptr; //!< the pool it came from, if any
	Sample::PlaybackState m_state;
	f_cnt_t m_frame = 0;
	Sample* m_sample = nullptr;
	std::unique_ptr<Sample> m_ownSample; //!< When the sample is its own
	std::unique_ptr<AudioBusHandle> m_ownBus; //!< When the bus is its own
	Track* m_track = nullptr;
	SampleClip* m_clip = nullptr;
	PatternTrack* m_patternTrack = nullptr;
	bool m_doneMayReturnTrue = true;
} ;


} // namespace lmms

#endif // LMMS_SAMPLE_PLAY_HANDLE_H
