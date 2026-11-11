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

#include <array>
#include <atomic>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <vector>
#include <span>
#include <vector>

#include "AudioBuffer.h"
#include "LockFreeQueue.h"
#include "PluginEvent.h"
#include "AudioPortsModel.h"
#include "AudioProcessor.h"
#include "Host.h"
#include "Plugin.h"
#include "SharedMemory.h"

class QDomDocument;
class QDomElement;
class QString;

namespace lmms {

class Effect;
class Instrument;
class NotePlayHandle;
class SampleFrame;

/**
 * Manages a plugin instance and implements the host side of the LMMS plugin API.
 *
 * Owns the plugin, the buffers for its inputs and outputs, and the pin connector (AudioPortsModel) that
 * connects them to the track channels of the instrument track or mixer channel the plugin belongs to.
 *
 * For a plugin implementing AudioProcessor, process() routes the track channels to the plugin's inputs,
 * runs the plugin and routes its outputs back to the track channels. Input and output channels the plugin
 * declares as in-place pairs share a buffer, and when the pins connect a stereo in-place plugin straight to
 * the main track channels, the plugin works on the track channels directly without any copying.
 *
 * Instruments that do not implement AudioProcessor yet are still owned here, but play through their own
 * methods (see InstrumentTrack).
 *
 * This class is not accessible to plugins - it is host-side only.
 */
class LMMS_EXPORT PluginInstance : public Host
{
public:
	using State = AudioProcessor::State;

	/**
	 * Loads a plugin.
	 *
	 * @param type Plugin::Type::Instrument or Plugin::Type::Effect
	 * @param parent the InstrumentTrack for instruments, the EffectChain for effects
	 * @param key sub-plugin key, if any
	 */
	PluginInstance(Plugin::Type type, const QString& pluginName, Model* parent,
		const Plugin::Descriptor::SubPluginFeatures::Key* key, bool keyFromDnd = false);

	//! Takes ownership of an already created plugin
	PluginInstance(std::unique_ptr<Plugin> plugin, Plugin::Type type);

	~PluginInstance() override;

	PluginInstance(const PluginInstance&) = delete;
	PluginInstance(PluginInstance&&) = delete;
	auto operator=(const PluginInstance&) -> PluginInstance& = delete;
	auto operator=(PluginInstance&&) -> PluginInstance& = delete;

	auto isLoaded() const -> bool { return m_plugin != nullptr; }
	auto isInstrument() const -> bool { return m_type == Plugin::Type::Instrument; }

	auto plugin() const -> const Plugin* { return m_plugin.get(); }
	auto plugin() -> Plugin* { return m_plugin.get(); }
	auto effect() noexcept -> Effect*;
	auto effect() const noexcept -> const Effect*;
	auto instrument() noexcept -> Instrument*;
	auto instrument() const noexcept -> const Instrument*;
	auto effectRef() -> Effect&;
	auto instrumentRef() -> Instrument&;

	//! @returns the plugin's audio processor, or nullptr if the plugin does not implement AudioProcessor yet
	auto audioProcessor() -> AudioProcessor* { return m_processor; }

	//! @returns the pin connector, or nullptr if the plugin does not implement AudioProcessor yet
	auto audioPortsModel() -> AudioPortsModel* { return m_audioPorts.get(); }
	auto audioPortsModel() const -> const AudioPortsModel* { return m_audioPorts.get(); }

	auto state() const -> std::uint8_t { return m_state.load(std::memory_order_relaxed); }
	auto hasState(State flag) const -> bool { return state() & static_cast<std::uint8_t>(flag); }
	auto isRemotePlugin() const -> bool { return m_isRemotePlugin; }

	/**
	 * Processes one period. [audio thread]
	 *
	 * Effects: wakes up, sleeps (auto-quit) or bypasses the effect as needed, and sanitizes its output.
	 *
	 * @param trackChannels the track channels of the instrument track or mixer channel
	 * @returns for effects, whether the effect should keep being processed; for instruments, true
	 */
	auto process(AudioBuffer& trackChannels, NotePlayHandle* nph = nullptr) -> bool;

	//! One of the plugin's output channel groups (see AudioProcessor::getChannelGroup())
	struct OutputGroup
	{
		std::string name;
		ch_cnt_t channels;
	};

	//! The plugin's output channel groups, e.g. the pieces of a drum kit [main thread]
	auto outputGroups() const -> std::vector<OutputGroup>;

	/**
	 * Connects output group g to the track channel pair g + 1 (a mono group to both of its channels) and nothing
	 * to the main pair, so that each group can have a mixer channel of its own (Mixer::setUpChannelPerOutput()).
	 * [main thread]
	 * @returns false if the track has too few channels for that
	 */
	auto routeOutputGroupsToPairs() -> bool;

	//! When the outputs have several channel groups, connects each one to the main track channels
	void connectOutputGroupsByDefault();

	/**
	 * The plugin's latency (see AudioProcessor::latency()); 0 while inactive and for disabled or bypassed
	 * effects, whose audio passes through undelayed. [audio thread]
	 */
	auto latency() const -> f_cnt_t;

