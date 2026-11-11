/*
 * PluginInstance.cpp
 *
 * Copyright (c) 2026 Dalton Messmer <messmer.dalton/at/gmail.com>
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

#include "PluginInstance.h"

#include <QDomElement>

#include "AudioEngine.h"
#include "AudioPortsRouter.h"
#include "DummyEffect.h"
#include "Effect.h"
#include "Engine.h"
#include "Instrument.h"
#include "InstrumentTrack.h"

namespace lmms {

namespace
{

auto instantiate(Plugin::Type type, const QString& pluginName, Model* parent,
	const Plugin::Descriptor::SubPluginFeatures::Key* key, bool keyFromDnd) -> std::unique_ptr<Plugin>
{
	if (type == Plugin::Type::Instrument)
	{
		return std::unique_ptr<Plugin>{
			Instrument::instantiate(pluginName, dynamic_cast<InstrumentTrack*>(parent), key, keyFromDnd)};
	}
	return std::unique_ptr<Plugin>{Effect::instantiate(pluginName, parent, key)};
}

} // namespace




PluginInstance::PluginInstance(Plugin::Type type, const QString& pluginName, Model* parent,
	const Plugin::Descriptor::SubPluginFeatures::Key* key, bool keyFromDnd)
	: m_type{type}
	, m_remoteBufferResource{m_remoteBufferData}
{
	attach(instantiate(type, pluginName, parent, key, keyFromDnd));
}




PluginInstance::PluginInstance(std::unique_ptr<Plugin> plugin, Plugin::Type type)
	: m_type{type}
	, m_remoteBufferResource{m_remoteBufferData}
{
	attach(std::move(plugin));
}




PluginInstance::~PluginInstance()
{
	detach();
}




auto PluginInstance::effect() noexcept -> Effect*
{
	return dynamic_cast<Effect*>(m_plugin.get());
}




auto PluginInstance::effect() const noexcept -> const Effect*
{
	return dynamic_cast<const Effect*>(m_plugin.get());
}




auto PluginInstance::instrument() noexcept -> Instrument*
{
	return dynamic_cast<Instrument*>(m_plugin.get());
}




auto PluginInstance::instrument() const noexcept -> const Instrument*
{
	return dynamic_cast<const Instrument*>(m_plugin.get());
}




auto PluginInstance::effectRef() -> Effect&
{
	assert(m_plugin != nullptr);
	return dynamic_cast<Effect&>(*m_plugin);
}




auto PluginInstance::instrumentRef() -> Instrument&
{
	assert(m_plugin != nullptr);
	return dynamic_cast<Instrument&>(*m_plugin);
}




void PluginInstance::attach(std::unique_ptr<Plugin> plugin)
{
	m_plugin = std::move(plugin);
	if (!m_plugin) { return; }

	m_plugin->m_host = this;

	setState(State::Loaded, true);

	m_processor = dynamic_cast<AudioProcessor*>(m_plugin.get());
	if (!m_processor) { return; }

	m_audioPorts = std::make_unique<AudioPortsModel>(isInstrument());
	setUpAudioPorts();
	activate();

	// The processor was activated for the current sample rate; prepare it again when that changes.
	// The connection goes away together with the pin connector when the plugin is detached.
	QObject::connect(Engine::audioEngine(), &AudioEngine::sampleRateChanged, m_audioPorts.get(), [this] {
		if (Engine::audioEngine()->outputSampleRate() != m_activeSampleRate) { reactivate(); }
	});
}




void PluginInstance::detach()
{
	if (!m_plugin) { return; }

	deactivate();

	// The views refer to the buffer data, so drop them first
	m_inputs = AudioBuffer{};
	m_outputs = AudioBuffer{};
	m_localBuffer.destroy();
	m_remoteBuffer.destroy();

	m_audioPorts.reset();
	m_processor = nullptr;
	m_plugin.reset();
	m_state.store(0, std::memory_order_relaxed);
}




void PluginInstance::setUpAudioPorts()
{
	assert(m_processor != nullptr);
	auto& proc = *m_processor;

	const f_cnt_t frames = Engine::audioEngine()->framesPerPeriod();
	const ch_cnt_t channelsIn = proc.channelCount(true);
	const ch_cnt_t channelsOut = proc.channelCount(false);
	const auto totalChannels = static_cast<ch_cnt_t>(channelsIn + channelsOut);

	m_inputs = AudioBuffer{};
	m_outputs = AudioBuffer{};

	if (totalChannels == 0)
	{
		// A processor without audio ports cannot do anything
		setState(State::Error, true);
		return;
	}

	m_interleaved = proc.usesInterleavedBuffers();
	if (m_interleaved && ((channelsIn != 0 && channelsIn != 2) || channelsOut != 2))
	{
		// Interleaved buffers are only supported for legacy stereo plugins
		setState(State::Error, true);
		return;
	}

	// Find the in-place pairs: each pair needs one channel buffer, not two
	m_inplacePairsInToOut.assign(channelsIn, std::nullopt);
	m_inplacePairsOutToIn.assign(channelsOut, std::nullopt);
	auto inplacePairCount = ch_cnt_t{0};
	for (ch_cnt_t input = 0; input < channelsIn && channelsOut > 0; ++input)
	{
		const auto pair = proc.getInplacePair(input);
		if (!pair) { continue; }

		// Each side may only be part of one pair
		if (*pair >= channelsOut || m_inplacePairsOutToIn[*pair].has_value())
		{
			assert(false && "invalid AudioProcessor::getInplacePair() implementation");
			continue;
		}

		m_inplacePairsInToOut[input] = pair;
		m_inplacePairsOutToIn[*pair] = input;
		++inplacePairCount;
	}

	m_fullyInPlace = channelsIn == 0 || (inplacePairCount == channelsIn && channelsIn == channelsOut);
	if (m_interleaved && !m_fullyInPlace)
	{
		setState(State::Error, true);
		return;
	}

	// Remote plugins get their buffers in shared memory
	m_isRemotePlugin = proc.usesSharedMemoryBuffers();
	auto& data = m_isRemotePlugin ? m_remoteBuffer : m_localBuffer;
	const auto resource = m_isRemotePlugin
		? static_cast<std::pmr::memory_resource*>(&m_remoteBufferResource)
		: std::pmr::get_default_resource();

	// Access buffer: inputs followed by outputs. Source buffer: one buffer per input and per unpaired output.
	const auto sourceChannels = static_cast<ch_cnt_t>(totalChannels - inplacePairCount);
	data.create(frames, totalChannels, sourceChannels, m_interleaved, resource, std::pmr::get_default_resource());

	auto inputSource = std::vector<ch_cnt_t>(channelsIn);
	auto nextSource = ch_cnt_t{0};
	for (ch_cnt_t input = 0; input < channelsIn; ++input)
	{
		inputSource[input] = nextSource++;
		data.mapAccessBuffer(input, inputSource[input]);
	}
	for (ch_cnt_t output = 0; output < channelsOut; ++output)
	{
		const auto pairedInput = m_inplacePairsOutToIn[output];
		data.mapAccessBuffer(channelsIn + output, pairedInput ? inputSource[*pairedInput] : nextSource++);
	}
	assert(nextSource == sourceChannels);

	m_inputs = AudioBuffer{data, ch_cnt_t{0}, channelsIn};
	m_outputs = AudioBuffer{data, channelsIn, channelsOut};

	if (channelsIn > 0)
	{
		m_inputs.setGroups(proc.channelGroupCount(true), [&](group_cnt_t index, AudioBuffer::ChannelGroup& group) {
			return proc.getChannelGroup(index, true, group);
		});
	}
	m_outputs.setGroups(proc.channelGroupCount(false), [&](group_cnt_t index, AudioBuffer::ChannelGroup& group) {
		return proc.getChannelGroup(index, false, group);
	});

	// TODO: Get the track channel count from the instrument track or mixer channel once they have more than 2
	m_audioPorts->setAllChannelCounts(DEFAULT_CHANNELS, channelsIn, channelsOut);

	if (m_isRemotePlugin)
	{
		proc.onSharedMemoryChange({m_remoteBufferData.key(), totalChannels, sourceChannels, frames});
	}

	setState(State::Error, false);
}




void PluginInstance::activate()
{
	if (!m_processor || (state() & static_cast<std::uint8_t>(State::Error))) { return; }

	const auto engine = Engine::audioEngine();
	m_activeSampleRate = engine->outputSampleRate();
	if (!m_processor->activate(m_activeSampleRate, engine->framesPerPeriod()))
	{
		setState(State::Error, true);
		return;
	}
	setState(State::Active, true);

	if (m_processor->startProcessing()) { setState(State::Processing, true); }
}




void PluginInstance::deactivate()
{
	if (!m_processor) { return; }

	if (state() & static_cast<std::uint8_t>(State::Processing))
	{
		m_processor->stopProcessing();
		setState(State::Processing, false);
	}
	if (state() & static_cast<std::uint8_t>(State::Active))
	{
		m_processor->deactivate();
		setState(State::Active, false);
	}
}




void PluginInstance::setState(State flag, bool on)
{
	const auto bit = static_cast<std::uint8_t>(flag);
	if (on) { m_state.fetch_or(bit, std::memory_order_relaxed); }
	else { m_state.fetch_and(static_cast<std::uint8_t>(~bit), std::memory_order_relaxed); }
}




void PluginInstance::reactivate()
{
	if (!m_processor) { return; }

	Engine::audioEngine()->requestChangeInModel();
	deactivate();
	setUpAudioPorts();
	activate();
	Engine::audioEngine()->doneChangeInModel();
}




auto PluginInstance::process(AudioBuffer& trackChannels, NotePlayHandle* nph) -> bool
{
	const auto runnable = static_cast<std::uint8_t>(State::Active) | static_cast<std::uint8_t>(State::Processing);
	if (!m_processor || (state() & runnable) != runnable) { return false; }

	auto effect = isInstrument() ? nullptr : static_cast<Effect*>(m_plugin.get());
	const auto& usedOutputs = m_audioPorts->out().usedTrackChannels();

	if (effect)
	{
		if (!effect->isAwake())
		{
			if (!trackChannels.hasSignal(m_audioPorts->in().usedTrackChannels()))
			{
				// Sleeping plugins zero the track channels their outputs are routed to, so that waking up
				// does not suddenly switch from passthrough to processed audio. Otherwise auto-quit could
				// become audibly noticeable.
				trackChannels.silenceChannels(usedOutputs);
				return false;
			}
			effect->wakeUp();
			setState(State::Sleeping, false);
		}

		if (!effect->isProcessingAudio())
		{
			// Disabled or bypassed: the audio passes through unchanged
			effect->processBypassedImpl();
			return false;
		}
	}

	const auto status = processAudio(trackChannels, nph);

	const bool corrupted = Engine::audioEngine()->sanitizationEnabled() && trackChannels.sanitize(usedOutputs);
	const bool silentOutput = trackChannels.updateSilenceFlags(usedOutputs);

	if (!effect) { return true; }

	effect->m_corrupted.store(corrupted, std::memory_order_relaxed);

	switch (status)
	{
		case ProcessStatus::Continue:
			break;
		case ProcessStatus::ContinueIfNotQuiet:
			effect->handleAutoQuit(silentOutput);
			break;
		case ProcessStatus::Sleep:
			effect->goToSleep();
			break;
	}

	if (!effect->isAwake()) { setState(State::Sleeping, true); }
	return effect->isAwake();
}




auto PluginInstance::processAudio(AudioBuffer& trackChannels, NotePlayHandle* nph) -> ProcessStatus
{
	auto& proc = *m_processor;

	// Direct routing: a stereo in-place plugin connected L --> L, R --> R to the main track channels
	// works on the track channels themselves, without any copying
	const auto direct = m_audioPorts->directRouting();
	const bool stereoInPlace = m_fullyInPlace && m_inputs.channels() == 2 && m_outputs.channels() == 2;
	if (direct && *direct == 0 && stereoInPlace
		&& (m_interleaved ? trackChannels.hasInterleavedBuffer() : trackChannels.channels() == 2))
	{
		auto context = ProcessContext{nullptr, &trackChannels, nph};
		const auto status = proc.process(context);

		// Keep the track's planar and interleaved buffers in sync
		if (m_interleaved) { toPlanar(trackChannels.interleavedBuffer(), trackChannels.groupBuffers(0)); }
		else if (trackChannels.hasInterleavedBuffer())
		{
			toInterleaved(trackChannels.groupBuffers(0), trackChannels.interleavedBuffer());
		}
		for (ch_cnt_t ch = 0; ch < 2; ++ch) { trackChannels.assumeNonSilent(ch); }
		return status;
	}

	AudioPortsRouter::send(*m_audioPorts, trackChannels, m_inputs);

	if (m_interleaved)
	{
		if (m_inputs.channels() == 0)
		{
			// Legacy instruments render into a silent buffer
			auto interleaved = m_outputs.interleavedBuffer();
			std::fill_n(interleaved.data(), interleaved.frames() * 2, 0.f);
		}
		else { toInterleaved(m_outputs.groupBuffers(0), m_outputs.interleavedBuffer()); }
	}

	auto context = m_fullyInPlace
		? ProcessContext{nullptr, &m_outputs, nph}
		: ProcessContext{&m_inputs, &m_outputs, nph};
	const auto status = proc.process(context);

	if (m_interleaved) { toPlanar(m_outputs.interleavedBuffer(), m_outputs.groupBuffers(0)); }

	AudioPortsRouter::receive(*m_audioPorts, m_outputs, trackChannels);

	// Keep the track's interleaved buffer in sync with the main track channels
	const auto& usedOutputs = m_audioPorts->out().usedTrackChannels();
	if (trackChannels.hasInterleavedBuffer() && (usedOutputs[0] || usedOutputs[1]))
	{
		toInterleaved(trackChannels.groupBuffers(0), trackChannels.interleavedBuffer());
	}

	return status;
}




void PluginInstance::playNote(NotePlayHandle* nph, std::span<SampleFrame> out)
{
	auto instrument = this->instrument();
	if (!instrument) { return; }

	instrument->playNote(nph, out);

	if (!m_audioPorts || out.empty()) { return; }
	const auto direct = m_audioPorts->directRouting();
	if (direct && *direct == 0) { return; }

	// Route the note's stereo output to the main track channels as the pins say.
	// TODO: Route to the other track channels once tracks have more than two
	const auto& pins = m_audioPorts->out();
	if (pins.channelCount() != 2 || pins.trackChannelCount() < 2) { return; }

	float gain[2][2];
	for (ch_cnt_t trackChannel = 0; trackChannel < 2; ++trackChannel)
	{
		for (ch_cnt_t channel = 0; channel < 2; ++channel)
		{
			gain[trackChannel][channel] = pins.enabled(trackChannel, channel) ? 1.f : 0.f;
		}
	}

	for (auto& frame : out)
	{
		const float left = frame.left();
		const float right = frame.right();
		frame.setLeft(gain[0][0] * left + gain[0][1] * right);
		frame.setRight(gain[1][0] * left + gain[1][1] * right);
	}
}




void PluginInstance::convertToDummyEffect(Model* parent, const QDomElement& originalPluginData)
{
	Engine::audioEngine()->requestChangeInModel();
	detach();
	m_type = Plugin::Type::Effect;
	attach(std::make_unique<DummyEffect>(parent, originalPluginData));
	Engine::audioEngine()->doneChangeInModel();
}




void PluginInstance::savePins(QDomDocument& doc, QDomElement& pluginElement)
{
	if (m_audioPorts) { m_audioPorts->saveState(doc, pluginElement); }
}




void PluginInstance::loadPins(const QDomElement& pluginElement)
{
	if (!m_audioPorts) { return; }

	const auto pins = pluginElement.firstChildElement(m_audioPorts->nodeName());
	if (!pins.isNull()) { m_audioPorts->restoreState(pins); }
}




void PluginInstance::rescanAudioPorts(AudioPortsRescanFlag flags)
{
	// Any change to the audio ports means new buffers and pin connector dimensions
	(void)flags;
	reactivate();
}




void PluginInstance::useRemotePluginBuffers(bool remote)
{
	if (!m_processor || remote == m_isRemotePlugin) { return; }

	// setUpAudioPorts() asks the processor whether it uses shared memory buffers
	reactivate();
}

} // namespace lmms
