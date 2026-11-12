/*
 * TrackContainer.cpp - implementation of base class for all trackcontainers
 *                      like Song-Editor, Pattern Editor...
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


#include <QCoreApplication>
#include <QProgressDialog>
#include <QDomElement>
#include <QWriteLocker>

#include "AutomationClip.h"
#include "embed.h"
#include "FolderTrack.h"
#include "Mixer.h"
#include "TrackContainer.h"
#include "PatternClip.h"
#include "PatternStore.h"
#include "PatternTrack.h"
#include "Song.h"

#include "GuiApplication.h"
#include "MainWindow.h"
#include "TextFloat.h"

namespace lmms
{


TrackContainer::TrackContainer() :
	Model( nullptr ),
	JournallingObject(),
	m_tracksMutex(),
	m_tracks()
{
}




TrackContainer::~TrackContainer()
{
	clearAllTracks();
}




void TrackContainer::saveSettings( QDomDocument & _doc, QDomElement & _this )
{
	_this.setTagName( classNodeName() );
	_this.setAttribute( "type", nodeName() );

	// save settings of each track
	m_tracksMutex.lockForRead();
	for (const auto& track : m_tracks)
	{
		track->saveState(_doc, _this);
	}
	m_tracksMutex.unlock();
}




void TrackContainer::loadSettings( const QDomElement & _this )
{
	bool journalRestore = _this.parentNode().nodeName() == "journaldata";
	if( journalRestore )
	{
		clearAllTracks();
	}

	static QProgressDialog * pd = nullptr;
	bool was_null = ( pd == nullptr );
	if (!journalRestore && gui::getGUI() != nullptr)
	{
		if( pd == nullptr )
		{
			pd = new QProgressDialog( tr( "Loading project..." ),
						tr( "Cancel" ), 0,
						Engine::getSong()->getLoadingTrackCount(),
						gui::getGUI()->mainWindow());
			pd->setWindowModality( Qt::ApplicationModal );
			pd->setWindowTitle( tr( "Please wait..." ) );
			pd->show();
		}
	}

	QDomNode node = _this.firstChild();
	while( !node.isNull() )
	{
		if( pd != nullptr )
		{
			pd->setValue( pd->value() + 1 );
			QCoreApplication::instance()->processEvents(
						QEventLoop::AllEvents, 100 );
			if( pd->wasCanceled() )
			{
				if (gui::getGUI() != nullptr)
				{
					gui::TextFloat::displayMessage( tr( "Loading cancelled" ),
					tr( "Project loading was cancelled." ),
					embed::getIconPixmap( "project_file", 24, 24 ),
					2000 );
				}
				Engine::getSong()->loadingCancelled();
				break;
			}
		}

		if( node.isElement() &&
			!node.toElement().attribute( "metadata" ).toInt() )
		{
			QString trackName = node.toElement().hasAttribute( "name" ) ?
						node.toElement().attribute( "name" ) :
						node.firstChild().toElement().attribute( "name" );
			if( pd != nullptr )
			{
				pd->setLabelText( tr("Loading Track %1 (%2/Total %3)").arg( trackName ).
						  arg( pd->value() + 1 ).arg( Engine::getSong()->getLoadingTrackCount() ) );
			}
			Track::create( node.toElement(), this );
		}
		node = node.nextSibling();
	}

	if( pd != nullptr )
	{
		if( was_null )
		{
			delete pd;
			pd = nullptr;
		}
	}
}




int TrackContainer::countTracks( Track::Type _tt ) const
{
	int cnt = 0;
	m_tracksMutex.lockForRead();
	for (const auto& track : m_tracks)
	{
		if (track->type() == _tt || _tt == Track::Type::Count)
		{
			++cnt;
		}
	}
	m_tracksMutex.unlock();
	return( cnt );
}




void TrackContainer::addTrack( Track * _track )
{
	if( _track->type() != Track::Type::HiddenAutomation )
	{
		_track->lock();
		m_tracksMutex.lockForWrite();
		m_tracks.push_back( _track );
		m_tracksMutex.unlock();
		_track->unlock();
		emit trackAdded( _track );
	}
}




void TrackContainer::removeTrack( Track * _track )
{
	// need a read locker to ensure that m_tracks doesn't change after reading index.
	//   After checking that index != -1, we need to upgrade the lock to a write locker before changing m_tracks.
	//   But since Qt offers no function to promote a read lock to a write lock, we must start with the write locker.
	QWriteLocker lockTracksAccess(&m_tracksMutex);
	auto it = std::find(m_tracks.begin(), m_tracks.end(), _track);
	if (it != m_tracks.end())
	{
		// What a folder holds moves up a level
		for (auto inner = std::next(it); _track->type() == Track::Type::Folder && inner != m_tracks.end()
			&& (*inner)->folderDepth() > _track->folderDepth(); ++inner)
		{
			(*inner)->setFolderDepth((*inner)->folderDepth() - 1);
		}
		m_tracks.erase(it);
		lockTracksAccess.unlock();

		if( Engine::getSong() )
		{
			Engine::getSong()->setModified();
		}
		updateFolders();
		emit trackRemoved();
	}
}

void TrackContainer::moveTrack(Track* track, int indexTo)
{
	m_tracks.erase(std::find(m_tracks.begin(), m_tracks.end(), track));
	m_tracks.insert(m_tracks.begin() + indexTo, track);

	updateFolders();
	emit trackMoved();
}




auto TrackContainer::folderOf(const Track* track) const -> FolderTrack*
{
	const auto it = std::find(m_tracks.begin(), m_tracks.end(), track);
	if (it == m_tracks.end() || track->folderDepth() == 0) { return nullptr; }

	// The nearest folder above that is one level up
	for (auto above = std::make_reverse_iterator(it); above != m_tracks.rend(); ++above)
	{
		if ((*above)->folderDepth() < track->folderDepth())
		{
			return dynamic_cast<FolderTrack*>(*above);
		}
	}
	return nullptr;
}




auto TrackContainer::folderContents(const Track* folder) const -> TrackList
{
	auto contents = TrackList{};
	if (!folder || folder->type() != Track::Type::Folder) { return contents; }

	auto it = std::find(m_tracks.begin(), m_tracks.end(), folder);
	if (it == m_tracks.end()) { return contents; }
	for (++it; it != m_tracks.end() && (*it)->folderDepth() > folder->folderDepth(); ++it)
	{
		contents.push_back(*it);
	}
	return contents;
}




auto TrackContainer::isInFolder(const Track* track, const Track* folder) const -> bool
{
	const auto contents = folderContents(folder);
	return std::find(contents.begin(), contents.end(), track) != contents.end();
}




void TrackContainer::changeFolderDepth(Track* track, int delta, bool addUndoStep)
{
	const auto it = std::find(m_tracks.begin(), m_tracks.end(), track);
	if (it == m_tracks.end() || delta == 0) { return; }

	// Into the folder above: only if there is one at the same level, or the track above is in one
	const int depth = track->folderDepth() + delta;
	const int maxDepth = it == m_tracks.begin() ? 0
		: (*std::prev(it))->folderDepth() + ((*std::prev(it))->type() == Track::Type::Folder ? 1 : 0);
	if (depth < 0 || depth > maxDepth) { return; }

	if (addUndoStep) { m_folderLayout.addJournalCheckPoint(); }

	// Out of a folder: below everything the folder holds, so that the tracks after it stay in the folder
	if (delta < 0)
	{
		if (const auto folder = folderOf(track))
		{
			const auto first = static_cast<std::size_t>(it - m_tracks.begin());
			const auto end = blockEnd(first);
			const auto folderIndex = static_cast<std::size_t>(
				std::find(m_tracks.begin(), m_tracks.end(), folder) - m_tracks.begin());
			const auto folderEnd = blockEnd(folderIndex);
			if (end < folderEnd)
			{
				moveTracks(first, end, folderEnd - (end - first));
				emit trackMoved();
			}
		}
	}

	const auto contents = folderContents(track);
	track->setFolderDepth(depth);
	for (const auto inner : contents) { inner->setFolderDepth(inner->folderDepth() + delta); }

	updateFolders();
}




auto TrackContainer::blockEnd(std::size_t index) const -> std::size_t
{
	auto end = index + 1;
	if (m_tracks[index]->type() != Track::Type::Folder) { return end; }
	while (end < m_tracks.size() && m_tracks[end]->folderDepth() > m_tracks[index]->folderDepth()) { ++end; }
	return end;
}




void TrackContainer::moveTracks(std::size_t first, std::size_t end, std::size_t to)
{
	if (first >= end || to == first) { return; }

	auto block = TrackList(m_tracks.begin() + first, m_tracks.begin() + end);
	auto rest = m_tracks;
	rest.erase(rest.begin() + first, rest.begin() + end);

	// The patterns follow their tracks, swapped with each pattern track passed on the way
	if (to > first)
	{
		for (auto passed = first; passed < to; ++passed)
		{
			for (auto moved = block.rbegin(); moved != block.rend(); ++moved)
			{
				PatternTrack::swapPatternTracks(*moved, rest[passed]);
			}
		}
	}
	else
	{
		for (auto passed = first; passed-- > to;)
		{
			for (const auto moved : block) { PatternTrack::swapPatternTracks(moved, rest[passed]); }
		}
	}

	rest.insert(rest.begin() + to, block.begin(), block.end());
	Engine::audioEngine()->requestChangeInModel();
	m_tracksMutex.lockForWrite();
	m_tracks = std::move(rest);
	m_tracksMutex.unlock();
	Engine::audioEngine()->doneChangeInModel();
}




auto TrackContainer::moveTrackStep(Track* track, bool down) -> bool
{
	const auto it = std::find(m_tracks.begin(), m_tracks.end(), track);
	if (it == m_tracks.end()) { return false; }

	auto isCollapsedFolder = [](const Track* t) {
		const auto folder = dynamic_cast<const FolderTrack*>(t);
		return folder && folder->isCollapsed();
	};

	// The outermost collapsed folder each track is hidden in, or the track itself if it is shown
	auto shownAs = std::vector<std::size_t>(m_tracks.size());
	auto collapsed = std::optional<std::size_t>{};
	for (auto i = std::size_t{0}; i < m_tracks.size(); ++i)
	{
		if (collapsed && m_tracks[i]->folderDepth() <= m_tracks[*collapsed]->folderDepth()) { collapsed.reset(); }
		shownAs[i] = collapsed.value_or(i);
		if (!collapsed && isCollapsedFolder(m_tracks[i])) { collapsed = i; }
	}

	const auto first = static_cast<std::size_t>(it - m_tracks.begin());
	const auto end = blockEnd(first);
	const auto size = end - first;

	// Where the block goes in the list without it
	std::size_t to = 0;
	if (down)
	{
		if (end >= m_tracks.size()) { return false; }
		to = (isCollapsedFolder(m_tracks[end]) ? blockEnd(end) : end + 1) - size;
	}
	else
	{
		if (first == 0) { return false; }
		to = shownAs[first - 1];
	}

	// The level of the track above the new place, or inside it if that is an open folder
	int depth = 0;
	const auto aboveIndex = to == 0 ? std::optional<std::size_t>{} : std::optional{to - 1 < first ? to - 1 : to - 1 + size};
	if (aboveIndex)
	{
		const auto above = m_tracks[shownAs[*aboveIndex]];
		depth = above->folderDepth() + (above->type() == Track::Type::Folder && !isCollapsedFolder(above) ? 1 : 0);
	}
	const int delta = depth - track->folderDepth();

	const auto block = TrackList(m_tracks.begin() + first, m_tracks.begin() + end);
	moveTracks(first, end, to);
	for (const auto moved : block) { moved->setFolderDepth(moved->folderDepth() + delta); }

	updateFolders();
	emit trackMoved();
	return true;
}




void TrackContainer::addLayoutCheckPoint()
{
	m_folderLayout.addJournalCheckPoint();
}




void TrackContainer::resolveLegacySolo()
{
	if (std::none_of(m_tracks.begin(), m_tracks.end(), [](const Track* track) { return track->legacySolo(); }))
	{
		return;
	}
	for (const auto track : m_tracks)
	{
		track->setMuted(track->legacySolo() ? false : track->legacyMutedBeforeSolo());
		const auto type = track->type();
		if (type != Track::Type::Instrument && type != Track::Type::Sample && type != Track::Type::Folder)
		{
			track->clearLegacySolo();
		}
	}
}




void TrackContainer::updateFolders()
{
	// A track is at most one level deeper than the track above, and only if that one is a folder
	int maxDepth = 0;
	for (const auto track : m_tracks)
	{
		if (track->folderDepth() > maxDepth) { track->setFolderDepth(maxDepth); }
		maxDepth = track->folderDepth() + (track->type() == Track::Type::Folder ? 1 : 0);
	}

	if (this == Engine::getSong() && !Engine::getSong()->isLoadingProject() && Engine::mixer())
	{
		Engine::mixer()->updateFolderRouting();
	}
	emit foldersChanged();
}




void TrackContainer::FolderLayout::saveSettings(QDomDocument& doc, QDomElement& element)
{
	(void)doc;
	auto depths = QStringList{};
	auto order = QStringList{};
	for (const auto track : m_container->tracks())
	{
		depths << QString::number(track->folderDepth());
		order << QString::number(track->id());
	}
	element.setAttribute("depths", depths.join(','));
	element.setAttribute("order", order.join(','));
}




void TrackContainer::FolderLayout::loadSettings(const QDomElement& element)
{
	const auto depths = element.attribute("depths").split(',', Qt::SkipEmptyParts);

	// The order: one track at a time into its place, so that the patterns follow (see moveTracks())
	const auto order = element.attribute("order").split(',', Qt::SkipEmptyParts);
	bool moved = false;
	auto place = std::size_t{0};
	for (const auto& id : order)
	{
		auto& current = m_container->m_tracks;
		const auto it = std::find_if(current.begin() + place, current.end(), [&id](const Track* track) {
			return track->id() == static_cast<jo_id_t>(id.toInt());
		});
		if (it == current.end()) { continue; }
		const auto index = static_cast<std::size_t>(it - current.begin());
		if (index != place)
		{
			m_container->moveTracks(index, index + 1, place);
			moved = true;
		}
		++place;
	}
	if (moved) { emit m_container->trackMoved(); }

	const auto& tracks = m_container->tracks();
	for (auto i = std::size_t{0}; i < tracks.size() && i < static_cast<std::size_t>(depths.size()); ++i)
	{
		tracks[i]->setFolderDepth(depths[static_cast<int>(i)].toInt());
	}
	m_container->updateFolders();
}



void TrackContainer::updateAfterTrackAdd()
{
}




void TrackContainer::clearAllTracks()
{
	//m_tracksMutex.lockForWrite();
	while (!m_tracks.empty())
	{
		delete m_tracks.front();
	}
	//m_tracksMutex.unlock();
}




bool TrackContainer::isEmpty() const
{
	for (const auto& track : m_tracks)
	{
		if (!track->getClips().empty())
		{
			return false;
		}
	}
	return true;
}



AutomatedValueMap TrackContainer::automatedValuesAt(TimePos time, int clipNum) const
{
	return automatedValuesFromTracks(tracks(), time, clipNum);
}


AutomatedValueMap TrackContainer::automatedValuesFromTracks(const TrackList &tracks, TimePos time, int clipNum)
{
	Track::clipVector clips;

	for (Track* track: tracks)
	{
		if (track->isMuted()) {
			continue;
		}

		switch(track->type())
		{
		case Track::Type::Automation:
		case Track::Type::HiddenAutomation:
		case Track::Type::Pattern:
			if (clipNum < 0) {
				track->getClipsInRange(clips, 0, time);
			} else {
				Q_ASSERT(track->numOfClips() > clipNum);
				clips.push_back(track->getClip(clipNum));
			}
		default:
			break;
		}
	}

	AutomatedValueMap valueMap;

	Q_ASSERT(std::is_sorted(clips.begin(), clips.end(), Clip::comparePosition));

	for(Clip* clip : clips)
	{
		if (clip->isMuted() || clip->startPosition() > time) {
			continue;
		}

		if (auto* p = dynamic_cast<AutomationClip *>(clip))
		{
			if (! p->hasAutomation()) {
				continue;
			}
			TimePos relTime = time - p->startPosition() - p->startTimeOffset();
			if (!p->isInPattern()) {
				relTime = std::min(static_cast<int>(relTime), p->length() - p->startTimeOffset());
			}
			float value = p->valueAt(relTime);

			for (AutomatableModel* model : p->objects())
			{
				valueMap[model] = value;
			}
		}
		else if (auto* pattern = dynamic_cast<PatternClip*>(clip))
		{
			auto patIndex = dynamic_cast<class PatternTrack*>(pattern->getTrack())->patternIndex();
			auto patStore = Engine::patternStore();

			TimePos patTime = time - clip->startPosition();
			patTime = std::min(patTime, clip->length());
			patTime = patTime % (patStore->lengthOfPattern(patIndex) * TimePos::ticksPerBar());

			auto patValues = patStore->automatedValuesAt(patTime, patIndex);
			for (auto it=patValues.begin(); it != patValues.end(); it++)
			{
				// override old values, pattern track with the highest index takes precedence
				valueMap[it.key()] = it.value();
			}
		}
		else
		{
			continue;
		}
	}

	return valueMap;
};


} // namespace lmms
