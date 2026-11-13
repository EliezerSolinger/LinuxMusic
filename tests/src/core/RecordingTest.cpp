/*
 * RecordingTest.cpp - audio recording lands where it was played
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

#include <QDomDocument>
#include <QObject>
#include <QTemporaryDir>
#include <QtTest>

#include <algorithm>
#include <cmath>
#include <map>
#include <vector>

#include <sndfile.h>

#include "AudioDevice.h"
#include "AudioEngine.h"
#include "AudioRecorder.h"
#include "AudioTrack.h"
#include "LatencyCalibration.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "MidiRecorder.h"
#include "ConfigManager.h"
#include "PathUtil.h"
#include "PeakCache.h"
#include "ProjectJournal.h"
#include "Engine.h"
#include "SampleClip.h"
#include "SampleTrack.h"
#include "Song.h"

namespace
{

using namespace lmms;

//! A duplex device whose output comes back into its input after its latencies: a cable from output to input
class LoopbackDevice : public AudioDevice
{
public:
	//! @param hidden latency the device does not report (what the calibration finds)
	LoopbackDevice(AudioEngine* engine, f_cnt_t inputLatency, f_cnt_t outputLatency, f_cnt_t hidden = 0)
		: AudioDevice{2, engine}
		, m_inputLatency{inputLatency}
		, m_outputLatency{outputLatency}
		, m_hidden{hidden}
	{
		setInputChannels(2);
	}

	auto inputLatency() const -> f_cnt_t override { return m_inputLatency; }
	//! As if the device dropped out (an xrun)
	void dropOut() { reportXrun(); }
	auto outputLatency() const -> f_cnt_t override { return m_outputLatency; }

	/**
	 * One callback of @p frames frames: the output a frame leaves the device with is heard after the output latency,
	 * and what is heard reaches the input after the input latency
	 */
	void callback(f_cnt_t frames)
	{
		auto input = std::vector<std::vector<float>>(2, std::vector<float>(frames));
		for (f_cnt_t frame = 0; frame < frames; ++frame)
		{
			const auto device = m_frame + frame;
			const auto source = device - static_cast<std::int64_t>(m_inputLatency + m_outputLatency + m_hidden);
			for (int channel = 0; channel < 2; ++channel)
			{
				input[channel][frame] = source >= 0 && source < static_cast<std::int64_t>(m_output[channel].size())
					? m_output[channel][source]
					: 0.f;
			}
		}
		const float* inputPointers[] = {input[0].data(), input[1].data()};
		auto output = std::vector<float>(2 * frames);
		audioEngine()->renderNextBuffer(PlanarBufferView<const float>{inputPointers, 2, frames},
			InterleavedBufferView<float>{output.data(), 2, frames});
		for (f_cnt_t frame = 0; frame < frames; ++frame)
		{
			m_output[0].push_back(output[2 * frame]);
			m_output[1].push_back(output[2 * frame + 1]);
		}
		m_frame += frames;
	}

	auto outputFrames() const -> const std::vector<float>& { return m_output[0]; }

	//! The frame and level of the loudest output sample of channel 0
	auto outputPeak() const -> std::pair<std::int64_t, float>
	{
		auto peak = std::pair<std::int64_t, float>{-1, 0.f};
		for (std::size_t frame = 0; frame < m_output[0].size(); ++frame)
		{
			if (std::abs(m_output[0][frame]) > peak.second) { peak = {static_cast<std::int64_t>(frame), std::abs(m_output[0][frame])}; }
		}
		return peak;
	}

private:
	void startProcessingImpl() override {}
	void stopProcessingImpl() override {}

	f_cnt_t m_inputLatency;
	f_cnt_t m_outputLatency;
	f_cnt_t m_hidden = 0;
	std::int64_t m_frame = 0;
	std::vector<float> m_output[2]; //!< everything the device played
};

//! Writes a mono float WAV of @p frames frames, silent but for a full-scale sample at @p impulse
void writeImpulse(const QString& path, sample_rate_t rate, int frames, int impulse)
{
	auto info = SF_INFO{};
	info.samplerate = static_cast<int>(rate);
	info.channels = 1;
	info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
	auto* file = sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
	QVERIFY(file);
	auto samples = std::vector<float>(frames, 0.f);
	samples[impulse] = 1.f;
	sf_writef_float(file, samples.data(), frames);
	sf_close(file);
}

//! @returns the frame of the largest absolute sample of channel 0 of a WAV file, or -1
auto peakFrame(const QString& path) -> std::int64_t
{
	auto info = SF_INFO{};
	auto* file = sf_open(QFile::encodeName(path).constData(), SFM_READ, &info);
	if (!file) { return -1; }
	auto samples = std::vector<float>(static_cast<std::size_t>(info.frames * info.channels));
	sf_readf_float(file, samples.data(), info.frames);
	sf_close(file);
	auto peak = std::int64_t{-1};
	auto level = 0.f;
	for (sf_count_t frame = 0; frame < info.frames; ++frame)
	{
		const auto value = std::abs(samples[frame * info.channels]);
		if (value > level) { level = value; peak = frame; }
	}
	return level > 0.1f ? peak : -1;
}

} // namespace