	/**
	 * Renders a note of a NotePlayHandle-based instrument into the note's own buffer and applies the
	 * output pins to it. [audio thread, possibly in parallel for different notes]
	 */
	void playNote(NotePlayHandle* nph, std::span<SampleFrame> out);

	/**
	 * Queues an event for the plugin, delivered with the next period it is processed in (see
	 * AudioProcessor::wantsEvents()). The event's time is its frame within that period. [any thread]
	 *
	 * @returns false if the queue is full
	 */
	auto sendEvent(const PluginEvent& event) -> bool;

	//! Whether the plugin takes events from sendEvent()
	auto wantsEvents() const -> bool { return m_processor && m_processor->wantsEvents(); }

	//! Receives the events the plugin sends to the host, after each period [audio thread]
	using OutputEventHandler = std::function<void(std::span<const PluginEvent>)>;
	void setOutputEventHandler(OutputEventHandler handler) { m_outputEventHandler = std::move(handler); }

	//! Replaces the plugin with a DummyEffect keeping @p originalPluginData (e.g. a plugin that failed to load)
	void convertToDummyEffect(Model* parent, const QDomElement& originalPluginData);

	//! Prepares the plugin again, e.g. after the sample rate or period size changed. [main thread]
	void reactivate();

	//! Saves the pin connector into the plugin's element @p pluginElement
	void savePins(QDomDocument& doc, QDomElement& pluginElement);

	//! Loads the pin connector from the plugin's element @p pluginElement, if saved there
	void loadPins(const QDomElement& pluginElement);

	/**
	 * Sets how many channels the track (instrument track or mixer channel) has: the main pair followed by
	 * auxiliary pairs. The pins keep their connections. [main thread, audio engine paused]
	 */
	void setTrackChannelCount(ch_cnt_t channels);
	auto trackChannelCount() const -> ch_cnt_t { return m_trackChannels; }

protected:
	// Host implementation
	void rescanAudioPorts(AudioPortsRescanFlag flags) override;
	void useRemotePluginBuffers(bool remote) override;

private:
	void attach(std::unique_ptr<Plugin> plugin);
	void detach();

	//! Queries the processor's audio ports, then creates the buffers and sets up the pin connector
	void setUpAudioPorts();

	/**
	 * Connects the sidechain inputs (the second input channel group) to track channels 3/4, once the track
	 * has them, unless the pins were loaded or the inputs are connected already.
	 */
	void connectSidechainByDefault();

	void activate();
	void deactivate();
	void setState(State flag, bool on);

	auto processAudio(AudioBuffer& trackChannels, NotePlayHandle* nph) -> ProcessStatus;

	Plugin::Type m_type;
	std::unique_ptr<Plugin> m_plugin;
	AudioProcessor* m_processor = nullptr;

	//! How the track channels are connected to the plugin
	std::unique_ptr<AudioPortsModel> m_audioPorts;

	//! Non-owning views of the plugin's inputs and outputs within the active AudioBufferData
	AudioBuffer m_inputs;
	AudioBuffer m_outputs;

	//! input --> output in-place pair mapping; @see AudioProcessor::getInplacePair()
	std::vector<std::optional<ch_cnt_t>> m_inplacePairsInToOut;

	//! output --> input in-place pair mapping
	std::vector<std::optional<ch_cnt_t>> m_inplacePairsOutToIn;

	//! Every input shares its buffer with an output, so the plugin gets a single in/out buffer
	bool m_fullyInPlace = false;

	//! The plugin uses a 2-channel interleaved buffer (legacy plugins)
	bool m_interleaved = false;

	//! Buffers for local plugins
	AudioBufferData m_localBuffer;

	//! Buffers for remote plugins, in shared memory
	AudioBufferData m_remoteBuffer;
	SharedMemory<std::byte[]> m_remoteBufferData;
	SharedMemoryResource m_remoteBufferResource;

	std::atomic<std::uint8_t> m_state = 0;
	bool m_isRemotePlugin = false;

	//! The sample rate the processor was last activated with
	sample_rate_t m_activeSampleRate = 0;

	//! Events for the plugin from any thread, and the ones of the current period
	static constexpr std::size_t MaxEventsPerPeriod = 1024;
	LockFreeQueue<PluginEvent, MaxEventsPerPeriod> m_eventQueue;
	std::vector<PluginEvent> m_periodEvents;

	//! Events for later periods (at or after the end of the period they arrived in)
	std::vector<PluginEvent> m_laterEvents;
	std::vector<PluginEvent> m_heldEvents;

	//! Events from the plugin
	std::array<PluginEvent, 256> m_outputEventStorage;
	OutputEvents m_outputEvents{m_outputEventStorage.data(), m_outputEventStorage.size()};
	OutputEventHandler m_outputEventHandler;

	//! Channels of the track the plugin belongs to
	ch_cnt_t m_trackChannels = DEFAULT_CHANNELS;

	//! Whether the sidechain inputs got their default connection (or pins were loaded)
	bool m_sidechainConnected = false;
};

} // namespace lmms

#endif // LMMS_PLUGIN_INSTANCE_H
