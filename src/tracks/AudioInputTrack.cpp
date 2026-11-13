/*
 * AudioInputTrack.cpp
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

#include "AudioInputTrack.h"

#include <QDomElement>

#include "AudioEngine.h"
#include "EffectChain.h"
#include "Engine.h"
#include "InputMonitorHandle.h"
#include "Mixer.h"
#include "panning.h"
#include "Song.h"
#include "volume.h"

namespace lmms
{

AudioInputTrack::AudioInputTrack(Type type, TrackContainer* container, const QString& busName)
	: Track{type, container}
	, m_volumeModel{DefaultVolume, MinVolume, MaxTrackVolume, 0.1f, this, tr("Volume")}
	, m_panningModel{DefaultPanning, PanningLeft, PanningRight, 0.1f, this, tr("Panning")}
	, m_recordArmModel{false, this, tr("Record arm")}
	, m_monitorModel{static_cast<int>(MonitorMode::Auto), static_cast<int>(MonitorMode::Off),
		static_cast<int>(MonitorMode::Auto), this, tr("Input monitoring")}
	, m_audioBusHandle{busName, true, &m_volumeModel, &m_panningModel, this}
{
	m_panningModel.setCenterValue(DefaultPanning);

	connect(&m_mixerChannelModel, SIGNAL(dataChanged()), this, SLOT(updateMixerChannel()));

	// Software monitoring: the handle plays the input while monitoring is on (see InputMonitorHandle)
	Engine::audioEngine()->addPlayHandle(new InputMonitorHandle{this});

	// The audio device opens its input while a track needs it
	const auto captureNeedMayChange = [] { if (auto song = Engine::getSong()) { song->scheduleCaptureUpdate(); } };
	connect(this, &AudioInputTrack::inputChanged, this, captureNeedMayChange);
	connect(&m_recordArmModel, &BoolModel::dataChanged, this, captureNeedMayChange);
	connect(&m_monitorModel, &IntModel::dataChanged, this, captureNeedMayChange);
}




AudioInputTrack::~AudioInputTrack()
{
	Engine::audioEngine()->removePlayHandlesOfTypes(this, PlayHandle::Type::InputMonitorHandle);
}




void AudioInputTrack::setInput(const AudioInput& input)
{
	if (m_input == input) { return; }
	m_input = input;
	emit inputChanged();
}




void AudioInputTrack::updateMixerChannel()
{
	m_audioBusHandle.setNextMixerChannel(m_mixerChannelModel.value());
}




void AudioInputTrack::saveAudioSettings(QDomDocument& doc, QDomElement& element)
{
	m_audioBusHandle.effects()->saveState(doc, element);
	m_volumeModel.saveSettings(doc, element, "vol");
	m_panningModel.saveSettings(doc, element, "pan");
	m_mixerChannelModel.saveSettings(doc, element, "mixch");
	m_recordArmModel.saveSettings(doc, element, "recordarm");
	m_monitorModel.saveSettings(doc, element, "monitor");
	if (m_input.source == AudioInput::Source::Hardware)
	{
		auto input = doc.createElement("input");
		input.setAttribute("source", "hardware");
		input.setAttribute("channel", m_input.firstChannel);
		input.setAttribute("channels", m_input.channelCount);
		element.appendChild(input);
	}
}




void AudioInputTrack::loadAudioSettings(const QDomElement& element)
{
	m_audioBusHandle.effects()->clear();
	for (auto node = element.firstChild(); !node.isNull(); node = node.nextSibling())
	{
		if (node.isElement() && m_audioBusHandle.effects()->nodeName() == node.nodeName())
		{
			m_audioBusHandle.effects()->restoreState(node.toElement());
		}
	}
	m_volumeModel.loadSettings(element, "vol");
	m_panningModel.loadSettings(element, "pan");
	m_mixerChannelModel.setRange(0, Engine::mixer()->numChannels() - 1);
	m_mixerChannelModel.loadSettings(element, "mixch");
	m_recordArmModel.loadSettings(element, "recordarm");
	m_monitorModel.loadSettings(element, "monitor");
	const auto input = element.firstChildElement("input");
	setInput(!input.isNull() && input.attribute("source") == "hardware"
		? AudioInput{AudioInput::Source::Hardware, input.attribute("channel").toInt(),
			std::clamp(input.attribute("channels", "2").toInt(), 1, 2)}
		: AudioInput{});
}

} // namespace lmms
