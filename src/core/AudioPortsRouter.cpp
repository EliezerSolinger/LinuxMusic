

#include "AudioPortsRouter.h"

namespace lmms {

void AudioPortsRouter::send(const AudioBuffer& in, AudioBuffer& out) const
{
	if (m_ap->in().channelCount() == 0) { return; }

	// Ignore all unused track channels for better performance
	const auto inSizeConstrained = m_ap->trackChannelsUpperBound();
	assert(inSizeConstrained <= in.totalChannels());

	for (ch_cnt_t outChannel = 0; outChannel < out.channels(); ++outChannel)
	{
		SampleT* outPtr = out.bufferPtr(outChannel);

		// Zero the output buffer
		std::fill_n(outPtr, out.frames(), SampleT{});

		for (std::uint8_t inChannelPairIdx = 0; inChannelPairIdx < inSizeConstrained; ++inChannelPairIdx)
		{
			const float* inPtr = in[inChannelPairIdx]; // track channel pair - 2-channel interleaved

			const std::uint8_t inChannel = inChannelPairIdx * 2;
			const std::uint8_t enabledPins =
				(static_cast<std::uint8_t>(m_ap->in().enabled(inChannel, outChannel)) << 1u)
				| static_cast<std::uint8_t>(m_ap->in().enabled(inChannel + 1, outChannel));

			switch (enabledPins)
			{
				case 0b00: break;
				case 0b01: // R channel only
				{
					for (f_cnt_t frame = 0; frame < in.frames(); ++frame)
					{
						outPtr[frame] += convertSample<SampleT>(inPtr[frame * 2 + 1]);
					}
					break;
				}
				case 0b10: // L channel only
				{
					for (f_cnt_t frame = 0; frame < in.frames(); ++frame)
					{
						outPtr[frame] += convertSample<SampleT>(inPtr[frame * 2]);
					}
					break;
				}
				case 0b11: // Both channels
				{
					for (f_cnt_t frame = 0; frame < in.frames(); ++frame)
					{
						outPtr[frame] += convertSample<SampleT>(inPtr[frame * 2] + inPtr[frame * 2 + 1]);
					}
					break;
				}
				default:
					unreachable();
					break;
			}
		}
	}
}

void AudioPortsRouter::receive(const AudioBuffer& in, AudioBuffer& inOut) const
{

}

void AudioPortsRouter::send(const AudioBuffer& in, PlanarBufferView<float> out) const
{
	assert(m_ap->in().channelCount() != DynamicChannelCount);
	if (m_ap->in().channelCount() == 0) { return; }

	// Ignore all unused track channels for better performance
	const auto inSizeConstrained = m_ap->trackChannelsUpperBound() / 2;
	assert(inSizeConstrained <= in.channelPairs());

	for (ch_cnt_t outChannel = 0; outChannel < out.channels(); ++outChannel)
	{
		SampleT* outPtr = out.bufferPtr(outChannel);

		// Zero the output buffer
		std::fill_n(outPtr, out.frames(), SampleT{});

		for (std::uint8_t inChannelPairIdx = 0; inChannelPairIdx < inSizeConstrained; ++inChannelPairIdx)
		{
			const float* inPtr = in[inChannelPairIdx]; // track channel pair - 2-channel interleaved

			const std::uint8_t inChannel = inChannelPairIdx * 2;
			const std::uint8_t enabledPins =
				(static_cast<std::uint8_t>(m_ap->in().enabled(inChannel, outChannel)) << 1u)
				| static_cast<std::uint8_t>(m_ap->in().enabled(inChannel + 1, outChannel));

			switch (enabledPins)
			{
				case 0b00: break;
				case 0b01: // R channel only
				{
					for (f_cnt_t frame = 0; frame < in.frames(); ++frame)
					{
						outPtr[frame] += convertSample<SampleT>(inPtr[frame * 2 + 1]);
					}
					break;
				}
				case 0b10: // L channel only
				{
					for (f_cnt_t frame = 0; frame < in.frames(); ++frame)
					{
						outPtr[frame] += convertSample<SampleT>(inPtr[frame * 2]);
					}
					break;
				}
				case 0b11: // Both channels
				{
					for (f_cnt_t frame = 0; frame < in.frames(); ++frame)
					{
						outPtr[frame] += convertSample<SampleT>(inPtr[frame * 2] + inPtr[frame * 2 + 1]);
					}
					break;
				}
				default:
					unreachable();
					break;
			}
		}
	}
}

void AudioPortsRouter::receive(PlanarBufferView<const float> in, AudioBuffer& inOut) const
{
	assert(m_ap->out().channelCount() != DynamicChannelCount);
	if (m_ap->out().channelCount() == 0) { return; }

	// Ignore all unused track channels for better performance
	const auto inOutSizeConstrained = m_ap->trackChannelsUpperBound() / 2;
	assert(inOutSizeConstrained <= inOut.channelPairs());

	/*
	 * Routes processor audio to track channel pair and normalizes the result. For track channels
	 * without any processor audio routed to it, the track channel is unmodified for "passthrough"
	 * behavior.
	 */
	const auto routeNx2 = [&](float* outPtr, ch_cnt_t outChannel, auto usedTrackChannels) {
		constexpr std::uint8_t utc = usedTrackChannels();

		if constexpr (utc == 0b00)
		{
			// Both track channels pass through - nothing to do
			return;
		}

		const auto samples = inOut.frames() * 2;

		// We know at this point that we are writing to at least one of the track channels
		// rather than letting them pass through, so it is safe to set the output buffer of those
		// channels to zero prior to accumulation
		// TODO: This would benefit from keeping track of silent track channels

		if constexpr (utc == 0b11)
		{
			std::fill_n(outPtr, samples, 0.f);
		}
		else
		{
			for (f_cnt_t sampleIdx = 0; sampleIdx < samples; sampleIdx += 2)
			{
				if constexpr ((utc & 0b10) != 0)
				{
					outPtr[sampleIdx] = 0.f;
				}
				if constexpr ((utc & 0b01) != 0)
				{
					outPtr[sampleIdx + 1] = 0.f;
				}
			}
		}

		for (ch_cnt_t inChannel = 0; inChannel < in.channels(); ++inChannel)
		{
			const SampleT* inPtr = in.bufferPtr(inChannel);

			if constexpr (utc == 0b11)
			{
				// This input channel could be routed to either left, right, both, or neither output channels
				if (m_ap->out().enabled(outChannel, inChannel))
				{
					if (m_ap->out().enabled(outChannel + 1, inChannel))
					{
						for (f_cnt_t sampleIdx = 0; sampleIdx < samples; sampleIdx += 2)
						{
							outPtr[sampleIdx]     += inPtr[sampleIdx / 2];
							outPtr[sampleIdx + 1] += inPtr[sampleIdx / 2];
						}
					}
					else
					{
						for (f_cnt_t sampleIdx = 0; sampleIdx < samples; sampleIdx += 2)
						{
							outPtr[sampleIdx] += inPtr[sampleIdx / 2];
						}
					}
				}
				else if (m_ap->out().enabled(outChannel + 1, inChannel))
				{
					for (f_cnt_t sampleIdx = 0; sampleIdx < samples; sampleIdx += 2)
					{
						outPtr[sampleIdx + 1] += inPtr[sampleIdx / 2];
					}
				}
			}
			else if constexpr (utc == 0b10)
			{
				// This input channel may or may not be routed to the left output channel
				if (!m_ap->out().enabled(outChannel, inChannel)) { continue; }

				for (f_cnt_t sampleIdx = 0; sampleIdx < samples; sampleIdx += 2)
				{
					outPtr[sampleIdx] += inPtr[sampleIdx / 2];
				}
			}
			else if constexpr (utc == 0b01)
			{
				// This input channel may or may not be routed to the right output channel
				if (!m_ap->out().enabled(outChannel + 1, inChannel)) { continue; }

				for (f_cnt_t sampleIdx = 0; sampleIdx < samples; sampleIdx += 2)
				{
					outPtr[sampleIdx + 1] += inPtr[sampleIdx / 2];
				}
			}
		}
	};


	for (std::uint8_t outChannelPairIdx = 0; outChannelPairIdx < inOutSizeConstrained; ++outChannelPairIdx)
	{
		float* outPtr = inOut[outChannelPairIdx]; // track channel pair - 2-channel interleaved
		const auto outChannel = static_cast<ch_cnt_t>(outChannelPairIdx * 2);

		const std::uint8_t usedTrackChannels =
			(static_cast<std::uint8_t>(m_ap->out().usedTrackChannels()[outChannel]) << 1u)
			| static_cast<std::uint8_t>(m_ap->out().usedTrackChannels()[outChannel + 1]);

		switch (usedTrackChannels)
		{
			case 0b00:
				// Both track channels pass through, so nothing is allowed to be written to output
				break;
			case 0b01:
				routeNx2(outPtr, outChannel, std::integral_constant<std::uint8_t, 0b01>{});
				break;
			case 0b10:
				routeNx2(outPtr, outChannel, std::integral_constant<std::uint8_t, 0b10>{});
				break;
			case 0b11:
				routeNx2(outPtr, outChannel, std::integral_constant<std::uint8_t, 0b11>{});
				break;
			default:
				unreachable();
				break;
		}
	}
}

void AudioPortsRouter::send(const AudioBuffer& in, InterleavedBufferView<float> out) const
{
	assert(m_ap->in().channelCount() != DynamicChannelCount);
	if (m_ap->in().channelCount() == 0) { return; }
	assert(m_ap->in().channelCount() == 2); // Interleaved routing only allows exactly 0 or 2 channels

	// Ignore all unused track channels for better performance
	const auto inSizeConstrained = m_ap->trackChannelsUpperBound() / 2;
	assert(inSizeConstrained <= in.channelPairs());
	assert(out.data() != nullptr);

	// Zero the output buffer
	std::fill_n(out.data(), out.frames() * 2, 0.f);

	/*
	 * This is essentially a function template with specializations for each
	 * of the 16 total routing combinations of an input 2-channel interleaved buffer to an
	 * output 2-channel interleaved buffer. The purpose is to eliminate all branching within
	 * the inner for-loop in hopes of better performance.
	 */
	auto route2x2 = [samples = in.frames() * 2, outPtr = out.data()](const float* inPtr, auto enabledPins) {
		constexpr auto epL =  static_cast<std::uint8_t>(enabledPins() >> 2); // for L out channel
		constexpr auto epR = static_cast<std::uint8_t>(enabledPins() & 0b0011); // for R out channel

		if constexpr (enabledPins() == 0) { return; }

		for (f_cnt_t sampleIdx = 0; sampleIdx < samples; sampleIdx += 2)
		{
			// Route to left output channel
			if constexpr ((epL & 0b01) != 0)
			{
				outPtr[sampleIdx] += inPtr[sampleIdx + 1];
			}
			if constexpr ((epL & 0b10) != 0)
			{
				outPtr[sampleIdx] += inPtr[sampleIdx];
			}

			// Route to right output channel
			if constexpr ((epR & 0b01) != 0)
			{
				outPtr[sampleIdx + 1] += inPtr[sampleIdx + 1];
			}
			if constexpr ((epR & 0b10) != 0)
			{
				outPtr[sampleIdx + 1] += inPtr[sampleIdx];
			}
		}
	};


	for (std::uint8_t inChannelPairIdx = 0; inChannelPairIdx < inSizeConstrained; ++inChannelPairIdx)
	{
		const float* inPtr = in[inChannelPairIdx]; // track channel pair - 2-channel interleaved

		const std::uint8_t inChannel = inChannelPairIdx * 2;
		const std::uint8_t enabledPins =
			(static_cast<std::uint8_t>(m_ap->in().enabled(inChannel, 0)) << 3u)
			| (static_cast<std::uint8_t>(m_ap->in().enabled(inChannel + 1, 0)) << 2u)
			| (static_cast<std::uint8_t>(m_ap->in().enabled(inChannel, 1)) << 1u)
			| static_cast<std::uint8_t>(m_ap->in().enabled(inChannel + 1, 1));

		switch (enabledPins)
		{
			case 0: break;
			case 1: route2x2(inPtr, std::integral_constant<std::uint8_t, 1>{}); break;
			case 2: route2x2(inPtr, std::integral_constant<std::uint8_t, 2>{}); break;
			case 3: route2x2(inPtr, std::integral_constant<std::uint8_t, 3>{}); break;
			case 4: route2x2(inPtr, std::integral_constant<std::uint8_t, 4>{}); break;
			case 5: route2x2(inPtr, std::integral_constant<std::uint8_t, 5>{}); break;
			case 6: route2x2(inPtr, std::integral_constant<std::uint8_t, 6>{}); break;
			case 7: route2x2(inPtr, std::integral_constant<std::uint8_t, 7>{}); break;
			case 8: route2x2(inPtr, std::integral_constant<std::uint8_t, 8>{}); break;
			case 9: route2x2(inPtr, std::integral_constant<std::uint8_t, 9>{}); break;
			case 10: route2x2(inPtr, std::integral_constant<std::uint8_t, 10>{}); break;
			case 11: route2x2(inPtr, std::integral_constant<std::uint8_t, 11>{}); break;
			case 12: route2x2(inPtr, std::integral_constant<std::uint8_t, 12>{}); break;
			case 13: route2x2(inPtr, std::integral_constant<std::uint8_t, 13>{}); break;
			case 14: route2x2(inPtr, std::integral_constant<std::uint8_t, 14>{}); break;
			case 15: route2x2(inPtr, std::integral_constant<std::uint8_t, 15>{}); break;
			default:
				unreachable();
				break;
		}
	}
}

void AudioPortsRouter::receive(InterleavedBufferView<const float> in, AudioBuffer& inOut) const
{
	assert(m_ap->out().channelCount() != DynamicChannelCount);
	if (m_ap->out().channelCount() == 0) { return; }
	assert(m_ap->out().channelCount() == 2); // Interleaved routing only allows exactly 0 or 2 channels

	// Ignore all unused track channels for better performance
	const auto inOutSizeConstrained = m_ap->trackChannelsUpperBound() / 2;
	assert(inOutSizeConstrained <= inOut.channelPairs());
	assert(in.data() != nullptr);

	/*
	 * This is essentially a function template with specializations for each
	 * of the 16 total routing combinations of an input 2-channel interleaved buffer to an
	 * output 2-channel interleaved buffer. The purpose is to eliminate all branching within
	 * the inner for-loop in hopes of better performance.
	 */
	auto route2x2 = [samples = inOut.frames() * 2, inPtr = in.data()](float* outPtr, auto enabledPins) {
		constexpr auto epL =  static_cast<std::uint8_t>(enabledPins() >> 2); // for L out channel
		constexpr auto epR = static_cast<std::uint8_t>(enabledPins() & 0b0011); // for R out channel

		if constexpr (enabledPins() == 0) { return; }

		// We know at this point that we are writing to at least one of the track channels rather
		// than letting them pass through, so it is safe to overwrite the contents of the output buffer

		for (f_cnt_t sampleIdx = 0; sampleIdx < samples; sampleIdx += 2)
		{
			// Route to left output channel
			if constexpr (epL == 0b11)
			{
				outPtr[sampleIdx] = inPtr[sampleIdx] + inPtr[sampleIdx + 1];
			}
			else if constexpr (epL == 0b01)
			{
				outPtr[sampleIdx] = inPtr[sampleIdx + 1];
			}
			else if constexpr (epL == 0b10)
			{
				outPtr[sampleIdx] = inPtr[sampleIdx];
			}

			// Route to right output channel
			if constexpr (epR == 0b11)
			{
				outPtr[sampleIdx + 1] = inPtr[sampleIdx] + inPtr[sampleIdx + 1];
			}
			else if constexpr (epR == 0b01)
			{
				outPtr[sampleIdx + 1] = inPtr[sampleIdx + 1];
			}
			else if constexpr (epR == 0b10)
			{
				outPtr[sampleIdx + 1] = inPtr[sampleIdx];
			}
		}
	};


	for (std::uint8_t outChannelPairIdx = 0; outChannelPairIdx < inOutSizeConstrained; ++outChannelPairIdx)
	{
		float* outPtr = inOut[outChannelPairIdx]; // track channel pair - 2-channel interleaved
		assert(outPtr != nullptr);

		const auto outChannel = static_cast<ch_cnt_t>(outChannelPairIdx * 2);
		const std::uint8_t enabledPins =
			(static_cast<std::uint8_t>(m_ap->out().enabled(outChannel, 0)) << 3u)
			| (static_cast<std::uint8_t>(m_ap->out().enabled(outChannel, 1)) << 2u)
			| (static_cast<std::uint8_t>(m_ap->out().enabled(outChannel + 1, 0)) << 1u)
			| static_cast<std::uint8_t>(m_ap->out().enabled(outChannel + 1, 1));

		switch (enabledPins)
		{
			case 0: break;
			case 1: route2x2(outPtr, std::integral_constant<std::uint8_t, 1>{}); break;
			case 2: route2x2(outPtr, std::integral_constant<std::uint8_t, 2>{}); break;
			case 3: route2x2(outPtr, std::integral_constant<std::uint8_t, 3>{}); break;
			case 4: route2x2(outPtr, std::integral_constant<std::uint8_t, 4>{}); break;
			case 5: route2x2(outPtr, std::integral_constant<std::uint8_t, 5>{}); break;
			case 6: route2x2(outPtr, std::integral_constant<std::uint8_t, 6>{}); break;
			case 7: route2x2(outPtr, std::integral_constant<std::uint8_t, 7>{}); break;
			case 8: route2x2(outPtr, std::integral_constant<std::uint8_t, 8>{}); break;
			case 9: route2x2(outPtr, std::integral_constant<std::uint8_t, 9>{}); break;
			case 10: route2x2(outPtr, std::integral_constant<std::uint8_t, 10>{}); break;
			case 11: route2x2(outPtr, std::integral_constant<std::uint8_t, 11>{}); break;
			case 12: route2x2(outPtr, std::integral_constant<std::uint8_t, 12>{}); break;
			case 13: route2x2(outPtr, std::integral_constant<std::uint8_t, 13>{}); break;
			case 14: route2x2(outPtr, std::integral_constant<std::uint8_t, 14>{}); break;
			case 15: route2x2(outPtr, std::integral_constant<std::uint8_t, 15>{}); break;
			default:
				unreachable();
				break;
		}
	}
}

template<class F>
auto AudioPortsRouter::processNormally(AudioBuffer& coreInOut,
	AudioPortsBuffer& processorBuffers, F&& processFunc) -> ProcessStatus
{
	ProcessStatus status;
	if constexpr (settings.inplace)
	{
		const auto processorInOut = processorBuffers.inputOutput();

		// Write core to processor input buffer
		if constexpr (settings.inputs != 0)
		{
			send(coreInOut, processorInOut);
		}

		// Process
		status = processFunc(processorInOut);

		// Write processor output buffer to core
		if constexpr (settings.outputs != 0)
		{
			receive(processorInOut, coreInOut);
		}
	}
	else
	{
		const auto processorIn = processorBuffers.input();
		const auto processorOut = processorBuffers.output();

		// Write core to processor input buffer
		if constexpr (settings.inputs != 0)
		{
			send(coreInOut, processorIn);
		}

		// Process
		status = processFunc(processorIn, processorOut);

		// Write processor output buffer to core
		if constexpr (settings.outputs != 0)
		{
			receive(processorOut, coreInOut);
		}
	}
	return status;
}

template<class F>
auto AudioPortsRouter::processWithDirectRouting(InterleavedBufferView<float, 2> coreBuffer,
	AudioPortsBuffer& processorBuffers, F&& processFunc) -> ProcessStatus
{
	ProcessStatus status;
	if constexpr (settings.inplace)
	{
		if constexpr (settings.buffered)
		{
			// Can avoid calling routing methods, but must write to and read from processor's buffers

			// Write core to processor input buffer (if it has one)
			const auto processorInOut = processorBuffers.inputOutput();
			if constexpr (settings.inputs != 0)
			{
				if (m_ap->in().channelCount() != 0)
				{
					assert(processorInOut.data() != nullptr);
					std::memcpy(processorInOut.data(), coreBuffer.data(), coreBuffer.dataSizeBytes());
				}
			}

			// Process
			status = processFunc(processorInOut);

			// Write processor output buffer (if it has one) to core
			if constexpr (settings.outputs != 0)
			{
				if (m_ap->out().channelCount() != 0)
				{
					assert(processorInOut.data() != nullptr);
					std::memcpy(coreBuffer.data(), processorInOut.data(), coreBuffer.dataSizeBytes());
				}
			}
		}
		else
		{
			// Can avoid using processor's input/output buffers AND avoid calling routing methods
			status = processFunc(coreBuffer);
		}
	}
	else
	{
		if constexpr (settings.buffered)
		{
			// Can avoid calling routing methods, but must write to and read from processor's buffers

			const auto processorIn = processorBuffers.input();
			const auto processorOut = processorBuffers.output();

			// Write core to processor input buffer (if it has one)
			if constexpr (settings.inputs != 0)
			{
				if (m_ap->in().channelCount() != 0)
				{
					assert(processorIn.data() != nullptr);
					std::memcpy(processorIn.data(), coreBuffer.data(), coreBuffer.dataSizeBytes());
				}
			}

			// Process
			status = processFunc(processorIn, processorOut);

			// Write processor output buffer (if it has one) to core
			if constexpr (settings.outputs != 0)
			{
				if (m_ap->out().channelCount() != 0)
				{
					assert(processorOut.data() != nullptr);
					std::memcpy(coreBuffer.data(), processorOut.data(), coreBuffer.dataSizeBytes());
				}
			}
		}
		else
		{
			// Can avoid calling routing methods, but a buffer copy may be needed

			const auto processorIn = processorBuffers.input();
			const auto processorOut = processorBuffers.output();

			// Check if processor is dynamically using in-place processing
			if (processorIn.data() != processorOut.data() || processorIn.size() != processorOut.size())
			{
				// Probably not using in-place processing - the processor implementation may be written under the
				// assumption that the input and output buffers are two different buffers, so we can't break
				// that assumption here. If the processor has both inputs and outputs, a buffer copy is needed.

				if (!processorIn.empty())
				{
					assert(m_ap->in().channelCount() == 2);
					if (!processorOut.empty())
					{
						// Processor has inputs and outputs - need to copy buffer
						assert(m_ap->out().channelCount() == 2);
						assert(processorOut.data() != nullptr);

						// Process
						status = processFunc(coreBuffer, processorOut);

						// Write processor output buffer to core
						std::memcpy(coreBuffer.data(), processorOut.data(), coreBuffer.dataSizeBytes());
					}
					else
					{
						// Input-only processor - no buffer copy needed
						status = processFunc(coreBuffer, processorOut);
					}
				}
				else
				{
					// Output-only processor - no buffer copy needed
					status = processFunc(processorIn, coreBuffer);
				}
			}
			else
			{
				// Using in-place processing - the input and output buffers
				// are allowed to be the same buffer, so no buffer copy is needed.
				status = processFunc(coreBuffer, coreBuffer);
			}
		}
	}

	return status;
}

} // namespace lmms
