/*
 * TrackChannelControls.cpp - a track's volume, panning and effects controls, acting on its mixer channel
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

#include "TrackChannelControls.h"

#include <algorithm>
#include <cassert>

#include "AudioBusHandle.h"
#include "AutomatableButton.h"
#include "EffectRackView.h"
#include "Engine.h"
#include "FolderTrack.h"
#include "InstrumentTrack.h"
#include "Knob.h"
#include "Mixer.h"
#include "SampleTrack.h"
#include "Track.h"
#include "TrackContainer.h"
#include "volume.h"

namespace lmms::gui
{

TrackChannelControls::TrackChannelControls(Track* track, QObject* parent) :
	QObject(parent),
	m_track(track)
{
	if (auto instrumentTrack = dynamic_cast<InstrumentTrack*>(track))
	{
		m_channel = instrumentTrack->mixerChannelModel();
		m_volume = instrumentTrack->volumeModel();
		m_panning = instrumentTrack->panningModel();
		m_effects = instrumentTrack->audioBusHandle()->effects();
	}
	else if (auto sampleTrack = dynamic_cast<SampleTrack*>(track))
	{
		m_channel = sampleTrack->mixerChannelModel();
		m_volume = sampleTrack->volumeModel();
		m_panning = sampleTrack->panningModel();
		m_effects = sampleTrack->audioBusHandle()->effects();
	}
	else if (auto folderTrack = dynamic_cast<FolderTrack*>(track))
	{
		m_channel = folderTrack->mixerChannelModel();
	}
	assert(m_channel);

	const auto mixer = Engine::mixer();
	connect(mixer, &Mixer::trackChannelChanged, this, [this](Track* changed) {
		if (changed == m_track) { update(); }
	});
	connect(mixer, &Mixer::channelDeleted, this, &TrackChannelControls::update);
	connect(m_channel, &IntModel::dataChanged, this, &TrackChannelControls::update);
	connect(track->getMutedModel(), &BoolModel::dataChanged, this, &TrackChannelControls::update);
	// Automating the track's volume or panning keeps them on the track
	if (m_volume) { connect(m_volume, &FloatModel::dataChanged, this, &TrackChannelControls::update); }
	if (m_panning) { connect(m_panning, &FloatModel::dataChanged, this, &TrackChannelControls::update); }
}




TrackChannelControls::~TrackChannelControls()
{
	disconnect(m_channelMute);
}




auto TrackChannelControls::replace(TrackChannelControls* controls, Track* track, QObject* parent,
	const Widgets& widgets) -> TrackChannelControls*
{
	// The widgets may point at models of the old controls, so those go after the widgets point elsewhere
	auto replacement = new TrackChannelControls(track, parent);
	replacement->m_volumeKnob = widgets.volume;
	replacement->m_panningKnob = widgets.panning;
	replacement->m_effectRack = widgets.effects;
	replacement->m_muteButton = widgets.mute;
	replacement->m_soloButton = widgets.solo;
	if (controls) { replacement->m_boundChannel = controls->m_boundChannel; }
	replacement->update();
	delete controls;
	return replacement;
}




void TrackChannelControls::update()
{
	const auto mixer = Engine::mixer();
	auto channel = mixer->trackChannel(m_track);

	// A folder uses whatever channel it sends to, having nothing of its own
	if (!channel && !m_volume)
	{
		channel = mixer->mixerChannel(std::clamp(m_channel->value(), 0, mixer->numChannels() - 1));
	}

	// The controls may still point at a channel deleted meanwhile
	bool boundExists = false;
	for (int i = 0; i < mixer->numChannels(); ++i)
	{
		if (mixer->mixerChannel(i) == m_boundChannel) { boundExists = true; }
	}
	const bool oldModelsValid = !m_boundChannel || boundExists;
	m_boundChannel = channel;

	// The track looks muted while its channel is (Track::isMutedInMix())
	disconnect(m_channelMute);
	if (channel)
	{
		const auto container = m_track->trackContainer();
		m_channelMute = connect(&channel->m_muteModel, &BoolModel::dataChanged, container,
			[container] { emit container->silenceChanged(); });
	}

	// The track's volume and panning are its own, on the source before the channel; a folder, which has none,
	// shows its group channel's
	if (m_volumeKnob)
	{
		const bool onChannel = !m_volume && channel;
		FloatModel* model = onChannel ? &channel->m_volumeModel : m_volume;
		if (m_volumeKnob->model() != model || !oldModelsValid)
		{
			m_volumeKnob->setZeroDbfsPoint(onChannel ? 1.f : DefaultVolume);
			m_volumeKnob->setModel(model, oldModelsValid);
		}
	}
	if (m_panningKnob)
	{
		FloatModel* model = !m_panning && channel ? &channel->m_panModel : m_panning;
		if (m_panningKnob->model() != model || !oldModelsValid) { m_panningKnob->setModel(model, oldModelsValid); }
	}
	// Mute and solo are the track's own, on its source (see TrackContainer::updateSilence()); the channel has its
	// own in the mixer
	if (m_muteButton && m_muteButton->model() != m_track->getMutedModel())
	{
		m_muteButton->setModel(m_track->getMutedModel());
	}
	if (m_soloButton && m_soloButton->model() != m_track->soloModel()) { m_soloButton->setModel(m_track->soloModel()); }
	if (m_effectRack && m_effects)
	{
		// The track's own effects stay visible while it has any (e.g. on a shared channel)
		EffectChain* model = channel && m_effects->effects().empty() ? &channel->m_fxChain : m_effects;
		if (m_effectRack->model() != model || !oldModelsValid) { m_effectRack->setModel(model, oldModelsValid); }
	}
}

} // namespace lmms::gui