class RecordingTest : public QObject
{
	Q_OBJECT

private:
	/**
	 * Plays an impulse on one track and records it on an armed track through a loopback cable, with callbacks of
	 * @p callbackFrames frames, from @p startTick on
	 * @returns the song frame the recorded impulse landed on, minus the song frame it was heard at
	 */
	auto recordLoopback(f_cnt_t inputLatency, f_cnt_t outputLatency, f_cnt_t callbackFrames, tick_t startTick)
		-> double
	{
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		auto device = new LoopbackDevice{engine, inputLatency, outputLatency};
		engine->setAudioDevice(device, false);

		// The impulse, 100 frames into a clip at bar 3
		constexpr tick_t ClipTick = 2 * 192;
		constexpr int ImpulseOffset = 100;
		const auto impulsePath = m_directory.filePath("impulse.wav");
		writeImpulse(impulsePath, engine->outputSampleRate(), 20000, ImpulseOffset);

		auto* player = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		auto* recorder = dynamic_cast<AudioTrack*>(Track::create(Track::Type::Audio, song));
		auto* clip = new SampleClip{player};
		clip->setSampleFile(impulsePath);
		clip->movePosition(TimePos{ClipTick});
		recorder->setInput({AudioInput::Source::Hardware, 0, 1});
		recorder->recordArmModel()->setValue(true);
		// Monitoring would send the input back out through the cable, round and round
		recorder->monitorModel()->setValue(static_cast<int>(SampleTrack::MonitorMode::Off));

		song->setPlayPos(startTick, Song::PlayMode::Song);
		auto problem = QString{};
		const auto connection = connect(song, &Song::recordingProblem, this, [&](const QString& m) { problem = m; });
		const bool recording = song->record();
		disconnect(connection);
		if (!recording)
		{
			qWarning("not recording: %s", qPrintable(problem));
			return 1e9;
		}
		const auto end = (ClipTick + 192) * static_cast<double>(Engine::framesPerTick());
		for (double played = startTick * static_cast<double>(Engine::framesPerTick()); played < end + 3 * callbackFrames;
			played += callbackFrames)
		{
			device->callback(callbackFrames);
		}
		song->stop();

		// Where the impulse was heard: the device played song frame s at device frame s - start + 1 period (the
		// engine hands the device each period one period late). Measuring against what was heard keeps this test
		// about the recording, whatever the playback does.
		const auto outputPeak = device->outputPeak();
		if (outputPeak.second < 0.5f) { qWarning("the impulse was not played"); }
		const auto framesPerTick = static_cast<double>(Engine::framesPerTick());
		const auto playedAt = startTick * framesPerTick + static_cast<double>(outputPeak.first)
			- static_cast<double>(engine->framesPerPeriod());

		// The take is the recording track's clip
		auto result = 1e9;
		if (recorder->numOfClips() != 1) { qWarning("%d takes (%s)", recorder->numOfClips(), qPrintable(problem)); }
		else
		{
			const auto* take = dynamic_cast<AudioItem*>(recorder->getClip(0));
			const auto peak = peakFrame(PathUtil::toAbsolute(take->file()));
			if (peak < 0) { qWarning("no impulse in %s", qPrintable(take->file())); }
			else { result = take->startPosition().getTicks() * framesPerTick + peak - playedAt; }
		}

		const auto guard = engine->requestChangesGuard();
		delete player;
		delete recorder;
		return result;
	}

private slots:
	void initTestCase()
	{
		QVERIFY(m_directory.isValid());
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	void recordingLandsWhereItWasPlayed_data()
	{
		QTest::addColumn<int>("inputLatency");
		QTest::addColumn<int>("outputLatency");
		QTest::addColumn<int>("callbackFrames");
		QTest::addColumn<int>("startTick");
		// A device's latencies hold at least its buffer on each side (what comes in reaches the engine a buffer later)
		QTest::newRow("buffers only, callbacks of a period") << 256 << 256 << 256 << 0;
		QTest::newRow("latencies, callbacks of a period") << 256 + 37 << 256 + 211 << 256 << 0;
		QTest::newRow("latencies, odd callbacks") << 97 + 113 << 97 + 517 << 97 << 0;
		QTest::newRow("long callbacks, started mid-song") << 1024 + 64 << 1024 + 1024 << 1024 << 200;
	}

	void recordingLandsWhereItWasPlayed()
	{
		QFETCH(int, inputLatency);
		QFETCH(int, outputLatency);
		QFETCH(int, callbackFrames);
		QFETCH(int, startTick);
		const auto offset = recordLoopback(inputLatency, outputLatency, callbackFrames, startTick);
		qInfo("recorded %.2f frames from where it was played", offset);
		QVERIFY2(std::abs(offset) <= 1.0, qPrintable(QString::number(offset)));
	}

	void punchInAndOutWhilePlaying()
	{
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		auto* device = new LoopbackDevice{engine, 256 + 50, 256 + 300};
		engine->setAudioDevice(device, false);

		constexpr tick_t ClipTick = 3 * 192;
		const auto impulsePath = m_directory.filePath("impulse-punch.wav");
		writeImpulse(impulsePath, engine->outputSampleRate(), 20000, 0);
		auto* player = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		auto* recorder = dynamic_cast<AudioTrack*>(Track::create(Track::Type::Audio, song));
		auto* clip = new SampleClip{player};
		clip->setSampleFile(impulsePath);
		clip->movePosition(TimePos{ClipTick});
		recorder->setInput({AudioInput::Source::Hardware, 0, 1});
		recorder->recordArmModel()->setValue(true);
		// Monitoring would send the input back out through the cable, round and round
		recorder->monitorModel()->setValue(static_cast<int>(SampleTrack::MonitorMode::Off));
		const auto cleanUp = qScopeGuard([&] {
			song->stop();
			const auto guard = engine->requestChangesGuard();
			delete player;
			delete recorder;
		});

		const auto framesPerTick = static_cast<double>(Engine::framesPerTick());
		const auto callbacksUntil = [&](double frame) {
			return static_cast<int>(std::ceil(frame / 256.0));
		};

		// Play from the start; punch in at bar 2, out after the impulse at bar 4
		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		for (int callback = 0; callback < callbacksUntil(192 * framesPerTick); ++callback) { device->callback(256); }
		QVERIFY(song->record());
		for (int callback = 0; callback < callbacksUntil(2 * 192 * framesPerTick) + 20; ++callback) { device->callback(256); }
		song->stopRecording();
		QVERIFY(!song->isRecordingAudio());
		QVERIFY(song->isPlaying());
		song->stop();

		QCOMPARE(recorder->numOfClips(), 1);
		const auto* take = dynamic_cast<AudioItem*>(recorder->getClip(0));

		// The take starts where the punch in was heard: two periods and the latencies before bar 2
		const auto expectedStart = (192 * framesPerTick - 2 * 256.0 - (50 + 300 + 512)) / framesPerTick;
		QVERIFY(std::abs(take->startPosition().getTicks() - expectedStart) <= 1.0);
		const auto peak = peakFrame(PathUtil::toAbsolute(take->file()));
		const auto outputPeak = device->outputPeak();
		const auto heardAt = static_cast<double>(outputPeak.first) - static_cast<double>(engine->framesPerPeriod());
		const auto recordedAt = take->startPosition().getTicks() * framesPerTick + static_cast<double>(peak);
		qInfo("punched take at tick %d, impulse %.2f frames from where it was heard", take->startPosition().getTicks(),
			recordedAt - heardAt);
		QVERIFY(peak >= 0);
		QVERIFY(std::abs(recordedAt - heardAt) <= 1.0);
	}

	void preRollStartsEarlierAndTheTakeAtTheCursor()
	{
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		auto* device = new LoopbackDevice{engine, 256 + 20, 256 + 100};
		engine->setAudioDevice(device, false);
		auto* recorder = dynamic_cast<AudioTrack*>(Track::create(Track::Type::Audio, song));
		recorder->setInput({AudioInput::Source::Hardware, 0, 1});
		recorder->recordArmModel()->setValue(true);
		// Monitoring would send the input back out through the cable, round and round
		recorder->monitorModel()->setValue(static_cast<int>(SampleTrack::MonitorMode::Off));
		ConfigManager::inst()->setValue("recording", "prerollbars", "1");
		ConfigManager::inst()->setValue("recording", "countin", "1");
		const auto cleanUp = qScopeGuard([&] {
			ConfigManager::inst()->setValue("recording", "prerollbars", "0");
			ConfigManager::inst()->setValue("recording", "countin", "0");
			song->stop();
			const auto guard = engine->requestChangesGuard();
			delete recorder;
		});

		// From the cursor at bar 3, the song plays from bar 2 and the take starts at bar 3
		const auto cursor = 2 * TimePos::ticksPerBar();
		song->setPlayPos(cursor, Song::PlayMode::Song);
		QVERIFY(song->record());
		QCOMPARE(song->getPlayPos(Song::PlayMode::Song).getTicks(), cursor - TimePos::ticksPerBar());
		for (int callback = 0; callback < 600; ++callback) { device->callback(256); }
		song->stop();
		QCOMPARE(recorder->numOfClips(), 1);
		QCOMPARE(recorder->getClip(0)->startPosition().getTicks(), cursor);

		// The count-in clicked during the pre-roll (nothing else plays)
		QVERIFY(device->outputPeak().second > 0.05f);
	}

	void theWaveformGrowsWhileRecordingAndIsKept()
	{
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		auto* device = new LoopbackDevice{engine, 256 + 40, 256 + 200};
		engine->setAudioDevice(device, false);

		// Loop passes, so there are segments of their own
		const auto bar = TimePos::ticksPerBar();
		const auto impulsePath = m_directory.filePath("impulse-peaks.wav");
		writeImpulse(impulsePath, engine->outputSampleRate(), 4000, 0);
		auto* player = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		auto* recorder = dynamic_cast<AudioTrack*>(Track::create(Track::Type::Audio, song));
		auto* clip = new SampleClip{player};
		clip->setSampleFile(impulsePath);
		clip->movePosition(TimePos{2 * bar});
		recorder->setInput({AudioInput::Source::Hardware, 0, 1});
		recorder->recordArmModel()->setValue(true);
		recorder->monitorModel()->setValue(static_cast<int>(SampleTrack::MonitorMode::Off));
		auto& timeline = song->getTimeline(Song::PlayMode::Song);
		timeline.setLoopPoints(TimePos{bar}, TimePos{3 * bar});
		timeline.setLoopEnabled(true);
		const auto cleanUp = qScopeGuard([&] {
			song->getTimeline(Song::PlayMode::Song).setLoopEnabled(false);
			song->stop();
			const auto guard = engine->requestChangesGuard();
			delete player;
			delete recorder;
		});

		const auto framesPerTick = static_cast<double>(Engine::framesPerTick());
		song->setPlayPos(bar, Song::PlayMode::Song);
		QVERIFY(song->record());
		const auto passFrames = 2 * bar * framesPerTick;
		auto grown = std::vector<std::size_t>{};
		for (double played = 0; played < 2.5 * passFrames; played += 256)
		{
			device->callback(256);
			if (static_cast<int>(played / 256) % 200 == 0)
			{
				QTest::qWait(150); // (the disk thread writes every so often)
				song->refreshLiveTakes();
				const auto& live = song->liveTakes();
				QCOMPARE(live.size(), std::size_t{1});
				QCOMPARE(live[0].track, static_cast<Track*>(recorder));
				auto peaks = std::size_t{0};
				for (const auto& segment : live[0].segments) { peaks += segment.peaks.size(); }
				grown.push_back(peaks);
			}
		}
		QVERIFY(grown.size() >= 3);
		QVERIFY(std::is_sorted(grown.begin(), grown.end()));
		QVERIFY(grown.back() > grown.front());
		QTest::qWait(150);
		song->refreshLiveTakes();
		QVERIFY(song->liveTakes().front().segments.size() >= 2); // a segment for each pass
		song->stop();
		song->refreshLiveTakes();
		QVERIFY(song->liveTakes().empty());

		// Each take's peaks were kept as it was written: the same as built from its file afterwards
		// (a lane for each pass)
		QVERIFY(recorder->items().size() >= 2);
		for (const auto* item : recorder->items())
		{
			const auto& file = item->file();
			const auto path = PathUtil::toAbsolute(file);
			const auto kept = PeakCache::readPeakFile(path);
			QVERIFY2(kept, qPrintable(file));
			const auto built = PeakCache::build(path);
			QVERIFY(built);
			QCOMPARE(kept->sampleCount(), built->sampleCount());
			const auto& a = kept->nearestThumbnailForWidth(kept->sampleCount());
			const auto& b = built->nearestThumbnailForWidth(built->sampleCount());
			QCOMPARE(a.width(), b.width());
			auto loudest = 0.f;
			for (f_cnt_t peak = 0; peak < a.width(); ++peak)
			{
				QCOMPARE(a[peak].min, b[peak].min);
				QCOMPARE(a[peak].max, b[peak].max);
				loudest = std::max(loudest, a[peak].max);
			}
			QVERIFY(loudest > 0.5f); // (the impulse is in it)
		}
	}

	void loopPassesOnAnAudioTrackBecomeLanes()
	{
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		auto* device = new LoopbackDevice{engine, 256 + 40, 256 + 200};
		engine->setAudioDevice(device, false);

		const auto bar = TimePos::ticksPerBar();
		const auto impulsePath = m_directory.filePath("impulse-lanes.wav");
		writeImpulse(impulsePath, engine->outputSampleRate(), 4000, 0);
		auto* player = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		auto* recorder = dynamic_cast<AudioTrack*>(Track::create(Track::Type::Audio, song));
		QVERIFY(recorder);
		auto* clip = new SampleClip{player};
		clip->setSampleFile(impulsePath);
		clip->movePosition(TimePos{2 * bar});
		// An item recorded before: it goes below the new lanes
		auto* before = recorder->addItem(impulsePath, TimePos{0});
		const auto beforeId = before->id();
		recorder->setInput({AudioInput::Source::Hardware, 0, 1});
		recorder->recordArmModel()->setValue(true);
		recorder->monitorModel()->setValue(static_cast<int>(AudioInputTrack::MonitorMode::Off));
		auto& timeline = song->getTimeline(Song::PlayMode::Song);
		timeline.setLoopPoints(TimePos{bar}, TimePos{3 * bar});
		timeline.setLoopEnabled(true);
		const auto cleanUp = qScopeGuard([&] {
			song->getTimeline(Song::PlayMode::Song).setLoopEnabled(false);
			song->stop();
			const auto guard = engine->requestChangesGuard();
			delete player;
			delete recorder;
		});

		const auto framesPerTick = static_cast<double>(Engine::framesPerTick());
		song->setPlayPos(bar, Song::PlayMode::Song);
		Engine::projectJournal()->setJournalling(true);
		QVERIFY(song->record());
		const auto passFrames = 2 * bar * framesPerTick;
		for (double played = 0; played < 3.5 * passFrames; played += 256) { device->callback(256); }
		song->stop();

		// Three whole passes on three new lanes, the last pass on the lane heard; the item before below them
		const auto items = recorder->items();
		QCOMPARE(items.size(), std::size_t{4});
		QCOMPARE(recorder->item(beforeId)->lane(), 3);
		QCOMPARE(recorder->playingLane(), 0);
		QCOMPARE(recorder->laneCount(), 4);
		auto onLane = std::map<int, AudioItem*>{};
		for (auto* item : items) { if (item->id() != beforeId) { onLane[item->lane()] = item; } }
		QCOMPARE(onLane.size(), std::size_t{3});
		for (const auto& [lane, item] : onLane)
		{
			QCOMPARE(item->startPosition().getTicks(), bar);
			QVERIFY(item->isTempoBound());
			// Each pass heard the impulse at the same place of the loop
			const auto peak = peakFrame(PathUtil::toAbsolute(item->file()));
			QVERIFY(peak >= 0);
			QVERIFY2(std::abs(bar * framesPerTick + peak - 2 * bar * framesPerTick - 1.0) <= 1.0,
				qPrintable(QString{"lane %1: impulse %2 frames off"}.arg(lane).arg(bar * framesPerTick + peak - 2 * bar * framesPerTick)));
		}
		// The passes' files are "<take>.wav", "<take>-2.wav", "<take>-3.wav": the last one heard, the first lowest
		QVERIFY2(onLane[0]->file().endsWith("-3.wav"), qPrintable(onLane[0]->file()));
		QVERIFY2(onLane[1]->file().endsWith("-2.wav"), qPrintable(onLane[1]->file()));

		// One undo takes the recording back
		Engine::projectJournal()->undo();
		QCOMPARE(recorder->items().size(), std::size_t{1});
		QCOMPARE(recorder->items().front()->lane(), 0);
	}

	void withoutLanesTheLastPassReplacesWhatItCovers()
	{
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		auto* device = new LoopbackDevice{engine, 256 + 40, 256 + 200};
		engine->setAudioDevice(device, false);
		ConfigManager::inst()->setValue("recording", "lanes", "0");

		const auto bar = TimePos::ticksPerBar();
		const auto impulsePath = m_directory.filePath("impulse-simple.wav");
		writeImpulse(impulsePath, engine->outputSampleRate(), 4000, 0);
		auto* player = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		auto* recorder = dynamic_cast<AudioTrack*>(Track::create(Track::Type::Audio, song));
		auto* clip = new SampleClip{player};
		clip->setSampleFile(impulsePath);
		clip->movePosition(TimePos{2 * bar});
		// An item over bars 1 to 4: the loop over bars 2 and 3 cuts a hole in it
		auto* before = recorder->addItem(impulsePath, TimePos{0});
		before->changeLength(TimePos{4 * bar});
		recorder->setInput({AudioInput::Source::Hardware, 0, 1});
		recorder->recordArmModel()->setValue(true);
		recorder->monitorModel()->setValue(static_cast<int>(AudioInputTrack::MonitorMode::Off));
		auto& timeline = song->getTimeline(Song::PlayMode::Song);
		timeline.setLoopPoints(TimePos{bar}, TimePos{3 * bar});
		timeline.setLoopEnabled(true);
		const auto cleanUp = qScopeGuard([&] {
			ConfigManager::inst()->setValue("recording", "lanes", "1");
			song->getTimeline(Song::PlayMode::Song).setLoopEnabled(false);
			song->stop();
			const auto guard = engine->requestChangesGuard();
			delete player;
			delete recorder;
		});

		const auto framesPerTick = static_cast<double>(Engine::framesPerTick());
		song->setPlayPos(bar, Song::PlayMode::Song);
		QVERIFY(song->record());
		for (double played = 0; played < 3.5 * 2 * bar * framesPerTick; played += 256) { device->callback(256); }
		song->stop();

		// All on one lane: the item before around the take, the take the last whole pass
		auto items = recorder->items();
		std::sort(items.begin(), items.end(), [](auto* a, auto* b) { return a->startPosition() < b->startPosition(); });
		QCOMPARE(items.size(), std::size_t{3});
		for (const auto* item : items) { QCOMPARE(item->lane(), 0); }
		QCOMPARE(recorder->laneCount(), 1);
		QCOMPARE(items[0]->endPosition().getTicks(), bar);
		QCOMPARE(items[1]->startPosition().getTicks(), bar);
		QVERIFY2(items[1]->file().endsWith("-3.wav"), qPrintable(items[1]->file()));
		QCOMPARE(items[2]->startPosition().getTicks(), items[1]->endPosition().getTicks());
		QCOMPARE(items[2]->endPosition().getTicks(), 4 * bar);
	}

	void theCalibrationFindsTheLatencyTheDeviceDoesNotReport_data()
	{
		QTest::addColumn<int>("hidden");
		QTest::newRow("none") << 0;
		QTest::newRow("37 frames") << 37;
		QTest::newRow("a period and more") << 300;
	}

	void theCalibrationFindsTheLatencyTheDeviceDoesNotReport()
	{
		QFETCH(int, hidden);
		auto* engine = Engine::audioEngine();
		Engine::getSong()->createNewProject();
		auto* device = new LoopbackDevice{engine, 256 + 40, 256 + 200, static_cast<f_cnt_t>(hidden)};
		engine->setAudioDevice(device, false);
		auto calibration = LatencyCalibration{};
		calibration.start();
		for (int callback = 0; callback < 2000 && !calibration.isDone(); ++callback) { device->callback(256); }
		QVERIFY(calibration.isDone());
		const auto result = calibration.result();
		QVERIFY(result.found);
		qInfo("pulse back after %d frames, calibration %lld", static_cast<int>(result.delay), result.offset);
		QCOMPARE(result.offset, static_cast<long long>(hidden));
	}

	void aDropoutIsMarkedOnTheTakeAndTold()
	{
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* device = new LoopbackDevice{engine, 256 + 40, 256 + 200};
		engine->setAudioDevice(device, false);
		auto* recorder = dynamic_cast<AudioTrack*>(Track::create(Track::Type::Audio, song));
		recorder->setInput({AudioInput::Source::Hardware, 0, 1});
		recorder->recordArmModel()->setValue(true);
		recorder->monitorModel()->setValue(static_cast<int>(AudioInputTrack::MonitorMode::Off));
		auto told = QString{};
		const auto connection = connect(song, &Song::recordingProblem, this, [&](const QString& message) { told = message; });

		song->setPlayPos(0, Song::PlayMode::Song);
		QVERIFY(song->record());
		for (int callback = 0; callback < 200; ++callback) { device->callback(256); }
		device->dropOut();
		for (int callback = 0; callback < 200; ++callback) { device->callback(256); }
		song->stop();
		disconnect(connection);

		QCOMPARE(recorder->items().size(), std::size_t{1});
		const auto* take = recorder->items().front();
		QCOMPARE(take->xruns().size(), std::size_t{1});
		// Where it was: 200 periods in, less what the take starts after (the latencies and the engine's two periods)
		const auto at = take->xruns().front();
		QCOMPARE(at, static_cast<std::int64_t>(200 * 256 - (2 * 256 + 256 + 40 + 256 + 200)));
		QVERIFY2(told.contains("xrun"), qPrintable(told));

		// Kept with the project
		auto document = QDomDocument{};
		auto element = document.createElement("audioitem");
		const_cast<AudioItem*>(take)->saveSettings(document, element);
		auto* loaded = static_cast<AudioItem*>(recorder->createClip(TimePos{0}));
		loaded->loadSettings(element);
		QCOMPARE(loaded->xruns(), take->xruns());

		const auto guard = engine->requestChangesGuard();
		delete recorder;
	}

	void aMidiTakeInALaneLeavesWhatWasThereBelow()
	{
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* device = new LoopbackDevice{engine, 256, 256};
		engine->setAudioDevice(device, false);
		const auto bar = TimePos::ticksPerBar();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		track->recordArmModel()->setValue(true);
		track->midiRecordModeModel()->setValue(static_cast<int>(MidiRecorder::Mode::TakeLane));
		auto* existing = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		existing->addNote(Note{TimePos{24}, TimePos{bar + 10}}, false);
		Engine::projectJournal()->setJournalling(true);

		song->setPlayPos(0, Song::PlayMode::Song);
		QVERIFY(song->record());
		auto& recorder = song->midiRecorder();
		recorder.noteOn(track, 64, 90, bar + 50);
		recorder.noteOff(track, 64, bar + 74);
		for (int callback = 0; callback < 10000 && song->heardTick() <= 2 * bar; ++callback) { device->callback(256); }
		song->stop();

		// What was there a lane down, the take on the lane heard
		QCOMPARE(existing->lane(), 1);
		QCOMPARE(track->playingLane(), 0);
		QCOMPARE(track->laneCount(), 2);
		auto* take = static_cast<MidiClip*>(nullptr);
		for (auto* clip : track->getClips()) { if (clip != existing) { take = dynamic_cast<MidiClip*>(clip); } }
		QVERIFY(take);
		QCOMPARE(take->lane(), 0);
		QCOMPARE(take->notes().size(), std::size_t{1});
		QCOMPARE(existing->notes().size(), std::size_t{1}); // (untouched)

		// One undo takes it all back
		Engine::projectJournal()->undo();
		QCOMPARE(track->numOfClips(), 1);
		QCOMPARE(track->getClip(0)->lane(), 0);
		QCOMPARE(track->laneCount(), 1);

		const auto guard = engine->requestChangesGuard();
		delete track;
	}

	void recordedNotesAreQuantizedWhenTheTrackAsks()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		// Off by default: as played
		QCOMPARE(track->quantizedRecordTick(53), 53);
		// To 1/16 (12 ticks), all the way and half way
		track->recordQuantizeModel()->setValue(16);
		QCOMPARE(track->quantizedRecordTick(53), 48);
		QCOMPARE(track->quantizedRecordTick(55), 60);
		track->recordQuantizeStrengthModel()->setValue(50);
		QCOMPARE(track->quantizedRecordTick(53), 51); // (half of the 5 ticks, rounded)

		// A note recorded keeps its length
		track->recordQuantizeStrengthModel()->setValue(100);
		track->recordArmModel()->setValue(true);
		auto& recorder = song->midiRecorder();
		recorder.start({track}, 0);
		recorder.noteOn(track, 60, 100, 53);
		recorder.noteOff(track, 60, 53 + 30);
		recorder.stop(400);
		QCOMPARE(track->numOfClips(), 1);
		const auto* clip = dynamic_cast<MidiClip*>(track->getClip(0));
		QCOMPARE(clip->notes().size(), std::size_t{1});
		QCOMPARE(clip->startPosition().getTicks() + clip->notes().front()->pos().getTicks(), 48);
		QCOMPARE(clip->notes().front()->length().getTicks(), 30);
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		delete track;
	}

