/*
 * AudioStream.h
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

#ifndef LMMS_AUDIO_STREAM_H
#define LMMS_AUDIO_STREAM_H

#include <atomic>
#include <condition_variable>
#include <cstdint>
#include <memory>
#include <mutex>
#include <thread>
#include <vector>

#include <QString>

#include "LmmsTypes.h"
#include "SampleFrame.h"
#include "lmms_export.h"

struct sf_private_tag;

namespace lmms
{

/**
 * An audio file played from disk, without loading it (docs/plano-gravacao.md, "Mídia do projeto"): a long
 * recording is never in memory as a whole. The file is cut in blocks of BlockFrames frames; the disk thread
 * (DiskReader) reads ahead the blocks that are about to play, and the audio thread takes them without waiting.
 *
 * A block the audio thread needs but does not find is asked for and plays as silence meanwhile (a "miss", counted).
 * When the song is rendered offline (export), the audio thread reads a missing block itself instead: nothing is
 * left out of an export.
 *
 * Blocks no longer needed are released by the disk thread, and freed once the audio thread has finished the
 * period it might have been reading them in (AudioEngine::periodsRendered()).
 */
class LMMS_EXPORT AudioStream
{
public:
	static constexpr f_cnt_t BlockFrames = 32768;

	//! Opens @p file (as a project names it: relative, "local:"...) for streaming: null if libsndfile cannot read
	//! it or seek in it
	static auto open(const QString& file) -> std::shared_ptr<AudioStream>;

	~AudioStream();
	AudioStream(const AudioStream&) = delete;
	auto operator=(const AudioStream&) -> AudioStream& = delete;

	//! The file as the project names it (as SampleBuffer::audioFile())
	auto file() const -> const QString& { return m_file; }
	//! Where it is
	auto path() const -> const QString& { return m_path; }
	auto frames() const -> f_cnt_t { return m_frames; }
	auto sampleRate() const -> int { return m_sampleRate; }
	auto blockCount() const -> std::size_t { return m_blockCount; }

	/**
	 * Audio thread: the block holding frame @p index (its first frame is index - index % BlockFrames), or null if
	 * it is not in memory (it is then asked for; offline, it is read at once). Never blocks while playing live.
	 */
	auto blockAt(f_cnt_t index) const -> const SampleFrame*;

	//! Whether block @p block is in memory
	auto isLoaded(std::size_t block) const -> bool;

	//! The blocks the audio thread missed since the last call (each once). Disk thread.
	auto takeMisses() -> std::vector<std::size_t>;

	//! Reads block @p block if it is not in memory. Disk thread (or the audio thread offline). @returns false if
	//! the file could not be read
	auto load(std::size_t block) -> bool;

	//! Releases block @p block: it is freed once no period can still be reading it. Disk thread.
	void release(std::size_t block);

	//! The blocks in memory
	auto loadedBlocks() const -> std::vector<std::size_t>;

	//! The last period the audio thread used block @p block in
	auto lastUsed(std::size_t block) const -> std::int64_t;

	//! Closes the file (it is opened again when a block is read). Disk thread.
	void closeFile();
	auto isFileOpen() const -> bool;

	//! The blocks the audio thread missed, in all streams, since the program started
	static auto missCount() -> std::int64_t;

	//! Whether a missing block is read by the audio thread itself: when rendering offline (Automatic), always, or
	//! never (tests of either way)
	enum class MissPolicy { Automatic, ReadAtOnce, PlaySilence };
	static void setMissPolicy(MissPolicy policy);

	//! Audio thread: a period has ended (AudioEngine), at @p periodsPerSecond periods a second. The disk thread
	//! frees a released block once the period that might have read it has ended, and keeps what played recently.
	static void periodRendered(double periodsPerSecond);
	//! The periods ended so far
	static auto periods() -> std::int64_t;
	static auto periodsPerSecond() -> double;

private:
	AudioStream(const QString& file, const QString& path, f_cnt_t frames, int sampleRate, int channels);
	auto openFile() -> bool;

	struct Block
	{
		std::vector<SampleFrame> frames;
	};

	const QString m_file;
	const QString m_path;
	const f_cnt_t m_frames;
	const int m_sampleRate;
	const int m_channels;
	const std::size_t m_blockCount;

	std::unique_ptr<std::atomic<Block*>[]> m_blocks;
	mutable std::unique_ptr<std::atomic<std::int64_t>[]> m_lastUsed;
	mutable std::unique_ptr<std::atomic<bool>[]> m_missed;
	mutable std::atomic<bool> m_anyMissed = false;
	bool m_failed = false;

	mutable std::mutex m_fileMutex; //!< the file and loading (never taken by the audio thread while playing live)
	sf_private_tag* m_sndFile = nullptr;
	std::vector<float> m_readBuffer;

	friend class DiskReader;
};




/**
 * The disk thread (docs/plano-gravacao.md, "Leitura em streaming"): it keeps in memory the blocks of the streams
 * that are about to play (the demand, given by the main thread from the song's position), reads at once the blocks
 * the audio thread missed, releases the others, and frees released blocks when it is safe.
 */
class LMMS_EXPORT DiskReader
{
public:
	//! Seconds read ahead of the song's position
	static constexpr double ReadAheadSeconds = 6.0;

	//! The frames of a stream that are about to play, nearest first
	struct Demand
	{
		std::shared_ptr<AudioStream> stream;
		f_cnt_t first = 0;
		f_cnt_t last = 0; //!< inclusive
	};

	static auto instance() -> DiskReader&;

	//! Main thread: what is about to play (replaces the former demand)
	void setDemand(std::vector<Demand> demand);
	//! A stream to look after (its misses are read even with no demand)
	void add(const std::shared_ptr<AudioStream>& stream);
	//! Audio thread: a block was missed
	static void wake();

	//! Waits until the demand is in memory, for at most @p timeoutMs (tests, and before an offline render)
	auto waitUntilLoaded(int timeoutMs) -> bool;
	//! Whether every demanded block is in memory
	auto demandLoaded() -> bool;

	//! Stops the thread and frees every block (when quitting)
	void shutdown();

	~DiskReader();

private:
	DiskReader();
	void run();
	void freeRetired();

	struct Retired
	{
		void* block;
		void (*free)(void*);
		std::int64_t period;
	};
	void retire(Retired retired);

	std::mutex m_mutex;
	std::condition_variable m_wake;
	std::vector<Demand> m_demand;
	std::vector<std::weak_ptr<AudioStream>> m_streams;
	bool m_demandChanged = false;
	bool m_running = true;
	std::vector<Retired> m_retired; //!< (disk thread only)
	std::thread m_thread;

	static std::atomic<bool> s_wake;

	friend class AudioStream;
};

} // namespace lmms

#endif // LMMS_AUDIO_STREAM_H
