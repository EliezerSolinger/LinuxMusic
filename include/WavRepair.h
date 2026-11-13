/*
 * WavRepair.h
 *
 * This file is part of LinuxMusic, a fork of LMMS - https://lmms.io
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

#ifndef LMMS_WAV_REPAIR_H
#define LMMS_WAV_REPAIR_H

#include <cstdint>

#include <QString>

#include "lmms_export.h"

namespace lmms::WavRepair
{

/**
 * Makes the header of the WAV file @p path say all the whole frames the file holds: a file being written when the
 * program stopped has a header updated now and then (see AudioRecorder), which says less than was written.
 * @returns the frames the file holds, or -1 if it is not a WAV file that can be completed
 */
LMMS_EXPORT auto completeLength(const QString& path) -> std::int64_t;

} // namespace lmms::WavRepair

#endif // LMMS_WAV_REPAIR_H