	void choosingATakeKeepsThePartOfTheClip()
	{
		auto* song = Engine::getSong();
		const auto first = m_directory.filePath("take-a.wav");
		const auto second = m_directory.filePath("take-b.wav");
		writeImpulse(first, Engine::audioEngine()->outputSampleRate(), 200000, 10);
		writeImpulse(second, Engine::audioEngine()->outputSampleRate(), 200000, 20);
		auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		const auto cleanUp = qScopeGuard([&] {
			const auto guard = Engine::audioEngine()->requestChangesGuard();
			delete track;
		});

		// The second part of a split clip: a later start, an offset into the recording, a shorter length
		auto* clip = new SampleClip{track};
		clip->setSampleFile(first);
		clip->addTake(second);
		clip->movePosition(TimePos{300});
		clip->setAutoResize(false);
		clip->changeLength(TimePos{100});
		clip->setStartTimeOffset(TimePos{-50});

		clip->setActiveTake(0);
		QCOMPARE(clip->activeTake(), 0);
		QCOMPARE(PathUtil::toAbsolute(clip->sampleFile()), first);
		QCOMPARE(clip->startPosition().getTicks(), 300);
		QCOMPARE(clip->length().getTicks(), 100);
		QCOMPARE(clip->startTimeOffset().getTicks(), -50);

		// Saved and loaded, the takes and the active one come back
		auto document = QDomDocument{};
		auto element = document.createElement("sampleclip");
		document.appendChild(element);
		clip->saveSettings(document, element);
		auto* loaded = new SampleClip{track};
		loaded->loadSettings(element);
		QCOMPARE(loaded->takes().size(), qsizetype{2});
		QCOMPARE(loaded->activeTake(), 0);
		QCOMPARE(loaded->startTimeOffset().getTicks(), -50);

		// Removing a take leaves a clip of one sample
		clip->removeTake(0);
		QVERIFY(clip->takes().isEmpty());
		QCOMPARE(PathUtil::toAbsolute(clip->sampleFile()), second);
	}

