/*
 * AudioProcessor.h
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

#ifndef LMMS_AUDIO_PROCESSOR_H
#define LMMS_AUDIO_PROCESSOR_H

#include "AudioBuffer.h"

#include <optional>
#include <stdexcept>
#include <string_view>

namespace lmms {

class NotePlayHandle;

//! Tells the host what to do with a processor after processing
enum class ProcessStatus
{
	/**
	 * Unconditionally continue processing.
	 *
	 * @note Instruments currently only support this option, so they must return this value.
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

//! Arguments passed to an audio processor's process method
class ProcessContext
{
public:
	ProcessContext(const AudioBuffer* in, AudioBuffer* out, NotePlayHandle* nph = nullptr) noexcept;

	//! @returns the processor's input buffer if it exists
	auto in() const -> const AudioBuffer&;

	//! @returns the processor's output buffer if it exists
	auto out() -> AudioBuffer&;

	//! @returns the processor's in-place input/output buffer if it exists
	auto inOut() -> AudioBuffer&;

	//! @returns the current NotePlayHandle if it exists
	auto nph() -> NotePlayHandle&;

	// TODO: In the future, a note event queue will go here

private:
	AudioBuffer* m_out = nullptr;
	const AudioBuffer* m_in = nullptr;
	NotePlayHandle* m_nph;
};

/**
 * Interface for an audio processor.
 *
 * Contains the process method called by the host each processing period,
 * as well as methods describing the processor's audio ports so the host
 * can know how to interact with the processor, what buffers it needs, what
 * optimizations are possible, etc.
 */
class AudioProcessor
{
public:
	enum class State : std::uint8_t
	{
		//! Processor is loaded and initialized.
		//! Most native LMMS plugins are always loaded, but some have
		//! an unloaded state (i.e. VeSTige with no VST loaded).
		Loaded     = 1 << 0,

		//! Processor is prepared for processing.
		//! Many aspects of the processor are not allowed to change while the processor
		//! is active. For example, the sampling rate, maximum frame count, and
		//! the audio port settings.
		//! @pre Loaded
		Active     = 1 << 1,

		//! Processor is processing.
		//! @pre Loaded & Active
		Processing = 1 << 2,

		//! Processor is sleeping due to auto-quit or other means.
		//! @pre Loaded & Active
		Sleeping   = 1 << 3,

		//! An unrecoverable error occurred
		Error      = 1 << 7
	};

	////////////////////////////
	// Main processor methods //
	////////////////////////////

	virtual auto activate(double sampleRate, f_cnt_t maxFrameCount) -> bool = 0;
	virtual void deactivate() = 0;

	virtual auto startProcessing() -> bool = 0;
	virtual void stopProcessing() = 0;

	virtual void reset() = 0;

	virtual auto process(ProcessContext& context) -> ProcessStatus = 0;

	//! TODO: Remove once multi-stream instruments are refactored (see issue #8294)
	virtual void playNote(ProcessContext& context) {}

	/////////////////////////
	// Audio port settings //
	/////////////////////////

	/**
	 * @param isInput whether the host is requesting the channel count for the input side or the output side
	 * @returns how many channels the processor currently has
	 */
	virtual auto channelCount(bool isInput) const -> ch_cnt_t = 0;

	/**
	 * @brief how many channel groups the processor currently has
	 *
	 * This method is called by the host after the processor is instantiated.
	 * After that, the channel groups are not allowed to change until the plugin restarts.
	 *
	 * @param isInput whether the host is requesting the group count for the input side or the output side
	 * @returns the number of channel groups for the specified side - valid range is [1, MaxGroupsPerAudioBuffer]
	 */
	virtual auto channelGroupCount(bool isInput) const -> group_cnt_t = 0;

	/**
	 * @brief Allows the host to retrieve info about the processor's channel groups.
	 *
	 * This method is called by the host after the processor is instantiated.
	 * After that, the channel groups are not allowed to change until the plugin restarts.
	 *
	 * The sum of channel counts across all channel groups must be equivalent to @ref channelCount().
	 *
	 * @param index which group is being queried - range is [0, group count)
	 * @param isInput whether the host is requesting info about the inputs or the outputs
	 * @param group allows the processor to optionally set metadata for the group
	 * @returns the number of channels in the group (must be >0)
	 */
	virtual auto getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup& group) const
		-> ch_cnt_t = 0;

	/**
	 * @brief Allows to host to know which input channels are allowed to share buffers
	 *        with which output channels for in-place processing.
	 *
	 * For example, `getInplacePair(0)` would return 1 if channel #0 on the input side
	 * was allowed to share the same buffers as channel #1 on the output side.
	 *
	 * Only a 1-to-1 mapping of inputs to outputs is allowed, so each channel on the input and output
	 * sides must have exactly 0 or 1 channels on the opposite side that they are mapped to.
	 *
	 * @param inputIndex which input channel is being queried - range is [0, total input channels)
	 * @returns the output channel whose buffers can be shared with the input channel
	 *          (i.e. an in-place pair), or nullopt if no such in-place pair exists
	 */
	virtual auto getInplacePair(ch_cnt_t inputIndex) const -> std::optional<ch_cnt_t> = 0;

	//! @returns whether the processor uses interleaved buffers or planar buffers
	//! TODO: Remove interleaved buffer support in the future
	virtual auto usesInterleavedBuffers() const -> bool { return false; }

	//! @returns whether the processor uses shared memory buffers
	//! TODO: In the future, all plugins should be capable of running as a remote plugin, and
	//!       this would be handled transparently on the host side without the knowledge of the plugin.
	virtual auto usesSharedMemoryBuffers() const -> bool { return false; }

	//! Allows the processor to respond to changes in the shared memory allocation.
	//! TODO: Remove once remote plugins are an implementation detail invisible to plugin implementations.
	virtual void onSharedMemoryChange(std::string_view newKey) const {}
};

////////////////////
// Implementation //
////////////////////

inline ProcessContext::ProcessContext(const AudioBuffer* in, AudioBuffer* out, NotePlayHandle* nph) noexcept
	: m_out{out}
	, m_in{in}
	, m_nph{nph}
{}

inline auto ProcessContext::in() const -> const AudioBuffer&
{
	if (!m_in) { throw std::logic_error{"ProcessContext: The processor does not have an input buffer"}; }
	return *m_in;
}

inline auto ProcessContext::out() -> AudioBuffer&
{
	if (!m_out) { throw std::logic_error{"ProcessContext: The processor does not have an output buffer"}; }
	return *m_out;
}

inline auto ProcessContext::inOut() -> AudioBuffer&
{
	if (!m_out || m_in) { throw std::logic_error{"ProcessContext: The processor is not fully in-place"}; }
	return *m_out;
}

inline auto ProcessContext::nph() -> NotePlayHandle&
{
	if (!m_nph) { throw std::logic_error{"ProcessContext: The processor does not have a NotePlayHandle"}; }
	return *m_nph;
}

} // namespace lmms

#endif // LMMS_AUDIO_PROCESSOR_H
