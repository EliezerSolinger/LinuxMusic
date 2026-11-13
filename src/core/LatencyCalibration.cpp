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
	std::vector<float> pulse; //!< a maximum length sequence: only it correlates with itself
	std::size_t played = 0; //!< (the audio thread's)
	std::vector<float> heard; //!< the inputs summed, frame by frame from the pulse on
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
		// The sequence, from the first frame of the first period on
		for (std::size_t frame = 0; frame < buffer.size() && shared.played < shared.pulse.size(); ++frame)
		{
			const auto value = shared.pulse[shared.played++];
			buffer[frame] = SampleFrame{value, value};
		}
		// (without an input, the time goes by all the same: the result says nothing came back)
		const auto capture = Engine::audioEngine()->capture();
		const auto available = capture.channels() > 0 ? capture.frames() : buffer.size();
		const auto frames = std::min<std::size_t>(available, shared.heard.size() - shared.position);
		for (std::size_t frame = 0; frame < frames && capture.channels() > 0; ++frame)
		{
			auto sum = 0.f;
			for (ch_cnt_t channel = 0; channel < capture.channels(); ++channel) { sum += capture.bufferPtr(channel)[frame]; }
			shared.heard[shared.position + frame] = sum;
		}
		shared.position += frames;
		if (shared.position >= shared.heard.size()) { shared.done.store(true, std::memory_order_release); }
	}

	bool isFinished() const override { return m_shared->done.load(std::memory_order_acquire); }
	bool isFromTrack(const Track*) const override { return false; }

private:
	std::shared_ptr<LatencyCalibration::Shared> m_shared;
	AudioBusHandle* m_bus;
};

} // namespace




LatencyCalibration::LatencyCalibration()
	: m_shared{std::make_shared<Shared>()}
{
	auto* engine = Engine::audioEngine();
	m_shared->heard.assign(static_cast<std::size_t>(ListenSeconds * engine->outputSampleRate()), 0.f);
	// A maximum length sequence of 127 (x^7 + x^6 + 1)
	auto state = 0x7fu;
	for (int index = 0; index < 127; ++index)
	{
		const auto bit = ((state >> 6) ^ (state >> 5)) & 1u;
		m_shared->pulse.push_back((state & 1u) ? PulseLevel : -PulseLevel);
		state = ((state << 1) | bit) & 0x7fu;
	}
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
	// Where the sequence came back: the best correlation with it (what else is heard does not correlate). The first
	// period heard was captured before it played (the input is taken a period late), and it cannot come back in less
	const auto& heard = m_shared->heard;
	const auto& pulse = m_shared->pulse;
	if (heard.size() <= pulse.size()) { return result; }
	auto energy = 0.0;
	for (const auto value : pulse) { energy += static_cast<double>(value) * value; }
	auto best = 0.0;
	auto bestAt = std::size_t{0};
	for (auto at = static_cast<std::size_t>(m_shared->period); at + pulse.size() <= heard.size(); ++at)
	{
		auto correlation = 0.0;
		for (std::size_t index = 0; index < pulse.size(); ++index)
		{
			correlation += static_cast<double>(heard[at + index]) * pulse[index];
		}
		if (correlation > best) { best = correlation; bestAt = at; }
	}
	// How loud it came back, against how loud it was played
	result.peak = static_cast<float>(best / energy * PulseLevel);
	result.found = result.peak >= MinimumPeak;
	if (!result.found) { return result; }
	result.delay = static_cast<f_cnt_t>(bestAt);
	// What the recorder already counts: the devices' latencies and the engine's two periods (the output is handed
	// to the device a period late, and the input taken a period late)
	const auto expected = static_cast<long long>(m_shared->inputLatency) + m_shared->outputLatency + 2LL * m_shared->period;
	result.offset = static_cast<long long>(result.delay) - expected;
	return result;
}

} // namespace lmms
