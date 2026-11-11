/*
 * AudioPorts.h
 *
 * Copyright (c) 2025 Dalton Messmer <messmer.dalton/at/gmail.com>
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

#ifndef LMMS_AUDIO_PORTS_H
#define LMMS_AUDIO_PORTS_H

#include "AudioBuffer.h"
#include "AudioBufferView.h"
#include "AudioEngine.h"
#include "AudioPortsSettings.h"
#include "AudioPortsModel.h"
#include "Engine.h"
#include "LmmsPolyfill.h"

namespace lmms
{

//! Tells the Core what to do with a processor after processing
enum class ProcessStatus
{
	/**
	 * Unconditionally continue processing.
	 *
	 * NOTE: Instruments currently only support this option, so they must return this value.
	 */
	Continue,

	/**
	 * When the "Keep effects running even without input" setting is off (i.e. "auto-quit" is on):
	 *    The samples of each processor output channel currently in use (that is, output channels routed to
	 *    a track channel) are compared to a silence threshold, and if all the samples are silent for enough
	 *    periods (see `Effect::handleAutoQuit`), the plugin will be put to sleep (`ProcessStatus::Sleep`).
	 *
	 * Otherwise, this is equivalent to `ProcessStatus::Continue`.
	 */
	ContinueIfNotQuiet,

	//! Do not continue processing
	Sleep
};


namespace detail {

//! Metafunction to select the appropriate non-owning audio buffer view
template<AudioPortsSettings settings, bool isOutput, bool isConst>
class GetAudioBufferViewTypeHelper
{
	static constexpr auto s_channels = settings.inplace
		? std::max(settings.inputs, settings.outputs)
		: (isOutput ? settings.outputs : settings.inputs);

public:
	using SampleT = std::conditional_t<isConst,
		const GetAudioDataType<settings.kind>,
		GetAudioDataType<settings.kind>
	>;

	using type = std::conditional_t<settings.interleaved,
		InterleavedBufferView<SampleT, s_channels>,
		PlanarBufferView<SampleT, s_channels>
	>;
};

} // namespace detail


//! Metafunction to select the appropriate non-owning audio buffer view
template<AudioPortsSettings settings, bool isOutput, bool isConst>
using GetAudioBufferViewType = typename detail::GetAudioBufferViewTypeHelper<settings, isOutput, isConst>::type;


// Forward declaration
template<AudioPortsSettings settings>
class AudioPorts;

namespace detail {

/**
 * Base interface for input/output audio buffers.
 * Only the methods found here are safe to call when the buffers are not initialized.
 */
class AudioPortsBufferBase
{
public:
	virtual ~AudioPortsBufferBase() = default;

	//! Whether the buffers can be used
	virtual auto initialized() const -> bool = 0;

	//! May not return a meaningful result unless initialized
	virtual auto frames() const -> f_cnt_t = 0;

	/**
	 * Initialize or update the buffers.
	 * Never call while `AudioPorts` is active because the processor could be using the buffers.
	 *
	 * TODO: Rework plugin threading conventions + introduce plugin state (inactive/active/processing)
	 *       so that this can be safely called after buffers are already initialized.
	 */
	virtual void updateBuffers(ch_cnt_t channelsIn, ch_cnt_t channelsOut, f_cnt_t frames) = 0;
};

/**
 * Interface for accessing input/output audio buffers.
 * Do not call the methods here unless the buffers are initialized.
 */
template<AudioPortsSettings settings, bool inplace = settings.inplace>
class AudioPortsBuffer;

//! Dynamically in-place specialization
template<AudioPortsSettings settings>
class AudioPortsBuffer<settings, false> : public AudioPortsBufferBase
{
public:
	virtual auto input() -> GetAudioBufferViewType<settings, false, false> = 0;
	virtual auto output() -> GetAudioBufferViewType<settings, true, false> = 0;
};

//! Statically in-place specialization
template<AudioPortsSettings settings>
class AudioPortsBuffer<settings, true> : public AudioPortsBufferBase
{
public:
	virtual auto inputOutput() -> GetAudioBufferViewType<settings, false, false> = 0;
};


//! Performs pin connector routing. See `AudioPorts::Router`.
template<AudioPortsSettings settings>
class AudioPortsRouter
{
	using SampleT = GetAudioDataType<settings.kind>;

