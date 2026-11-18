/*
 * NotePlayHandle.cpp - implementation of class NotePlayHandle which manages
 *                      playback of a single note by an instrument
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

#include "NotePlayHandle.h"

#include "AudioEngine.h"
#include "DetuningHelper.h"
#include "InstrumentSoundShaping.h"
#include "InstrumentTrack.h"
#include "Instrument.h"
#include "Song.h"
#include "lmms_math.h"

namespace lmms
{

std::atomic<std::uint32_t> NotePlayHandle::s_nextRootSerial{0};

NotePlayHandle::BaseDetuning::BaseDetuning( DetuningHelper *detuning ) :
	m_value( detuning ? detuning->automationClip()->valueAt( 0 ) : 0 )
{
}






NotePlayHandle::NotePlayHandle( InstrumentTrack* instrumentTrack,
								const f_cnt_t _offset,
								const f_cnt_t _frames,
								const Note& n,
								NotePlayHandle *parent,
								int midiEventChannel,
								Origin origin ) :
	PlayHandle( PlayHandle::Type::NotePlayHandle, _offset ),
	Note(n),
	m_pluginData( nullptr ),
	m_instrumentTrack( instrumentTrack ),
	m_frames( 0 ),
	m_totalFramesPlayed( 0 ),
	m_framesBeforeRelease( 0 ),
	m_releaseFramesToDo( 0 ),
	m_releaseFramesDone( 0 ),
	m_subNotes(),
	m_released( false ),
	m_releaseStarted( false ),
	m_hasMidiNote( false ),
	m_hasParent( parent != nullptr  ),
	m_parent( parent ),
	m_hadChildren( false ),
	m_muted( false ),
	m_patternTrack( nullptr ),
	m_origTempo( Engine::getSong()->getTempo() ),
	m_origBaseNote( instrumentTrack->baseNote() ),
	m_frequency( 0 ),
	m_unpitchedFrequency( 0 ),
	m_baseDetuning( nullptr ),
	m_songGlobalParentOffset( 0 ),
	m_midiChannel( midiEventChannel >= 0 ? midiEventChannel : instrumentTrack->midiPort()->realOutputChannel() ),
	m_origin( origin ),
	m_frequencyNeedsUpdate( false )
{
	lock();
	if( hasParent() == false )
	{
		m_mixOrder = static_cast<std::uint64_t>(s_nextRootSerial.fetch_add(1, std::memory_order_relaxed)) << 32;
		m_ownBaseDetuning = BaseDetuning{detuning().get()};
		m_baseDetuning = &m_ownBaseDetuning;
		m_instrumentTrack->m_processHandles.push_back( this );
	}
	else
	{
		m_baseDetuning = parent->m_baseDetuning;

		// The parent makes its chord and arpeggio notes one after another, so their numbers are always the same
		m_depth = parent->m_depth + 1;
		const auto index = static_cast<std::uint64_t>(std::min(parent->m_childrenMade++, 254) + 1);
		m_mixOrder = parent->m_mixOrder;
		if (m_depth <= 4) { m_mixOrder |= index << (32 - 8 * m_depth); }

		parent->m_subNotes.push_back( this );
		parent->m_hadChildren = true;

		m_patternTrack = parent->m_patternTrack;

		parent->setUsesBuffer( false );
	}

	updateFrequency();

	setFrames( _frames );

	// inform attached components about new MIDI note (used for recording in Piano Roll)
	if( m_origin == Origin::MidiInput )
	{
		m_instrumentTrack->midiNoteOn( *this );
	}

	if (m_instrumentTrack->instrument() && m_instrumentTrack->instrument()->isSingleStreamed())
	{
		setUsesBuffer( false );
	}

	setAudioBusHandle(instrumentTrack->audioBusHandle());

	unlock();
}


NotePlayHandle::~NotePlayHandle()
{
	lock();
	noteOff( 0 );

	if( hasParent() == false )
	{
		m_instrumentTrack->m_processHandles.removeOne( this );
	}
	else
	{
		m_parent->m_subNotes.removeOne( this );
	}

	if( m_pluginData != nullptr )
	{
		// TODO:
		m_instrumentTrack->deleteNotePluginData( this );
	}

	// (a note's key may be out of the keyboard: one from a damaged file, or transposed past it)
	if (key() >= 0 && key() < NumKeys && m_instrumentTrack->m_notes[key()] == this)
	{
		m_instrumentTrack->m_notes[key()] = nullptr;
	}

	m_subNotes.clear();

	if (buffer().data() != nullptr)
	{
		releaseBuffer();
	}

	unlock();
}




void NotePlayHandle::setVolume( volume_t _volume )
{
	Note::setVolume( _volume );

	const int baseVelocity = m_instrumentTrack->midiPort()->baseVelocity();

	m_instrumentTrack->processOutEvent( MidiEvent( MidiKeyPressure, midiChannel(), midiKey(), midiVelocity( baseVelocity ) ) );
}




void NotePlayHandle::setPanning( panning_t panning )
{
	Note::setPanning( panning );
}




int NotePlayHandle::midiKey() const
{
	return key() - m_origBaseNote + instrumentTrack()->baseNote();
}




f_cnt_t NotePlayHandle::framesPlayedAtPeriodStart() const
{
	// The total first, then the period it was last played in: if it was played this period, what it had before; if
	// not, the total read first is still what it had (it adds this period's frames only after it says so, see play())
	const auto total = m_totalFramesPlayed.load(std::memory_order_acquire);
	if (m_playedInPeriod.load(std::memory_order_acquire) == Engine::audioEngine()->periodsRendered())
	{
		return m_framesBeforePeriod.load(std::memory_order_relaxed);
	}
	return total;
}




void NotePlayHandle::play(std::span<SampleFrame> buffer)
{
	// What it played before this period, for the sibling notes that read it this period (framesPlayedAtPeriodStart()):
	// the frames first, then the period (the frames played this period are added at the end)
	m_framesBeforePeriod.store(m_totalFramesPlayed.load(std::memory_order_relaxed), std::memory_order_relaxed);
	m_playedInPeriod.store(Engine::audioEngine()->periodsRendered(), std::memory_order_release);

	if (m_muted)
	{
		return;
	}

	// if the note offset falls over to next period, then don't start playback yet
	if( offset() >= Engine::audioEngine()->framesPerPeriod() )
	{
		setOffset( offset() - Engine::audioEngine()->framesPerPeriod() );
		return;
	}

	lock();

	// Don't play the note if it falls outside of the user defined key range
	// TODO: handle the range check by Microtuner, and if the key becomes "not mapped", save the current frequency
	// so that the note release can finish playing using a valid frequency instead of a 1 Hz placeholder
	if (key() < m_instrumentTrack->m_firstKeyModel.value() ||
		key() > m_instrumentTrack->m_lastKeyModel.value())
	{
		// Release the note in case it started playing before going out of range
		noteOff(0);
		// Exit if the note did not start playing before going out of range (i.e. there is no need to play release)
		if (m_totalFramesPlayed == 0)
		{
			unlock();
			return;
		}
	}

	/* It is possible for NotePlayHandle::noteOff to be called before NotePlayHandle::play,
	 * which results in a note-on message being sent without a subsequent note-off message.
	 * Therefore, we check here whether the note has already been released before sending
	 * the note-on message. */
	if( !m_released
		&& m_totalFramesPlayed == 0 && !m_hasMidiNote
		&& ( hasParent() || ! m_instrumentTrack->isArpeggioEnabled() ) )
	{
		m_hasMidiNote = true;

		const int baseVelocity = m_instrumentTrack->midiPort()->baseVelocity();

		// send MidiNoteOn event
		m_instrumentTrack->processOutEvent(
			MidiEvent( MidiNoteOn, midiChannel(), midiKey(), midiVelocity( baseVelocity ) ),
			TimePos::fromFrames( offset(), Engine::framesPerTick() ),
			offset() );
	}

	if( m_frequencyNeedsUpdate )
	{
		updateFrequency();
	}

	// number of frames that can be played this period
	const f_cnt_t framesThisPeriod = m_totalFramesPlayed == 0
		? Engine::audioEngine()->framesPerPeriod() - offset()
		: Engine::audioEngine()->framesPerPeriod();

	// check if we start release during this period (usually done already, see releaseIfEnding())
	releaseIfEnding();

	// under some circumstances we're called even if there's nothing to play
	// therefore do an additional check which fixes crash e.g. when
	// decreasing release of an instrument-track while the note is active
	if( framesLeft() > 0 )
	{
		// play note!
		m_instrumentTrack->playNote(this, buffer);
	}

	if( m_released && (!instrumentTrack()->isSustainPedalPressed() ||
		m_releaseStarted) )
	{
		m_releaseStarted = true;

		f_cnt_t todo = framesThisPeriod;

		// if this note is base-note for arpeggio, always set
		// m_releaseFramesToDo to bigger value than m_releaseFramesDone
		// because we do not allow NotePlayHandle::isFinished() to be true
		// until all sub-notes are completely played and no new ones
		// are inserted by arpAndChordsTabWidget::processNote()
		if( ! m_subNotes.isEmpty() )
		{
			m_releaseFramesToDo = m_releaseFramesDone + 2 * Engine::audioEngine()->framesPerPeriod();
		}
		// look whether we have frames left to be done before release
		if( m_framesBeforeRelease )
		{
			// yes, then look whether these samples can be played
			// within one audio-buffer
			if( m_framesBeforeRelease <= framesThisPeriod )
			{
				// yes, then we did less releaseFramesDone
				todo -= m_framesBeforeRelease;
				m_framesBeforeRelease = 0;
			}
			else
			{
				// no, then just decrease framesBeforeRelease
				// and wait for next loop... (we're not in
				// release-phase yet)
				todo = 0;
				m_framesBeforeRelease -= framesThisPeriod;
			}
		}
		// look whether we're in release-phase
		if( todo && m_releaseFramesDone < m_releaseFramesToDo )
		{
			// check whether we have to do more frames in current
			// loop than left in current loop
			if( m_releaseFramesToDo - m_releaseFramesDone >= todo )
			{
				// yes, then increase number of release-frames
				// done
				m_releaseFramesDone += todo;
			}
			else
			{
				// no, we did all in this loop!
				m_releaseFramesDone = m_releaseFramesToDo;
			}
		}
	}

	// update internal data
	m_totalFramesPlayed += framesThisPeriod;
	unlock();
}




