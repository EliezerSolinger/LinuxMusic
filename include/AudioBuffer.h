/*
 * AudioBuffer.h
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

#ifndef LMMS_AUDIO_BUFFER_H
#define LMMS_AUDIO_BUFFER_H

#include <bitset>
#include <memory_resource>
#include <vector>

#include "AudioBufferView.h"
#include "ArrayVector.h"
#include "Flags.h"
#include "LmmsTypes.h"
#include "lmms_constants.h"
#include "lmms_export.h"

namespace lmms
{

/**
 * Allocation and storage for audio buffers.
 *
 * Features:
 * - Can contain planar buffers, an interleaved buffer, or both
 * - Can own its own all its buffers, some of its buffers, or act as
 *       a view for a subset of another AudioBufferData
 * - All buffers have sufficient alignment for SIMD (@see lmms::SimdAlignment)
 * - Can use any memory resource including `SharedMemoryResource`
 * - Supports custom mapping for the planar access buffer, which allows certain routing optimizations
 *       and buffer re-use for in-place processing (see in-depth explanation below).
 *
 * About access buffer mapping:
 * - The access buffer provides fast channel-wise access to channel buffers within the source buffer.
 *   Normally the access buffer maps each index to a buffer using 1-to-1 mapping, so if the source
 *   buffer contains the buffers for 4 channels, accessBuffer()[0] points to the 1st channel buffer,
 *   accessBuffer()[1] points to the 2nd channel buffer, etc.
 * - What this class does is allow alternative mappings that are not necessarily 1-to-1 or in sequential
 *   order.
 *   - For example, accessBuffer()[0] could point to the 2nd buffer while accessBuffer()[1] points
 *     to the 1st buffer. Anyone reading the access buffer would think they're getting left and right
 *     stereo buffers, but they're actually getting right and left buffers - we've swapped the left and
 *     right channels by just swapping two pointers rather than performing expensive buffer copies.
 *   - As another example, accessBuffer()[0] in the input AudioBuffer could point to the same buffer as
 *     accessBuffer()[2] in the output AudioBuffer. Those input and output buffers are the same, which
 *     makes them in-place. This gives it a lot of flexibility for in-place processing.
 *   - As a 3rd example, we could have an immutable silent buffer which is available program-wide, and
 *     we could make a channel silent by simply setting the access buffer pointer of that channel to
 *     the address of the silent buffer. This avoids needing to clear the buffer.
 * - The access buffer mapping can be set with mapAccessBuffer() or reset with resetAccessBuffer().
 * - When using a shared memory resource, access buffers cannot be mapped to any arbitrary buffer - they
 *   can only be mapped to a buffer within the source buffer.
 *
 * Q) Why is there special handling for shared memory resources?
 * - A) Because pointers cannot be stored in shared memory and still be guaranteed to work correctly on
 *      the other end. So instead of storing the access buffer (which is an array of float pointers) in
 *      shared memory, we instead store pointer offsets. However, plugins and other users still expect
 *      a float** access buffer, so it must be allocated separately in non-shared memory. When changing
 *      the access buffer mapping on the server side, both the access buffer and their pointer offsets
 *      in shared memory are updated. But on the client side, refreshSharedMemoryAccessBuffer() must be
 *      called to update the client's access buffer pointers by reading their pointer offsets in shared
 *      memory. This is only required following a change in the access buffer mapping on the server side.
 *
 * Q) Why can AudioBufferData act as a view for a subset of another AudioBufferData?
 * - A) Because the buffers for both the input channels and output channels need to be created in
 *      one contiguous shared memory allocation for use by RemotePlugin, but two AudioBuffers are
 *      needed - one for the inputs and one for the outputs. So if there's one "source" AudioBufferData
 *      which contains the data for all inputs and outputs, it can be split into two non-owning
 *      sections (inputs and outputs) which are stored by an input AudioBuffer and an output AudioBuffer.
 */
class LMMS_EXPORT AudioBufferData
{
public:
	//! The ownership and semantics of the audio buffer
	enum class Ownership : std::uint8_t
	{
		//! A normal, non-shared audio buffer which owns all its buffers.
		//!     Use case: General use
		Internal           = 1 << 0,