	void notesPlayedIntoAnArmedTrackLandInItsClips_data()
	{
		QTest::addColumn<int>("mode");
		QTest::newRow("overdub") << static_cast<int>(MidiRecorder::Mode::Overdub);
		QTest::newRow("replace") << static_cast<int>(MidiRecorder::Mode::Replace);
	}

	void notesPlayedIntoAnArmedTrackLandInItsClips()
	{
		QFETCH(int, mode);
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		auto* device = new LoopbackDevice{engine, 256, 256};
		engine->setAudioDevice(device, false);
		const auto bar = TimePos::ticksPerBar();

		auto* empty = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		auto* busy = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		const auto cleanUp = qScopeGuard([&] {
			song->stop();
			const auto guard = engine->requestChangesGuard();
			delete empty;
			delete busy;
		});
		for (auto* track : {empty, busy})
		{
			track->recordArmModel()->setValue(true);
			track->midiRecordModeModel()->setValue(mode);
		}

		// The busy track has a clip over bars 1 and 2 with a note in bar 2
		auto* existing = dynamic_cast<MidiClip*>(busy->createClip(TimePos{0}));
		existing->addNote(Note{TimePos{24}, TimePos{bar + 10}}, false);

		song->setPlayPos(0, Song::PlayMode::Song);
		QVERIFY(song->record());
		QVERIFY(song->isRecording());

		// A note in bar 3 into the empty track, one in bar 2 into the busy one
		auto& recorder = song->midiRecorder();
		recorder.noteOn(empty, 60, 100, 2 * bar + 8);
		recorder.noteOff(empty, 60, 2 * bar + 104);
		recorder.noteOn(busy, 64, 90, bar + 50);
		recorder.noteOff(busy, 64, bar + 74);

		// Play on past bar 3 (at whatever tempo the new project has), then stop
		for (int callback = 0; callback < 10000 && song->heardTick() <= 3 * bar + 48; ++callback) { device->callback(256); }
		QVERIFY(song->heardTick() > 3 * bar);
		song->stop();
		QVERIFY(!song->isRecording());

		// The empty track has a new clip from bar 3, with the note where it was played
		QCOMPARE(empty->numOfClips(), 1);
		const auto* clip = dynamic_cast<MidiClip*>(empty->getClip(0));
		QCOMPARE(clip->startPosition().getTicks(), 2 * bar);
		QCOMPARE(clip->notes().size(), std::size_t{1});
		QCOMPARE(clip->notes()[0]->pos().getTicks(), 8);
		QCOMPARE(clip->notes()[0]->length().getTicks(), 96);
		QCOMPARE(clip->notes()[0]->key(), 60);

		// The busy track's clip took the note; replacing, its old note in the recorded stretch went
		QCOMPARE(busy->numOfClips(), 1);
		QCOMPARE(existing->notes().size(), std::size_t{mode == static_cast<int>(MidiRecorder::Mode::Replace) ? 1u : 2u});
		const bool hasNew = std::any_of(existing->notes().begin(), existing->notes().end(),
			[bar](const Note* note) { return note->pos().getTicks() == bar + 50 && note->key() == 64; });
		QVERIFY(hasNew);
	}