f_cnt_t NotePlayHandle::framesLeft() const
{
	if( instrumentTrack()->isSustainPedalPressed() )
	{
		return 4 * Engine::audioEngine()->framesPerPeriod();
	}
	else if( m_released && actualReleaseFramesToDo() == 0 )
	{
		return m_framesBeforeRelease;
	}
	else if( m_released )
	{
		return m_framesBeforeRelease + m_releaseFramesToDo - m_releaseFramesDone;
	}
	return m_frames+actualReleaseFramesToDo()-m_totalFramesPlayed;
}




f_cnt_t NotePlayHandle::framesLeftForCurrentPeriod() const
{
	if( m_totalFramesPlayed == 0 )
	{
		return std::min(framesLeft(), Engine::audioEngine()->framesPerPeriod() - offset());
	}
	return std::min(framesLeft(), Engine::audioEngine()->framesPerPeriod());
}




bool NotePlayHandle::isFromTrack( const Track * _track ) const
{
	return m_instrumentTrack == _track || m_patternTrack == _track;
}




void NotePlayHandle::releaseIfEnding()
{
	// Not playing in this period yet
	if (m_released || offset() >= Engine::audioEngine()->framesPerPeriod()) { return; }

	// number of frames that can be played this period
	const f_cnt_t framesThisPeriod = m_totalFramesPlayed == 0
		? Engine::audioEngine()->framesPerPeriod() - offset()
		: Engine::audioEngine()->framesPerPeriod();

	if (!instrumentTrack()->isSustainPedalPressed() && m_totalFramesPlayed + framesThisPeriod > m_frames)
	{
		noteOff( m_totalFramesPlayed == 0
			? ( m_frames + offset() ) // if we have noteon and noteoff during the same period, take offset in account for release frame
			: ( m_frames - m_totalFramesPlayed ) ); // otherwise, the offset is already negated and can be ignored
	}
}