		//! A non-shared audio buffer which owns only the access buffer, and the rest (the
		//! planar and/or interleaved buffers) are non-owning views into external buffers.
		//!     Use case: Plugin buffers sourced from a large buffer pool
		External           = 1 << 1,

		//! Owns only the access buffer, and the rest are sourced from server-side SharedMemory
		//! using a SharedMemoryResource.
		//!     Use case: RemotePlugin's buffers
		SharedMemoryServer = 1 << 2,

		//! Owns only the access buffer, and the rest are sourced from client-side SharedMemory
		//! using a SharedMemoryResource.
		//!     Use case: RemotePluginClient's buffers
		SharedMemoryClient = 1 << 3,

		//! A non-owning view into a subset of channels of another AudioBufferData.
		//! This can be combined with any of the other Ownership options. For example,
		//! Ownership::Internal | Ownership::View is a view into an Internal AudioBufferData.
		//!     Use case: Splitting a single shared memory AudioBufferData which contains both the
		//!               input and output buffers for a plugin into 2 separate AudioBufferData objects.
		View               = 1 << 7
	};

	friend LMMS_DECLARE_OPERATORS_FOR_FLAGS(Ownership);

	//! Creates an empty AudioBufferData with no channels or frames (e.g. before a plugin's buffers are known)
	AudioBufferData() = default;

	/**
	 * @brief Creates an AudioBufferData with Internal, SharedMemoryServer, or SharedMemoryClient
	 *        ownership, depending on the sourceBufferResource used
	 *
	 * @param frames the number of frames per channel
	 * @param accessChannels the number of planar channels for the access buffer (0 for no planar buffers)
	 * @param sourceChannels the number of planar channels for the source buffer. Must be >= @p channels.
	 *                           If greater than @p channels, the AudioBufferData should use @ref mapAccessBuffer()
	 *                           to map the access buffer indexes to the actual buffers within the source buffer.
	 * @param interleaved whether to allocate a 2-channel interleaved buffer
	 * @param sourceBufferResource memory resource to allocate the source buffer (and interleaved buffer) with
	 * @param accessBufferResource memory resource to allocate the access buffers with - if equal to
	 *                             sourceBufferResource, all buffers will be allocated together
	 * @throws exception upon failure
	 */
	explicit AudioBufferData(f_cnt_t frames, ch_cnt_t accessChannels = DEFAULT_CHANNELS,
		ch_cnt_t sourceChannels = DEFAULT_CHANNELS, bool interleaved = false,
		std::pmr::memory_resource* sourceBufferResource = std::pmr::get_default_resource(),
		std::pmr::memory_resource* accessBufferResource = std::pmr::get_default_resource());

	/**
	 * @brief Creates an AudioBufferData with External ownership using external source/interleaved buffers
	 *
	 * The source/interleaved buffers are not owned by this class but the access buffer is.
	 * This is not meant for use with shared memory since the access buffer's offset pointers are not used.
	 *
	 * @param source planar source buffer to use. Not shared memory.
	 * @param frames the number of frames per channel in @p source and/or @p interleaved
	 * @param accessChannels the number of planar channels for the access buffer (can be zero)
	 * @param sourceChannels the number of planar channels in @p source
	 * @param interleaved 2-channel interleaved buffer, or nullptr if not provided. Not shared memory.
	 * @param accessBufferResource memory resource to allocate the access buffers with. Not a SharedMemoryResource.
	 * @throws exception upon failure
	 */
	explicit AudioBufferData(float* source, f_cnt_t frames, ch_cnt_t accessChannels,
		ch_cnt_t sourceChannels, float* interleaved = nullptr,
		std::pmr::memory_resource* accessBufferResource = std::pmr::get_default_resource());

	/**
	 * @brief Creates an AudioBufferData with View ownership using a subset of
	 *        another AudioBufferData.
	 *
	 * This view uses the full source buffer of @p source and a subset of its access buffer.
	 *
	 * @param source the buffer to create a view for
	 * @param accessStart the access buffer index from @p source which marks the start of this view
	 * @param accessChannels how many access buffer channels this view should have
	 */
	AudioBufferData(AudioBufferData& source, ch_cnt_t accessStart, ch_cnt_t accessChannels);

	//! Deallocates the buffers if needed
	~AudioBufferData();