	void oneUndoTakesTheRecordingBack()
	{
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		auto* journal = Engine::projectJournal();
		auto* device = new LoopbackDevice{engine, 256, 256};
		engine->setAudioDevice(device, false);
		const auto bar = TimePos::ticksPerBar();

		auto* first = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		auto* second = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		const auto wasJournalling = journal->isJournalling();
		journal->setJournalling(true);
		const auto cleanUp = qScopeGuard([&] {
			song->stop();
			journal->setJournalling(wasJournalling);
			const auto guard = engine->requestChangesGuard();
			delete first;
			delete second;
		});
		first->recordArmModel()->setValue(true);
		second->recordArmModel()->setValue(true);

		song->setPlayPos(0, Song::PlayMode::Song);
		QVERIFY(song->record());
		auto& recorder = song->midiRecorder();
		recorder.noteOn(first, 60, 100, 10);
		recorder.noteOff(first, 60, 50);
		recorder.noteOn(second, 62, 100, bar + 10);
		recorder.noteOff(second, 62, bar + 50);
		for (int callback = 0; callback < 400; ++callback) { device->callback(256); }
		song->stop();
		QCOMPARE(first->numOfClips(), 1);
		QCOMPARE(second->numOfClips(), 1);

		// One undo: both tracks as they were
		journal->undo();
		QCOMPARE(first->numOfClips(), 0);
		QCOMPARE(second->numOfClips(), 0);

		// One redo: the recording again
		journal->redo();
		QCOMPARE(first->numOfClips(), 1);
		QCOMPARE(second->numOfClips(), 1);
	}

