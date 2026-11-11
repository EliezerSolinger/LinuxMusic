/*
 * AudioBuffer.cpp
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

#include "AudioBuffer.h"

#include <algorithm>
#include <cmath>

#include "ConfigManager.h"
#include "MixHelpers.h"
#include "SharedMemory.h"
#include "SharedMemoryPointer.h"

namespace lmms {

namespace {

//! @returns Bitset with all bits at or above `pos` set to `value` and the rest set to `!value`
template<bool value>
auto createMask(ch_cnt_t pos) noexcept -> AudioBuffer::ChannelFlags
{
	assert(pos <= MaxChannelsPerAudioBuffer);

	AudioBuffer::ChannelFlags mask;
	mask.set();

	if constexpr (value)
	{
		mask <<= pos;
	}
	else
	{
		mask >>= (MaxChannelsPerAudioBuffer - pos);
	}

	return mask;
}

//! Determines how many padding bytes need to be added for SIMD alignment
constexpr auto paddingBytesNeeded(std::size_t currentBytes) -> std::size_t
{
	constexpr auto mask = SimdAlignment - 1;

	const auto currentAlignment = currentBytes & mask;
	const auto nextMaxAlignment = SimdAlignment - currentAlignment;

	return nextMaxAlignment == SimdAlignment
		? 0 : nextMaxAlignment;
};

// Quick unit tests
static_assert(paddingBytesNeeded(0) == 0);
static_assert(paddingBytesNeeded(1) == SimdAlignment - 1);
static_assert(paddingBytesNeeded(7) == SimdAlignment - 7);
static_assert(paddingBytesNeeded(SimdAlignment) == 0);
static_assert(paddingBytesNeeded(SimdAlignment + 5) == SimdAlignment - 5);

} // namespace


AudioBufferData::AudioBufferData(f_cnt_t frames, ch_cnt_t accessChannels, ch_cnt_t sourceChannels, bool interleaved,
	std::pmr::memory_resource* sourceBufferResource, std::pmr::memory_resource* accessBufferResource)
	: m_sourceBufferResource{sourceBufferResource}
	, m_accessBufferResource{accessBufferResource}
{
	if (sourceChannels == DynamicChannelCount) { sourceChannels = accessChannels; }
	create(frames, accessChannels, sourceChannels, interleaved, sourceBufferResource, accessBufferResource);

	// The shared memory client side gets its channel mapping from the server side
	if (accessChannels > 0 && !m_ownership.testFlag(Ownership::SharedMemoryClient)) { resetAccessBuffer(); }
}

AudioBufferData::AudioBufferData(float* source, f_cnt_t frames, ch_cnt_t accessChannels,
	ch_cnt_t sourceChannels, float* interleaved, std::pmr::memory_resource* accessBufferResource)
	: m_sourceBuffer{source}
	, m_interleavedBuffer{interleaved}
	, m_frames{frames}
	, m_accessChannels{accessChannels}
	, m_sourceChannels{sourceChannels}
	, m_sourceBufferResource{nullptr}
	, m_accessBufferResource{accessBufferResource}
	, m_ownership{Ownership::External}
{
	assert(m_accessBufferResource != nullptr);
	if (dynamic_cast<SharedMemoryResource*>(m_accessBufferResource) != nullptr)
	{
		throw std::invalid_argument{
			"AudioBufferData: accessBufferResource cannot be a SharedMemoryResource"};
	}

	if (accessChannels > 0)
	{
		assert(source != nullptr);
		m_accessBuffer = static_cast<float**>(m_accessBufferResource->allocate(accessChannels * sizeof(float*)));
		new (m_accessBuffer) float*[accessChannels]();
	}

	resetAccessBuffer();
}

AudioBufferData::AudioBufferData(AudioBufferData& source, ch_cnt_t accessStart, ch_cnt_t accessChannels)
	: m_accessBuffer{source.m_accessBuffer + accessStart}
	, m_sourceBuffer{source.m_sourceBuffer}
	, m_interleavedBuffer{source.m_interleavedBuffer}
	, m_frames{source.frames()}
	, m_accessChannels{accessChannels}
	, m_sourceChannels{source.m_sourceChannels}
	, m_allocation{source.m_allocation}
	, m_allocationSize{source.m_allocationSize}
	, m_sourceBufferResource{source.m_sourceBufferResource}
	, m_accessBufferResource{source.m_accessBufferResource}
	, m_ownership{source.m_ownership | Ownership::View}
{
	if (accessStart + accessChannels > source.channels())
	{
		throw std::invalid_argument{"AudioBufferData: invalid access buffer subset"};
	}
}

AudioBufferData::AudioBufferData(AudioBufferData&& other) noexcept
	: m_accessBuffer{std::exchange(other.m_accessBuffer, nullptr)}
	, m_sourceBuffer{std::exchange(other.m_sourceBuffer, nullptr)}
	, m_interleavedBuffer{std::exchange(other.m_interleavedBuffer, nullptr)}
	, m_frames{std::exchange(other.m_frames, 0)}
	, m_accessChannels{std::exchange(other.m_accessChannels, 0)}
	, m_sourceChannels{std::exchange(other.m_sourceChannels, 0)}
	, m_allocation{std::exchange(other.m_allocation, nullptr)}
	, m_allocationSize{std::exchange(other.m_allocationSize, 0)}
	, m_sourceBufferResource{std::exchange(other.m_sourceBufferResource, nullptr)}
	, m_accessBufferResource{std::exchange(other.m_accessBufferResource, nullptr)}
	, m_ownership{std::exchange(other.m_ownership, Ownership::View)}
{
}

auto AudioBufferData::operator=(AudioBufferData&& other) noexcept -> AudioBufferData&
{
	if (&other != this)
	{
		m_accessBuffer = std::exchange(other.m_accessBuffer, nullptr);
		m_sourceBuffer = std::exchange(other.m_sourceBuffer, nullptr);
		m_interleavedBuffer = std::exchange(other.m_interleavedBuffer, nullptr);
		m_frames = std::exchange(other.m_frames, 0);
		m_accessChannels = std::exchange(other.m_accessChannels, 0);
		m_sourceChannels = std::exchange(other.m_sourceChannels, 0);
		m_allocation = std::exchange(other.m_allocation, nullptr);
		m_allocationSize = std::exchange(other.m_allocationSize, 0);
		m_sourceBufferResource = std::exchange(other.m_sourceBufferResource, nullptr);
		m_accessBufferResource = std::exchange(other.m_accessBufferResource, nullptr);
		m_ownership = std::exchange(other.m_ownership, Ownership::View);
	}
	return *this;
}

AudioBufferData::~AudioBufferData()
{
	destroy();
}

void AudioBufferData::create(f_cnt_t frames, ch_cnt_t accessChannels, ch_cnt_t sourceChannels, bool interleaved,
	std::pmr::memory_resource* newSourceBufferResource, std::pmr::memory_resource* newAccessBufferResource)
{
	if (isView())
	{
		// For simplicity's sake, let's prevent this until there's a use case for it
		throw std::logic_error{"AudioBufferData: non-owning to owning not allowed"};
	}

	if (frames == 0) { throw std::invalid_argument{"AudioBufferData: cannot allocate zero frames"}; }

	// Deallocate any old buffers
	destroy();

	// Update resources if needed
	if (newSourceBufferResource)
	{
		m_sourceBufferResource = newSourceBufferResource;
	}
	if (newAccessBufferResource)
	{
		if (dynamic_cast<SharedMemoryResource*>(newAccessBufferResource) != nullptr)
		{
			throw std::invalid_argument{
				"AudioBufferData: newAccessBufferResource cannot be a SharedMemoryResource"};
		}
		m_accessBufferResource = newAccessBufferResource;
	}
	assert(m_sourceBufferResource != nullptr);
	assert(m_accessBufferResource != nullptr);

	// Set ownership
	if (auto smr = dynamic_cast<SharedMemoryResource*>(m_sourceBufferResource))
	{
		m_ownership = smr->isServerSide()
			? Ownership::SharedMemoryServer
			: Ownership::SharedMemoryClient;
	}
	else
	{
		m_ownership = Ownership::Internal;
	}

	m_frames = frames;
	m_accessChannels = accessChannels;
	m_sourceChannels = sourceChannels;

	// If the source and access buffer resources are the same,
	// we can create everything in one allocation for better locality
	const auto singleAllocation = m_sourceBufferResource->is_equal(*m_accessBufferResource);

	// Bytes needed for source buffer or the single allocation buffer
	auto bytesNeeded = std::size_t{0};

	// Buffers will exist in memory in this order:
	//     [access buffer (or offsets)][padding][source buffer][padding][interleaved buffer]
	// Depending on the arguments to this method, not all of these will necessarily exist
	auto sourceBufferOffset = std::size_t{0};
	auto interleavedBufferOffset = std::size_t{0};

	// Planar access buffer
	if (accessChannels > 0)
	{
		// Space for planar access buffer
		if (usesSharedMemory())    { bytesNeeded += accessChannels * sizeof(PointerOffset); }
		else if (singleAllocation) { bytesNeeded += accessChannels * sizeof(float*); }
	}

	// Planar source buffer
	if (sourceChannels > 0)
	{
		// Ensure the source buffer is properly aligned
		bytesNeeded += paddingBytesNeeded(bytesNeeded);
		sourceBufferOffset = bytesNeeded;

		// Space for planar source buffer
		bytesNeeded += sourceChannels * frames * sizeof(float);
	}

	// Interleaved buffer
	if (interleaved)
	{
		// Ensure the interleaved buffer is properly aligned
		bytesNeeded += paddingBytesNeeded(bytesNeeded);
		interleavedBufferOffset = bytesNeeded;

		// Space for interleaved buffer
		bytesNeeded += 2 * frames * sizeof(float);
	}

	// Now we can allocate
	auto ptr = static_cast<std::byte*>(m_sourceBufferResource->allocate(bytesNeeded, SimdAlignment));
	m_allocation = ptr;
	m_allocationSize = bytesNeeded;

	// Initialize the source buffer
	if (sourceChannels > 0)
	{
		if (m_ownership.testFlag(Ownership::SharedMemoryClient))
		{
			// For the Client side of shared memory, don't direct initialize
			// so we don't overwrite the data from the Server side
			m_sourceBuffer = new (ptr + sourceBufferOffset) float[sourceChannels * frames];
		}
		else
		{
			// In all other cases, direct initialize (zeroed)
			m_sourceBuffer = new (ptr + sourceBufferOffset) float[sourceChannels * frames]();
		}
	}

	// Initialize (and possibly allocate) the access buffer
	if (accessChannels > 0)
	{
		if (singleAllocation)
		{
			// Access buffer has already been allocated
			assert(!usesSharedMemory());
			m_accessBuffer = reinterpret_cast<float**>(ptr);
		}
		else
		{
			// Access buffer is allocated separately
			m_accessBuffer = static_cast<float**>(m_accessBufferResource->allocate(accessChannels * sizeof(float*)));
		}

		// Explicitly start lifetime and direct-initialize
		new (m_accessBuffer) float*[accessChannels]();

		// If using shared memory, there are also pointer offsets to initialize
		if (m_ownership.testFlag(Ownership::SharedMemoryServer))
		{
			// Server side - direct initialize (zeroed)
			new (ptr) PointerOffset[accessChannels]();
		}
		else if (m_ownership.testFlag(Ownership::SharedMemoryClient))
		{
			// Client side - don't direct initialize so we don't overwrite the data
			new (ptr) PointerOffset[accessChannels];

			// Initialize the access buffer from whatever the server side set it to
			refreshSharedMemoryAccessBuffer();
		}
	}

	// Initialize interleaved buffer
	if (interleaved)
	{
		m_interleavedBuffer = new (ptr + interleavedBufferOffset) float[2 * frames]();
	}
}

void AudioBufferData::destroy()
{
	// Nothing was ever allocated (e.g. an empty AudioBufferData)
	if (!m_sourceBufferResource || !m_accessBufferResource)
	{
		m_accessBuffer = nullptr;
		m_sourceBuffer = nullptr;
		m_interleavedBuffer = nullptr;
		m_frames = 0;
		m_accessChannels = 0;
		m_sourceChannels = 0;
		m_allocation = nullptr;
		m_allocationSize = 0;
		return;
	}

	if (!isView())
	{
		// m_accessBuffer is owning only if the access buffer was allocated separately from the rest
		const bool singleAllocation = m_sourceBufferResource->is_equal(*m_accessBufferResource);
		if (m_accessBuffer && !singleAllocation)
		{
			m_accessBufferResource->deallocate(m_accessBuffer, m_accessChannels * sizeof(float*));
		}

		// Deallocate the rest
		if (m_allocation)
		{
			m_sourceBufferResource->deallocate(m_allocation, m_allocationSize);
		}
	}

	m_accessBuffer = nullptr;
	m_sourceBuffer = nullptr;
	m_interleavedBuffer = nullptr;
	m_frames = 0;
	m_accessChannels = 0;
	m_sourceChannels = 0;
	m_allocation = nullptr;
	m_allocationSize = 0;
}

void AudioBufferData::refreshSharedMemoryAccessBuffer()
{
	assert(m_accessBuffer != nullptr);
	assert(m_sourceBuffer != nullptr);

	if (!m_ownership.testFlag(Ownership::SharedMemoryClient))
	{
		throw std::logic_error{
			"AudioBufferData: Only the shared memory client side can refresh the access buffer mapping"};
	}

	auto offsets = static_cast<PointerOffset*>(m_allocation);
	assert(offsets != nullptr);
	for (ch_cnt_t ch = 0; ch < channels(); ++ch)
	{
		m_accessBuffer[ch] = toPointer<float>(offsets[ch]);
	}
}

void AudioBufferData::resetAccessBuffer(ch_cnt_t sourceStart)
{
	assert(m_accessBuffer != nullptr);
	assert(m_sourceBuffer != nullptr);

	if (m_ownership.testFlag(Ownership::SharedMemoryClient))
	{
		throw std::logic_error {
			"AudioBufferData: The shared memory client side cannot change the access buffer mapping"
		};
	}

	if (m_accessChannels > static_cast<int>(m_sourceChannels) - sourceStart)
	{
		throw std::logic_error {
			"AudioBufferData: There are not enough channel buffers in the source buffer "
			"to map each access buffer with"
		};
	}

	float* ptr = m_sourceBuffer + sourceStart * m_frames;
	ch_cnt_t channel = 0;
	while (channel < channels())
	{
		m_accessBuffer[channel] = ptr;

		ptr += m_frames;
		++channel;
	}

	if (m_ownership.testFlag(Ownership::SharedMemoryServer))
	{
		// Also update the pointer offsets in shared memory
		auto offsets = static_cast<PointerOffset*>(m_allocation);
		assert(offsets != nullptr);

		channel = 0;
		while (channel < channels())
		{
			assignPointer(offsets[channel], m_accessBuffer[channel]);
			++channel;
		}
	}
}

void AudioBufferData::mapAccessBuffer(ch_cnt_t channel, ch_cnt_t mappedTo)
{
	assert(m_accessBuffer != nullptr);
	assert(m_sourceBuffer != nullptr);
	assert(channel < m_accessChannels);
	assert(mappedTo < m_sourceChannels);

	if (m_ownership.testFlag(Ownership::SharedMemoryClient))
	{
		throw std::logic_error{
			"AudioBufferData: The shared memory client side cannot change the access buffer mapping"};
	}

	float* buffer = &m_sourceBuffer[mappedTo * m_frames];
	m_accessBuffer[channel] = buffer;

	if (m_ownership.testFlag(Ownership::SharedMemoryServer))
	{
		// Also update the pointer offset in shared memory
		auto offsets = static_cast<PointerOffset*>(m_allocation);
		assert(offsets != nullptr);
		assignPointer(offsets[channel], buffer);
	}
}

void AudioBufferData::mapAccessBuffer(ch_cnt_t channel, float* mappedTo)
{
	assert(m_accessBuffer != nullptr);
	assert(m_sourceBuffer != nullptr);
	assert(channel < channels());
	assert(mappedTo != nullptr);

	if (usesSharedMemory())
	{
		throw std::logic_error{
			"AudioBufferData: Shared memory doesn't support arbitrary access buffer mapping"};
	}

	m_accessBuffer[channel] = mappedTo;
}


auto AudioBuffer::addGroup(ch_cnt_t channels) -> ChannelGroup*
{
	if (m_groups.size() >= m_groups.capacity())
	{
		// Maximum groups reached
		return nullptr;
	}

	if (channels == 0)
	{
		// Invalid channel count for a group
		return nullptr;
	}

	const auto oldTotalChannels = this->channels();
	const auto newTotalChannels = static_cast<ch_cnt_t>(oldTotalChannels + channels);
	if (newTotalChannels > MaxChannelsPerAudioBuffer)
	{
		// Not enough room for requested channels
		return nullptr;
	}

	if (isView() || m_ownership.testFlag(Ownership::External))
	{
		// Non-owning buffers cannot grow
		return nullptr;
	}

	// Keep the existing audio, since reallocating starts from silence
	const bool interleaved = hasInterleavedBuffer();
	auto oldPlanar = std::vector<float>(static_cast<std::size_t>(oldTotalChannels) * m_frames);
	for (ch_cnt_t ch = 0; ch < oldTotalChannels; ++ch)
	{
		std::ranges::copy(buffer(ch), oldPlanar.begin() + ch * m_frames);
	}
	auto oldInterleaved = std::vector<float>{};
	if (interleaved) { oldInterleaved.assign(m_interleavedBuffer, m_interleavedBuffer + 2 * m_frames); }

	try
	{
		create(m_frames, newTotalChannels, newTotalChannels, interleaved);
		resetAccessBuffer();
	}
	catch (...)
	{
		return nullptr;
	}

	for (ch_cnt_t ch = 0; ch < oldTotalChannels; ++ch)
	{
		std::ranges::copy_n(oldPlanar.begin() + ch * m_frames, m_frames, buffer(ch).begin());
	}
	if (interleaved) { std::ranges::copy(oldInterleaved, m_interleavedBuffer); }

	// Fix group buffers
	ch_cnt_t channel = 0;
	for (ChannelGroup& group : m_groups)
	{
		group.setBuffers(&m_accessBuffer[channel]);
		channel += group.channels();
	}

	// Ensure the new channels (and all the higher, unused
	// channels) are set to "silent"
	m_silenceFlags |= createMask<true>(oldTotalChannels);

	// Append new group
	return &m_groups.emplace_back(&m_accessBuffer[oldTotalChannels], channels);
}

void AudioBuffer::enableSilenceTracking(bool enabled)
{
	const auto oldValue = m_silenceTrackingEnabled;
	m_silenceTrackingEnabled = enabled;
	if (!oldValue && enabled)
	{
		updateAllSilenceFlags();
	}
}

void AudioBuffer::mixSilenceFlags(const AudioBuffer& other)
{
	m_silenceFlags &= other.silenceFlags();
}

auto AudioBuffer::hasSignal(const ChannelFlags& channels) const -> bool
{
	auto nonSilent = ~m_silenceFlags;
	nonSilent &= channels;
	return nonSilent.any();
}

auto AudioBuffer::hasAnySignal() const -> bool
{
	// This is possible due to the invariant that any channel bits
	// at or above `channels()` must always be marked silent
	return !m_silenceFlags.all();
}

auto AudioBuffer::sanitize(const ChannelFlags& channels, ch_cnt_t upperBound) -> bool
{
	bool changesMade = false;

	const auto totalChannels = std::min(upperBound, this->channels());
	for (ch_cnt_t ch = 0; ch < totalChannels; ++ch)
	{
		if (channels[ch])
		{
			// This channel needs to be sanitized
			if (std::ranges::any_of(buffer(ch), [](auto val) { return !std::isfinite(val); }))
			{
				// Inf/NaN detected and buffer cleared
				std::ranges::fill(buffer(ch), 0.f);
				m_silenceFlags[ch] = true;
				changesMade = true;
			}
		}
	}

	if (changesMade && hasInterleavedBuffer() && (channels[0] || channels[1]))
	{
		// Keep the temporary interleaved buffer in sync
		toInterleaved(groupBuffers(0), interleavedBuffer());
	}

	return changesMade;
}

auto AudioBuffer::sanitizeAll() -> bool
{
	bool changesMade = false;
	for (ch_cnt_t ch = 0; ch < channels(); ++ch)
	{
		if (std::ranges::any_of(buffer(ch), [](auto val) { return !std::isfinite(val); }))
		{
			// Inf/NaN detected and buffer cleared
			std::ranges::fill(buffer(ch), 0.f);
			m_silenceFlags[ch] = true;
			changesMade = true;
		}
	}

	if (changesMade && hasInterleavedBuffer())
	{
		// Keep the temporary interleaved buffer in sync
		toInterleaved(groupBuffers(0), interleavedBuffer());
	}

	return changesMade;
}

auto AudioBuffer::updateSilenceFlags(const ChannelFlags& channels, ch_cnt_t upperBound) -> bool
{
	assert(upperBound <= MaxChannelsPerAudioBuffer);

	// Invariant: Any channel bits at or above `channels()` must be marked silent
	assert((~m_silenceFlags & createMask<true>(this->channels())).none());

	// If no channels are selected, return true (all selected channels are silent)
	if (channels.none()) { return true; }

	const auto totalChannels = std::min(upperBound, this->channels());

	if (!m_silenceTrackingEnabled)
	{
		// Mark specified channels (up to the upper bound) as non-silent
		auto temp = ~channels;
		temp |= createMask<true>(totalChannels);
		m_silenceFlags &= temp;
		return false;
	}

	bool allQuiet = true;
	for (ch_cnt_t ch = 0; ch < totalChannels; ++ch)
	{
		if (channels[ch])
		{
			// This channel needs to be updated
			const auto quiet = MixHelpers::isSilent(buffer(ch));

			m_silenceFlags[ch] = quiet;
			allQuiet = allQuiet && quiet;
		}
	}

	return allQuiet;
}

auto AudioBuffer::updateAllSilenceFlags() -> bool
{
	// Invariant: Any channel bits at or above `channels()` must be marked silent
	assert((~m_silenceFlags & createMask<true>(channels())).none());

	// If there are no channels, return true (all channels are silent)
	if (channels() == 0) { return true; }

	if (!m_silenceTrackingEnabled)
	{
		// Mark all channels below `channels()` as non-silent
		m_silenceFlags &= createMask<true>(channels());
		return false;
	}

	bool allQuiet = true;
	for (ch_cnt_t ch = 0; ch < channels(); ++ch)
	{
		const auto quiet = MixHelpers::isSilent(buffer(ch));

		m_silenceFlags[ch] = quiet;
		allQuiet = allQuiet && quiet;
	}

	return allQuiet;
}

void AudioBuffer::silenceChannels(const ChannelFlags& channels, ch_cnt_t upperBound)
{
	auto needSilenced = ~m_silenceFlags;
	needSilenced &= channels;

	const auto totalChannels = std::min(upperBound, this->channels());
	for (ch_cnt_t ch = 0; ch < totalChannels; ++ch)
	{
		if (needSilenced[ch])
		{
			std::ranges::fill(buffer(ch), 0.f);
		}
	}

	if (hasInterleavedBuffer() && (needSilenced[0] || needSilenced[1]))
	{
		// Keep the temporary interleaved buffer in sync
		toInterleaved(groupBuffers(0), interleavedBuffer());
	}

	m_silenceFlags |= channels;
}

void AudioBuffer::silenceAllChannels()
{
	// Only this buffer's channels: a view shares its source buffer with other views
	for (ch_cnt_t ch = 0; ch < channels(); ++ch)
	{
		std::ranges::fill(buffer(ch), 0.f);
	}
	if (m_interleavedBuffer) { std::fill_n(m_interleavedBuffer, 2 * m_frames, 0.f); }

	m_silenceFlags.set();
}

auto AudioBuffer::absPeakValue(ch_cnt_t channel) const -> float
{
	if (m_silenceFlags[channel])
	{
		// Skip calculation if channel is already known to be silent
		return 0;
	}

	return std::abs(std::ranges::max(buffer(channel), {}, static_cast<float(&)(float)>(std::abs)));
}

void AudioBuffer::initAudioBuffer()
{
	m_silenceTrackingEnabled = ConfigManager::inst()->value("ui", "disableautoquit", "1").toInt() == 0;

	if (channels() == 0)
	{
		m_silenceFlags.set();
		return;
	}

	if (channels() > MaxChannelsPerAudioBuffer)
	{
		throw std::invalid_argument{"AudioBuffer: Too many channels"};
	}

	if (isView() || m_ownership.testFlag(Ownership::External) || m_ownership.testFlag(Ownership::SharedMemoryClient))
	{
		// The contents are someone else's and unknown, so the channels may have a signal.
		// All the higher, unused channels are "silent".
		m_silenceFlags = createMask<true>(channels());
	}
	else
	{
		// Freshly allocated buffers are zeroed
		m_silenceFlags.set();
	}

	// Add single group
	if (hasPlanarBuffers())
	{
		m_groups.emplace_back(&m_accessBuffer[0], channels());
	}
}

} // namespace lmms