void NotePlayHandle::noteOff( const f_cnt_t _s )
{
	if( m_released )
	{
		return;
	}
	m_released = true;

	// first note-off all sub-notes
	for( NotePlayHandle * n : m_subNotes )
	{
		n->lock();
		n->noteOff( _s );
		n->unlock();
	}

	// then set some variables indicating release-state
	m_framesBeforeRelease = _s;
	m_releaseFramesToDo = std::max<f_cnt_t>(0, actualReleaseFramesToDo());

	if( m_hasMidiNote )
	{
		m_hasMidiNote = false;

		// send MidiNoteOff event
		m_instrumentTrack->processOutEvent(
				MidiEvent( MidiNoteOff, midiChannel(), midiKey(), 0 ),
				TimePos::fromFrames( _s, Engine::framesPerTick() ),
				_s );
	}

	// inform attached components about MIDI finished (used for recording in Piano Roll)
	if (!instrumentTrack()->isSustainPedalPressed())
	{
		if( m_origin == Origin::MidiInput )
		{
			setLength( TimePos( static_cast<f_cnt_t>( totalFramesPlayed() / Engine::framesPerTick() ) ) );
			m_instrumentTrack->midiNoteOff( *this );
		}
	}
}




f_cnt_t NotePlayHandle::actualReleaseFramesToDo() const
{
	return m_instrumentTrack->m_soundShaping.releaseFrames();
}