	void aChangeOfTempoDoesNotSplitATake()
	{
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		auto* device = new LoopbackDevice{engine, 256 + 30, 256 + 60};
		engine->setAudioDevice(device, false);
		auto* recorder = dynamic_cast<AudioTrack*>(Track::create(Track::Type::Audio, song));
		recorder->setInput({AudioInput::Source::Hardware, 0, 1});
		recorder->recordArmModel()->setValue(true);
		recorder->monitorModel()->setValue(static_cast<int>(SampleTrack::MonitorMode::Off));
		const auto tempo = song->getTempo();
		const auto cleanUp = qScopeGuard([&] {
			song->stop();
			song->tempoModel().setValue(tempo);
			const auto guard = engine->requestChangesGuard();
			delete recorder;
		});

		// The tempo changes twice while recording
		song->setPlayPos(0, Song::PlayMode::Song);
		QVERIFY(song->record());
		for (int callback = 0; callback < 200; ++callback) { device->callback(256); }
		song->tempoModel().setValue(97);
		for (int callback = 0; callback < 200; ++callback) { device->callback(256); }
		song->tempoModel().setValue(173);
		for (int callback = 0; callback < 200; ++callback) { device->callback(256); }
		song->stop();

		// One take, all of it
		QCOMPARE(recorder->numOfClips(), 1);
	}