	AudioBufferData(const AudioBufferData&) = delete;
	AudioBufferData(AudioBufferData&& other) noexcept;
	auto operator=(const AudioBufferData&) -> AudioBufferData& = delete;
	auto operator=(AudioBufferData&& other) noexcept -> AudioBufferData&;

	/**
	 * @brief Allocates buffers using the provided memory resources.
	 *
	 * If buffers were already allocated, they will be deallocated first.
	 * This method does not set the access buffer's channel mapping, so remember
	 * to set it as needed after this.
	 *
	 * @param frames the number of frames per channel
	 * @param accessChannels the number of planar channels for the access buffer
	 * @param sourceChannels the number of planar channels for the source buffer
	 * @param interleaved whether to allocate a 2-channel interleaved buffer
	 * @param newSourceBufferResource new memory resource to allocate the source buffer (and interleaved buffer)
	 *                                with, or nullptr to keep current resource
	 * @param newAccessBufferResource new memory resource to allocate the access buffers with, or nullptr to
	 *                                keep current resource. If equal to newSourceBufferResource, all buffers
	 *                                will be allocated together.
	 * @throws exception upon failure or if the AudioBufferData is non-owning
	 */
	void create(f_cnt_t frames, ch_cnt_t accessChannels, ch_cnt_t sourceChannels, bool interleaved,
		std::pmr::memory_resource* newSourceBufferResource = nullptr,
		std::pmr::memory_resource* newAccessBufferResource = nullptr);

	//! Deallocates the memory previously allocated with create()
	void destroy();

	auto hasPlanarBuffers() const -> bool { return m_accessBuffer != nullptr; }
	auto hasInterleavedBuffer() const -> bool { return m_interleavedBuffer != nullptr; }

	//! @returns the buffers for all planar channels, assuming they exist
	auto allBuffers() const -> PlanarBufferView<const float>
	{
		assert(m_accessBuffer != nullptr);
		return {m_accessBuffer, m_accessChannels, m_frames};
	}

	//! @returns the buffers for all planar channels, assuming they exist
	auto allBuffers() -> PlanarBufferView<float>
	{
		assert(m_accessBuffer != nullptr);
		return {m_accessBuffer, m_accessChannels, m_frames};
	}

	//! @returns the buffer for the given planar channel, assuming it exists
	auto buffer(ch_cnt_t channel) const -> std::span<const float>
	{
		assert(m_accessBuffer != nullptr);
		assert(channel < m_accessChannels);
		return {m_accessBuffer[channel], m_frames};
	}

	//! @returns the buffer for the given planar channel, assuming it exists
	auto buffer(ch_cnt_t channel) -> std::span<float>
	{
		assert(m_accessBuffer != nullptr);
		assert(channel < m_accessChannels);
		return {m_accessBuffer[channel], m_frames};
	}

	//! @returns the number of channels in the access buffer, or the accessible channel count
	auto channels() const -> ch_cnt_t { return m_accessChannels; }

	//! @returns the frame count for each channel buffer
	auto frames() const -> f_cnt_t { return m_frames; }

	//! @returns scratch buffer for conversions between interleaved and planar TODO: Remove once using planar only
	auto interleavedBuffer() const -> InterleavedBufferView<const float, 2>
	{
		assert(m_interleavedBuffer != nullptr);
		return {m_interleavedBuffer, m_frames};
	}

	//! @returns scratch buffer for conversions between interleaved and planar TODO: Remove once using planar only
	auto interleavedBuffer() -> InterleavedBufferView<float, 2>
	{
		assert(m_interleavedBuffer != nullptr);
		return {m_interleavedBuffer, m_frames};
	}

	/**
	 * @returns the access buffer, assuming there are planar buffers
	 *
	 * If using a shared memory resource, the access buffer is stored in shared memory as
	 * pointer offsets, not as pointers, so this will become invalid on the client side
	 * if the server side changes the access buffer mapping.
	 *
	 * @see refreshSharedMemoryAccessBuffer() for more info.
	 */
	auto accessBuffer() -> std::span<float*>
	{
		assert(m_accessBuffer != nullptr);
		return {m_accessBuffer, m_accessChannels};
	}

	//! @returns the source buffer, assuming there are planar buffers
	auto sourceBuffer() -> std::span<float>
	{
		assert(m_sourceBuffer != nullptr);
		return {m_sourceBuffer, m_sourceChannels * m_frames};
	}

