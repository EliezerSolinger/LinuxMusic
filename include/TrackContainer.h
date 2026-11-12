/*
 * TrackContainer.h - base-class for all track-containers like Song-Editor,
 *                    Pattern Editor...
 *
 * Copyright (c) 2004-2009 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_TRACK_CONTAINER_H
#define LMMS_TRACK_CONTAINER_H

#include <QReadWriteLock>

#include "Track.h"
#include "JournallingObject.h"

namespace lmms
{

class AutomationClip;
class FolderTrack;
class InstrumentTrack;

namespace gui
{

class TrackContainerView;

}


class LMMS_EXPORT TrackContainer : public Model, public JournallingObject
{
	Q_OBJECT
public:
	using TrackList = std::vector<Track*>;
	enum class Type
	{
		Pattern,
		Song
	} ;

	TrackContainer();
	~TrackContainer() override;

	void saveSettings( QDomDocument & _doc, QDomElement & _parent ) override;

	void loadSettings( const QDomElement & _this ) override;

	int countTracks( Track::Type _tt = Track::Type::Count ) const;


	void addTrack( Track * _track );
	void removeTrack( Track * _track );
	void moveTrack(Track* track, int indexTo);

	// Folders (see Track::folderDepth() and FolderTrack)

	//! The folder holding @p track directly, or nullptr at the top
	auto folderOf(const Track* track) const -> FolderTrack*;

	//! The tracks @p folder holds, directly or in its subfolders, in order; empty if it is not a folder
	auto folderContents(const Track* folder) const -> TrackList;

	//! Whether @p track is held by @p folder, directly or in a subfolder
	auto isInFolder(const Track* track, const Track* folder) const -> bool;

	/**
	 * Moves @p track into the folder above it (@p delta 1) or out of its folder (@p delta -1), together with what it
	 * holds if it is a folder. One undo step.
	 */
	void changeFolderDepth(Track* track, int delta);

	/**
	 * Moves @p track one place up or down among the visible tracks, with what it holds if it is a folder, as when
	 * dragged by its grip: past a collapsed folder at once, and into and out of folders on the way (it takes the
	 * level of the track above its new place, or goes into that track if it is an open folder).
	 * @returns whether it moved
	 */
	auto moveTrackStep(Track* track, bool down) -> bool;

	//! Adds an undo step for the order and folder depths of the tracks, e.g. before a track is dragged
	void addLayoutCheckPoint();

	/**
	 * Makes the folder depths valid (a track can only be one level deeper than the track above it, and only if that
	 * one is a folder) and lets the mixer route the tracks' channels to their folders' channels
	 * (Mixer::updateFolderRouting()). Called after tracks are added, removed or moved.
	 */
	void updateFolders();

	virtual void updateAfterTrackAdd();

	void clearAllTracks();

	const TrackList & tracks() const
	{
		return m_tracks;
	}

	bool isEmpty() const;

	static const QString classNodeName()
	{
		return "trackcontainer";
	}

	inline void setType( Type newType )
	{
		m_TrackContainerType = newType;
	}

	inline Type type() const
	{
		return m_TrackContainerType;
	}

	virtual AutomatedValueMap automatedValuesAt(TimePos time, int clipNum = -1) const;

signals:
	void trackAdded( lmms::Track * _track );
	void trackRemoved();
	void trackMoved();
	//! The folder depths changed (see updateFolders())
	void foldersChanged();

protected:
	static AutomatedValueMap automatedValuesFromTracks(const TrackList &tracks, TimePos timeStart, int clipNum = -1);

	mutable QReadWriteLock m_tracksMutex;

private:
	/**
	 * Moves the tracks from @p first to @p end (exclusive) to index @p to of the list without them, keeping the
	 * patterns of the pattern editor in the order of their pattern tracks
	 */
	void moveTracks(std::size_t first, std::size_t end, std::size_t to);

	//! The index one past @p index and what it holds if it is a folder
	auto blockEnd(std::size_t index) const -> std::size_t;

	//! The order and folder depths of all tracks, journalled as one object so that a folder change is one undo step
	class FolderLayout : public JournallingObject
	{
	public:
		explicit FolderLayout(TrackContainer* container) : m_container{container} {}
		void saveSettings(QDomDocument& doc, QDomElement& element) override;
		void loadSettings(const QDomElement& element) override;
		auto nodeName() const -> QString override { return "folderlayout"; }

	private:
		TrackContainer* m_container;
	};

	TrackList m_tracks;

	Type m_TrackContainerType;

	FolderLayout m_folderLayout{this};


	friend class gui::TrackContainerView;
	friend class Track;

} ;

} // namespace lmms

#endif // LMMS_TRACK_CONTAINER_H
