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
		, segmentStarts{256}
		, interleaved(static_cast<std::size_t>(period) * channels)
		, diskBuffer(8192 * static_cast<std::size_t>(channels))
	{
	}

	//! Where a segment begins: after how many frames of the ring, and on which tick
	struct SegmentStart
	{
		std::int64_t frame = 0;
		std::int64_t tick = 0;
	};

	Track* track;
	AudioInput input;
	QString path; //!< the first segment's file; the others' are named after it
	int channels;
	SpscRing<float> ring; //!< interleaved samples, from the audio thread to the disk thread
	SpscRing<SegmentStart> segmentStarts; //!< from the audio thread to the disk thread

	// The audio thread's
	std::vector<float> interleaved; //!< one period, interleaved
	bool started = false; //!< whether the first frame was seen
	bool writing = false; //!< whether the frames go to a segment, or wait for its first tick
	double expectedTick = 0; //!< where the next frame belongs if the song goes straight on
	std::int64_t segmentTick = 0;
	std::int64_t framesQueued = 0; //!< frames given to the ring so far, silence included
	std::int64_t pendingSilence = 0; //!< frames that did not fit in the ring: silence, written before what follows
	std::atomic<std::int64_t> lostFrames = 0;

	// The disk thread's
	std::vector<float> diskBuffer;
	SNDFILE* file = nullptr;
	std::int64_t framesWritten = 0; //!< frames taken from the ring so far
	std::vector<Segment> segments;
	bool firstSegmentOpen = false;
	std::atomic<bool> diskFailed = false;
	SegmentStart nextStart;
	bool hasNextStart = false;
	sample_rate_t sampleRate = 0;
};




struct AudioRecorder::Session
{
	std::vector<std::unique_ptr<TakeState>> takes;
	f_cnt_t latency = 0;
	f_cnt_t period = 0;
	std::int64_t fromTick = 0;

	//! Where each of the latest periods started on the song's timeline, and its tempo, by period number (the audio
	//! thread's): positions are in ticks, so that a change of tempo is no jump
	struct PeriodStart
	{
		double tick = 0;
		double framesPerTick = 1;
	};
	std::vector<PeriodStart> periodStarts;
	std::int64_t periods = 0;
};




AudioRecorder::AudioRecorder() = default;




AudioRecorder::~AudioRecorder()
{
	if (m_ownedSession) { stop(); }
}




namespace
{

auto openFile(const QString& path, sample_rate_t sampleRate, int channels) -> SNDFILE*
{
	QDir{}.mkpath(QFileInfo{path}.absolutePath());
	auto info = SF_INFO{};
	info.samplerate = static_cast<int>(sampleRate);
	info.channels = channels;
	info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
	return sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
}

//! The file of segment @p index (from 0) of a take: the take's own for the first, then "<take>-2.wav", ...
auto segmentPath(const QString& path, std::size_t index) -> QString
{
	if (index == 0) { return path; }
	const auto info = QFileInfo{path};
	return QString{"%1/%2-%3.%4"}.arg(info.absolutePath(), info.completeBaseName()).arg(index + 1)
		.arg(info.suffix());
}

} // namespace




auto AudioRecorder::start(const std::vector<Request>& requests, f_cnt_t latency, f_cnt_t period,
	sample_rate_t sampleRate, std::int64_t fromTick) -> QString
{
	if (m_ownedSession) { return QObject::tr("Already recording"); }
	if (requests.empty()) { return QObject::tr("No track to record"); }

	auto session = std::make_unique<Session>();
	session->latency = latency;
	session->period = period;
	session->fromTick = std::max<std::int64_t>(0, fromTick);
	// Enough periods to look back through the latency
	session->periodStarts.assign(static_cast<std::size_t>(latency / std::max<f_cnt_t>(period, 1)) + 8, {});

	for (const auto& request : requests)
	{
		const int channels = std::clamp(request.input.channelCount, 1, 2);
		auto take = std::make_unique<TakeState>(request, channels,
			static_cast<std::size_t>(sampleRate) * RingSeconds, period);
		take->sampleRate = sampleRate;

		// The first file is made now, so that a folder that cannot be written is reported before recording
		take->file = openFile(request.path, sampleRate, channels);
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
		if (!state->diskFailed && state->file && state->pendingSilence > 0)
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
				if (!state->segments.empty()) { state->segments.back().frames += frames; }
				left -= frames;
			}
		}
		if (state->file) { sf_close(state->file); }
		state->file = nullptr;

		// A first file nothing went into is not kept
		if (!state->firstSegmentOpen) { QFile::remove(state->path); }

		auto take = Take{};
		take.track = state->track;
		take.segments = state->segments;
		take.lostFrames = state->lostFrames.load();
		take.diskFailed = state->diskFailed.load();
		takes.push_back(take);
	}
	m_ownedSession.reset();
	return takes;
}




