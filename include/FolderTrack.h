/*
 * FolderTrack.h - a track grouping the tracks below it, with a group channel in the mixer
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

#ifndef LMMS_FOLDER_TRACK_H
#define LMMS_FOLDER_TRACK_H

#include "MixerChannelLcdModel.h"
#include "Track.h"

namespace lmms
{

/**
 * Groups the tracks right below it that are deeper in folders (see Track::folderDepth()), as in REAPER. It has no
 * clips and records nothing: it only groups.
 *
 * Its mixer channel is a group channel (Mixer::setUpTrackChannel()), which the channels of the tracks it holds send
 * to (Mixer::updateFolderRouting()), so its fader, pan, effects and mute act on all of them.
 */
class LMMS_EXPORT FolderTrack : public Track
{
	Q_OBJECT
public:
	explicit FolderTrack(TrackContainer* tc);

	bool play(const TimePos& start, const f_cnt_t frames, const f_cnt_t frameBase, int clipNum = -1) override;
	QString nodeName() const override { return "foldertrack"; }

	gui::TrackView* createView(gui::TrackContainerView* view) override;

	//! Folders have no clips
	Clip* createClip(const TimePos& pos) override;

	void saveTrackSpecificSettings(QDomDocument& doc, QDomElement& parent, bool presetMode) override;
	void loadTrackSpecificSettings(const QDomElement& element) override;

	auto mixerChannelModel() -> IntModel* { return &m_mixerChannelModel; }

	//! Whether the editor hides the tracks the folder holds
	auto isCollapsed() const -> bool { return m_collapsed; }
	void setCollapsed(bool collapsed);

signals:
	void collapsedChanged();

private:
	//! The folder's mute is its group channel's
	void applyMuteToChannel();

	MixerChannelLcdModel m_mixerChannelModel;
	bool m_collapsed = false;
};

} // namespace lmms

#endif // LMMS_FOLDER_TRACK_H