void NotePlayHandle::setFrames( const f_cnt_t _frames )
{
	m_frames = _frames;
	if( m_frames == 0 )
	{
		m_frames = m_instrumentTrack->beatLen( this );
	}
	m_origFrames = m_frames;
}




float NotePlayHandle::volumeLevel( const f_cnt_t _frame )
{
	return m_instrumentTrack->m_soundShaping.volumeLevel( this, _frame );
}




void NotePlayHandle::mute()
{
	// mute all sub-notes
	for (const auto& subNote : m_subNotes)
	{
		subNote->mute();
	}
	m_muted = true;
}




int NotePlayHandle::index() const
{
	const PlayHandleList & playHandles = Engine::audioEngine()->playHandles();
	int idx = 0;
	for (const auto& playHandle : playHandles)
	{
		const auto nph = dynamic_cast<const NotePlayHandle*>(playHandle);
		if( nph == nullptr || nph->m_instrumentTrack != m_instrumentTrack || nph->isReleased() || nph->hasParent() )
		{
			continue;
		}
		if( nph == this )
		{
			return idx;
		}
		++idx;
	}
	return -1;
}




namespace
{

template<class List>
void collectNotesOf(const InstrumentTrack* track, List& list, bool all, auto&& instrumentTrackOf)
{
	const PlayHandleList & playHandles = Engine::audioEngine()->playHandles();
	list.clear();

	for (const auto& playHandle : playHandles)
	{
		const auto nph = dynamic_cast<const NotePlayHandle*>(playHandle);
		if (nph != nullptr && instrumentTrackOf(nph) == track
			&& ((nph->isReleased() == false && nph->hasParent() == false) || all))
		{
			if constexpr (requires { list.full(); })
			{
				if (list.full()) { return; }
			}
			list.push_back(nph);
		}
	}
}

} // namespace