	/**
	 * Updates the access buffer pointers from the pointer offsets in shared memory.
	 *
	 * This must be called in RemotePluginClient whenever the access buffer mapping
	 * changes on the server side.
	 *
	 * @throws exception if not using client-side shared memory
	 */
	void refreshSharedMemoryAccessBuffer();

	/**
	 * Sets the access buffer to the straightforward 1-to-1 mapping of each
	 * access buffer channel to a corresponding channel buffer in the source buffer.
	 *
	 * Also updates the pointer offset in shared memory if applicable.
	 *
	 * @param sourceStart starting channel in the source buffer
	 *
	 * @throws exception if not using server-side shared memory
	 * @throws exception if there are not enough channel buffers in the source buffer to map each access buffer with
	 */
	void resetAccessBuffer(ch_cnt_t sourceStart = 0);

	/**
	 * Makes the access buffer pointer for a given channel point to a different buffer in the source buffer.
	 * Also updates the pointer offset in shared memory if applicable.
	 *
	 * @param channel access buffer index whose mapping will be changed
	 * @param mappedTo the buffer within the source buffer that @p channel will point to
	 *
	 * @throws exception if not using server-side shared memory
	 */
	void mapAccessBuffer(ch_cnt_t channel, ch_cnt_t mappedTo);

	/**
	 * Makes the access buffer pointer for a given channel point to a different buffer.
	 *
	 * @param channel access buffer index whose mapping will be changed
	 * @param mappedTo the buffer that @p channel will point to. Does not need to exist within the source buffer.
	 *
	 * @throws exception if using shared memory, since the access buffer's offset pointers
	 *         can only refer to channel buffers within the shared memory's source buffer.
	 */
	void mapAccessBuffer(ch_cnt_t channel, float* mappedTo);

	auto usesSharedMemory() const -> bool
	{
		return m_ownership.testFlag(
			static_cast<Ownership>(Ownership::SharedMemoryServer | Ownership::SharedMemoryClient));
	}

	auto isView() const -> bool { return m_ownership.testFlag(Ownership::View); }

protected:
	//! Provides access to individual channel buffers within the source buffer.
	//! If m_accessBufferResource is nullptr, this is an non-owning pointer, otherwise it's owning.
	//! [channel index][frame index]
	float** m_accessBuffer = nullptr;

	//! Large buffer that all channel buffers are sourced from.
	//! Non-owning pointer.
	//! [channel index]
	float* m_sourceBuffer = nullptr;

	//! Interleaved buffer for legacy reasons.
	//! Non-owning pointer. TODO: Remove once using planar only
	float* m_interleavedBuffer = nullptr;

	f_cnt_t m_frames = 0;
	ch_cnt_t m_accessChannels = 0;
	ch_cnt_t m_sourceChannels = 0; //!< this is always >= m_accessChannels

	//! If the source and access buffer resources are the same memory resource, this
	//! is a pointer to the single allocated block of memory which contains everything.
	//!
	//! Otherwise, if the two memory resources differ, this is a pointer to the non-access-buffer
	//! block of allocated memory (source buffer, interleaved buffer, and for shared memory,
	//! the offset pointers for the access buffer).
	void* m_allocation = nullptr;
	std::size_t m_allocationSize = 0;

	//! For allocating the source buffer and interleaved buffer.
	//! If this is a SharedMemoryResource, it is also used to allocate space for
	//! offset pointers so that access buffer channel mappings can be passed via shared memory.
	std::pmr::memory_resource* m_sourceBufferResource = nullptr;

	//! For allocating the access buffer. Cannot be a SharedMemoryResource.
	std::pmr::memory_resource* m_accessBufferResource = nullptr;

	Flags<Ownership> m_ownership;
};