	static_assert(!settings.interleaved || settings.sampleFrameCompatible(),
		"AudioPorts::Router currently only supports interleaved buffers if they are SampleFrame-compatible");

public:
	explicit AudioPortsRouter(AudioPorts<settings>& parent)
		: m_ap{&parent}
	{
	}

	/**
	 * Routes core audio to the processor's input buffers, calls `processFunc` (the processor's process
	 * method), then routes audio from the processor's output buffers back to the track channels.
	 *
	 * `coreInOut`        : track channels from LMMS core (currently just the main track channel pair)
	 * `processFunc`      : the processor's process method - a callable object with the signature
	 *                      `ProcessStatus(buffers...)` where `buffers` is the expected audio buffer(s)
	 *                      (if any) for the given `settings`.
	 */
	template<class F>
	auto process(AudioBuffer& coreInOut, F&& processFunc) -> ProcessStatus
	{
		m_silentOutput = false;

		assert(m_ap->active() == true);
		auto processorBuffers = m_ap->buffers();
		assert(processorBuffers != nullptr);

		ProcessStatus status;
		if constexpr (settings.sampleFrameCompatible())
		{
			if (const auto dr = m_ap->directRouting())
			{
				// The "direct routing" optimization can be applied
				status = processWithDirectRouting(coreInOut.trackChannelPair(*dr),
					*processorBuffers, std::forward<F>(processFunc));
			}
			else
			{
				status = processNormally(coreInOut, *processorBuffers, std::forward<F>(processFunc));
			}
		}
		else
		{
			status = processNormally(coreInOut, *processorBuffers, std::forward<F>(processFunc));
		}

		coreInOut.sanitize(*m_ap);

		// Update silence status for track channels the processor wrote to
		if (!m_ap->isInstrument()) // TODO: Remove condition once NotePlayHandle-based instruments are supported?
		{
			m_silentOutput = coreInOut.update(*m_ap);
		}

		return status;
	}

	/**
	 * Whether the processor output channels currently in use (processor channels that are routed to
	 * at least one track channel) are silent.
	 *
	 * Only calculated for effects. Otherwise the outputs are assumed to not be silent.
	 */
	auto silentOutput() const -> bool { return m_silentOutput; }

#ifdef LMMS_TESTING
	friend class ::AudioPortsTest;
#endif

private:
	/**
	 * `send`
	 *     Routes audio from track channels to processor inputs according to the pin connections.
	 *
	 *     For each processor input channel, mixes together audio from all track channels
	 *     that are routed to that processor input channel, then writes the result to that channel.
	 *     If no audio is routed to a processor input channel, that channel's buffer is zeroed.
	 *
	 *     `in`     : track channels from LMMS core (currently just the main track channel pair)
	 *                `in.frames()` provides the number of frames in each `in`/`out` audio buffer
	 *     `out`    : processor input buffers
	 *
	 * `receive`
	 *     Routes audio from processor outputs to track channels according to the pin connections.
	 *
	 *     For each track channel, mixes together audio from all processor output channels
	 *     that are routed to that track channel, then writes the result to that channel.
	 *     If no audio is routed to a track channel, that channel remains unchanged for "passthrough" behavior.
	 *
	 *     `in`      : processor output buffers
	 *     `inOut`   : track channels from/to LMMS core (currently just the main track channel pair)
	 *                 `inOut.frames()` provides the number of frames in each `in`/`inOut` audio buffer
	 */

	void send(const AudioBuffer& in, PlanarBufferView<SampleT, settings.inputs> out) const
		requires (!settings.interleaved);
	void receive(PlanarBufferView<const SampleT, settings.outputs> in, AudioBuffer& inOut) const
		requires (!settings.interleaved);

	void send(const AudioBuffer& in, InterleavedBufferView<float, settings.inputs> out) const
		requires (settings.interleaved);
	void receive(InterleavedBufferView<const float, settings.outputs> in, AudioBuffer& inOut) const
		requires (settings.interleaved);