void NotePlayHandle::nphsOfInstrumentTrack(const InstrumentTrack* _it, ConstNotePlayHandleList& cnphv, bool _all_ph)
{
	collectNotesOf(_it, cnphv, _all_ph, [](const NotePlayHandle* nph) { return nph->m_instrumentTrack; });
}


void NotePlayHandle::nphsOfInstrumentTrack(const InstrumentTrack* _it, FixedNotePlayHandleList& cnphv, bool _all_ph)
{
	collectNotesOf(_it, cnphv, _all_ph, [](const NotePlayHandle* nph) { return nph->m_instrumentTrack; });
}




bool NotePlayHandle::operator==( const NotePlayHandle & _nph ) const
{
	return length() == _nph.length() &&
			pos() == _nph.pos() &&
			key() == _nph.key() &&
			getVolume() == _nph.getVolume() &&
			getPanning() == _nph.getPanning() &&
			m_instrumentTrack == _nph.m_instrumentTrack &&
			m_frames == _nph.m_frames &&
			offset() == _nph.offset() &&
			m_totalFramesPlayed == _nph.m_totalFramesPlayed &&
			m_released == _nph.m_released &&
			m_hasParent == _nph.m_hasParent &&
			m_origBaseNote == _nph.m_origBaseNote &&
			m_muted == _nph.m_muted &&
			m_midiChannel == _nph.m_midiChannel &&
			m_origin == _nph.m_origin;
}




void NotePlayHandle::updateFrequency()
{
	int masterPitch = m_instrumentTrack->m_useMasterPitchModel.value() ? Engine::getSong()->masterPitch() : 0;
	int baseNote = m_instrumentTrack->baseNoteModel()->value();
	float detune = m_baseDetuning->value();
	float instrumentPitch = m_instrumentTrack->pitchModel()->value();

	if (m_instrumentTrack->m_microtuner.enabled())
	{
		// custom key mapping and scale: get frequency from the microtuner
		const auto transposedKey = key() + masterPitch;

		if (m_instrumentTrack->isKeyMapped(transposedKey))
		{
			const auto frequency = m_instrumentTrack->m_microtuner.keyToFreq(transposedKey, baseNote);
			m_frequency = frequency * std::exp2((detune + instrumentPitch / 100) / 12.f);
			m_unpitchedFrequency = frequency * std::exp2(detune / 12.f);
		}
		else
		{
			m_frequency = m_unpitchedFrequency = 0;
		}
	}
	else
	{
		// default key mapping and 12-TET frequency computation with default 440 Hz base note frequency
		const float pitch = (key() - baseNote + masterPitch + detune) / 12.0f;
		m_frequency = DefaultBaseFreq * std::exp2(pitch + instrumentPitch / (100 * 12.0f));
		m_unpitchedFrequency = DefaultBaseFreq * std::exp2(pitch);
	}

	// Each sub-note (of a chord, an arpeggio) works its own out when it plays: it may be playing now, on another of
	// the audio threads, and its oscillators read its frequency
	for (auto it : m_subNotes)
	{
		it->setFrequencyUpdate();
	}
}




void NotePlayHandle::processTimePos(const TimePos& time, float pitchValue, bool isRecording)
{
	if (!detuning() || time < songGlobalParentOffset() + pos()) { return; }

	if (isRecording && m_origin == Origin::MidiInput)
	{
		detuning()->automationClip()->recordValue(time - songGlobalParentOffset() - pos(), pitchValue / 100);
	}
	else
	{
		const float v = detuning()->automationClip()->valueAt(time - songGlobalParentOffset() - pos());
		if (!approximatelyEqual(v, m_baseDetuning->value()))
		{
			m_baseDetuning->setValue(v);
			updateFrequency();
		}
	}
}