/**
 * AudioBuffer is an AudioBufferData with some additional features (and restrictions).
 *
 * Restrictions:
 * - Unlike AudioBufferData which supports up to (2^16)-2 channels (the full ch_cnt_t range minus the
 *       DynamicChannelCount magic number), AudioBuffer is restricted to just MaxChannelsPerAudioBuffer channels.
 *
 * Features:
 * - Silence tracking for each channel (NOTE: requires careful use so that non-silent data is not written to a
 *       channel marked silent without updating that channel's silence flag afterward)
 * - Methods for sanitizing, silencing, and calculating the absolute peak value of channels, and doing so more
 *       efficiently using the data from silence tracking
 * - Can organize channels into arbitrary groups. For example, you could have 6 total channels divided into 2 groups
 *       where the 1st group contains 2 channels (stereo) and the 2nd contains 4 channels (quadraphonic).
 * - Extensive unit testing - @ref AudioBufferTest.cpp
 *
 *
 *
 * An owning or non-owning collection of audio channels for an instrument track, mixer channel, or audio processor.
 *
 * Features:
 * - Up to `MaxChannelsPerAudioBuffer` total channels
 * - Audio data in planar format (plus a temporary interleaved buffer for conversions until we use planar only)
 * - All planar buffers are sourced from the same large buffer for better cache locality
 * - Custom allocator support
 * - Silence tracking for each channel (NOTE: requires careful use so that non-silent data is not written to a
 *       channel marked silent without updating that channel's silence flag afterward)
 * - Methods for sanitizing, silencing, and calculating the absolute peak value of channels, and doing so more
 *       efficiently using the data from silence tracking
 * - Can organize channels into arbitrary groups. For example, you could have 6 total channels divided into 2 groups
 *       where the 1st group contains 2 channels (stereo) and the 2nd contains 4 channels (quadraphonic).
 * - Extensive unit testing - @ref AudioBufferTest.cpp
 *
 * Audio data layout explanation:
 * - All planar audio data for all channels in an AudioBuffer is sourced from the same large contiguous
 *       buffer called the source buffer (m_sourceBuffer).
 * - The source buffer consists of the buffer for 1st channel followed by the buffer for the 2nd channel, and so on
 *       for all channels. In total, the number of elements is `channels * frames`.
 * - A separate vector of non-owning pointers to channel buffers is also maintained. In this vector, each index
 *       corresponds to a channel, providing a mapping from the channel index to a pointer to the start of that
 *       channel's buffer within the source buffer. This is called the access buffer (m_accessBuffer).
 * - The purpose of the access buffer is to provide channel-wise access to buffers within the source buffer, so
 *       it's `m_accessBuffer[channelIdx][frameIdx]` instead of `m_sourceBuffer[channelIdx * frames + frameIdx]`.
 *       This is very important since many APIs dealing with planar audio expect it in this `float**` 2D array form.
 * - Groups have no effect on the audio data layout in the source/access buffers and are merely a layer built on top.
 *       Conveniently, if you take `m_accessBuffer` and offset it by `channelIndex`, you get another `float**`
 *       starting at that channel. This what the `float**` buffer stored in each ChannelGroup is.
 *
 * Naming notes:
 * - When this class is used in an instrument track or mixer channel, its channels could be referred to
 *       as "track channels" or "internal channels", since they are equivalent to the "track channels" used
 *       in other DAWs such as REAPER.
 * - When this class is used in an audio processor or audio plugin, its channels could be referred to
 *       as "processor channels" or "plugin channels".
 */
class LMMS_EXPORT AudioBuffer : public AudioBufferData
{
public:
	using ChannelFlags = std::bitset<MaxChannelsPerAudioBuffer>;

	//! Non-owning collection of audio channels + metadata
	class ChannelGroup
	{
	public:
		ChannelGroup() = default;
		ChannelGroup(float** buffers, ch_cnt_t channels)
			: m_buffers{buffers}
			, m_channels{channels}
		{}

		auto buffers() const -> const float* const* { return m_buffers; }
		auto buffers() -> float** { return m_buffers; }

		auto buffer(ch_cnt_t channel) const -> const float*
		{
			assert(channel < m_channels);
			return m_buffers[channel];
		}

		auto buffer(ch_cnt_t channel) -> float*
		{
			assert(channel < m_channels);
			return m_buffers[channel];
		}

		auto channels() const -> ch_cnt_t { return m_channels; }

		void setBuffers(float** newBuffers) { m_buffers = newBuffers; }
		void setChannels(ch_cnt_t channels) { m_channels = channels; }

		// TODO: Future additions: Group names, type (main/aux), speaker arrangements (for surround sound), ...

	private:
		/**
		 * Provides access to individual channel buffers.
		 * [channel index][frame index]
		 */
		float** m_buffers = nullptr;

