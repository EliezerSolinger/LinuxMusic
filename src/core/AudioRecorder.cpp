/*
 * AudioRecorder.cpp - records the audio input of armed tracks to disk, aligned with the song
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

#include "AudioRecorder.h"

#include <algorithm>
#include <chrono>
#include <cmath>

#include <QDir>
#include <QFile>
#include <QObject>
#include <QFileInfo>
#include <sndfile.h>

#include "SpscRing.h"

namespace lmms
{

namespace
{

//! How much a ring holds: the disk may fall this far behind before frames are lost
constexpr auto RingSeconds = 10;

//! How often the disk thread empties the rings
constexpr auto DiskInterval = std::chrono::milliseconds{20};

//! How often the files' headers are updated, so that they are valid up to there if the program stops
constexpr auto HeaderInterval = std::chrono::seconds{1};

} // namespace




struct AudioRecorder::TakeState
{
	TakeState(const Request& request, int channels, std::size_t ringFrames, f_cnt_t period)
		: track{request.track}
		, input{request.input}
		, path{request.path}
		, channels{channels}
		, ring{ringFrames * static_cast<std::size_t>(channels)}
		, interleaved(static_cast<std::size_t>(period) * channels)
		, diskBuffer(8192 * static_cast<std::size_t>(channels))
	{
	}

	Track* track;
	AudioInput input;
	QString path;
	int channels;
	SNDFILE* file = nullptr;
	SpscRing<float> ring; //!< interleaved samples, from the audio thread to the disk thread

	// The audio thread's
	std::vector<float> interleaved; //!< one period, interleaved
	bool started = false;
	std::int64_t startTick = 0;
	std::int64_t framesToSkip = 0; //!< frames before the take's first tick, not recorded
	std::int64_t pendingSilence = 0; //!< frames that did not fit in the ring: silence, written before what follows
	std::atomic<std::int64_t> lostFrames = 0;

	// The disk thread's
	std::vector<float> diskBuffer;
	std::int64_t framesWritten = 0;
	std::atomic<bool> diskFailed = false;
};




struct AudioRecorder::Session
{
	std::vector<std::unique_ptr<TakeState>> takes;
	f_cnt_t latency = 0;
	f_cnt_t period = 0;
	std::int64_t fromTick = 0;
};




AudioRecorder::AudioRecorder() = default;




AudioRecorder::~AudioRecorder()
{
	if (m_ownedSession) { stop(); }
}




auto AudioRecorder::start(const std::vector<Request>& requests, f_cnt_t latency, f_cnt_t period,
	sample_rate_t sampleRate, std::int64_t fromTick) -> QString
{
	if (m_ownedSession) { return QObject::tr("Already recording"); }
	if (requests.empty()) { return QObject::tr("No track to record"); }

	auto session = std::make_unique<Session>();
	session->latency = latency;
	session->period = period;
	session->fromTick = std::max<std::int64_t>(0, fromTick);

	for (const auto& request : requests)
	{
		const int channels = std::clamp(request.input.channelCount, 1, 2);
		auto take = std::make_unique<TakeState>(request, channels,
			static_cast<std::size_t>(sampleRate) * RingSeconds, period);

		QDir{}.mkpath(QFileInfo{request.path}.absolutePath());
		auto info = SF_INFO{};
		info.samplerate = static_cast<int>(sampleRate);
		info.channels = channels;
		info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
		take->file = sf_open(QFile::encodeName(request.path).constData(), SFM_WRITE, &info);
		if (!take->file)
		{
			const auto error = QObject::tr("Could not create %1: %2").arg(request.path, sf_strerror(nullptr));
			for (auto& made : session->takes)
			{
				sf_close(made->file);
				QFile::remove(made->path);
			}
			return error;
		}
		session->takes.push_back(std::move(take));
	}

	m_ownedSession = std::move(session);
	{
		const auto lock = std::lock_guard{m_diskMutex};
		m_diskQuit = false;
	}
	m_disk = std::thread{[this] { diskThread(); }};

	// From here on the audio thread records
	m_session.store(m_ownedSession.get(), std::memory_order_release);
	return {};
}




auto AudioRecorder::stop() -> std::vector<Take>
{
	if (!m_ownedSession) { return {}; }

	// The audio thread is not in process() (see the header): from its next period on it records nothing
	m_session.store(nullptr, std::memory_order_release);

	{
		const auto lock = std::lock_guard{m_diskMutex};
		m_diskQuit = true;
	}
	m_diskWake.notify_one();
	m_disk.join();

	auto takes = std::vector<Take>{};
	for (auto& state : m_ownedSession->takes)
	{
		// What is left: the ring (the disk thread emptied it before it quit, unless the disk failed) and the
		// frames that never fit, as silence at the end
		while (!state->diskFailed && drain(*state)) {}
		if (!state->diskFailed && state->pendingSilence > 0)
		{
			std::fill(state->diskBuffer.begin(), state->diskBuffer.end(), 0.f);
			const auto chunk = static_cast<std::int64_t>(state->diskBuffer.size()) / state->channels;
			for (auto left = state->pendingSilence; left > 0;)
			{
				const auto frames = std::min(left, chunk);
				if (sf_writef_float(state->file, state->diskBuffer.data(), frames) != frames)
				{
					state->diskFailed = true;
					break;
				}
				state->framesWritten += frames;
				left -= frames;
			}
		}
		sf_close(state->file);
		state->file = nullptr;

		auto take = Take{};
		take.track = state->track;
		take.path = state->path;
		take.started = state->started;
		take.startTick = state->startTick;
		take.frames = state->framesWritten;
		take.lostFrames = state->lostFrames.load();
		take.diskFailed = state->diskFailed.load();
		takes.push_back(take);
	}
	m_ownedSession.reset();
	return takes;
}




void AudioRecorder::process(PlanarBufferView<const float> capture, double periodStartFrame, double framesPerTick)
{
	auto* session = m_session.load(std::memory_order_acquire);
	if (!session) { return; }

	const auto frames = static_cast<std::int64_t>(capture.frames() > 0 ? capture.frames() : session->period);

	// The song frame of the capture's first frame (see the header)
	const double firstFrame = periodStartFrame - 2.0 * session->period - session->latency;

	for (auto& takePointer : session->takes)
	{
		auto& take = *takePointer;
		if (!take.started)
		{
			// The take begins on the first tick at or after its first frame, and not before the session's first tick
			take.startTick = std::max(session->fromTick,
				static_cast<std::int64_t>(std::ceil(firstFrame / framesPerTick)));
			take.framesToSkip = std::max<std::int64_t>(0, std::llround(take.startTick * framesPerTick - firstFrame));
			take.started = true;
		}

		const auto skipped = std::min(take.framesToSkip, frames);
		take.framesToSkip -= skipped;
		const auto count = frames - skipped;
		if (count == 0) { continue; }

		// The track's input channels, interleaved; a channel the device does not have is silence
		for (int channel = 0; channel < take.channels; ++channel)
		{
			const auto source = take.input.firstChannel + channel;
			const float* samples = source < capture.channels() ? capture.bufferPtr(source) + skipped : nullptr;
			for (std::int64_t frame = 0; frame < count; ++frame)
			{
				take.interleaved[frame * take.channels + channel] = samples ? samples[frame] : 0.f;
			}
		}

		// Silence for frames that did not fit before, in their place; then this period. Whole frames only, so that
		// the ring always holds whole frames.
		const auto channels = static_cast<std::size_t>(take.channels);
		const auto freeFrames = [&] { return static_cast<std::int64_t>(take.ring.writeSpace() / channels); };
		if (take.pendingSilence > 0)
		{
			const auto silence = std::min(take.pendingSilence, freeFrames());
			take.ring.write(nullptr, static_cast<std::size_t>(silence) * channels);
			take.pendingSilence -= silence;
		}
		auto written = std::int64_t{0};
		if (take.pendingSilence == 0)
		{
			written = std::min(count, freeFrames());
			take.ring.write(take.interleaved.data(), static_cast<std::size_t>(written) * channels);
		}
		if (const auto missing = count - written; missing > 0)
		{
			take.pendingSilence += missing;
			take.lostFrames.fetch_add(missing, std::memory_order_relaxed);
		}
	}
}




auto AudioRecorder::drain(TakeState& take) -> bool
{
	const auto channels = static_cast<std::size_t>(take.channels);
	const auto available = take.ring.readSpace() / channels * channels;
	if (available == 0) { return false; }
	const auto count = take.ring.read(take.diskBuffer.data(), std::min(available, take.diskBuffer.size()));
	const auto frames = static_cast<sf_count_t>(count / channels);
	if (sf_writef_float(take.file, take.diskBuffer.data(), frames) != frames)
	{
		take.diskFailed = true;
		return false;
	}
	take.framesWritten += frames;
	return true;
}




void AudioRecorder::diskThread()
{
	auto* session = m_ownedSession.get();
	auto lastHeaderUpdate = std::chrono::steady_clock::now();
	while (true)
	{
		bool quit;
		{
			auto lock = std::unique_lock{m_diskMutex};
			m_diskWake.wait_for(lock, DiskInterval, [this] { return m_diskQuit; });
			quit = m_diskQuit;
		}

		for (auto& take : session->takes)
		{
			if (take->diskFailed)
			{
				// Keep the ring empty, so that the audio thread does not count the frames as lost twice
				auto discard = std::vector<float>(take->ring.readSpace());
				take->ring.read(discard.data(), discard.size());
				continue;
			}
			while (drain(*take)) {}
		}

		const auto now = std::chrono::steady_clock::now();
		if (now - lastHeaderUpdate >= HeaderInterval)
		{
			for (auto& take : session->takes)
			{
				if (!take->diskFailed) { sf_command(take->file, SFC_UPDATE_HEADER_NOW, nullptr, 0); }
			}
			lastHeaderUpdate = now;
		}

		if (quit) { break; }
	}
}

} // namespace lmms
