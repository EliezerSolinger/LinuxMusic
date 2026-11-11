/*
 * Oscilloscope.h - Oscilloscope effect to preview the incoming waveform
 *
 * Copyright (c) 2025 Keratin
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

#ifndef LMMS_OSCILLOSCOPE_H
#define LMMS_OSCILLOSCOPE_H

#include "Effect.h"
#include "OscilloscopeControls.h"
#include <vector>

#include "LocklessRingBuffer.h"

namespace lmms
{

class Oscilloscope : public Effect
{
public:
	Oscilloscope(Model* parent, const Descriptor::SubPluginFeatures::Key* key);

	// AudioProcessor implementation: a stereo input and no outputs, since the audio is only looked at
	auto process(ProcessContext& context) -> ProcessStatus override;
	auto activate(double sampleRate, f_cnt_t maxFrameCount) -> bool override;
	auto channelCount(bool isInput) const -> ch_cnt_t override { return isInput ? 2 : 0; }
	auto channelGroupCount(bool isInput) const -> group_cnt_t override { return isInput ? 1 : 0; }
	auto getChannelGroup(group_cnt_t, bool isInput, AudioBuffer::ChannelGroup&) const -> ch_cnt_t override
	{
		return isInput ? 2 : 0;
	}
	auto getInplacePair(ch_cnt_t) const -> std::optional<ch_cnt_t> override { return std::nullopt; }

	EffectControls* controls() override { return &m_controls; }

	//! Ring buffer size for the data being transfered from the audio thread to the gui thread
	//! This is not the history length! The actual size of the history buffer is defined in OscilloscopeGraph.h
	static constexpr int InputBufferSize = 4096 * 4;

	LocklessRingBuffer<SampleFrame>& inputBuffer() { return m_inputBuffer; }

private:
	OscilloscopeControls m_controls;

	LocklessRingBuffer<SampleFrame> m_inputBuffer;

	//! The planar input interleaved for the ring buffer; sized in activate()
	std::vector<SampleFrame> m_interleaved;

	friend class OscilloscopeControls;
};

} // namespace lmms

#endif // LMMS_OSCILLOSCOPE_H
