/*
 * FolderTrack.cpp - a track grouping the tracks below it, with a group channel in the mixer
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

#include "FolderTrack.h"

#include <QDomElement>

#include "Engine.h"
#include "FolderTrackView.h"
#include "Mixer.h"

namespace lmms
{

FolderTrack::FolderTrack(TrackContainer* tc) :
	Track(Track::Type::Folder, tc),
	m_mixerChannelModel(this)
{
	setName(tr("Folder"));
}




bool FolderTrack::play(const TimePos& start, const f_cnt_t frames, const f_cnt_t frameBase, int clipNum)
{
	(void)start;
	(void)frames;
	(void)frameBase;
	(void)clipNum;
	return false;
}




gui::TrackView* FolderTrack::createView(gui::TrackContainerView* view)
{
	return new gui::FolderTrackView(this, view);
}




Clip* FolderTrack::createClip(const TimePos& pos)
{
	(void)pos;
	return nullptr;
}




void FolderTrack::saveTrackSpecificSettings(QDomDocument& doc, QDomElement& parent, bool presetMode)
{
	(void)presetMode;
	m_mixerChannelModel.saveSettings(doc, parent, "mixch");
	if (m_collapsed) { parent.setAttribute("collapsed", 1); }
}




void FolderTrack::loadTrackSpecificSettings(const QDomElement& element)
{
	m_mixerChannelModel.setRange(0, Engine::mixer()->numChannels() - 1);
	m_mixerChannelModel.loadSettings(element, "mixch");
	setCollapsed(element.attribute("collapsed", "0").toInt() != 0);
}




void FolderTrack::setCollapsed(bool collapsed)
{
	if (collapsed == m_collapsed) { return; }
	m_collapsed = collapsed;
	emit collapsedChanged();
}

} // namespace lmms
