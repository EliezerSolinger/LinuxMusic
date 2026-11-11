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

#include "AudioEngine.h"
#include "DummyEffect.h"
#include "DummyInstrument.h"
#include "Effect.h"
#include "Instrument.h"

namespace lmms {

PluginInstance::PluginInstance(bool isInstrument, const QString& pluginName, InstrumentTrack* track,
	const Plugin::Descriptor::SubPluginFeatures::Key* key, bool keyFromDnd)
	: m_remoteBufferResource{m_remoteBufferData}
{
	load(isInstrument, pluginName, track, key, keyFromDnd);
}

void PluginInstance::load(bool isInstrument, const QString& pluginName, InstrumentTrack* track,
	const Plugin::Descriptor::SubPluginFeatures::Key* key, bool keyFromDnd)
{
	if (isInstrument)
	{
		m_plugin = Instrument::instantiate(pluginName, track, key, keyFromDnd);
	}
	else
	{
		m_plugin = Effect::instantiate(pluginName, track, key);
	}

	if (!m_plugin) { return; }

	auto& proc = audioProcessor();

	const f_cnt_t = frames = Engine::audioEngine()->framesPerPeriod();

	const ch_cnt_t channelsIn   = proc.channelCount(true);
	const group_cnt_t groupsIn  = proc.channelGroupCount(true);
	const ch_cnt_t channelsOut  = proc.channelCount(false);
	const group_cnt_t groupsOut = proc.channelGroupCount(false);

	const ch_cnt_t totalChannels = channelsIn + channelsOut;
	if (totalChannels == 0) { /* TODO: ERROR */ }

	const bool isShared = proc.usesSharedMemoryBuffers();
	const auto resource = isShared
		? &m_remoteBufferResource
		: std::pmr::get_default_resource();
	auto& bufferData = isShared ? m_remoteBuffer : m_localBuffer;

	if (proc.usesInterleavedBuffers())
	{
		// Legacy plugin which uses interleaved 2-channel buffer
		assert(channelsIn == 0 || channelsIn == 2);
		assert(channelsOut == 0 || channelsOut == 2);
		bufferData.create(frames, 0, true, resource);

		// The input and output AudioBuffers share the same interleaved buffer
		// for in-place processing
		m_inputs  = AudioBuffer{bufferData, 0, 0};
		m_outputs = AudioBuffer{bufferData, 0, 0};
	}
	else
	{
		// First, check if there are any in-place pairs which would let us
		// reduce the number of channel buffers needed, since an in-place pair
		// only needs one channel buffer, not two
		auto channelBuffersToAllocate = totalChannels;
		m_inplacePairsInToOut.clear();
		m_inplacePairsInToOut.resize(channelsIn, std::nullopt);
		m_inplacePairsOutToIn.clear();
		m_inplacePairsOutToIn.resize(channelsOut, std::nullopt);
		if (channelsOut > 0)
		{
			auto inplacePairCount = ch_cnt_t{0};

			for (ch_cnt_t input = 0; input < channelsIn; ++input)
			{
				auto pair = proc.getInplacePair(input);
				if (!pair) { continue; }

				if (*pair >= channelsOut)
				{
					// Invalid output channel index
					assert(false);
					continue;
				}

				if (m_inplacePairsOutToIn.at(*pair).has_value())
				{
					// Two input-side buffers are paired with the same output-side buffer
					assert(false);
					continue;
				}

				m_inplacePairsInToOut[input] = pair;
				m_inplacePairsOutToIn[*pair] = input;
				++inplacePairCount;
			}

			// Check for an incorrect getInplacePair() implementation
			if (inplacePairCount > std::min(channelsIn, channelsOut))
			{
				assert(false);
				std::ranges::fill(m_inplacePairsInToOut, std::nullopt);
				std::ranges::fill(m_inplacePairsOutToIn, std::nullopt);
				inplacePairCount = 0;
			}

			// Only one buffer needs to be allocated for each in-place pair, not two
			channelBuffersToAllocate -= inplacePairCount;
		}

		// TODO: For this use case, `channels` is greater than `channelsToAllocate` rather than the other way around with m_inputs and m_outputs.
		bufferData.create(frames, totalChannels, channelBuffersToAllocate, false, resource);

		m_inputs  = AudioBuffer{bufferData, 0, channelsIn};
		m_outputs = AudioBuffer{bufferData, channelsIn, channelsOut};

		// TODO: Channel mapping







		// The input and output AudioBuffers are sourced from a single AudioBufferData
		m_inputs  = AudioBuffer{bufferData, 0, channelsIn};
		m_outputs = AudioBuffer{bufferData, channelsIn, channelsOut};
	}

	m_inputs.setGroups(proc.channelGroupCount(), [&](group_cnt_t idx, ChannelGroup& group) -> ch_cnt_t {
		return proc.getChannelGroup(idx, true, group);
	});

	m_outputs.setGroups(proc.channelGroupCount(), [&](group_cnt_t idx, ChannelGroup& group) -> ch_cnt_t {
		return proc.getChannelGroup(idx, false, group);
	});

}

void PluginInstance::unload(bool deletePlugin)
{

}

auto PluginInstance::process(AudioBuffer& trackChannels, NotePlayHandle* nph) -> bool
{
	// TODO: Route trackChannels into plugin here

	auto context = ProcessContext{&m_inputs, &m_outputs, nph};
	const auto status = audioProcessor().process(context);

	// TODO: Route out of plugin here

	return status;
}

auto PluginInstance::isRemotePlugin() const -> bool
{
	return m_isRemotePlugin;
}

auto PluginInstance::effect() noexcept -> Effect*
{
	return dynamic_cast<Effect*>(m_plugin);
}

auto PluginInstance::instrument() noexcept -> Instrument*
{
	return dynamic_cast<Instrument*>(m_plugin);
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

void PluginInstance::convertToDummyEffect(Model* parent, const QDomElement& originalPluginData)
{
	delete m_plugin;
	m_plugin = new DummyEffect(parent, originalPluginData);
}

void PluginInstance::rescanAudioPorts(AudioPortsRescanFlag flags)
{
	auto& ap = audioProcessor();

	AudioEngine::requestChangeInModel();
	m_state = State::
	AudioEngine::doneChangeInModel();

	auto _ = AudioEngine::requestChangesGuard();

	ap.
}

void PluginInstance::useRemotePluginBuffers(bool remote)
{
	if (remote == m_isRemotePlugin) { return; }

	AudioEngine::requestChangeInModel();

	AudioBufferData& data = remote ? m_remoteBuffer : m_localBuffer;
	m_inputs = AudioBuffer{data, 0, m_inputs.channels()};
	m_outputs = AudioBuffer{data, m_inputs.channels(), m_outputs.channels()};

	m_isRemotePlugin = remote;

	AudioEngine::doneChangeInModel();
}

} // namespace lmms