	void movingATakeAtTheSameTempoNeedsNoRendering()
	{
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		auto* device = new LoopbackDevice{engine, 256 + 30, 256 + 60};
		engine->setAudioDevice(device, false);
		auto* recorder = dynamic_cast<AudioTrack*>(Track::create(Track::Type::Audio, song));
		recorder->setInput({AudioInput::Source::Hardware, 0, 1});
		recorder->recordArmModel()->setValue(true);
		recorder->monitorModel()->setValue(static_cast<int>(SampleTrack::MonitorMode::Off));
		const auto tempo = song->getTempo();
		const auto cleanUp = qScopeGuard([&] {
			song->stop();
			song->tempoModel().setValue(tempo);
			const auto guard = engine->requestChangesGuard();
			delete recorder;
		});

		// A take whose end falls inside a tick (a few frames past one)
		song->setPlayPos(0, Song::PlayMode::Song);
		QVERIFY(song->record());
		for (int callback = 0; callback < 437; ++callback) { device->callback(256); }
		song->stop();
		QCOMPARE(recorder->numOfClips(), 1);
		auto* take = dynamic_cast<AudioItem*>(recorder->getClip(0));
		QVERIFY(take->isTempoBound());
		QVERIFY(song->stretchJobs().empty());

		// Moved, at the same tempo: nothing to render
		take->movePosition(TimePos{take->startPosition().getTicks() + 3 * TimePos::ticksPerBar() + 7});
		QVERIFY(song->stretchJobs().empty());

		// At another tempo: rendered
		song->tempoModel().setValue(tempo + 13);
		QCOMPARE(song->stretchJobs().size(), std::size_t{1});
	}