void AudioRecorder::process(PlanarBufferView<const float> capture, double periodStartTick, double framesPerTick)
{
	auto* session = m_session.load(std::memory_order_acquire);
	if (!session) { return; }

	const auto period = static_cast<std::int64_t>(session->period);
	const auto frames = static_cast<std::int64_t>(capture.frames() > 0 ? capture.frames() : session->period);
	auto& starts = session->periodStarts;
	const auto history = static_cast<std::int64_t>(starts.size());

	// This period's start, for the frames that will be heard against it
	const auto now = session->periods++;
	starts[static_cast<std::size_t>(now % history)] = {periodStartTick, framesPerTick};
	const auto firstStart = now < history ? starts[0] : Session::PeriodStart{}; // the first period, while remembered

	// The song position (in ticks) heard when the capture's frame @p index was played, and the tempo then (see the
	// header): the capture of period n holds the device frames of the period before; minus the latency, they were
	// played against the output of the period after the one rendered then
	struct Heard
	{
		double tick;
		double framesPerTick;
	};
	const auto heardAt = [&](std::int64_t index) {
		const auto device = (now - 1) * period + index - static_cast<std::int64_t>(session->latency);
		const auto outputPeriod = (device >= 0 ? device / period : -((-device + period - 1) / period)) - 1;
		const auto offset = static_cast<double>(device - (outputPeriod + 1) * period);
		if (outputPeriod >= 0 && outputPeriod > now - history)
		{
			const auto& start = starts[static_cast<std::size_t>(outputPeriod % history)];
			return Heard{start.tick + offset / start.framesPerTick, start.framesPerTick};
		}
		// Before recording began: as if the song had gone straight on to its first period
		return Heard{firstStart.tick + (static_cast<double>(outputPeriod * period) + offset) / firstStart.framesPerTick,
			firstStart.framesPerTick};
	};

	for (auto& takePointer : session->takes)
	{
		auto& take = *takePointer;
		const auto channels = static_cast<std::size_t>(take.channels);
		std::int64_t count = 0; // frames of this period for the ring

		const auto flush = [&] {
			// Silence for frames that did not fit before, in their place; then the period's frames. Whole frames
			// only, so that the ring always holds whole frames.
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
			take.framesQueued += count;
			count = 0;
		};

		for (std::int64_t frame = 0; frame < frames; ++frame)
		{
			const auto heard = heardAt(frame);
			const auto halfFrame = 0.5 / heard.framesPerTick; // half a frame, in ticks

			// The song went on from elsewhere (or this is the first frame): a new segment, on the tick it went on in.
			// A loop goes back into its first tick, not to its start (the song keeps the fraction of a tick it was
			// at): the part of the tick before is silence, so that every pass begins on the loop's first tick.
			// (A change of tempo is no jump, though the song may then move by less than a tick: it reckons the part of
			// the tick it is in again, at the new tempo. A loop or a move of the play head goes a tick or more.)
			if (!take.started || std::abs(heard.tick - take.expectedTick) >= 1.0)
			{
				const auto earliest = take.started ? std::int64_t{0} : session->fromTick;
				take.segmentTick = std::max(earliest, static_cast<std::int64_t>(std::floor(heard.tick + 1e-9)));
				take.started = true;
				if (take.writing)
				{
					flush();
					take.writing = false;
				}
			}
			take.expectedTick = heard.tick + 1.0 / heard.framesPerTick;

			if (!take.writing)
			{
				if (heard.tick < static_cast<double>(take.segmentTick) - halfFrame) { continue; }
				// The segment's first tick: the disk thread starts its file at this frame
				flush();
				// (the frames queued count those that will be silence)
				take.segmentStarts.write(nullptr, 1, TakeState::SegmentStart{take.framesQueued, take.segmentTick});
				take.writing = true;

				// The part of the first tick the song skipped: silence, before the segment's first frame
				const auto skipped = std::llround((heard.tick - static_cast<double>(take.segmentTick))
					* heard.framesPerTick);
				if (skipped > 0)
				{
					take.pendingSilence += skipped;
					take.framesQueued += skipped;
				}
			}

			for (std::size_t channel = 0; channel < channels; ++channel)
			{
				const auto source = take.input.firstChannel + static_cast<int>(channel);
				take.interleaved[static_cast<std::size_t>(count) * channels + channel] = source < capture.channels()
					? capture.bufferPtr(static_cast<ch_cnt_t>(source))[frame]
					: 0.f;
			}
			++count;
		}
		flush();
	}
}




auto AudioRecorder::drain(TakeState& take) -> bool
{
	const auto channels = static_cast<std::size_t>(take.channels);

	// A segment starting here: the file so far is done, the segment's begins
	if (!take.hasNextStart) { take.hasNextStart = take.segmentStarts.read(&take.nextStart, 1) == 1; }
	if (take.hasNextStart && take.nextStart.frame <= take.framesWritten)
	{
		if (take.firstSegmentOpen)
		{
			sf_close(take.file);
			take.file = openFile(segmentPath(take.path, take.segments.size()), take.sampleRate, take.channels);
			if (!take.file)
			{
				take.diskFailed = true;
				return false;
			}
		}
		take.firstSegmentOpen = true;
		take.segments.push_back({segmentPath(take.path, take.segments.size()), take.nextStart.tick, 0});
		take.hasNextStart = false;
		return true;
	}

	// The frames up to the next segment's start, or all there are
	auto available = static_cast<std::int64_t>(take.ring.readSpace() / channels);
	if (take.hasNextStart) { available = std::min(available, take.nextStart.frame - take.framesWritten); }
	available = std::min(available, static_cast<std::int64_t>(take.diskBuffer.size() / channels));
	if (available <= 0) { return false; }
	const auto count = take.ring.read(take.diskBuffer.data(), static_cast<std::size_t>(available) * channels);
	const auto frames = static_cast<sf_count_t>(count / channels);
	take.framesWritten += frames;
	if (take.segments.empty()) { return true; } // before the first segment nothing is recorded (cannot happen)
	if (sf_writef_float(take.file, take.diskBuffer.data(), frames) != frames)
	{
		take.diskFailed = true;
		return false;
	}
	take.segments.back().frames += frames;
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
				if (!take->diskFailed && take->file) { sf_command(take->file, SFC_UPDATE_HEADER_NOW, nullptr, 0); }
			}
			lastHeaderUpdate = now;
		}

		if (quit) { break; }
	}
}

} // namespace lmms