		//! Number of channels in `m_buffers`
		ch_cnt_t m_channels = 0;
	};

	//! Creates an empty AudioBuffer with no channels or frames
	AudioBuffer()
		: AudioBufferData{}
	{
		initAudioBuffer();
	}

	AudioBuffer(const AudioBuffer&) = delete;
	AudioBuffer(AudioBuffer&&) noexcept = default;
	auto operator=(const AudioBuffer&) -> AudioBuffer& = delete;
	auto operator=(AudioBuffer&&) noexcept -> AudioBuffer& = default;

	/**
	 * Creates an AudioBuffer with one channel group.
	 * Silence tracking is enabled or disabled depending on the auto-quit setting.
	 *
	 * @see AudioBufferData constructors for the parameters
	 */
	template<typename... Args>
	explicit AudioBuffer(Args&&... args)
		: AudioBufferData{std::forward<Args>(args)...}
	{
		initAudioBuffer();
	}

	//! @returns current number of channel groups
	auto groupCount() const -> group_cnt_t { return static_cast<group_cnt_t>(m_groups.size()); }

	auto group(group_cnt_t index) const -> const ChannelGroup& { return m_groups[index]; }
	auto group(group_cnt_t index) -> ChannelGroup& { return m_groups[index]; }

	//! @returns the buffers of the given channel group
	auto groupBuffers(group_cnt_t index) const -> PlanarBufferView<const float>
	{
		assert(index < groupCount());
		const ChannelGroup& g = m_groups[index];
		return {g.buffers(), g.channels(), m_frames};
	}

	//! @returns the buffers of the given channel group
	auto groupBuffers(group_cnt_t index) -> PlanarBufferView<float>
	{
		assert(index < groupCount());
		ChannelGroup& g = m_groups[index];
		return {g.buffers(), g.channels(), m_frames};
	}

	/**
	 * @brief Adds a new channel group at the end of the list.
	 *
	 * If the memory resource is `SharedMemoryResource`, all buffers (source, channels,
	 * and interleaved) will be reallocated. The number of bytes allocated will be
	 * `allocationSize(frames(), totalChannels() + channels, hasInterleavedBuffer())`.
	 *
	 * @param channels how many channels the new group should have
	 * @returns the newly created group, or nullptr upon failure
	 */
	auto addGroup(ch_cnt_t channels) -> ChannelGroup*;

	/**
	 * @brief Changes the channel grouping without changing the channel count.
	 *        Does not reallocate any buffers.
	 *
	 * @param groups the new group count
	 * @param groupVisitor called for each new group, passed the index and group reference, and is
	 *                     expected to return the channel count for that group. The visitor may
	 *                     also set the group's metadata.
	 */
	template<class F>
	void setGroups(group_cnt_t groups, F&& groupVisitor)
	{
		static_assert(std::is_invocable_r_v<ch_cnt_t, F, group_cnt_t, ChannelGroup&>,
			"groupVisitor is passed the group index + group reference and must return the group's channel count");

		m_groups.clear();
		ch_cnt_t ch = 0;
		for (group_cnt_t idx = 0; idx < groups; ++idx)
		{
			auto& group = m_groups.emplace_back();

			const auto channels = groupVisitor(idx, group);
			if (channels == 0) { throw std::runtime_error{"group cannot have zero channels"}; }

			group.setBuffers(&m_accessBuffer[ch]);
			group.setChannels(channels);

			ch += channels;
			if (ch > this->channels())
			{
				throw std::runtime_error{"sum of group channel counts exceeds total channels"};
			}
		}
	}

	/**
	 * Channels which are known to be quiet, AKA the silence status.
	 * 1 = channel is known to be silent
	 * 0 = channel is assumed to be non-silent (or, when silence tracking
	 *     is enabled, *known* to be non-silent)
	 *
	 * NOTE: If any channel buffers are used and their data modified outside of this class,
	 *       their silence flags will be invalidated until `updateSilenceFlags()` is called.
	 *       Therefore, calling code must be careful to always keep the silence flags up-to-date.
	 */
	auto silenceFlags() const -> const ChannelFlags& { return m_silenceFlags; }

	//! Forcibly pessimizes silence tracking for a specific channel
	void assumeNonSilent(ch_cnt_t channel) { m_silenceFlags[channel] = false; }

