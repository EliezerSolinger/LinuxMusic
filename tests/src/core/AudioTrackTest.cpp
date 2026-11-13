/*
 * AudioTrackTest.cpp
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


#include <algorithm>
#include <cmath>
#include <numbers>
#include <vector>

#include <QDomDocument>
#include <QObject>
#include <QTemporaryDir>
#include <QtTest>
#include <sndfile.h>

#include "AudioEngine.h"
#include "AudioItem.h"
#include "AudioStream.h"
#include "AudioTrack.h"
#include "DataFile.h"
#include "Engine.h"
#include "ProjectJournal.h"
#include "SampleClip.h"
#include "SampleTrack.h"
#include "PathUtil.h"
#include "Song.h"
#include "StreamScheduler.h"

using namespace lmms;

namespace
{

//! A float WAV of a constant @p value (stereo, at @p rate)
void writeWav(const QString& path, std::int64_t frames, float value, int rate)
{
	auto info = SF_INFO{};
	info.samplerate = rate;
	info.channels = 2;
	info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
	auto* file = sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
	QVERIFY(file);
	auto samples = std::vector<float>(static_cast<std::size_t>(frames * 2), value);
	sf_writef_float(file, samples.data(), frames);
	sf_close(file);
}

} // namespace

class AudioTrackTest : public QObject
{
	Q_OBJECT
private:
	QTemporaryDir m_directory;
	int m_rate = 44100;

	//! Plays the song from @p tick for @p periods periods: the left channel
	auto render(tick_t tick, int periods) -> std::vector<float>
	{
		auto* song = Engine::getSong();
		auto* engine = Engine::audioEngine();
		for (int period = 0; period < 2; ++period) { engine->renderNextPeriod(); } // (the one before is flushed)
		song->setPlayPos(tick, Song::PlayMode::Song);
		song->playSong();
		auto left = std::vector<float>{};
		for (int period = 0; period < periods; ++period)
		{
			for (const auto& frame : engine->renderNextPeriod()) { left.push_back(frame.left()); }
		}
		song->stop();
		return left;
	}

	auto newTrack() -> AudioTrack*
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		song->setTempo(120);
		auto* track = dynamic_cast<AudioTrack*>(Track::create(Track::Type::Audio, song));
		[&] { QVERIFY(track); }();
		return track;
	}

private slots:
	void initTestCase()
	{
		QVERIFY(m_directory.isValid());
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
		m_rate = static_cast<int>(Engine::audioEngine()->outputSampleRate());
		Engine::projectJournal()->setJournalling(true);
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	void anItemPlaysItsFileWhereItIs()
	{
		auto* track = newTrack();
		const auto file = m_directory.filePath("half.wav");
		writeWav(file, m_rate, 0.5f, m_rate);
		const auto bar = TimePos::ticksPerBar();
		auto* item = track->addItem(file, TimePos{bar});
		QCOMPARE(item->startPosition().getTicks(), bar);
		QVERIFY(item->length().getTicks() > 0);
		track->publish();

		// Silent before the item, its file in it (between its fades), silent after it
		const auto framesPerTick = static_cast<double>(Engine::framesPerTick());
		const auto out = render(0, static_cast<int>((bar + item->length().getTicks()) * framesPerTick / 256) + 20);
		// (the engine hands out each period a period late)
		const auto lag = static_cast<std::size_t>(Engine::audioEngine()->framesPerPeriod());
		const auto start = static_cast<std::size_t>(bar * framesPerTick) + lag;
		const auto end = static_cast<std::size_t>((bar + item->length().getTicks()) * framesPerTick) + lag;
		const auto fade = static_cast<std::size_t>(AudioItem::DefaultFadeMs * m_rate / 1000.0) + 4;
		QVERIFY(std::abs(out[start - 300]) < 1e-6f);
		QVERIFY(std::abs(out[start + fade + 100] - 0.5f) < 1e-3f);
		QVERIFY(std::abs(out[(start + end) / 2] - 0.5f) < 1e-3f);
		QVERIFY(std::abs(out[end + 300]) < 1e-6f);
		// The fade in rises from silence
		QVERIFY(std::abs(out[start + 1]) < 0.1f);
	}

	void gainFadesLaneAndStorageAreCommands()
	{
		auto* track = newTrack();
		const auto file = m_directory.filePath("cmd.wav");
		writeWav(file, m_rate, 0.5f, m_rate);
		auto* item = track->addItem(file, TimePos{0});
		auto* journal = Engine::projectJournal();

		auto settings = AudioTrack::settingsOf(*item);
		settings.gainDb = -6.0206;
		track->changeItem(item, settings);
		QCOMPARE(item->gainDb(), -6.0206);
		settings.lane = 2;
		settings.fadeInMs = 50;
		settings.storage = AudioItem::Storage::Disk;
		track->changeItem(item, settings);
		QCOMPARE(item->lane(), 2);
		QVERIFY(item->isStreamed());
		QCOMPARE(track->laneCount(), 3);

		journal->undo();
		QCOMPARE(item->lane(), 0);
		QCOMPARE(item->fadeInMs(), AudioItem::DefaultFadeMs);
		QVERIFY(!item->isStreamed());
		QCOMPARE(item->gainDb(), -6.0206);
		journal->undo();
		QCOMPARE(item->gainDb(), 0.0);
		journal->redo();
		journal->redo();
		QCOMPARE(item->lane(), 2);

		// The gain is heard: -6 dB halves the level
		settings = AudioTrack::settingsOf(*item);
		settings.lane = 0;
		settings.storage = AudioItem::Storage::Automatic;
		track->changeItem(item, settings);
		track->publish();
		const auto out = render(0, 40);
		QVERIFY(std::abs(out[5000] - 0.25f) < 1e-3f);
	}

	void onlyThePlayingLaneIsHeard()
	{
		auto* track = newTrack();
		const auto low = m_directory.filePath("lane0.wav");
		const auto high = m_directory.filePath("lane1.wav");
		writeWav(low, m_rate, 0.25f, m_rate);
		writeWav(high, m_rate, 0.75f, m_rate);
		track->addItem(low, TimePos{0}, 0);
		track->addItem(high, TimePos{0}, 1);
		track->publish();
		QVERIFY(std::abs(render(0, 40)[5000] - 0.25f) < 1e-3f);
		track->setPlayingLane(1);
		track->publish();
		QVERIFY(std::abs(render(0, 40)[5000] - 0.75f) < 1e-3f);
		Engine::projectJournal()->undo();
		QCOMPARE(track->playingLane(), 0);
	}

	void overlappingItemsCrossfadeAtAnEvenPower()
	{
		auto* track = newTrack();
		const auto file = m_directory.filePath("cross.wav");
		writeWav(file, 4 * m_rate, 0.5f, m_rate);
		const auto bar = TimePos::ticksPerBar();
		auto* first = track->addItem(file, TimePos{0});
		first->changeLength(TimePos{2 * bar});
		auto* second = track->addItem(file, TimePos{bar});
		second->changeLength(TimePos{2 * bar});
		track->publish();

		// In the middle of the overlap each is at sin(pi/4): the power sums to the level of one
		const auto framesPerTick = static_cast<double>(Engine::framesPerTick());
		const auto out = render(0, static_cast<int>(3 * bar * framesPerTick / 256) + 4);
		const auto middle = static_cast<std::size_t>(1.5 * bar * framesPerTick);
		QVERIFY(std::abs(out[middle] - 2 * 0.5f * std::sin(std::numbers::pi_v<float> / 4)) < 2e-3f);
		// Outside the overlap, one item at its level
		QVERIFY(std::abs(out[static_cast<std::size_t>(0.5 * bar * framesPerTick)] - 0.5f) < 1e-3f);
		QVERIFY(std::abs(out[static_cast<std::size_t>(2.5 * bar * framesPerTick)] - 0.5f) < 1e-3f);
	}

	void playingGoesOnAcrossALoop()
	{
		auto* track = newTrack();
		const auto file = m_directory.filePath("loop.wav");
		// A ramp: where the file is shows in the value
		{
			auto info = SF_INFO{};
			info.samplerate = m_rate;
			info.channels = 2;
			info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
			auto* sndFile = sf_open(QFile::encodeName(file).constData(), SFM_WRITE, &info);
			QVERIFY(sndFile);
			auto samples = std::vector<float>(static_cast<std::size_t>(8 * m_rate) * 2);
			for (std::size_t frame = 0; frame < samples.size() / 2; ++frame)
			{
				samples[2 * frame] = samples[2 * frame + 1] = static_cast<float>(frame) / (8.f * m_rate);
			}
			sf_writef_float(sndFile, samples.data(), 8 * m_rate);
			sf_close(sndFile);
		}
		const auto bar = TimePos::ticksPerBar();
		auto* item = track->addItem(file, TimePos{0});
		track->changeItem(item, [&] { auto s = AudioTrack::settingsOf(*item); s.fadeInMs = s.fadeOutMs = 0; return s; }());
		track->publish();
		auto& timeline = Engine::getSong()->getTimeline(Song::PlayMode::Song);
		timeline.setLoopPoints(TimePos{bar}, TimePos{2 * bar});
		timeline.setLoopEnabled(true);
		const auto framesPerTick = static_cast<double>(Engine::framesPerTick());
		const auto out = render(bar, static_cast<int>(2.5 * bar * framesPerTick / 256));
		timeline.setLoopEnabled(false);
		// Each pass plays the file from the loop's start again
		const auto pass = static_cast<std::size_t>(bar * framesPerTick);
		const auto expected = static_cast<float>(bar * framesPerTick + 1000) / (8.f * m_rate);
		QVERIFY(std::abs(out[1000] - expected) < 1e-3f);
		QVERIFY(std::abs(out[pass + 1000] - expected) < 2e-3f);
	}

	void aStreamedItemPlaysLiveWithoutMisses()
	{
		const auto file = m_directory.filePath("long-stream.wav");
		writeWav(file, 30 * m_rate, 0.5f, m_rate);
		auto renderWith = [&](AudioItem::Storage storage) {
			auto* track = newTrack();
			auto* item = track->addItem(file, TimePos{0});
			auto settings = AudioTrack::settingsOf(*item);
			settings.storage = storage;
			track->changeItem(item, settings);
			track->publish();
			if (item->isStreamed())
			{
				DiskReader::instance().setDemand(StreamScheduler::demandAt(*Engine::getSong(), 0, DiskReader::ReadAheadSeconds));
				[&] { QVERIFY(DiskReader::instance().waitUntilLoaded(3000)); }();
			}
			return render(0, 300);
		};
		AudioStream::setMissPolicy(AudioStream::MissPolicy::PlaySilence);
		const auto fromMemory = renderWith(AudioItem::Storage::Memory);
		const auto misses = AudioStream::missCount();
		const auto fromDisk = renderWith(AudioItem::Storage::Disk);
		AudioStream::setMissPolicy(AudioStream::MissPolicy::Automatic);
		DiskReader::instance().setDemand({});
		QCOMPARE(AudioStream::missCount(), misses);
		QCOMPARE(fromDisk, fromMemory);
		QVERIFY(std::abs(fromDisk[20000] - 0.5f) < 1e-3f);
	}

	void aNewTakeWithoutLanesReplacesWhatItCovers()
	{
		auto* track = newTrack();
		const auto file = m_directory.filePath("covered.wav");
		writeWav(file, 20 * m_rate, 0.5f, m_rate);
		const auto bar = TimePos::ticksPerBar();
		const auto recordingStartOf = [](const AudioItem* item) {
			return item->startPosition().getTicks() + item->startTimeOffset().getTicks();
		};

		// Around it: split in two, the file where it was under both parts
		auto* whole = track->addItem(file, TimePos{0});
		whole->changeLength(TimePos{4 * bar});
		track->clearSpan(0, bar, 2 * bar);
		auto items = track->items();
		QCOMPARE(items.size(), std::size_t{2});
		std::sort(items.begin(), items.end(), [](auto* a, auto* b) { return a->startPosition() < b->startPosition(); });
		QCOMPARE(items[0]->startPosition().getTicks(), 0);
		QCOMPARE(items[0]->endPosition().getTicks(), bar);
		QCOMPARE(items[1]->startPosition().getTicks(), 2 * bar);
		QCOMPARE(items[1]->endPosition().getTicks(), 4 * bar);
		QCOMPARE(recordingStartOf(items[1]), 0);
		QVERIFY(items[0]->id() != items[1]->id());

		// Over its end: trimmed; over its start: starts later in its file; all under it: gone
		track->clearSpan(0, bar / 2, bar + 1);
		QCOMPARE(items[0]->endPosition().getTicks(), bar / 2);
		track->clearSpan(0, 2 * bar - 10, 3 * bar);
		QCOMPARE(items[1]->startPosition().getTicks(), 3 * bar);
		QCOMPARE(items[1]->endPosition().getTicks(), 4 * bar);
		QCOMPARE(recordingStartOf(items[1]), 0);
		// (another lane is not touched)
		auto* other = track->addItem(file, TimePos{0}, 1);
		other->changeLength(TimePos{4 * bar});
		track->clearSpan(0, 0, 8 * bar);
		QCOMPARE(track->items().size(), std::size_t{1});
		QCOMPARE(track->items().front(), other);
	}

	void aRecordedItemFollowsTheTempo()
	{
		auto* track = newTrack();
		auto* song = Engine::getSong();
		const auto file = m_directory.filePath("bound.wav");
		writeWav(file, 4 * m_rate, 0.5f, m_rate);
		const auto bar = TimePos::ticksPerBar();
		auto* item = track->addItem(file, TimePos{bar});
		// Played at 120 BPM: a point at each bar
		auto timing = AudioItem::Timing{};
		for (tick_t tick = 0; tick <= 2 * bar; tick += bar) { timing.push_back({tick, song->framesBetween(bar, bar + tick)}); }
		item->setOriginalTiming(timing, file);
		QVERIFY(song->stretchJobs().empty());

		// Moved at the same tempo: nothing to render; at another tempo: rendered
		item->movePosition(TimePos{3 * bar});
		QVERIFY(song->stretchJobs().empty());
		song->setTempo(90);
		const auto jobs = song->stretchJobs();
		QCOMPARE(jobs.size(), std::size_t{1});
		QCOMPARE(jobs[0].item, item);
		QVERIFY(jobs[0].to.back().frame > jobs[0].from.back().frame); // (slower: longer)

		// Rendered, it plays the rendering; back at the tempo it was played at, the original again
		Song::applyStretch(jobs[0]); // (the target file does not exist here: nothing loads, the timing is kept)
		song->setTempo(120);
		QVERIFY(song->stretchJobs().empty());
		QCOMPARE(item->currentTiming(), item->originalTiming());
	}

	void aSampleTrackWithRecordingsOpensAsAnAudioTrack()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		const auto first = m_directory.filePath("old-take-a.wav");
		const auto second = m_directory.filePath("old-take-b.wav");
		writeWav(first, m_rate, 0.25f, m_rate);
		writeWav(second, m_rate, 0.75f, m_rate);
		const auto bar = TimePos::ticksPerBar();

		// A sample track of before: a clip with two takes (the second heard), bound to the tempo, and armed
		auto* old = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		old->setName("Guitar");
		old->recordArmModel()->setValue(true);
		auto* clip = new SampleClip{old};
		clip->setSampleFile(first);
		clip->addTake(second);
		clip->movePosition(TimePos{bar});
		clip->setOriginalTiming({{0, 0.0}, {bar, song->framesBetween(bar, 2 * bar)}}, second);
		auto document = QDomDocument{};
		auto element = document.createElement("tracks");
		document.appendChild(element);
		old->saveState(document, element);
		// (and one of samples on the grid, which stays as it is)
		auto* grid = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		(new SampleClip{grid})->setSampleFile(first);
		grid->saveState(document, element);

		auto* converted = dynamic_cast<AudioTrack*>(Track::create(element.firstChildElement(), song));
		QVERIFY(converted);
		QCOMPARE(converted->name(), QString{"Guitar"});
		QVERIFY(converted->isRecordArmed());
		auto items = converted->items();
		QCOMPARE(items.size(), std::size_t{2});
		std::sort(items.begin(), items.end(), [](auto* a, auto* b) { return a->lane() < b->lane(); });
		QCOMPARE(items[0]->lane(), 0);
		QCOMPARE(PathUtil::toAbsolute(items[0]->file()), QFileInfo{second}.absoluteFilePath());
		QVERIFY(items[0]->isTempoBound());
		QCOMPARE(items[0]->startPosition().getTicks(), bar);
		QCOMPARE(items[1]->lane(), 1);
		QCOMPARE(PathUtil::toAbsolute(items[1]->file()), QFileInfo{first}.absoluteFilePath());

		auto* stays = Track::create(element.firstChildElement().nextSiblingElement(), song);
		QCOMPARE(stays->type(), Track::Type::Sample);
	}

	void itemsAreSavedAndLoaded()
	{
		auto* track = newTrack();
		const auto file = m_directory.filePath("saved.wav");
		writeWav(file, m_rate, 0.5f, m_rate);
		auto* item = track->addItem(file, TimePos{96}, 1);
		auto settings = AudioTrack::settingsOf(*item);
		settings.gainDb = 3;
		settings.fadeInMs = 20;
		settings.fadeOutMs = 40;
		settings.storage = AudioItem::Storage::Disk;
		track->changeItem(item, settings);
		track->setPlayingLane(1);
		const auto id = item->id();

		auto document = QDomDocument{};
		auto element = document.createElement("track");
		document.appendChild(element);
		track->saveState(document, element);
		auto* loaded = dynamic_cast<AudioTrack*>(Track::create(element.firstChildElement(), Engine::getSong()));
		QVERIFY(loaded);
		QCOMPARE(loaded->type(), Track::Type::Audio);
		QCOMPARE(loaded->playingLane(), 1);
		QCOMPARE(loaded->items().size(), std::size_t{1});
		const auto* copy = loaded->items().front();
		QCOMPARE(copy->startPosition().getTicks(), 96);
		QCOMPARE(copy->lane(), 1);
		QCOMPARE(copy->gainDb(), 3.0);
		QCOMPARE(copy->fadeInMs(), 20.0);
		QCOMPARE(copy->fadeOutMs(), 40.0);
		QCOMPARE(copy->storage(), AudioItem::Storage::Disk);
		QVERIFY(copy->isStreamed());
		QCOMPARE(copy->id(), id); // (another track: the id stays)
	}
};

QTEST_GUILESS_MAIN(AudioTrackTest)
#include "AudioTrackTest.moc"
