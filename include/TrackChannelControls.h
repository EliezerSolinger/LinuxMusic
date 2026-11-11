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

class EffectRackView;
class Knob;
class VolumeKnob;

/**
 * Makes a track's volume and panning knobs and effect rack shortcuts to its mixer channel's fader, pan and effects
 * when the track has a channel of its own (see Mixer::setUpTrackChannel()), and to the track's own volume, panning
 * and effects otherwise (a track sending to a shared channel or to the master, or with automated volume or panning).
 * Follows the track when its channel changes.
 */
class TrackChannelControls : public QObject
{
	Q_OBJECT
public:
	//! @p track is an instrument or a sample track
	TrackChannelControls(Track* track, QObject* parent);

	//! Replaces @p controls (may be null) with controls for @p track and the given widgets (each may be null)
	static auto replace(TrackChannelControls* controls, Track* track, QObject* parent, VolumeKnob* volumeKnob,
		Knob* panningKnob, EffectRackView* effectRack) -> TrackChannelControls*;

	void setVolumeKnob(VolumeKnob* knob);
	void setPanningKnob(Knob* knob);
	void setEffectRack(EffectRackView* rack);

	//! Points the controls at the channel or the track
	void update();

private:
	Track* m_track;
	IntModel* m_channel = nullptr;
	FloatModel* m_volume = nullptr;
	FloatModel* m_panning = nullptr;
	EffectChain* m_effects = nullptr;

	QPointer<VolumeKnob> m_volumeKnob;
	QPointer<Knob> m_panningKnob;
	QPointer<EffectRackView> m_effectRack;

	//! The channel the controls point at, if any
	MixerChannel* m_boundChannel = nullptr;
};

} // namespace gui

} // namespace lmms

#endif // LMMS_GUI_TRACK_CHANNEL_CONTROLS_H
