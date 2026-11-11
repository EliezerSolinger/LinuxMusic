/*
 * PluginInstance.h
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

#ifndef LMMS_PLUGIN_INSTANCE_H
#define LMMS_PLUGIN_INSTANCE_H

#include <atomic>

#include "AudioBuffer.h"
#include "AudioPortsModel.h"
#include "AudioProcessor.h"
#include "Host.h"
#include "Plugin.h"
#include "SharedMemory.h"

class QString;

namespace lmms {

class Effect;
class Instrument;
class InstrumentTrack;

/**
 * Manages a plugin instance and implements the host side of the LMMS plugin API.
 *
 * This class is not accessible to plugins - it is host-side only.
 */
class PluginInstance : public Host
{
public:
	using State = AudioProcessor::State;

	PluginInstance(bool isInstrument, const QString& pluginName, InstrumentTrack* track,
		const Plugin::Descriptor::SubPluginFeatures::Key* key, bool keyFromDnd = false);

	void load(bool isInstrument, const QString& pluginName, InstrumentTrack* track,
		const Plugin::Descriptor::SubPluginFeatures::Key* key, bool keyFromDnd = false);

	void unload(bool deletePlugin = true);

	auto isLoaded() const -> bool { return m_plugin != nullptr; }

	auto process(AudioBuffer& trackChannels, NotePlayHandle* nph = nullptr) -> bool;

	auto isRemotePlugin() const -> bool;

	auto plugin() const -> const Plugin* { return m_plugin; }
	auto plugin() -> Plugin* { return m_plugin; }
	auto effect() noexcept -> Effect*;
	auto instrument() noexcept -> Instrument*;

	auto effectRef() -> Effect&;
	auto instrumentRef() -> Instrument&;
	auto audioProcessor() -> AudioProcessor& { return dynamic_cast<AudioProcessor&>(*m_plugin); }

	void convertToDummyEffect(Model* parent, const QDomElement& originalPluginData);

	auto operator->() -> Plugin& { assert(m_plugin != nullptr); return *m_plugin; }
	explicit operator bool() const { return m_plugin != nullptr; }

protected:
	void rescanAudioPorts(AudioPortsRescanFlag flags) override;
	void useRemotePluginBuffers(bool remote) override;

private:
	auto audioBufferData() -> AudioBufferData&
	{
		return m_isRemotePlugin ? m_remoteBuffer : m_localBuffer;
	}

	// The plugin interface
	Plugin* m_plugin = nullptr; // TODO: Better memory management

	//! Non-owning AudioBuffer of the plugin's inputs (if any).
	//! This AudioBuffer's data is owned by m_bufferData.
	AudioBuffer m_inputs;

	//! Non-owning AudioBuffer of the plugin's outputs (if any).
	//! This AudioBuffer's data is owned by m_bufferData.
	AudioBuffer m_outputs;

	//! input --> output in-place pair mapping; @see AudioProcessor::getInplacePair()
	std::vector<std::optional<ch_cnt_t>> m_inplacePairsInToOut;

	//! output --> input in-place pair mapping; @see AudioProcessor::getInplacePair()
	std::vector<std::optional<ch_cnt_t>> m_inplacePairsOutToIn;

	//! How the track channels are connected to the plugin
	AudioPortsModel m_audioPorts;

	//! Contains local input and output buffers
	//! For non-RemotePlugin only
	AudioBufferData m_localBuffer;

	//! Contains remote input and output buffers
	//! For RemotePlugin only
	AudioBufferData m_remoteBuffer;

	SharedMemory<std::byte[]> m_remoteBufferData; //!< for RemotePlugin only
	SharedMemoryResource m_remoteBufferResource;  //!< for RemotePlugin only

	std::atomic<State> m_state = State{0};
	bool m_isRemotePlugin = false;
};

} // namespace lmms

#endif // LMMS_PLUGIN_INSTANCE_H
