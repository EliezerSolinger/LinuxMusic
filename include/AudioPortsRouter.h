/*
 * AudioPortsRouter.h
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

#ifndef LMMS_AUDIO_PORTS_ROUTER_H
#define LMMS_AUDIO_PORTS_ROUTER_H

#include "AudioBuffer.h"
#include "AudioPortsSettings.h"

namespace lmms {

//! Performs pin connector routing. See `AudioPorts::Router`.
class AudioPortsRouter
{
public:
	explicit AudioPortsRouter(AudioPorts<settings>& parent, const AudioPortsSettings& settings)
		: m_ap{&parent}
		, m_settings{settings}
	{}

	// Whenever pin connections change, need to update
	void assignBuffers(const AudioPortsModel& model, AudioPortsBuffer& buffer);

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

	void send(const AudioBuffer& in, AudioBuffer& out) const;
	void receive(const AudioBuffer& in, AudioBuffer& inOut) const;

	void send(const AudioBuffer& in, PlanarBufferView<float> out) const;
	void receive(PlanarBufferView<const float> in, AudioBuffer& inOut) const;

	void send(const AudioBuffer& in, InterleavedBufferView<float> out) const;
	void receive(InterleavedBufferView<const float> in, AudioBuffer& inOut) const;

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
	AudioPortsSettings m_settings;
	bool m_silentOutput = false;
};

} // namespace lmms

#endif // LMMS_AUDIO_PORTS_ROUTER_H