	void monitoringPlaysTheInputThroughTheTrack_data()
	{
		QTest::addColumn<int>("mode");
		QTest::addColumn<bool>("armed");
		QTest::addColumn<bool>("heard");
		QTest::newRow("off") << static_cast<int>(SampleTrack::MonitorMode::Off) << true << false;
		QTest::newRow("on") << static_cast<int>(SampleTrack::MonitorMode::On) << false << true;
		QTest::newRow("auto, armed, stopped") << static_cast<int>(SampleTrack::MonitorMode::Auto) << true << true;
		QTest::newRow("auto, not armed") << static_cast<int>(SampleTrack::MonitorMode::Auto) << false << false;
	}

	void monitoringPlaysTheInputThroughTheTrack()
	{
		QFETCH(int, mode);
		QFETCH(bool, armed);
		QFETCH(bool, heard);

		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		auto* device = new LoopbackDevice{engine, 0, 0};
		engine->setAudioDevice(device, false);
		auto* track = dynamic_cast<AudioTrack*>(Track::create(Track::Type::Audio, song));
		track->setInput({AudioInput::Source::Hardware, 1, 1});
		track->monitorModel()->setValue(mode);
		track->recordArmModel()->setValue(armed);

		// A 1 kHz tone into input 2, with the song stopped
		const auto period = engine->framesPerPeriod();
		auto tone = std::vector<float>(period);
		auto silence = std::vector<float>(period, 0.f);
		const float* input[] = {silence.data(), tone.data()};
		auto output = std::vector<float>(2 * period);
		double energy = 0;
		for (int callback = 0; callback < 40; ++callback)
		{
			for (f_cnt_t frame = 0; frame < period; ++frame)
			{
				tone[frame] = 0.25f * std::sin(2 * 3.14159265 * 1000 * (callback * period + frame) / engine->outputSampleRate());
			}
			engine->renderNextBuffer(PlanarBufferView<const float>{input, 2, period},
				InterleavedBufferView<float>{output.data(), 2, period});
			if (callback >= 20)
			{
				for (const auto sample : output) { energy += sample * sample; }
			}
		}
		const auto rms = std::sqrt(energy / (20.0 * 2 * period));
		qInfo("output rms %.4f", rms);

		// The input meter sees the input whether it is heard or not
		QVERIFY(std::abs(track->takeInputPeak() - 0.25f) < 0.01f);
		QCOMPARE(track->takeInputPeak(), 0.f); // taken
		if (heard) { QVERIFY(rms > 0.05); }
		else { QVERIFY(rms < 1e-6); }

		const auto guard = engine->requestChangesGuard();
		delete track;
	}

	void nothingArmedRecordsNothing()
	{
		auto problem = QString{};
		auto* song = Engine::getSong();
		const auto connection = connect(song, &Song::recordingProblem, this, [&](const QString& m) { problem = m; });
		QVERIFY(!song->record());
		QVERIFY(!problem.isEmpty());
		QVERIFY(!song->isRecordingAudio());
		disconnect(connection);
	}

private:
	QTemporaryDir m_directory;
};

QTEST_GUILESS_MAIN(RecordingTest)
#include "RecordingTest.moc"
