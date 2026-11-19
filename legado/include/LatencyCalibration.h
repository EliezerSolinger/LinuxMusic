/*
 * LatencyCalibration.h
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

#ifndef LMMS_LATENCY_CALIBRATION_H
#define LMMS_LATENCY_CALIBRATION_H

#include <atomic>
#include <memory>
#include <vector>

#include "LmmsTypes.h"
#include "lmms_export.h"

namespace lmms
{

/**
 * The loopback calibration (docs/plano-gravacao.md, "Calibração por loopback"): with a cable from an output to an
 * input, a pulse is played and recorded back; where it arrives, less what the devices say their latencies are, is
 * the offset recordings need to land exactly where they were played ("recording/latencyoffset").
 *
 * start() plays the pulse (a maximum length sequence, found back by correlation) and listens for a second and a half, on the audio thread (nothing is allocated there);
 * result() tells, once isDone().
 */
class LMMS_EXPORT LatencyCalibration
{
public:
	static constexpr double ListenSeconds = 1.5;
	static constexpr float PulseLevel = 0.5f;
	static constexpr float MinimumPeak = 0.02f; //!< below: nothing came back (no cable)

	struct Result
	{
		bool found = false; //!< the pulse came back
		f_cnt_t delay = 0; //!< frames from the pulse to its return
		long long offset = 0; //!< the calibration: the delay less the devices' latencies and the engine's period
		float peak = 0.f; //!< how loud it came back
	};

	LatencyCalibration();
	~LatencyCalibration();

	//! Plays the pulse and listens (the device's input is opened for it)
	void start();
	auto isDone() const -> bool;
	auto result() const -> Result;

	struct Shared; //!< (between it and the audio thread's handle)

private:
	std::shared_ptr<Shared> m_shared;
	bool m_captureWasEnabled = false;
	bool m_started = false;
};

} // namespace lmms

#endif // LMMS_LATENCY_CALIBRATION_H
