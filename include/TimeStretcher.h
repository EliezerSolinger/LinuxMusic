/*
 * TimeStretcher.h - renders a recording to a new timing (a change of tempo), keeping its pitch
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

#ifndef LMMS_TIME_STRETCHER_H
#define LMMS_TIME_STRETCHER_H

#include <atomic>
#include <functional>
#include <QString>

#include "SampleClip.h"
#include "LmmsTypes.h"
#include "lmms_export.h"

namespace lmms
{

/**
 * Renders a recording to a new timing (docs/plano-gravacao.md, "Time-stretch renderizado").
 *
 * The timings pair ticks with frames: @p from says which frame of the source was played at which tick, @p to which
 * frame of the rendering must sound at that tick. Each point becomes a key frame, so a tempo map with ramps is
 * followed. With Rubber Band (R3 engine, offline, two passes) the pitch is kept; built without it, the source is
 * resampled piece by piece, which changes the pitch (canKeepPitch() says which).
 *
 * The source is read and the rendering written in blocks: a long recording does not have to fit in memory. The
 * rendering is a 32-bit float WAV at @p rate, exactly as long as the last point of @p to; it is written to a
 * temporary file and renamed when complete, so a cancelled or failed render leaves nothing behind.
 *
 * Any thread; one render per call.
 */
class LMMS_EXPORT TimeStretcher
{
public:
	using Timing = SampleClip::Timing;

	enum class Status
	{
		Done,
		Cancelled,
		Failed
	};

	struct Result
	{
		Status status = Status::Failed;
		QString error; //!< what failed (Status::Failed)
	};

	//! @returns whether a render keeps the pitch (built with Rubber Band)
	static auto canKeepPitch() -> bool;

	/**
	 * Renders @p source, played at @p from, to @p target at @p to. The frames of @p from are the source's, those of
	 * @p to are at @p rate. @p progress (0 to 1) is called from this thread now and then; @p cancel stops the render.
	 */
	static auto render(const QString& source, const QString& target, const Timing& from, const Timing& to,
		sample_rate_t rate, const std::atomic<bool>& cancel, const std::function<void(double)>& progress) -> Result;
};

} // namespace lmms

#endif // LMMS_TIME_STRETCHER_H
