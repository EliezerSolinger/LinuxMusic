/*
 * LatencyCalibration.cpp
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

#include "LatencyCalibration.h"

#include <algorithm>
#include <cmath>

#include "AudioBusHandle.h"
#include "AudioDevice.h"
#include "AudioEngine.h"
#include "Engine.h"
#include "PlayHandle.h"
#include "SampleFrame.h"

namespace lmms
{

struct LatencyCalibration::Shared
{
	std::vector<float> heard; //!< the loudest input channel, frame by frame from the pulse on
	std::size_t position = 0; //!< (the audio thread's)
	std::atomic<bool> done = false;
	f_cnt_t period = 0;
	f_cnt_t inputLatency = 0;
	f_cnt_t outputLatency = 0;
};




namespace
{

//! Plays the pulse in its first period, then copies the input until the buffer is full
class CalibrationHandle : public PlayHandle
{
public:
	explicit CalibrationHandle(std::shared_ptr<LatencyCalibration::Shared> shared)
		: PlayHandle{Type::CalibrationHandle}
		, m_shared{std::move(shared)}
		, m_bus{new AudioBusHandle{"Latency calibration", false}}
	{
		setAudioBusHandle(m_bus);
	}

	~CalibrationHandle() override { delete m_bus; }

	void play(std::span<SampleFrame> buffer) override
	{
		auto& shared = *m_shared;
		if (shared.done.load(std::memory_order_relaxed)) { return; }
		if (m_first && !buffer.empty())
		{
			buffer[0] = SampleFrame{LatencyCalibration::PulseLevel, LatencyCalibration::PulseLevel};
			m_first = false;
		}
		// (without an input, the time goes by all the same: the result says nothing came back)
		const auto capture = Engine::audioEngine()->capture();
		const auto available = capture.channels() > 0 ? capture.frames() : buffer.size();
		const auto frames = std::min<std::size_t>(available, shared.heard.size() - shared.position);
		for (std::size_t frame = 0; frame < frames && capture.channels() > 0; ++frame)
		{
			auto loudest = 0.f;
			for (ch_cnt_t channel = 0; channel < capture.channels(); ++channel)
			{
				loudest = std::max(loudest, std::abs(capture.bufferPtr(channel)[frame]));
			}
			shared.heard[shared.position + frame] = loudest;
		}
		shared.position += frames;
		if (shared.position >= shared.heard.size()) { shared.done.store(true, std::memory_order_release); }
	}

	bool isFinished() const override { return m_shared->done.load(std::memory_order_acquire); }
	bool isFromTrack(const Track*) const override { return false; }

private:
	std::shared_ptr<LatencyCalibration::Shared> m_shared;
	AudioBusHandle* m_bus;
	bool m_first = true;
};

} // namespace




LatencyCalibration::LatencyCalibration()
	: m_shared{std::make_shared<Shared>()}
{
	auto* engine = Engine::audioEngine();
	m_shared->heard.assign(static_cast<std::size_t>(ListenSeconds * engine->outputSampleRate()), 0.f);
}




LatencyCalibration::~LatencyCalibration()
{
	// (the handle keeps the shared state while the engine has it)
	if (m_started) { Engine::audioEngine()->setCaptureEnabled(m_captureWasEnabled); }
}




void LatencyCalibration::start()
{
	if (m_started) { return; }
	m_started = true;
	auto* engine = Engine::audioEngine();
	m_shared->period = engine->framesPerPeriod();
	if (const auto* device = engine->audioDev())
	{
		m_shared->inputLatency = device->inputLatency();
		m_shared->outputLatency = device->outputLatency();
	}
	m_captureWasEnabled = engine->captureEnabled();
	engine->setCaptureEnabled(true);
	engine->addPlayHandle(new CalibrationHandle{m_shared});
}




auto LatencyCalibration::isDone() const -> bool
{
	return m_shared->done.load(std::memory_order_acquire);
}




auto LatencyCalibration::result() const -> Result
{
	auto result = Result{};
	if (!isDone()) { return result; }
	const auto& heard = m_shared->heard;
	const auto peak = std::max_element(heard.begin(), heard.end());
	result.peak = *peak;
	result.found = result.peak >= MinimumPeak;
	if (!result.found) { return result; }
	result.delay = static_cast<f_cnt_t>(peak - heard.begin());
	// What the recorder already counts: the devices' latencies and the engine's two periods (the output is handed
	// to the device a period late, and the input taken a period late)
	const auto expected = static_cast<long long>(m_shared->inputLatency) + m_shared->outputLatency + 2LL * m_shared->period;
	result.offset = static_cast<long long>(result.delay) - expected;
	return result;
}

} // namespace lmms
