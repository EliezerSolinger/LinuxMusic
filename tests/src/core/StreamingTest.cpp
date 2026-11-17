/*
 * StreamingTest.cpp
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


#include <cmath>
#include <vector>

#include <QDomDocument>
#include <QObject>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QtTest>
#include <sndfile.h>

#include "AudioEngine.h"
#include "AudioStream.h"
#include "Engine.h"
#include "PatternStore.h"
#include "PeakCache.h"
#include "Sample.h"
#include "SampleBuffer.h"
#include "SampleClip.h"
#include "SampleTrack.h"
#include "Song.h"
#include "StreamScheduler.h"

using namespace lmms;

namespace
{

auto valueAt(std::int64_t frame, int channel) -> float
{
	const auto base = static_cast<float>((frame * 7919) % 2000 - 1000) / 1000.f;
	return channel == 0 ? base : -0.5f * base;
}

//! A float WAV of @p frames frames whose values are known (valueAt())
void writeWav(const QString& path, std::int64_t frames, int channels, int sampleRate)
{
	auto info = SF_INFO{};
	info.samplerate = sampleRate;
	info.channels = channels;
	info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
	auto* file = sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
	QVERIFY(file);
	auto samples = std::vector<float>(static_cast<std::size_t>(frames * channels));
	for (std::int64_t frame = 0; frame < frames; ++frame)
	{
		for (int channel = 0; channel < channels; ++channel) { samples[frame * channels + channel] = valueAt(frame, channel); }
	}
	sf_writef_float(file, samples.data(), frames);
	sf_close(file);
}

//! Waits for @p done, rendering periods meanwhile (blocks are released by the period count)
template<class Done>
auto waitFor(Done done, int timeoutMs = 3000, bool renderPeriods = false) -> bool
{
	auto timer = QElapsedTimer{};
	timer.start();
	while (!done())
	{
		if (timer.elapsed() > timeoutMs) { return false; }
		if (renderPeriods) { Engine::audioEngine()->renderNextPeriod(); }
		QTest::qWait(2);
	}
	return true;
}

} // namespace

class StreamingTest : public QObject
{
	Q_OBJECT

private:
	QTemporaryDir m_directory;
	int m_rate = 44100;

	auto path(const QString& name) const -> QString { return m_directory.filePath(name); }

private slots:
	void initTestCase()
	{
		QVERIFY(m_directory.isValid());
		qputenv("LINUXMUSIC_CACHE_DIR", QFile::encodeName(m_directory.filePath("cache")));
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
		m_rate = static_cast<int>(Engine::audioEngine()->outputSampleRate());
	}

	void cleanupTestCase()
	{
		AudioStream::setMissPolicy(AudioStream::MissPolicy::Automatic);
		Engine::destroy();
	}

	void cleanup()
	{
		AudioStream::setMissPolicy(AudioStream::MissPolicy::Automatic);
		DiskReader::instance().setDemand({});
	}

	void aStreamReadsTheFileAsTheDecoderDoes()
	{
		for (const auto channels : {1, 2})
		{
			const auto file = path(QString{"read%1.wav"}.arg(channels));
			const auto frames = std::int64_t{3 * AudioStream::BlockFrames + 1234};
			writeWav(file, frames, channels, m_rate);
			auto stream = AudioStream::open(file);
			QVERIFY(stream);
			QCOMPARE(stream->frames(), static_cast<f_cnt_t>(frames));
			QCOMPARE(stream->blockCount(), std::size_t{4});
			const auto buffer = SampleBuffer::fromFile(file);
			QCOMPARE(buffer->size(), static_cast<std::size_t>(frames));

			AudioStream::setMissPolicy(AudioStream::MissPolicy::ReadAtOnce);
			for (const auto index : {f_cnt_t{0}, f_cnt_t{1}, AudioStream::BlockFrames - 1, AudioStream::BlockFrames,
				2 * AudioStream::BlockFrames + 77, static_cast<f_cnt_t>(frames - 1)})
			{
				const auto* block = stream->blockAt(index);
				QVERIFY(block);
				const auto frame = block[index % AudioStream::BlockFrames];
				QCOMPARE(frame.left(), buffer->data()[index].left());
				QCOMPARE(frame.right(), buffer->data()[index].right());
			}
			QVERIFY(!stream->blockAt(static_cast<f_cnt_t>(frames)));
		}
	}

	void aFileThatCannotBeReadIsNotStreamed()
	{
		auto bad = QFile{path("bad.wav")};
		QVERIFY(bad.open(QIODevice::WriteOnly));
		bad.write("not audio at all");
		bad.close();
		QVERIFY(!AudioStream::open(path("bad.wav")));
		QVERIFY(!AudioStream::open(path("missing.wav")));
	}

	void aMissPlaysSilenceAndTheDiskThreadReadsIt()
	{
		const auto file = path("miss.wav");
		writeWav(file, 2 * AudioStream::BlockFrames, 2, m_rate);
		auto stream = AudioStream::open(file);
		QVERIFY(stream);
		AudioStream::setMissPolicy(AudioStream::MissPolicy::PlaySilence);
		const auto misses = AudioStream::missCount();
		QVERIFY(!stream->blockAt(AudioStream::BlockFrames + 5));
		QCOMPARE(AudioStream::missCount(), misses + 1);
		QVERIFY(waitFor([&] { return stream->isLoaded(1); }));
		QVERIFY(!stream->isLoaded(0)); // only what was missed
		const auto* block = stream->blockAt(AudioStream::BlockFrames + 5);
		QVERIFY(block);
		QCOMPARE(block[5].left(), valueAt(AudioStream::BlockFrames + 5, 0));
	}

	void theDemandIsReadAheadAndReleasedAfter()
	{
		const auto file = path("demand.wav");
		writeWav(file, 8 * AudioStream::BlockFrames, 2, m_rate);
		auto stream = AudioStream::open(file);
		QVERIFY(stream);
		DiskReader::instance().setDemand({{stream, AudioStream::BlockFrames / 2, 3 * AudioStream::BlockFrames + 10}});
		QVERIFY(DiskReader::instance().waitUntilLoaded(3000));
		for (std::size_t block = 0; block <= 3; ++block) { QVERIFY(stream->isLoaded(block)); }
		QVERIFY(!stream->isLoaded(4));

		// Out of the demand and not played: released once enough periods have gone by
		DiskReader::instance().setDemand({});
		QVERIFY(waitFor([&] { return stream->loadedBlocks().empty(); }, 10000, true));
	}

	void streamedPlaybackIsTheSameAsFromMemory()
	{
		const auto file = path("same.wav");
		writeWav(file, 3 * AudioStream::BlockFrames, 2, m_rate);
		AudioStream::setMissPolicy(AudioStream::MissPolicy::ReadAtOnce);
		for (const auto reversed : {false, true})
		{
			auto fromMemory = Sample{SampleBuffer::fromFile(file)};
			auto fromDisk = Sample{AudioStream::open(file)};
			QVERIFY(fromDisk.isStreamed());
			QCOMPARE(fromDisk.sampleSize(), fromMemory.sampleSize());
			fromMemory.setReversed(reversed);
			fromDisk.setReversed(reversed);
			auto memoryState = Sample::PlaybackState{};
			auto diskState = Sample::PlaybackState{};
			memoryState.setFrameIndex(AudioStream::BlockFrames - 300); // across a block's end
			diskState.setFrameIndex(AudioStream::BlockFrames - 300);
			const auto period = static_cast<std::size_t>(Engine::audioEngine()->framesPerPeriod());
			auto memoryOut = std::vector<SampleFrame>(period);
			auto diskOut = std::vector<SampleFrame>(period);
			for (std::size_t done = 0; done < 3000; done += period)
			{
				QVERIFY(fromMemory.play(memoryOut.data(), &memoryState, memoryOut.size()));
				QVERIFY(fromDisk.play(diskOut.data(), &diskState, diskOut.size()));
				for (std::size_t frame = 0; frame < memoryOut.size(); ++frame)
				{
					QCOMPARE(diskOut[frame].left(), memoryOut[frame].left());
					QCOMPARE(diskOut[frame].right(), memoryOut[frame].right());
				}
			}
		}
	}

	void atARatioOfOneASamplePlaysItsFramesExactly()
	{
		// Each resampler (a linear one keeps a frame to interpolate from; a sinc one filters) would delay or change
		// them: at a ratio of exactly one the frames are played as they are
		const auto file = path("exact.wav");
		writeWav(file, 4096, 2, m_rate);
		const auto buffer = SampleBuffer::fromFile(file);
		for (const auto mode : {AudioResampler::Mode::Linear, AudioResampler::Mode::SincMedium})
		{
			auto sample = Sample{buffer};
			auto state = Sample::PlaybackState{mode};
			auto out = std::vector<SampleFrame>(1000);
			QVERIFY(sample.play(out.data(), &state, out.size()));
			for (std::size_t frame = 0; frame < out.size(); ++frame)
			{
				QCOMPARE(out[frame].left(), buffer->data()[frame].left());
				QCOMPARE(out[frame].right(), buffer->data()[frame].right());
			}
		}
	}

	void aClipChoosesMemoryOrDisk()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		QVERIFY(track);
		const auto longFile = path("long.wav");
		const auto shortFile = path("short.wav");
		writeWav(longFile, static_cast<std::int64_t>((SampleClip::StreamAboveSeconds + 1) * m_rate), 2, m_rate);
		writeWav(shortFile, 2 * m_rate, 2, m_rate);

		// Automatic: long from disk, short in RAM
		auto* longClip = new SampleClip{track};
		longClip->setSampleFile(longFile);
		QVERIFY(longClip->isStreamed());
		auto* shortClip = new SampleClip{track};
		shortClip->setSampleFile(shortFile);
		QVERIFY(!shortClip->isStreamed());

		// Chosen: the same file the other way, keeping the clip as it was
		longClip->changeLength(TimePos::ticksPerBar() * 3);
		longClip->movePosition(TimePos::ticksPerBar());
		longClip->sample().setReversed(true);
		longClip->setStorage(SampleClip::Storage::Memory);
		QVERIFY(!longClip->isStreamed());
		QCOMPARE(longClip->length().getTicks(), TimePos::ticksPerBar() * 3);
		QCOMPARE(longClip->startPosition().getTicks(), TimePos::ticksPerBar());
		QVERIFY(longClip->sample().reversed());
		shortClip->setStorage(SampleClip::Storage::Disk);
		QVERIFY(shortClip->isStreamed());
		QCOMPARE(shortClip->sample().sampleSize(), static_cast<std::size_t>(2 * m_rate));

		// Saved with the clip, and followed when loaded
		auto document = QDomDocument{};
		auto element = document.createElement("sampleclip");
		shortClip->saveSettings(document, element);
		QCOMPARE(element.attribute("storage"), QString{"disk"});
		auto* loaded = new SampleClip{track};
		loaded->loadSettings(element);
		QVERIFY(loaded->isStreamed());
		QCOMPARE(loaded->storage(), SampleClip::Storage::Disk);
		element = document.createElement("sampleclip");
		longClip->saveSettings(document, element);
		QCOMPARE(element.attribute("storage"), QString{"ram"});

		// The pattern editor plays its clips from memory whatever is chosen
		auto* patternTrack = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, Engine::patternStore()));
		if (patternTrack)
		{
			auto* patternClip = new SampleClip{patternTrack};
			patternClip->setStorage(SampleClip::Storage::Disk);
			patternClip->setSampleFile(longFile);
			QVERIFY(!patternClip->isStreamed());
			delete patternTrack;
		}
	}

	void theSchedulerAsksForWhatIsAboutToPlay()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		const auto file = path("scheduled.wav");
		writeWav(file, 60 * m_rate, 2, m_rate);
		auto* clip = new SampleClip{track};
		clip->setSampleFile(file);
		QVERIFY(clip->isStreamed());
		const auto bar = TimePos::ticksPerBar();
		clip->movePosition(4 * bar);

		// Before the clip, nearer than the read-ahead: its start
		const auto framesPerTick = Engine::framesPerTick();
		const auto ticksAhead = static_cast<tick_t>(DiskReader::ReadAheadSeconds * m_rate / framesPerTick);
		auto demand = StreamScheduler::demandAt(*song, 4 * bar - ticksAhead / 2, DiskReader::ReadAheadSeconds);
		QCOMPARE(demand.size(), std::size_t{1});
		QCOMPARE(demand[0].stream, clip->sample().stream());
		QCOMPARE(demand[0].first, f_cnt_t{0});
		QVERIFY(demand[0].last > 0);

		// Far before it: nothing
		QVERIFY(StreamScheduler::demandAt(*song, 0, 0.1).empty());

		// In it: from where the song is
		demand = StreamScheduler::demandAt(*song, 6 * bar, DiskReader::ReadAheadSeconds);
		QCOMPARE(demand.size(), std::size_t{1});
		QCOMPARE(demand[0].first, clip->fileFrameAt(2 * bar));

		// Near the end of the cycle: also the cycle's start
		auto& timeline = song->getTimeline(Song::PlayMode::Song);
		timeline.setLoopPoints(TimePos{5 * bar}, TimePos{6 * bar});
		timeline.setLoopEnabled(true);
		demand = StreamScheduler::demandAt(*song, 6 * bar - 2, DiskReader::ReadAheadSeconds);
		QCOMPARE(demand.size(), std::size_t{2});
		QCOMPARE(demand[1].first, clip->fileFrameAt(bar));
		timeline.setLoopEnabled(false);
	}

	void aLiveSongPlaysFromDiskWithoutMisses()
	{
		// The same song played live with the clip in RAM and from disk, read ahead by the scheduler
		auto* song = Engine::getSong();
		const auto file = path("live.wav");
		writeWav(file, 30 * m_rate, 2, m_rate);
		const auto render = [&](SampleClip::Storage storage) {
			song->createNewProject();
			auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
			auto* clip = new SampleClip{track};
			clip->setStorage(storage);
			clip->setSampleFile(file);
			clip->movePosition(TimePos::ticksPerBar() / 4);
			if (clip->isStreamed())
			{
				DiskReader::instance().setDemand(StreamScheduler::demandAt(*song, 0, DiskReader::ReadAheadSeconds));
				[&] { QVERIFY(DiskReader::instance().waitUntilLoaded(3000)); }();
			}
			auto output = std::vector<SampleFrame>{};
			song->setPlayPos(0, Song::PlayMode::Song);
			song->playSong();
			for (int period = 0; period < 400; ++period)
			{
				const auto out = Engine::audioEngine()->renderNextPeriod();
				output.insert(output.end(), out.begin(), out.end());
			}
			song->stop();
			return output;
		};
		AudioStream::setMissPolicy(AudioStream::MissPolicy::PlaySilence);
		const auto fromMemory = render(SampleClip::Storage::Memory);
		const auto misses = AudioStream::missCount();
		const auto fromDisk = render(SampleClip::Storage::Disk);
		QCOMPARE(AudioStream::missCount(), misses);
		QCOMPARE(fromDisk.size(), fromMemory.size());
		auto loudest = 0.f;
		for (std::size_t frame = 0; frame < fromMemory.size(); ++frame)
		{
			QCOMPARE(fromDisk[frame].left(), fromMemory[frame].left());
			QCOMPARE(fromDisk[frame].right(), fromMemory[frame].right());
			loudest = std::max(loudest, std::abs(fromMemory[frame].left()));
		}
		QVERIFY(loudest > 0.1f); // (it did play)
	}

	void peaksAreBuiltOnceAndKept()
	{
		const auto file = path("peaks.wav");
		const auto frames = std::int64_t{10 * PeakCache::FramesPerPeak + 17};
		writeWav(file, frames, 2, m_rate);
		QFile::remove(PeakCache::peakFileOf(file));
		QVERIFY(!PeakCache::readPeakFile(file));

		// Built in the background, then known
		auto spy = QSignalSpy{&PeakCache::instance(), &PeakCache::ready};
		QVERIFY(!PeakCache::instance().fetch(file));
		PeakCache::instance().waitForBuilds();
		// (other files' peaks, built for the clips above, may be told too)
		const auto told = [&] {
			return std::any_of(spy.begin(), spy.end(), [&](const QList<QVariant>& arguments) {
				return arguments.at(0).toString() == QFileInfo{file}.absoluteFilePath();
			});
		};
		QVERIFY(waitFor(told));
		const auto peaks = PeakCache::instance().fetch(file);
		QVERIFY(peaks);
		QCOMPARE(peaks->sampleCount(), static_cast<f_cnt_t>(frames));
		const auto& finest = peaks->nearestThumbnailForWidth(static_cast<f_cnt_t>(frames));
		QCOMPARE(finest.width(), f_cnt_t{11});
		for (f_cnt_t peak = 0; peak < finest.width(); ++peak)
		{
			auto low = 1e9f, high = -1e9f;
			for (auto frame = peak * PeakCache::FramesPerPeak;
				frame < std::min<std::int64_t>((peak + 1) * PeakCache::FramesPerPeak, frames); ++frame)
			{
				const auto value = (valueAt(frame, 0) + valueAt(frame, 1)) / 2.f;
				low = std::min(low, value);
				high = std::max(high, value);
			}
			QCOMPARE(finest[peak].min, low);
			QCOMPARE(finest[peak].max, high);
		}

		// Kept in the peak file, until the audio file changes
		QVERIFY(PeakCache::readPeakFile(file));
		writeWav(file, frames + 1000, 2, m_rate);
		QVERIFY(!PeakCache::readPeakFile(file));
	}
};

QTEST_GUILESS_MAIN(StreamingTest)
#include "StreamingTest.moc"
