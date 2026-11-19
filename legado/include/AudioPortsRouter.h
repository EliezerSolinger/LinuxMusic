/*
 * AudioPortsRouter.h - routes audio between track channels and a processor's channels
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

#include "lmms_export.h"

namespace lmms
{

class AudioBuffer;
class AudioPortsModel;

//! Pin connector routing between the track channels (of an instrument track or mixer channel) and the
//! input and output channels of an audio processor, following an AudioPortsModel's pin matrices.
namespace AudioPortsRouter
{

/**
 * Track channels --> processor inputs.
 *
 * Each processor input receives the sum of the track channels connected to it, or silence if none are.
 * The silence flags of @p processorIn are updated accordingly.
 */
LMMS_EXPORT void send(const AudioPortsModel& model, const AudioBuffer& track, AudioBuffer& processorIn);

/**
 * Processor outputs --> track channels.
 *
 * Each track channel connected to at least one processor output is replaced by the sum of those outputs.
 * Track channels without any connected output pass through unchanged. The silence flags of the replaced
 * track channels are left marked as non-silent; the caller updates them after sanitizing.
 */
LMMS_EXPORT void receive(const AudioPortsModel& model, const AudioBuffer& processorOut, AudioBuffer& track);

} // namespace AudioPortsRouter

} // namespace lmms

#endif // LMMS_AUDIO_PORTS_ROUTER_H
