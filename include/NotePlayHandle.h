/*
 * NotePlayHandle.h - declaration of class NotePlayHandle which manages
 *                    playback of a single note by an instrument
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

#ifndef LMMS_NOTE_PLAY_HANDLE_H
#define LMMS_NOTE_PLAY_HANDLE_H

#include <cstddef>
#include <atomic>
#include <memory>
#include <new>
#include <utility>
#include <vector>

#include "ArrayVector.h"
#include "BasicFilters.h"
#include "IntrusiveList.h"
#include "Note.h"
#include "PlayHandle.h"
#include "Track.h"

class QReadWriteLock;

namespace lmms
{

class InstrumentTrack;
class NotePlayHandle;

using NotePlayHandleList = QList<NotePlayHandle*>;
//! Filled again by each lookup, so that one kept (with its room) allocates no more (see nphsOfInstrumentTrack())
using ConstNotePlayHandleList = std::vector<const NotePlayHandle*>;
//! A list of a track's notes in fixed room, for a lookup that may run on any audio thread (see the arpeggio): it
//! holds the first 256 and never allocates
using FixedNotePlayHandleList = ArrayVector<const NotePlayHandle*, 256>;

class LMMS_EXPORT NotePlayHandle : public PlayHandle, public Note
{
public:
	//! The links of the lists a note is in, which allocate nothing (see IntrusiveList)
	NotePlayHandle* m_nextSubNote = nullptr; //!< in its parent's chord or arpeggio notes
	NotePlayHandle* m_nextProcessHandle = nullptr; //!< in its track's notes without a parent
	using SubNoteList = IntrusiveList<NotePlayHandle, &NotePlayHandle::m_nextSubNote>;
	using ProcessHandleList = IntrusiveList<NotePlayHandle, &NotePlayHandle::m_nextProcessHandle>;

	void * m_pluginData;

	//! The note's filter (InstrumentSoundShaping): the one of the note's place in the pool, made with it, or one
	//! made now for a note made elsewhere; cleared for each note
	auto filter() -> BasicFilters<>&;

	// length of the declicking fade in
	f_cnt_t m_fadeInLength;

	// specifies origin of NotePlayHandle
	enum class Origin
	{
		MidiClip,		/*! playback of a note from a MIDI clip */
		MidiInput,	/*! playback of a MIDI note input event */
		NoteStacking,	/*! created by note stacking instrument function */
		Arpeggio,		/*! created by arpeggio instrument function */
	};

	NotePlayHandle( InstrumentTrack* instrumentTrack,
					const f_cnt_t offset,
					const f_cnt_t frames,
					const Note& noteToPlay,
					NotePlayHandle* parent = nullptr,
					int midiEventChannel = -1,
					Origin origin = Origin::MidiClip );
	~NotePlayHandle() override;

	auto mixOrder() const -> std::uint64_t override { return m_mixOrder; }

	//! Back to the note pool (NotePlayHandleManager)
	void dispose() override;

	void * operator new ( size_t size, void * p )
	{
		return p;
	}

	//! Room for an instrument's data for the note, made with the note (in the note pool): data that fits is made
	//! there, without allocating on the audio thread
	static constexpr std::size_t PluginDataRoom = 16384;

	/**
	 * Makes the instrument's data for this note as m_pluginData, in the note's room: it must fit (checked when
	 * compiled), and allocate nothing itself. Destroy it with destroyPluginData<T>() (in
	 * Instrument::deleteNotePluginData()).
	 */
	template<class T, class... Args>
	auto makePluginData(Args&&... args) -> T*
	{
		static_assert(sizeof(T) <= PluginDataRoom, "the note's data must fit its room");
		static_assert(alignof(T) <= alignof(std::max_align_t), "the note's data must fit its room's alignment");
		auto* data = new (static_cast<void*>(m_pluginDataRoom)) T(std::forward<Args>(args)...);
		m_pluginData = data;
		return data;
	}

	//! Destroys the data made by makePluginData<T>()
	template<class T>
	void destroyPluginData()
	{
		if (auto* data = static_cast<T*>(m_pluginData)) { data->~T(); }
		m_pluginData = nullptr;
	}

	void setVolume( volume_t volume ) override;
	void setPanning( panning_t panning ) override;

	int midiKey() const;
	int midiChannel() const
	{
		return m_midiChannel;
	}

	/*! convenience function that returns offset for the first period and zero otherwise,
		used by instruments to handle the offset: instruments have to check this property and
		add the correct number of empty frames in the beginning of the period */
	f_cnt_t noteOffset() const
	{
		return m_totalFramesPlayed == 0
			? offset()
			: 0;
	}

	const float& frequency() const
	{
		return m_frequency;
	}

	/*! Returns frequency without pitch wheel influence */
	float unpitchedFrequency() const
	{
		return m_unpitchedFrequency;
	}

	//! Get the current per-note detuning for this note
	float currentDetuning() const { return m_baseDetuning->value(); }

	/*! Renders one chunk using the attached instrument into the buffer */
	void play(std::span<SampleFrame> buffer) override;

	/**
	 * Releases the note if it ends during the coming period, together with its sub-notes (e.g. of an arpeggio).
	 * Called for every note before the notes of the period are rendered in parallel: released from the parent's
	 * play() instead, a sub-note rendered before its parent would be released a period late, depending on which
	 * thread got there first. [audio engine thread, before rendering the period]
	 */
	void releaseIfEnding();

	/*! Returns whether playback of note is finished and thus handle can be deleted */
	bool isFinished() const override
	{
		return m_released && framesLeft() <= 0;
	}

	/*! Returns number of frames left for playback */
	f_cnt_t framesLeft() const;

	/*! Returns how many frames have to be rendered in current period */
	f_cnt_t framesLeftForCurrentPeriod() const;

	/*! Returns whether the play handle plays on a certain track */
	bool isFromTrack( const Track* _track ) const override;

	/*! Releases the note (and plays release frames) */
	void noteOff( const f_cnt_t offset = 0 );

	/*! Returns number of frames to be played until the note is going to be released */
	f_cnt_t framesBeforeRelease() const
	{
		return m_framesBeforeRelease;
	}

	/*! Returns how many frames were played since release */
	f_cnt_t releaseFramesDone() const
	{
		return m_releaseFramesDone;
	}

	/*! Returns the number of frames to be played after release according to
	    the release times in the envelopes */
	f_cnt_t actualReleaseFramesToDo() const;

	/*! Returns total numbers of frames to play (including release frames) */
	f_cnt_t frames() const
	{
		return m_frames;
	}

	/*! Sets the total number of frames to play (including release frames) */
	void setFrames( const f_cnt_t _frames );

	/*! Returns whether note was released */
	bool isReleased() const
	{
		return m_released;
	}

	bool isReleaseStarted() const
	{
		return m_releaseStarted;
	}

	/*! Returns total numbers of frames played so far */
	f_cnt_t totalFramesPlayed() const
	{
		return m_totalFramesPlayed;
	}

	/*! Returns volume level at given frame (envelope/LFO) */
	float volumeLevel( const f_cnt_t frame );

	/*! Returns instrument track which is being played by this handle (const version) */
	const InstrumentTrack* instrumentTrack() const
	{
		return m_instrumentTrack;
	}

	/*! Returns instrument track which is being played by this handle */
	InstrumentTrack* instrumentTrack()
	{
		return m_instrumentTrack;
	}

	/*! Returns whether note has a parent, e.g. is not part of an arpeggio or a chord */
	bool hasParent() const
	{
		return m_hasParent;
	}

	/*! Returns origin of note */
	Origin origin() const
	{
		return m_origin;
	}

	/*! Returns whether note has children */
	bool isMasterNote() const
	{
		return m_subNotes.size() > 0 || m_hadChildren;
	}

	void setMasterNote()
	{
		m_hadChildren = true;
		setUsesBuffer( false );
	}

	/*! Returns whether note is muted */
	bool isMuted() const
	{
		return m_muted;
	}

	/*! Mutes playback of note */
	void mute();

	/*! Returns index of NotePlayHandle in vector of note-play-handles
	    belonging to this instrument track - used by arpeggiator.
	    Ignores child note-play-handles, returns -1 when called on one */
	int index() const;

	/*! Fills @p list (emptied first) with the note-play-handles belonging to given instrument track.
	    If allPlayHandles = true, also released note-play-handles and children
	    are returned. A list kept from call to call allocates only when it needs more room than before */
	static void nphsOfInstrumentTrack(const InstrumentTrack* track, ConstNotePlayHandleList& list,
		bool allPlayHandles = false);
	//! The same, into fixed room: the notes past its 256 are left out
	static void nphsOfInstrumentTrack(const InstrumentTrack* track, FixedNotePlayHandleList& list,
		bool allPlayHandles = false);

	/*! Returns whether given NotePlayHandle instance is equal to *this */
	bool operator==( const NotePlayHandle & _nph ) const;

	/*! Returns whether NotePlayHandle belongs to pattern track and pattern track is muted */
	bool isPatternTrackMuted()
	{
		return m_patternTrack && m_patternTrack->isSilenced();
	}

	/*! Sets attached pattern track */
	void setPatternTrack(Track* t)
	{
		m_patternTrack = t;
	}

	/*! Process note detuning automation */
	void processTimePos(const TimePos& time, float pitchValue, bool isRecording);

	/*! Updates total length (m_frames) depending on a new tempo */
	void resize( const bpm_t newTempo );

	/*! Set song-global offset (relative to containing MIDI clip) in order to properly perform the note detuning */
	void setSongGlobalParentOffset( const TimePos& offset )
	{
		m_songGlobalParentOffset = offset;
	}

	/*! Returns song-global offset */
	const TimePos& songGlobalParentOffset() const
	{
		return m_songGlobalParentOffset;
	}

	void setFrequencyUpdate()
	{
		m_frequencyNeedsUpdate = true;
	}

