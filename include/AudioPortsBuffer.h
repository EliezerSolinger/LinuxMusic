/*
 * AudioPortsBuffer.h
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

#ifndef LMMS_AUDIO_PORTS_BUFFER_H
#define LMMS_AUDIO_PORTS_BUFFER_H

#include "AudioBuffer.h"
#include "AudioPortsSettings.h"

namespace lmms
{

class AudioPortsBuffer
{
public:
	// Buffer data
	auto inputs() const -> PlanarBufferView<const float>;
	auto outputs() -> PlanarBufferView<float>;
	auto inputsOutputs() -> PlanarBufferView<float>;
	auto legacyInterleaved() -> InterleavedBufferView<float>;

	//! When fully in-place, inputsOutputs() can be used
	auto isFullyInplace() const -> bool;

	//! Whenever pin connections change, need to update the access buffers
	void assignBuffers(const AudioPortsModel& model);

private:
	//std::array<const float*, MaxChannelsPerAudioBuffer> m_mappedInputs;
	//std::array<float*, MaxChannelsPerAudioBuffer> m_mappedOutputs;
	AudioBuffer m_inputs;
	AudioBuffer m_outputs;
	// put InPlaceMapping here
	bool m_fullyInPlace = false; // true = inputsOutputs() can be used
	bool m_directRoutingCompatible = false; // false = must write to buffers - no "direct routing" tricks allowed
	bool m_legacyInterleaved = false;
	bool m_mappingChanged = false;
};

} // namespace lmms

#endif // LMMS_AUDIO_PORTS_BUFFER_H
