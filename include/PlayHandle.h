/*
 * PlayHandle.h - base class PlayHandle - core of rendering engine
 *
 * Copyright (c) 2004-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_PLAY_HANDLE_H
#define LMMS_PLAY_HANDLE_H

#include <QList>
#include <QMutex>
#include <span>
#include <vector>

#include "BufferManager.h"
#include "Flags.h"
#include "LmmsTypes.h"
#include "SampleFrame.h"
#include "ThreadableJob.h"
#include "lmms_export.h"

class QThread;

namespace lmms
{

class Track;
class AudioBusHandle;
class SampleFrame;

class LMMS_EXPORT PlayHandle : public ThreadableJob
{
public:
	enum class Type
	{
		NotePlayHandle = 0x01,
		InstrumentPlayHandle = 0x02,
		SamplePlayHandle = 0x04,
		PresetPreviewHandle = 0x08,
		InputMonitorHandle = 0x10, //!< a track's input, monitored (see InputMonitorHandle)
		AudioTrackPlayHandle = 0x20, //!< an audio track's items (see AudioTrack)
		CalibrationHandle = 0x40, //!< the latency calibration's pulse (see LatencyCalibration)
		MetronomeHandle = 0x80 //!< the metronome's clicks (see Metronome)
	} ;
	using Types = Flags<Type>;

	//! Whether handles of @p type live as long as their track (which removes them), not as long as a sound
	static constexpr auto livesWithTrack(Type type) -> bool
	{
		return type == Type::InstrumentPlayHandle || type == Type::InputMonitorHandle
			|| type == Type::AudioTrackPlayHandle || type == Type::MetronomeHandle;
	}

	constexpr static std::size_t MaxNumber = 1024;

	PlayHandle( const Type type, f_cnt_t offset = 0 );

	PlayHandle & operator = ( PlayHandle & p )
	{
		m_type = p.m_type;
		m_offset = p.m_offset;
		m_affinity = p.m_affinity;
		m_usesBuffer = p.m_usesBuffer;
		m_audioBusHandle = p.m_audioBusHandle;
		return *this;
	}

	virtual ~PlayHandle();

	/**
	 * Gives the handle up once the engine is done with it: deleted, unless it goes back to the pool it came from (a
	 * note, a sample track's handle), so that the audio thread need not free it
	 */
	virtual void dispose() { delete this; }

	//! The thread the engine finishes the handle on (see affinityMatters()), for a handle used again on another
	void setAffinityToCurrentThread();

	virtual bool affinityMatters() const
	{
		return false;
	}

	const QThread* affinity() const
	{
		return m_affinity;
	}

	Type type() const
	{
		return m_type;
	}

	// required for ThreadableJob
	void doProcessing() override;

	bool requiresProcessing() const override
	{
		return !isFinished();
	}

	void lock()
	{
		m_processingLock.lock();
	}
	void unlock()
	{
		m_processingLock.unlock();
	}
	bool tryLock()
	{
		return m_processingLock.tryLock();
	}
	virtual void play(std::span<SampleFrame> buffer) = 0;
	virtual bool isFinished() const = 0;

	// returns the frameoffset at the start of the playhandle,
	// ie. how many empty frames should be inserted at the start of the first period
	f_cnt_t offset() const
	{
		return m_offset;
	}

	void setOffset( f_cnt_t _offset )
	{
		m_offset = _offset;
	}


	virtual bool isFromTrack( const Track * _track ) const = 0;

	bool usesBuffer() const
	{
		return m_usesBuffer;
	}
	
	void setUsesBuffer( const bool b )
	{
		m_usesBuffer = b;
	}
	
	AudioBusHandle* audioBusHandle()
	{
		return m_audioBusHandle;
	}
	
	void setAudioBusHandle(AudioBusHandle* busHandle)
	{
		m_audioBusHandle = busHandle;
	}
	
	void releaseBuffer();
	
	std::span<SampleFrame> buffer();

private:
	Type m_type;
	f_cnt_t m_offset;
	QThread* m_affinity;
	QMutex m_processingLock;
	BufferManager::Buffer m_playHandleBuffer; //!< from the pool, so a play handle made on the audio thread allocates none
	bool m_bufferReleased;
	bool m_usesBuffer;
	AudioBusHandle* m_audioBusHandle;
} ;

using PlayHandleList = QList<PlayHandle*>;
using ConstPlayHandleList = QList<const PlayHandle*>;

LMMS_DECLARE_OPERATORS_FOR_FLAGS(PlayHandle::Type)

} // namespace lmms

#endif // LMMS_PLAY_HANDLE_H