private:
	class BaseDetuning
	{
	public:
		BaseDetuning() = default;
		BaseDetuning( DetuningHelper* detuning );

		void setValue( float val )
		{
			m_value = val;
		}

		float value() const
		{
			return m_value;
		}


	private:
		float m_value = 0.f;

	} ;

	void updateFrequency();

	InstrumentTrack* m_instrumentTrack;		// needed for calling
											// InstrumentTrack::playNote
	f_cnt_t m_frames;						// total frames to play
	f_cnt_t m_totalFramesPlayed;			// total frame-counter - used for
											// figuring out whether a whole note
											// has been played
	f_cnt_t m_framesBeforeRelease;			// number of frames after which note
											// is released
	f_cnt_t m_releaseFramesToDo;			// total numbers of frames to be
											// played after release
	f_cnt_t m_releaseFramesDone;			// number of frames done after
											// release of note
	SubNoteList m_subNotes;			// used for chords and arpeggios
	std::atomic<bool> m_released;				// indicates whether note is released
	bool m_releaseStarted;
	bool m_hasMidiNote;
	bool m_hasParent;						// indicates whether note has parent
	NotePlayHandle * m_parent;			// parent note
	bool m_hadChildren;
	bool m_muted;							// indicates whether note is muted
	Track* m_patternTrack;						// related pattern track

	// tempo reaction
	bpm_t m_origTempo;						// original tempo
	f_cnt_t m_origFrames;					// original m_frames

	int m_origBaseNote;

	float m_frequency;
	float m_unpitchedFrequency;

	BaseDetuning m_ownBaseDetuning; //!< a note without a parent's; its chord or arpeggio notes share it
	BaseDetuning* m_baseDetuning;
	TimePos m_songGlobalParentOffset;

	int m_midiChannel;
	Origin m_origin;

	std::atomic<bool> m_frequencyNeedsUpdate;	// used to update pitch (set from other threads)

	//! See mixOrder(): the root note's serial in the high 32 bits, then the child's index at each level (8 bits each)
	std::uint64_t m_mixOrder = 0;
	int m_depth = 0; //!< 0 for a note without a parent
	int m_childrenMade = 0; //!< the chord and arpeggio notes made so far, which numbers the next one
	//! Numbers the notes without a parent in the order they are made (by their tracks, one after another)
	static std::atomic<std::uint32_t> s_nextRootSerial;

	alignas(std::max_align_t) std::byte m_pluginDataRoom[PluginDataRoom]; //!< see makePluginData()

	friend class NotePlayHandleManager;
	BasicFilters<>* m_poolFilter = nullptr; //!< the pool's filter for this place, lent while the note lives
	BasicFilters<>* m_filter = nullptr; //!< the filter in use, once one is (see filter())
	std::unique_ptr<BasicFilters<>> m_ownFilter; //!< a note from outside the pool makes its own
} ;


const int INITIAL_NPH_CACHE = 256;
const int NPH_CACHE_INCREMENT = 16;

class NotePlayHandleManager
{
public:
	static void init();
	static NotePlayHandle * acquire( InstrumentTrack* instrumentTrack,
					const f_cnt_t offset,
					const f_cnt_t frames,
					const Note& noteToPlay,
					NotePlayHandle* parent = nullptr,
					int midiEventChannel = -1,
					NotePlayHandle::Origin origin = NotePlayHandle::Origin::MidiClip );
	static void release( NotePlayHandle * nph );
	static void extend( int i );
	static void free();

private:
	//! A place for a note, with its filter (made with the place, so that a note made on the audio thread allocates
	//! nothing)
	struct Slot
	{
		NotePlayHandle* memory;
		BasicFilters<>* filter;
	};
	static void addSlots(int count);

	static Slot* s_available;
	static QReadWriteLock s_mutex;
	static std::atomic_int s_availableIndex;
	static int s_size;
};


} // namespace lmms

#endif // LMMS_NOTE_PLAY_HANDLE_H
