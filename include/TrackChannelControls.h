/*
 * TrackChannelControls.h - a track's volume, panning and effects controls, acting on its mixer channel
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

#ifndef LMMS_GUI_TRACK_CHANNEL_CONTROLS_H
#define LMMS_GUI_TRACK_CHANNEL_CONTROLS_H

#include <memory>

#include <QObject>
#include <QPointer>

namespace lmms
{

class EffectChain;
class FloatModel;
class IntModel;
class MixerChannel;
class Track;

namespace gui
{

class AutomatableButton;
class EffectRackView;
class Knob;
class VolumeKnob;

/**
 * Makes a track's volume and panning knobs and effect rack shortcuts to its mixer channel's fader, pan and effects
 * when the track has a channel of its own (see Mixer::setUpTrackChannel()), and to the track's own volume, panning
 * and effects otherwise (a track sending to a shared channel or to the master, or with automated volume or panning).
 * A folder has no volume, panning and effects of its own: its controls are always those of the channel it uses.
 * Mute and solo are the channel's too: the mute button is the channel's while the track has a channel of its own (and
 * no automated mute), and the solo button solos the channel the track plays through.
 * Follows the track when its channel changes.
 */
class TrackChannelControls : public QObject
{
	Q_OBJECT
public:
	//! @p track is an instrument, sample or folder track
	TrackChannelControls(Track* track, QObject* parent);
	~TrackChannelControls() override;

	//! The widgets the controls point; each may be null
	struct Widgets
	{
		VolumeKnob* volume = nullptr;
		Knob* panning = nullptr;
		EffectRackView* effects = nullptr;
		AutomatableButton* mute = nullptr;
		AutomatableButton* solo = nullptr;
	};

	//! Replaces @p controls (may be null) with controls for @p track and @p widgets
	static auto replace(TrackChannelControls* controls, Track* track, QObject* parent, const Widgets& widgets)
		-> TrackChannelControls*;

	//! Points the controls at the channel or the track
	void update();

private:
	Track* m_track;
	IntModel* m_channel = nullptr;
	FloatModel* m_volume = nullptr;		//!< the track's own, none for folders
	FloatModel* m_panning = nullptr;	//!< the track's own, none for folders
	EffectChain* m_effects = nullptr;	//!< the track's own, none for folders

	QPointer<VolumeKnob> m_volumeKnob;
	QPointer<Knob> m_panningKnob;
	QPointer<EffectRackView> m_effectRack;
	QPointer<AutomatableButton> m_muteButton;
	QPointer<AutomatableButton> m_soloButton;

	//! The channel the controls point at, if any
	MixerChannel* m_boundChannel = nullptr;
};

} // namespace gui

} // namespace lmms

#endif // LMMS_GUI_TRACK_CHANNEL_CONTROLS_H
