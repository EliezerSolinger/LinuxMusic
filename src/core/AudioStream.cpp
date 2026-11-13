/*
 * AudioStream.cpp
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

#include "AudioStream.h"

#include <algorithm>
#include <chrono>

#include <QFile>
#include <sndfile.h>

#include "AudioEngine.h"
#include "Engine.h"
#include "PathUtil.h"
#include "Song.h"

namespace lmms
{

namespace
{

std::atomic<std::int64_t> s_missCount = 0;
std::atomic<AudioStream::MissPolicy> s_missPolicy = AudioStream::MissPolicy::Automatic;

std::atomic<std::int64_t> s_periods = 0;
std::atomic<double> s_periodsPerSecond = 0.0;

auto currentPeriod() -> std::int64_t
{
	// (sequentially consistent with the block pointers: see release())
	return s_periods.load(std::memory_order_seq_cst);
}

//! Whether the song is being rendered offline: the audio thread may then wait for the disk
auto renderingOffline() -> bool
{
	switch (s_missPolicy.load(std::memory_order_relaxed))
	{
	case AudioStream::MissPolicy::ReadAtOnce: return true;
	case AudioStream::MissPolicy::PlaySilence: return false;
	case AudioStream::MissPolicy::Automatic: break;
	}
	const auto* engine = Engine::audioEngine();
	if (engine && engine->renderOnly()) { return true; }
	const auto* song = Engine::getSong();
	return song && song->isExporting();
}

constexpr auto MaxOpenFiles = std::size_t{64};

} // namespace




AudioStream::AudioStream(const QString& file, const QString& path, f_cnt_t frames, int sampleRate, int channels)
	: m_file{file}
	, m_path{path}
	, m_frames{frames}
	, m_sampleRate{sampleRate}
	, m_channels{channels}
	, m_blockCount{(frames + BlockFrames - 1) / BlockFrames}
	, m_blocks{std::make_unique<std::atomic<Block*>[]>(m_blockCount)}
	, m_lastUsed{std::make_unique<std::atomic<std::int64_t>[]>(m_blockCount)}
	, m_missed{std::make_unique<std::atomic<bool>[]>(m_blockCount)}
{
	for (std::size_t block = 0; block < m_blockCount; ++block)
	{
		m_blocks[block].store(nullptr);
		m_lastUsed[block].store(0);
		m_missed[block].store(false);
	}
}




AudioStream::~AudioStream()
{
	// The last owner is gone, so no period can be playing this stream any more
	for (std::size_t block = 0; block < m_blockCount; ++block) { delete m_blocks[block].exchange(nullptr); }
	closeFile();
}




auto AudioStream::open(const QString& file) -> std::shared_ptr<AudioStream>
{
	if (file.isEmpty()) { return nullptr; }
	const auto path = PathUtil::toAbsolute(file);
	auto info = SF_INFO{};
	auto* sndFile = sf_open(QFile::encodeName(path).constData(), SFM_READ, &info);
	if (!sndFile) { return nullptr; }
	sf_close(sndFile);
	if (!info.seekable || info.frames <= 0 || info.channels <= 0 || info.samplerate <= 0) { return nullptr; }
	auto stream = std::shared_ptr<AudioStream>{
		new AudioStream{PathUtil::toShortestRelative(file), path, static_cast<f_cnt_t>(info.frames), info.samplerate, info.channels}};
	DiskReader::instance().add(stream);
	return stream;
}




auto AudioStream::openFile() -> bool
{
	auto info = SF_INFO{};
	m_sndFile = sf_open(QFile::encodeName(m_path).constData(), SFM_READ, &info);
	if (!m_sndFile) { return false; }
	if (info.channels != m_channels) { closeFile(); return false; } // (the file was replaced)
	return true;
}




void AudioStream::closeFile()
{
	const auto lock = std::lock_guard{m_fileMutex};
	if (m_sndFile) { sf_close(m_sndFile); }
	m_sndFile = nullptr;
}




auto AudioStream::isFileOpen() const -> bool
{
	const auto lock = std::lock_guard{m_fileMutex};
	return m_sndFile != nullptr;
}




auto AudioStream::blockAt(f_cnt_t index) const -> const SampleFrame*
{
	if (index >= m_frames) { return nullptr; }
	const auto block = static_cast<std::size_t>(index / BlockFrames);
	if (const auto* found = m_blocks[block].load(std::memory_order_seq_cst))
	{
		m_lastUsed[block].store(currentPeriod(), std::memory_order_relaxed);
		return found->frames.data();
	}

	if (renderingOffline())
	{
		// Offline nothing waits for the audio: read it now
		if (const_cast<AudioStream*>(this)->load(block))
		{
			if (const auto* found = m_blocks[block].load(std::memory_order_seq_cst))
			{
				m_lastUsed[block].store(currentPeriod(), std::memory_order_relaxed);
				return found->frames.data();
			}
		}
		return nullptr;
	}

	// Live: silence for now, and the disk thread is told
	if (!m_missed[block].exchange(true, std::memory_order_relaxed)) { s_missCount.fetch_add(1, std::memory_order_relaxed); }
	m_anyMissed.store(true, std::memory_order_release);
	DiskReader::wake();
	return nullptr;
}




auto AudioStream::isLoaded(std::size_t block) const -> bool
{
	return block < m_blockCount && m_blocks[block].load(std::memory_order_acquire) != nullptr;
}




auto AudioStream::takeMisses() -> std::vector<std::size_t>
{
	auto missed = std::vector<std::size_t>{};
	if (!m_anyMissed.exchange(false, std::memory_order_acquire)) { return missed; }
	for (std::size_t block = 0; block < m_blockCount; ++block)
	{
		if (m_missed[block].exchange(false, std::memory_order_relaxed)) { missed.push_back(block); }
	}
	return missed;
}




auto AudioStream::load(std::size_t block) -> bool
{
	if (block >= m_blockCount) { return false; }
	const auto lock = std::lock_guard{m_fileMutex};
	if (m_blocks[block].load(std::memory_order_acquire)) { return true; }
	if (m_failed) { return false; }
	if (!m_sndFile && !openFile()) { m_failed = true; return false; }

	const auto first = static_cast<sf_count_t>(block * BlockFrames);
	const auto count = static_cast<sf_count_t>(std::min<f_cnt_t>(BlockFrames, m_frames - block * BlockFrames));
	if (sf_seek(m_sndFile, first, SEEK_SET) < 0) { m_failed = true; return false; }
	m_readBuffer.resize(static_cast<std::size_t>(count * m_channels));
	const auto read = std::max<sf_count_t>(sf_readf_float(m_sndFile, m_readBuffer.data(), count), 0);

	auto loaded = std::make_unique<Block>();
	loaded->frames.resize(static_cast<std::size_t>(count)); // (frames the file no longer has stay silent)
	for (sf_count_t frame = 0; frame < read; ++frame)
	{
		const auto* at = m_readBuffer.data() + frame * m_channels;
		// As the decoder does: mono on both sides, the first two channels of more
		loaded->frames[frame] = m_channels == 1 ? SampleFrame{at[0], at[0]} : SampleFrame{at[0], at[1]};
	}
	m_lastUsed[block].store(currentPeriod(), std::memory_order_relaxed);
	m_blocks[block].store(loaded.release(), std::memory_order_seq_cst);
	return true;
}




void AudioStream::release(std::size_t block)
{
	if (block >= m_blockCount) { return; }
	auto* released = [&] {
		const auto lock = std::lock_guard{m_fileMutex};
		return m_blocks[block].exchange(nullptr, std::memory_order_seq_cst);
	}();
	if (!released) { return; }
	// Read after the exchange: a period that may have taken the block has not ended yet when the counter still
	// shows it, so the block is freed once the counter is past it
	DiskReader::instance().retire({released, [](void* data) { delete static_cast<Block*>(data); }, currentPeriod()});
}




auto AudioStream::loadedBlocks() const -> std::vector<std::size_t>
{
	auto loaded = std::vector<std::size_t>{};
	for (std::size_t block = 0; block < m_blockCount; ++block)
	{
		if (m_blocks[block].load(std::memory_order_relaxed)) { loaded.push_back(block); }
	}
	return loaded;
}




auto AudioStream::lastUsed(std::size_t block) const -> std::int64_t
{
	return block < m_blockCount ? m_lastUsed[block].load(std::memory_order_relaxed) : 0;
}




auto AudioStream::missCount() -> std::int64_t
{
	return s_missCount.load(std::memory_order_relaxed);
}




void AudioStream::periodRendered(double periodsPerSecond)
{
	s_periodsPerSecond.store(periodsPerSecond, std::memory_order_relaxed);
	s_periods.fetch_add(1, std::memory_order_seq_cst);
}




auto AudioStream::periods() -> std::int64_t
{
	return currentPeriod();
}




auto AudioStream::periodsPerSecond() -> double
{
	return s_periodsPerSecond.load(std::memory_order_relaxed);
}




void AudioStream::setMissPolicy(MissPolicy policy)
{
	s_missPolicy.store(policy, std::memory_order_relaxed);
}




std::atomic<bool> DiskReader::s_wake = false;




DiskReader::DiskReader()
	: m_thread{[this] { run(); }}
{
}




DiskReader::~DiskReader()
{
	shutdown();
}




auto DiskReader::instance() -> DiskReader&
{
	static auto s_instance = DiskReader{};
	return s_instance;
}




void DiskReader::setDemand(std::vector<Demand> demand)
{
	{
		const auto lock = std::lock_guard{m_mutex};
		m_demand = std::move(demand);
		m_demandChanged = true;
	}
	m_wake.notify_one();
}




void DiskReader::add(const std::shared_ptr<AudioStream>& stream)
{
	const auto lock = std::lock_guard{m_mutex};
	std::erase_if(m_streams, [](const auto& weak) { return weak.expired(); });
	m_streams.push_back(stream);
}




void DiskReader::wake()
{
	// (no notification from the audio thread: the disk thread looks every few milliseconds)
	s_wake.store(true, std::memory_order_release);
}




auto DiskReader::demandLoaded() -> bool
{
	const auto demand = [&] {
		const auto lock = std::lock_guard{m_mutex};
		return m_demand;
	}();
	for (const auto& wanted : demand)
	{
		for (auto block = wanted.first / AudioStream::BlockFrames; block <= wanted.last / AudioStream::BlockFrames;
			++block)
		{
			if (!wanted.stream->isLoaded(block)) { return false; }
		}
	}
	return true;
}




auto DiskReader::waitUntilLoaded(int timeoutMs) -> bool
{
	const auto until = std::chrono::steady_clock::now() + std::chrono::milliseconds{timeoutMs};
	while (!demandLoaded())
	{
		if (std::chrono::steady_clock::now() >= until) { return false; }
		std::this_thread::sleep_for(std::chrono::milliseconds{2});
	}
	return true;
}




void DiskReader::retire(Retired retired)
{
	m_retired.push_back(retired);
}




void DiskReader::freeRetired()
{
	const auto period = currentPeriod();
	std::erase_if(m_retired, [period](const Retired& retired) {
		if (period <= retired.period) { return false; }
		retired.free(retired.block);
		return true;
	});
}




void DiskReader::shutdown()
{
	{
		const auto lock = std::lock_guard{m_mutex};
		if (!m_running) { return; }
		m_running = false;
		m_demand.clear();
	}
	m_wake.notify_one();
	if (m_thread.joinable()) { m_thread.join(); }
	// Nothing plays any more
	for (const auto& retired : m_retired) { retired.free(retired.block); }
	m_retired.clear();
}




void DiskReader::run()
{
	auto streams = std::vector<std::shared_ptr<AudioStream>>{};
	auto demand = std::vector<Demand>{};

	const auto readMisses = [&] {
		if (!s_wake.exchange(false, std::memory_order_acquire)) { return; }
		for (const auto& stream : streams)
		{
			for (const auto block : stream->takeMisses()) { stream->load(block); }
		}
	};

	while (true)
	{
		{
			auto lock = std::unique_lock{m_mutex};
			m_wake.wait_for(lock, std::chrono::milliseconds{4}, [this] { return !m_running || m_demandChanged; });
			if (!m_running) { break; }
			demand = m_demand;
			m_demandChanged = false;
			streams.clear();
			std::erase_if(m_streams, [](const auto& weak) { return weak.expired(); });
			for (const auto& weak : m_streams)
			{
				if (auto stream = weak.lock()) { streams.push_back(std::move(stream)); }
			}
		}
		// (the demand's streams first, so the files of what plays stay open)
		s_wake.store(true);
		readMisses();

		// What is about to play, nearest first, minding misses (they are later than the demand's next block)
		auto changed = false;
		for (const auto& wanted : demand)
		{
			for (auto block = wanted.first / AudioStream::BlockFrames;
				block <= wanted.last / AudioStream::BlockFrames && !changed; ++block)
			{
				if (!wanted.stream->isLoaded(block)) { wanted.stream->load(block); }
				readMisses();
				const auto lock = std::lock_guard{m_mutex};
				changed = m_demandChanged || !m_running;
			}
			if (changed) { break; }
		}
		if (changed) { continue; }

		// The rest is released: blocks out of the demand that have not played for a while
		const auto keepPeriods = std::max<std::int64_t>(2, static_cast<std::int64_t>(2.0 * AudioStream::periodsPerSecond()));
		const auto period = currentPeriod();
		auto openFiles = std::vector<AudioStream*>{};
		for (const auto& stream : streams)
		{
			auto wantedBlocks = std::vector<bool>(stream->blockCount(), false);
			auto inDemand = false;
			for (const auto& wanted : demand)
			{
				if (wanted.stream != stream) { continue; }
				inDemand = true;
				for (auto block = wanted.first / AudioStream::BlockFrames;
					block <= wanted.last / AudioStream::BlockFrames && block < wantedBlocks.size(); ++block)
				{
					wantedBlocks[block] = true;
				}
			}
			for (const auto block : stream->loadedBlocks())
			{
				if (!wantedBlocks[block] && period - stream->lastUsed(block) > keepPeriods) { stream->release(block); }
			}
			if (!inDemand && stream->isFileOpen()) { openFiles.push_back(stream.get()); }
		}
		// Files out of the demand are closed past a few, so many takes do not use up the open files
		for (std::size_t index = MaxOpenFiles; index < openFiles.size(); ++index) { openFiles[index]->closeFile(); }

		freeRetired();
	}
	streams.clear();
}

} // namespace lmms
