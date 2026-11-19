/*
 * Track.cpp - implementation of Track class
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

/** \file Track.cpp
 *  \brief Implementation of Track class
 */


#include "Track.h"
#include "UndoCommand.h"
#include "ProjectJournal.h"

#include <algorithm>

#include <QDomElement>
#include <QVariant>

#include "AutomationClip.h"
#include "AudioTrack.h"
#include "AutomationTrack.h"
#include "FolderTrack.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "PatternStore.h"
#include "PatternTrack.h"
#include "SampleTrack.h"
#include "Mixer.h"
#include "Song.h"


namespace lmms
{

Track::Track(Type type, TrackContainer* tc)
	: Model(tc)
	, m_trackContainer(tc)
	, m_type(type)
	, m_name()
	, m_mutedModel(false, this, tr("Mute"))
	, m_soloModel(false, this, tr("Solo"))
	, m_clips()
{
	// (published in the container once it is whole, by create())
	m_height = -1;
	// A solo, or a folder's mute, silences other tracks
	connect(&m_soloModel, &BoolModel::dataChanged, this, [this] { m_trackContainer->updateSilence(); });
	connect(&m_mutedModel, &BoolModel::dataChanged, this, [this] {
		if (m_type == Type::Folder) { m_trackContainer->updateSilence(); }
	});
}




Track::~Track()
{
	lock();
	emit destroyedTrack();

	// The derived track is gone already: nothing it connected to its own signals may run now (a clip removed
	// tells of the lanes, see removeClip())
	blockSignals(true);
	while (!m_clips.empty())
	{
		deleteClip(m_clips.back());
	}

	// (out of it already when deleted by destroy(): then nothing happens)
	m_trackContainer->removeTrack( this );
	unlock();
}




void Track::destroy(Track* track)
{
	if (!track) { return; }

	// Out of its container first: the audio thread no longer plays it, and it is whole when it leaves (each of these
	// takes the lock for its moment)
	track->m_trackContainer->removeTrack(track);
	// The automation of its controls (other tracks' clips set them) lets them go, all in one moment rather than one
	// moment each as they are deleted
	AutomationClip::forgetObjectsUnder(track);

	// Then deleted without the lock (see the documentation)
	delete track;
}




auto Track::make(Type tt, TrackContainer* tc) -> Track*
{
	// Made whole without the audio engine's change lock (the parts that the audio thread sees, its buses and MIDI
	// ports, take the lock themselves for their moment): the audio thread does not wait for it
	switch( tt )
	{
		case Type::Instrument: return new class InstrumentTrack( tc );
		case Type::Pattern: return new class PatternTrack( tc );
		case Type::Sample: return new class SampleTrack( tc );
//		case Type::Event:
//		case Type::Video:
		case Type::Automation: return new class AutomationTrack( tc );
		case Type::HiddenAutomation: return new class AutomationTrack( tc, true );
		case Type::Audio:
			// Audio tracks play on the song's timeline
			return tc == Engine::getSong() ? new AudioTrack(tc) : nullptr;
		case Type::Folder:
			// Folders only group the tracks of the song
			return tc == Engine::getSong() ? new FolderTrack(tc) : nullptr;
		default: return nullptr;
	}
}




void Track::publish(Track* t, TrackContainer* tc, bool newTrack)
{
	// A new track of the pattern editor has a clip for each pattern: made while no one else sees the track
	if (newTrack && tc == Engine::patternStore())
	{
		t->createClipsForPattern(Engine::patternStore()->numOfPatterns() - 1);
	}

	{
		// Under the lock, briefly: the audio thread goes through the tracks
		const auto guard = AudioEngine::changesGuard();
		tc->addTrack(t);
	}

	// E.g. the first track of the pattern editor makes the song's pattern track (a track of its own, published so)
	tc->updateAfterTrackAdd();

	// A track of the song or the pattern editor gets its own mixer channel: a new instrument or sample track, or a
	// saved one that would share another's (a cloned track); not while a project loads (it has its channels). Each
	// step of the mixer takes the lock for its own moment (a channel is made whole outside it): until then the track
	// plays into the channel its settings name, as when the user picks another
	if (Engine::getSong() && Engine::mixer() && Engine::patternStore() && !Engine::getSong()->isLoadingProject()
		&& (tc == Engine::getSong() || tc == Engine::patternStore()))
	{
		Engine::mixer()->setUpTrackChannel(t, newTrack);
	}
	tc->updateFolders();

	// Told last, the lock released: the GUI makes the track's views for it
	if (t->type() != Type::HiddenAutomation) { emit tc->trackAdded(t); }
}




Track * Track::create( Type tt, TrackContainer * tc )
{
	auto* t = make(tt, tc);
	if (t) { publish(t, tc, true); }
	return t;
}




Track * Track::create( const QDomElement & element, TrackContainer * tc )
{
	// A sample track with recordings is an audio track now (see AudioTrack::fromSampleTrack())
	const auto converted = tc == Engine::getSong() ? AudioTrack::fromSampleTrack(element) : std::nullopt;
	const auto& state = converted ? *converted : element;

	// Made and restored whole first (its instrument and samples loaded), then published
	auto* t = make(static_cast<Type>(state.attribute("type").toInt()), tc);
	if (!t) { return nullptr; }
	t->restoreState(state);
	publish(t, tc, false);
	return t;
}




Track* Track::clone()
{
	// Save track to temporary XML and load it to create a new identical track
	QDomDocument doc;
	QDomElement parent = doc.createElement("clonedtrack");
	cloneState(doc, parent);
	Track* t = create(parent.firstChild().toElement(), m_trackContainer);

	AutomationClip::resolveAllIDs();
	return t;
}


void Track::saveTrack(QDomDocument& doc, QDomElement& element, bool presetMode)
{
	if (!presetMode)
	{
		element.setTagName( "track" );
	}
	element.setAttribute( "type", static_cast<int>(type()) );
	element.setAttribute( "name", name() );
	m_mutedModel.saveSettings( doc, element, "muted" );
	m_soloModel.saveSettings( doc, element, "solo" );

	if( m_height >= MINIMAL_TRACK_HEIGHT )
	{
		element.setAttribute( "trackheight", m_height );
	}
	
	if (m_color.has_value())
	{
		element.setAttribute("color", m_color->name());
	}

	if (m_folderDepth > 0 && !presetMode) { element.setAttribute("folderdepth", m_folderDepth); }
	if (m_playingLane > 0 && !presetMode) { element.setAttribute("playinglane", m_playingLane); }
	
	QDomElement tsDe = doc.createElement( nodeName() );
	// let actual track (InstrumentTrack, PatternTrack, SampleTrack etc.) save its settings
	element.appendChild( tsDe );
	saveTrackSpecificSettings(doc, tsDe, presetMode);

	if (presetMode)
	{
		return;
	}

	// now save settings of all Clip's (with their lanes)
	for (const auto& clip : m_clips)
	{
		clip->saveState(doc, element);
		if (clip->lane() > 0) { element.lastChildElement().setAttribute("lane", clip->lane()); }
	}
}


void Track::loadTrack(const QDomElement& element, bool presetMode)
{
	if( static_cast<Type>(element.attribute( "type" ).toInt()) != type() )
	{
		qWarning( "Current track-type does not match track-type of "
							"settings-node!\n" );
	}

	setName( element.hasAttribute( "name" ) ? element.attribute( "name" ) :
			element.firstChild().toElement().attribute( "name" ) );

	m_mutedModel.loadSettings( element, "muted" );
	m_soloModel.loadSettings( element, "solo" );
	// LMMS soloed by muting the other tracks; see legacyMutedBeforeSolo()
	m_legacyMutedBeforeSolo = element.hasAttribute("mutedBeforeSolo")
		? std::optional{element.attribute("mutedBeforeSolo").toInt() != 0} : std::nullopt;

	if (element.hasAttribute("color"))
	{
		setColor(QColor{element.attribute("color")});
	}

	if (!presetMode)
	{
		setFolderDepth(element.attribute("folderdepth", "0").toInt());
		applyPlayingLane(element.attribute("playinglane", "0").toInt());
	}

	if (presetMode)
	{
		QDomNode node = element.firstChild();
		while( !node.isNull() )
		{
			if( node.isElement() && node.nodeName() == nodeName() )
			{
				loadTrackSpecificSettings( node.toElement() );
				break;
			}
			node = node.nextSibling();
		}

		return;
	}

	// (each clip leaves the track in a moment under the audio engine's lock and is deleted without it, see deleteClip())
	deleteClips();

	QDomNode node = element.firstChild();
	while( !node.isNull() )
	{
		if( node.isElement() )
		{
			if( node.nodeName() == nodeName() )
			{
				loadTrackSpecificSettings( node.toElement() );
			}
			else if( node.nodeName() != "muted"
			&& node.nodeName() != "solo"
			&& !node.toElement().attribute( "metadata" ).toInt() )
			{
				Clip * clip = createClip(
								TimePos( 0 ) );
				if (clip)
				{
					clip->restoreState(node.toElement());
					clip->setLane(node.toElement().attribute("lane", "0").toInt());
				}
			}
		}
		node = node.nextSibling();
	}

	int storedHeight = element.attribute( "trackheight" ).toInt();
	if( storedHeight >= MINIMAL_TRACK_HEIGHT )
	{
		m_height = storedHeight;
	}
}

void Track::savePreset(QDomDocument & doc, QDomElement & element)
{
	saveTrack(doc, element, true);
}

void Track::loadPreset(const QDomElement & element)
{
	loadTrack(element, true);
}

void Track::saveSettings(QDomDocument& doc, QDomElement& element)
{
	// Assume that everything should be saved if we are called through SerializingObject::saveSettings
	saveTrack(doc, element, false);
}

void Track::loadSettings(const QDomElement& element)
{
	// Assume that everything should be loaded if we are called through SerializingObject::loadSettings 
	loadTrack(element, false);
}




namespace
{

//! Chooses the lane heard (finds the track by its journal id)
class PlayingLaneCommand : public UndoCommand
{
public:
	PlayingLaneCommand(Track* track, int before, int after) : m_track{track->id()}, m_before{before}, m_after{after} {}
	void apply() override { set(m_after); }
	void revert() override { set(m_before); }
	auto text() const -> QString override { return Track::tr("Choose the lane heard"); }

private:
	void set(int lane)
	{
		if (auto* track = dynamic_cast<Track*>(Engine::projectJournal()->journallingObject(m_track)))
		{
			track->applyPlayingLane(lane);
		}
	}
	jo_id_t m_track;
	int m_before;
	int m_after;
};

} // namespace




void Track::setPlayingLane(int lane)
{
	lane = std::max(lane, 0);
	if (lane == m_playingLane) { return; }
	Engine::projectJournal()->execute(std::make_unique<PlayingLaneCommand>(this, m_playingLane, lane));
	if (auto* song = Engine::getSong()) { song->setModified(); }
}




void Track::applyPlayingLane(int lane)
{
	lane = std::max(lane, 0);
	if (lane == m_playingLane) { return; }
	m_playingLane = lane;
	checkLanes();
	emit dataChanged();
}




auto Track::laneCount() const -> int
{
	auto count = std::max(1, m_playingLane + 1);
	for (const auto* clip : m_clips) { count = std::max(count, clip->lane() + 1); }
	return count;
}




void Track::checkLanes()
{
	const auto lanes = laneCount();
	if (lanes == m_lastLaneCount && m_playingLane == m_lastPlayingLane) { return; }
	m_lastLaneCount = lanes;
	m_lastPlayingLane = m_playingLane;
	emit lanesChanged();
}




void Track::insertLanesOnTop(int count)
{
	if (count <= 0) { return; }
	for (auto* clip : m_clips) { clip->setLane(clip->lane() + count); }
	applyPlayingLane(0);
}




void Track::clearSpan(int lane, tick_t begin, tick_t end, const Clip* keep)
{
	if (end <= begin) { return; }
	const auto clips = m_clips; // (clips are removed and added below)
	for (auto* clip : clips)
	{
		if (clip == keep || clip->lane() != lane) { continue; }
		const auto clipBegin = clip->startPosition().getTicks();
		const auto clipEnd = clip->endPosition().getTicks();
		if (clipEnd <= begin || clipBegin >= end) { continue; }
		// Where its content starts on the song (it stays there under each part)
		const auto contentStart = clipBegin + clip->startTimeOffset().getTicks();
		if (clipBegin >= begin && clipEnd <= end)
		{
			deleteClip(clip); // (all under it)
			continue;
		}
		if (clipBegin < begin && clipEnd > end)
		{
			// Split around it: the part after it is a copy
			auto* after = clip->clone();
			after->movePosition(TimePos{end});
			after->changeLength(TimePos{clipEnd - end});
			after->setStartTimeOffset(TimePos{contentStart - end});
			clip->changeLength(TimePos{begin - clipBegin});
			continue;
		}
		if (clipBegin < begin) { clip->changeLength(TimePos{begin - clipBegin}); } // (its end under it)
		else
		{
			// Its start under it
			clip->movePosition(TimePos{end});
			clip->changeLength(TimePos{clipEnd - end});
			clip->setStartTimeOffset(TimePos{contentStart - end});
		}
	}
}




void Track::compClip(Clip* clip)
{
	if (!clip || clip->getTrack() != this || clip->lane() == m_playingLane) { return; }
	addJournalCheckPoint();
	const auto pause = ProjectJournal::Pause{Engine::projectJournal()};
	auto* copy = clip->clone();
	copy->setLane(m_playingLane);
	clearSpan(m_playingLane, copy->startPosition().getTicks(), copy->endPosition().getTicks(), copy);
	if (auto* song = Engine::getSong()) { song->setModified(); }
}




Clip * Track::addClip( Clip * clip )
{
	{
		// The audio thread goes through the clips (play(), the automation), and adding one may move them all
		const auto guard = AudioEngine::changesGuard();
		m_clips.push_back( clip );
	}
	connect(clip, &Clip::laneChanged, this, &Track::checkLanes);

	emit clipAdded( clip );

	return clip; // just for convenience
}




void Track::removeClip( Clip * clip )
{
	auto it = std::find(m_clips.begin(), m_clips.end(), clip);
	if( it != m_clips.end() )
	{
		{
			// The audio thread goes through the clips: it takes them as they are before or after, not halfway
			const auto guard = AudioEngine::changesGuard();
			m_clips.erase( it );
			// The piano roll may be playing it alone
			if (const auto song = Engine::getSong()) { song->forgetClip(clip); }
		}
		clip->disconnect(this);
		checkLanes();
		if( Engine::getSong() )
		{
			Engine::getSong()->updateLength();
			Engine::getSong()->setModified();
		}
	}
}


void Track::deleteClip(Clip* clip)
{
	if (!clip) { return; }
	removeClip(clip);
	delete clip;
}




void Track::deleteClips()
{
	while (!m_clips.empty())
	{
		deleteClip(m_clips.front());
	}
}




auto Track::getClip(std::size_t clipNum) -> Clip*
{
	if( clipNum < m_clips.size() )
	{
		return m_clips[clipNum];
	}
	printf( "called Track::getClip( %zu ), "
			"but Clip %zu doesn't exist\n", clipNum, clipNum );
	return createClip( clipNum * TimePos::ticksPerBar() );

}




int Track::getClipNum( const Clip * clip )
{
//	for( int i = 0; i < getTrackContentWidget()->numOfClips(); ++i )
	clipVector::iterator it = std::find( m_clips.begin(), m_clips.end(), clip );
	if( it != m_clips.end() )
	{
/*		if( getClip( i ) == _clip )
		{
			return i;
		}*/
		return it - m_clips.begin();
	}
	qWarning( "Track::getClipNum(...) -> _clip not found!\n" );
	return 0;
}




void Track::getClipsInRange( clipVector & clipV, const TimePos & start,
							const TimePos & end )
{
	for( Clip* clip : m_clips )
	{
		int s = clip->startPosition();
		int e = clip->endPosition();
		if( ( s <= end ) && ( e >= start ) )
		{
			// Clip is within given range
			// Insert sorted by Clip's position
			clipV.insert(std::upper_bound(clipV.begin(), clipV.end(), clip, Clip::comparePosition),
						clip);
		}
	}
}




void Track::swapPositionOfClips( int clipNum1, int clipNum2 )
{
	// The audio thread goes through the clips
	const auto guard = AudioEngine::changesGuard();
	qSwap( m_clips[clipNum1], m_clips[clipNum2] );

	const TimePos pos = m_clips[clipNum1]->startPosition();

	m_clips[clipNum1]->movePosition( m_clips[clipNum2]->startPosition() );
	m_clips[clipNum2]->movePosition( pos );
}




void Track::createClipsForPattern(int pattern)
{
	while( numOfClips() < pattern + 1 )
	{
		TimePos position = TimePos( numOfClips(), 0 );
		Clip * clip = createClip( position );
		clip->changeLength( TimePos( 1, 0 ) );
	}
}




//! @todo If we stepped through this list last to first, and the list was in ascending order by Clip time, once we hit
//! a Clip that was earlier than the insert time, we could fall out of the loop early.
void Track::insertBar(const TimePos& pos, tick_t length)
{
	if (length <= 0) { length = TimePos::ticksPerBar(); }
	// we'll increase the position of every Clip, positioned behind pos, by
	// one bar
	for (const auto& clip : m_clips)
	{
		if (clip->startPosition() >= pos)
		{
			clip->movePosition(clip->startPosition() + length);
		}
	}
}




void Track::removeBar(const TimePos& pos, tick_t length)
{
	if (length <= 0) { length = TimePos::ticksPerBar(); }
	// we'll decrease the position of every Clip, positioned behind pos, by
	// one bar
	for (const auto& clip : m_clips)
	{
		if (clip->startPosition() >= pos)
		{
			clip->movePosition(clip->startPosition() - length);
		}
	}
}




bar_t Track::length() const
{
	// find last end-position
	tick_t last = 0;
	for (const auto& clip : m_clips)
	{
		if (Engine::getSong()->isExporting() && clip->isMuted())
		{
			continue;
		}

		const tick_t cur = clip->endPosition();
		if( cur > last )
		{
			last = cur;
		}
	}

	return last / TimePos::ticksPerBar();
}



void Track::setFolderDepth(int depth)
{
	depth = std::max(depth, 0);
	if (depth == m_folderDepth) { return; }
	m_folderDepth = depth;
	emit folderDepthChanged();
}




auto Track::isMutedInMix() const -> bool
{
	if (isSilenced()) { return true; }
	const auto channel = Engine::mixer() ? Engine::mixer()->trackChannel(this) : nullptr;
	return channel && channel->m_muteModel.value();
}

void Track::setColor(const std::optional<QColor>& color)
{
	m_color = color;
	emit colorChanged();
}

BoolModel *Track::getMutedModel()
{
	return &m_mutedModel;
}

void Track::setName(const QString& newName)
{
	if (m_name != newName)
	{
		m_name = newName;

		if (auto song = Engine::getSong())
		{
			song->setModified();
		}
		
		emit nameChanged();
	}
}

} // namespace lmms