	/**
	 * Processes audio normally (no "direct routing").
	 * Uses `send` and `receive` to route audio to and from the processor.
	 */
	template<class F>
	auto processNormally(AudioBuffer& coreInOut, AudioPortsBuffer<settings>& processorBuffers, F&& processFunc)
		-> ProcessStatus;

	/**
	 * Processes audio using the "direct routing" optimization for more efficient processing.
	 * Does not use `send` or `receive` at all.
	 */
	template<class F>
	auto processWithDirectRouting(InterleavedBufferView<float, 2> coreBuffer,
		AudioPortsBuffer<settings>& processorBuffers, F&& processFunc) -> ProcessStatus;

	AudioPorts<settings>* const m_ap;
	bool m_silentOutput = false;
};

} // namespace detail


/**
 * Interface for an audio ports implementation.
 * Contains the `AudioPortsModel` and provides access to the audio buffers.
 *
 * Used by `AudioPlugin` to handle all the customizable aspects of the plugin's audio ports.
 */
template<AudioPortsSettings settings>
class AudioPorts
	: public AudioPortsModel
{
	static_assert(Validate<settings>{}());

public:
	explicit AudioPorts(bool isInstrument, Model* parent = nullptr)
		: AudioPortsModel{settings.inputs, settings.outputs, isInstrument, parent}
	{
	}

	/**
	 * AudioPorts buffer
	 *
	 * Interface for accessing input/output audio buffers.
	 * Implement this for custom AudioPorts buffers.
	 */
	using Buffer = detail::AudioPortsBuffer<settings>;

	/**
	 * AudioPorts router
	 *
	 * Performs pin connector routing for an audio processor.
	 */
	using Router = detail::AudioPortsRouter<settings>;

	template<AudioPortsSettings>
	friend class detail::AudioPortsRouter;

	/**
	 * Must be called after construction.
	 *
	 * NOTE: This cannot be called in the constructor due
	 *       to the use of a virtual method.
	 */
	void init()
	{
		if (!AudioPortsModel::initialized()) { return; }
		if (auto buffers = this->buffers())
		{
			buffers->updateBuffers(in().channelCount(), out().channelCount(),
				Engine::audioEngine()->framesPerPeriod());
		}
	}

	/**
	 * @returns whether `AudioPorts` is ready for use in the audio processor's process method.
	 *
	 * This requires `AudioPortsModel` to be initialized and the audio buffers to be available
	 * and initialized.
	 */
	auto active() const -> bool
	{
		if (!AudioPortsModel::initialized()) { return false; }
		if (auto buffers = constBuffers())
		{
			return buffers->initialized();
		}
		return false;
	}

	/**
	 * @returns the audio buffers if available, otherwise nullptr.
	 *
	 * If the buffers are available but not initialized, the input/output methods must not be called.
	 */
	virtual auto buffers() -> Buffer* = 0;

	auto constBuffers() const -> const Buffer*
	{
		// const cast to avoid duplicate code - should be safe since buffers() doesn't modify
		return const_cast<AudioPorts*>(this)->buffers();
	}

	auto model() const -> const AudioPortsModel&
	{
		return *static_cast<const AudioPortsModel*>(this);
	}

	auto model() -> AudioPortsModel&
	{
		return *static_cast<AudioPortsModel*>(this);
	}

	auto getRouter() -> Router
	{
		return Router{*this};
	}

	static constexpr auto audioPortsSettings() -> AudioPortsSettings { return settings; }
};


/**
 * Interface to help create a custom `AudioPorts` implementation.
 * `AudioPorts::Buffer` must be implemented in child class.
 */
template<AudioPortsSettings settings>
class CustomAudioPorts
	: public AudioPorts<settings>
	, protected AudioPorts<settings>::Buffer
{
public:
	using AudioPorts<settings>::AudioPorts;

protected:
	void bufferPropertiesChanging(ch_cnt_t inChannels, ch_cnt_t outChannels, f_cnt_t frames) override
	{
		// Connects `AudioPortsModel` to the buffers
		this->updateBuffers(inChannels, outChannels, frames);
	}
};


} // namespace lmms

#endif // LMMS_AUDIO_PORTS_H