	/**
	 * When silence tracking is enabled, channels will be checked for silence whenever their data may
	 * have changed, so it'll always be known whether they are silent or non-silent. There is a performance cost
	 * to this, but it is likely worth it since this information allows many effects to be put to sleep
	 * when their inputs are silent ("auto-quit"). When a channel is known to be silent, it also
	 * enables optimizations in buffer sanitization, buffer zeroing, and finding the absolute peak sample value.
	 *
	 * When silence tracking is disabled, channels are not checked for silence, so a silence flag may be
	 * unset despite the channel being silent. Non-silence must be assumed whenever the silence status is not
	 * known, so the optimizations which silent buffers allow will not be possible as often.
	 */
	void enableSilenceTracking(bool enabled);
	auto silenceTrackingEnabled() const -> bool { return m_silenceTrackingEnabled; }

	//! Mixes the silence flags of the other `AudioBuffer` with this `AudioBuffer`
	void mixSilenceFlags(const AudioBuffer& other);

	/**
	 * Checks whether any of the selected channels are non-silent (has a signal).
	 *
	 * If silence tracking is disabled, all channels that aren't marked
	 * as silent are assumed to be non-silent.
	 *
	 * A processor could check for a signal present at any of its inputs by
	 * calling this method selecting all of the track channels that are routed
	 * to at least one of its inputs.
	 *
	 * @param channels channels to check for a signal; 1 = selected, 0 = ignore
	 */
	auto hasSignal(const ChannelFlags& channels) const -> bool;

	//! Checks whether any channel is non-silent (has a signal). @see hasSignal
	auto hasAnySignal() const -> bool;

	/**
	 * @brief Sanitizes @a channels of any Inf/NaN by replacing them with zeros.
	 *
	 * If any invalid values are detected within a channel, the entire channel buffer is silenced, and its
	 * internal silence flag is set.
	 *
	 * @param channels channels to sanitize; 1 = selected, 0 = skip
	 * @param upperBound any channel indexes at or above this are skipped
	 *
	 * @returns true if sanitization occurred
	 */
	auto sanitize(const ChannelFlags& channels, ch_cnt_t upperBound = MaxChannelsPerAudioBuffer) -> bool;

	//! Sanitizes all channels. @see sanitize
	//! @returns true if sanitization occurred
	auto sanitizeAll() -> bool;

	/**
	 * @brief Updates the silence status of the given channels, up to the upperBound index.
	 *
	 * @param channels channels to update; 1 = selected, 0 = skip
	 * @param upperBound any channel indexes at or above this are skipped
	 * @returns true if all selected channels were silent
	 */
	auto updateSilenceFlags(const ChannelFlags& channels, ch_cnt_t upperBound = MaxChannelsPerAudioBuffer) -> bool;

	//! Updates the silence status of all channels. @see updateSilenceFlags
	auto updateAllSilenceFlags() -> bool;

	/**
	 * @brief Silences (zeroes) the given channels
	 *
	 * @param channels channels to silence; 1 = selected, 0 = skip
	 * @param upperBound any channel indexes at or above this are skipped
	 */
	void silenceChannels(const ChannelFlags& channels, ch_cnt_t upperBound = MaxChannelsPerAudioBuffer);

	//! Silences (zeroes) all channels. @see silenceChannels
	void silenceAllChannels();

	//! @returns absolute peak sample value for the given channel
	auto absPeakValue(ch_cnt_t channel) const -> float;

private:
	void initAudioBuffer();

	/**
	 * Stores which channels are known to be quiet, AKA the silence status.
	 *
	 * This must always be kept in sync with the buffer data when enabled - at minimum
	 * avoiding any false positives where a channel is marked as "silent" when it isn't.
	 * Any channel bits at or above `totalChannels()` must always be marked silent.
	 *
	 * 1 = channel is known to be silent
	 * 0 = channel is assumed to be non-silent (or, when silence tracking
	 *     is enabled, *known* to be non-silent)
	 */
	ChannelFlags m_silenceFlags;

	bool m_silenceTrackingEnabled = false;

	//! Divides channels into arbitrary groups
	ArrayVector<ChannelGroup, MaxGroupsPerAudioBuffer> m_groups;
};

} // namespace lmms

#endif // LMMS_AUDIO_BUFFER_H