void NotePlayHandle::resize( const bpm_t _new_tempo )
{
	if (origin() == Origin::MidiInput ||
		(origin() == Origin::NoteStacking && m_parent->origin() == Origin::MidiInput))
	{
		// Don't resize notes from MIDI input - they should continue to play
		// until the key is released, and their large duration can cause
		// overflows in this method.
		return;
	}

	double completed = m_totalFramesPlayed / (double) m_frames;
	double new_frames = m_origFrames * m_origTempo / (double) _new_tempo;
	m_frames = (f_cnt_t)new_frames;
	m_totalFramesPlayed = (f_cnt_t)( completed * new_frames );

	for (const auto& subNote : m_subNotes)
	{
		subNote->resize(_new_tempo);
	}
}


auto NotePlayHandle::filter() -> BasicFilters<>&
{
	if (m_filter == nullptr)
	{
		const auto rate = Engine::audioEngine()->outputSampleRate();
		if (m_poolFilter != nullptr)
		{
			m_poolFilter->setSampleRate(rate);
			m_poolFilter->clearHistory();
			m_filter = m_poolFilter;
		}
		else
		{
			m_ownFilter = std::make_unique<BasicFilters<>>(rate);
			m_filter = m_ownFilter.get();
		}
	}
	return *m_filter;
}




NotePlayHandleManager::Slot* NotePlayHandleManager::s_available;
QReadWriteLock NotePlayHandleManager::s_mutex;
std::atomic_int NotePlayHandleManager::s_availableIndex;
int NotePlayHandleManager::s_size;


void NotePlayHandleManager::addSlots(int count)
{
	// The list of available places takes all of them (they all may be given back at once)
	auto* list = new Slot[s_size + count];
	// How many are available, counted first: the first time there is no list (and adding -1 to it is undefined)
	const int available = s_availableIndex + 1;
	if (available > 0) { std::copy_n(s_available, available, list); }
	delete[] s_available;
	s_available = list;
	s_size += count;

	auto* memory = static_cast<NotePlayHandle*>(std::malloc(sizeof(NotePlayHandle) * count));
	for (int index = 0; index < count; ++index)
	{
		auto* filter = new BasicFilters<>(44100); // set to the engine's when a note uses it
		filter->reserveSubFilter();
		s_available[++s_availableIndex] = Slot{memory + index, filter};
	}
}


void NotePlayHandleManager::init()
{
	// (the engine's: Engine::init() and Engine::destroy() make and free it, once each)
	if (s_available) { return; }
	s_available = nullptr;
	s_size = 0;
	s_availableIndex = -1;
	addSlots(INITIAL_NPH_CACHE);
}


NotePlayHandle * NotePlayHandleManager::acquire( InstrumentTrack* instrumentTrack,
				const f_cnt_t offset,
				const f_cnt_t frames,
				const Note& noteToPlay,
				NotePlayHandle* parent,
				int midiEventChannel,
				NotePlayHandle::Origin origin )
{
	s_mutex.lockForWrite();
	// More notes at once than the pool holds: more places, allocated where they should not be
	if (s_availableIndex < 0) { addSlots(NPH_CACHE_INCREMENT); }
	const auto slot = s_available[s_availableIndex--];
	s_mutex.unlock();

	auto* nph = new( (void*)slot.memory ) NotePlayHandle( instrumentTrack, offset, frames, noteToPlay, parent,
		midiEventChannel, origin );
	nph->m_poolFilter = slot.filter;
	return nph;
}


void NotePlayHandle::dispose()
{
	NotePlayHandleManager::release(this);
}


void NotePlayHandleManager::release( NotePlayHandle * nph )
{
	auto* filter = nph->m_poolFilter;
	nph->NotePlayHandle::~NotePlayHandle();
	// Writes the list: a read lock let two releases (or a release and an acquire) write the same place
	s_mutex.lockForWrite();
	s_available[++s_availableIndex] = Slot{nph, filter};
	s_mutex.unlock();
}


void NotePlayHandleManager::extend( int c )
{
	s_mutex.lockForWrite();
	addSlots(c);
	s_mutex.unlock();
}

void NotePlayHandleManager::free()
{
	// The places' memory and filters stay until the program ends (a note may still be released while it does)
	delete[] s_available;
	s_available = nullptr;
}


} // namespace lmms
