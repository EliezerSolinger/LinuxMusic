/*
 * MixerView.h - effect-mixer-view for LMMS
 *
 * Copyright (c) 2008-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_GUI_MIXER_VIEW_H
#define LMMS_GUI_MIXER_VIEW_H

#include <functional>
#include <set>

#include <QWidget>

#include "MixerChannelView.h"
#include "ModelView.h"
#include "SerializingObject.h"

class QDomDocument;  // IWYU pragma: keep
class QDomElement;  // IWYU pragma: keep
class QHBoxLayout;
class QStackedLayout;
class QScrollArea;

namespace lmms
{
	class Mixer;
}

namespace lmms::gui
{

class Knob;
class LedCheckBox;

//! The ghost channel at the end of the mixer's channels: a click makes a channel in its place
class GhostChannel : public QWidget
{
	Q_OBJECT
public:
	GhostChannel(QWidget* parent, std::function<void()> add);

protected:
	void mousePressEvent(QMouseEvent* event) override;
	void paintEvent(QPaintEvent*) override;
	void enterEvent(QEnterEvent*) override;
	void leaveEvent(QEvent*) override;

private:
	std::function<void()> m_add;
};

class LMMS_EXPORT MixerView
	: public QWidget
	, public ModelView
	, public SerializingObjectHook
{
	Q_OBJECT
public:
	MixerView(Mixer* mixer);
	void keyPressEvent(QKeyEvent* e) override;

	void saveSettings(QDomDocument& doc, QDomElement& domElement) override;
	void loadSettings(const QDomElement& domElement) override;

	inline MixerChannelView* currentMixerChannel()
	{
		return m_currentMixerChannel;
	}

	inline MixerChannelView* channelView(int index)
	{
		return m_mixerChannelViews[index];
	}


	void setCurrentMixerChannel(MixerChannelView* channel);
	void setCurrentMixerChannel(int channel);

	void clear();


	// display the send button and knob correctly
	void updateMixerChannel(int index);

	// notify the view that a mixer channel was deleted
	void deleteChannel(int index);

	// delete all unused channels
	void deleteUnusedChannels();

	// move the channel to the left or right
	void moveChannelLeft(int index);
	void moveChannelRight(int index);

	void renameChannel(int index);

	// make sure the display syncs up with the mixer.
	// useful for loading projects
	void refreshDisplay();

	//! Whether the channels sending to group channel @p index (a folder's, see FolderTrack) are hidden
	auto isGroupCollapsed(int index) const -> bool;
	void toggleGroup(int index);

	//! Hides the channels in collapsed groups and shows the others
	void updateGroupVisibility();

public slots:
	int addNewChannel();

	//! Adds views for channels the mixer has but the view does not have yet
	void addMissingChannelViews();

	//! Removes the view of a channel the mixer deleted
	void removeChannelView(int index);

	//! Swaps the views of two neighboring channels the mixer swapped
	void swapChannelViews(int index, int otherIndex);

	//! Puts the channels' views in place: the effect returns next to the master, the others in order
	void placeChannelViews();

	//! Makes the upper areas of all strips as tall, so that the faders line up
	void alignStrips();

private slots:
	void updateFaders();
	// TODO This should be improved. Currently the solo and mute models are connected via
	// the MixerChannelView's constructor with the MixerView. It would already be an improvement
	// if the MixerView connected itself to each new MixerChannel that it creates/handles.
	void toggledSolo();
	void toggledMute();

private:
	Mixer* getMixer() const;
	void updateAllMixerChannels();
	void connectToSoloAndMute(int channelIndex);
	void disconnectFromSoloAndMute(int channelIndex);

private:
	QVector<MixerChannelView*> m_mixerChannelViews;

	//! The group channels whose channels are hidden (only in this view, not saved)
	std::set<const MixerChannel*> m_collapsedGroups;

	MixerChannelView* m_currentMixerChannel;

	QScrollArea* channelArea;
	QHBoxLayout* chLayout;
	QWidget* m_channelAreaWidget;
	QStackedLayout* m_racksLayout;
	QWidget* m_fxReturnsWidget;
	QHBoxLayout* m_fxReturnsLayout;
	QWidget* m_ghostChannel;
	QWidget* m_racksWidget;
	Mixer* m_mixer;

	//! The selected channel's strip settings, above its effects: input trim, L/R swap, stereo width and panning
	//! (phase invert is on each strip)
	Knob* m_trimKnob;
	LedCheckBox* m_swapChannelsBox;
	Knob* m_widthKnob;
	Knob* m_panKnob;

	friend class MixerChannelView;
} ;


} // namespace lmms::gui

#endif // LMMS_GUI_MIXER_VIEW_H
