/*
 * AudioPortsRouter.cpp - routes audio between track channels and a processor's channels
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

#include "AudioPortsRouter.h"

#include <algorithm>

#include "AudioBuffer.h"
#include "AudioPortsModel.h"

namespace lmms::AudioPortsRouter
{

void send(const AudioPortsModel& model, const AudioBuffer& track, AudioBuffer& processorIn)
{
	const auto& pins = model.in();
	const auto trackChannels = std::min<ch_cnt_t>(model.trackChannelsUpperBound(), track.channels());
	const auto processorChannels = std::min<ch_cnt_t>(pins.channelCount(), processorIn.channels());

	for (ch_cnt_t input = 0; input < processorChannels; ++input)
	{
		auto dst = processorIn.buffer(input);
		bool hasSignal = false;

		for (ch_cnt_t trackChannel = 0; trackChannel < trackChannels; ++trackChannel)
		{
			if (!pins.enabled(trackChannel, input)) { continue; }

			// Silent track channels contribute nothing
			if (!track.hasSignal(AudioBuffer::ChannelFlags{}.set(trackChannel))) { continue; }

			const auto src = track.buffer(trackChannel);
			if (!hasSignal)
			{
				std::ranges::copy(src, dst.begin());
				hasSignal = true;
			}
			else
			{
				std::ranges::transform(dst, src, dst.begin(), std::plus{});
			}
		}

		if (hasSignal)
		{
			processorIn.assumeNonSilent(input);
		}
		else
		{
			// Always zero: with in-place processing this buffer is shared with an output, which has its own
			// silence flags, so this input's flag may say "silent" while the buffer holds last period's output
			std::ranges::fill(dst, 0.f);
			processorIn.silenceChannels(AudioBuffer::ChannelFlags{}.set(input));
		}
	}
}




void receive(const AudioPortsModel& model, const AudioBuffer& processorOut, AudioBuffer& track)
{
	const auto& pins = model.out();
	const auto trackChannels = std::min<ch_cnt_t>(model.trackChannelsUpperBound(), track.channels());
	const auto processorChannels = std::min<ch_cnt_t>(pins.channelCount(), processorOut.channels());
	const auto& usedTrackChannels = pins.usedTrackChannels();

	for (ch_cnt_t trackChannel = 0; trackChannel < trackChannels; ++trackChannel)
	{
		// Track channels without any connected output pass through unchanged
		if (!usedTrackChannels[trackChannel]) { continue; }

		auto dst = track.buffer(trackChannel);
		bool written = false;

		for (ch_cnt_t output = 0; output < processorChannels; ++output)
		{
			if (!pins.enabled(trackChannel, output)) { continue; }

			const auto src = processorOut.buffer(output);
			if (!written)
			{
				// The output may share its buffer with the track channel (direct routing), so only copy if needed
				if (src.data() != dst.data()) { std::ranges::copy(src, dst.begin()); }
				written = true;
			}
			else
			{
				std::ranges::transform(dst, src, dst.begin(), std::plus{});
			}
		}

		if (!written) { std::ranges::fill(dst, 0.f); }
		track.assumeNonSilent(trackChannel);
	}
}

} // namespace lmms::AudioPortsRouter
