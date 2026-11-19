/* Vectorscope.h - declaration of Vectorscope class.
 *
 * Copyright (c) 2019 Martin Pavelek <he29/dot/HS/at/gmail/dot/com>
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

#ifndef VECTORSCOPE_H
#define VECTORSCOPE_H

#include <optional>
#include <vector>

#include "Effect.h"
#include "LocklessRingBuffer.h"
#include "VecControls.h"

namespace lmms
{


//! Top level class; handles LMMS interface and accumulates data for processing.
class Vectorscope : public Effect
{
public:
	Vectorscope(Model *parent, const Descriptor::SubPluginFeatures::Key *key);
	~Vectorscope() override = default;

	EffectControls *controls() override {return &m_controls;}
	LocklessRingBuffer<SampleFrame> *getBuffer() {return &m_inputBuffer;}

private:
	// Analysis only: two inputs and no outputs, so the audio passes through unchanged
	auto process(ProcessContext& context) -> ProcessStatus override;
	auto activate(double sampleRate, f_cnt_t maxFrameCount) -> bool override;
	auto channelCount(bool isInput) const -> ch_cnt_t override { return isInput ? 2 : 0; }
	auto channelGroupCount(bool isInput) const -> group_cnt_t override { return isInput ? 1 : 0; }
	auto getChannelGroup(group_cnt_t, bool isInput, AudioBuffer::ChannelGroup&) const -> ch_cnt_t override
	{
		return isInput ? 2 : 0;
	}
	auto getInplacePair(ch_cnt_t) const -> std::optional<ch_cnt_t> override { return std::nullopt; }

	//! The planar input interleaved for the ring buffer; sized in activate()
	std::vector<SampleFrame> m_interleaved;

	VecControls m_controls;

	// Maximum LMMS buffer size (hard coded, the actual constant is hard to get)
	const unsigned int m_maxBufferSize = 4096;
	LocklessRingBuffer<SampleFrame> m_inputBuffer;
};


} // namespace lmms

#endif // VECTORSCOPE_H

