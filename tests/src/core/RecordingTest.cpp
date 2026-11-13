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

#include <cmath>
#include <vector>

#include <sndfile.h>

#include "AudioDevice.h"
#include "AudioEngine.h"
#include "AudioRecorder.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "MidiRecorder.h"
#include "ConfigManager.h"
#include "PathUtil.h"
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
	LoopbackDevice(AudioEngine* engine, f_cnt_t inputLatency, f_cnt_t outputLatency)
		: AudioDevice{2, engine}
		, m_inputLatency{inputLatency}
		, m_outputLatency{outputLatency}
	{
		setInputChannels(2);
	}

	auto inputLatency() const -> f_cnt_t override { return m_inputLatency; }
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
			const auto source = device - static_cast<std::int64_t>(m_inputLatency + m_outputLatency);
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
		auto* recorder = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
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
			const auto* take = dynamic_cast<SampleClip*>(recorder->getClip(0));
			const auto peak = peakFrame(PathUtil::toAbsolute(take->sampleFile()));
			if (peak < 0) { qWarning("no impulse in %s", qPrintable(take->sampleFile())); }
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
		auto* recorder = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
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
		const auto* take = dynamic_cast<SampleClip*>(recorder->getClip(0));

		// The take starts where the punch in was heard: two periods and the latencies before bar 2
		const auto expectedStart = (192 * framesPerTick - 2 * 256.0 - (50 + 300 + 512)) / framesPerTick;
		QVERIFY(std::abs(take->startPosition().getTicks() - expectedStart) <= 1.0);
		const auto peak = peakFrame(PathUtil::toAbsolute(take->sampleFile()));
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
		auto* recorder = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
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

	void loopPassesBecomeTakesOfOneClip()
	{
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		auto* device = new LoopbackDevice{engine, 256 + 40, 256 + 200};
		engine->setAudioDevice(device, false);

		// A loop over bars 2 to 4, an impulse in it at bar 3
		const auto bar = TimePos::ticksPerBar();
		const auto impulsePath = m_directory.filePath("impulse-loop.wav");
		writeImpulse(impulsePath, engine->outputSampleRate(), 4000, 0);
		auto* player = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		auto* recorder = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		auto* clip = new SampleClip{player};
		clip->setSampleFile(impulsePath);
		clip->movePosition(TimePos{2 * bar});
		recorder->setInput({AudioInput::Source::Hardware, 0, 1});
		recorder->recordArmModel()->setValue(true);
		// Monitoring would send the input back out through the cable, round and round
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

		// From the loop's start, three passes and a half
		const auto framesPerTick = static_cast<double>(Engine::framesPerTick());
		song->setPlayPos(bar, Song::PlayMode::Song);
		QVERIFY(song->record());
		const auto passFrames = 2 * bar * framesPerTick;
		for (double played = 0; played < 3.5 * passFrames; played += 256) { device->callback(256); }
		song->stop();

		// One clip at the loop's start, with a take for each whole pass, the last one active
		QCOMPARE(recorder->numOfClips(), 1);
		const auto* take = dynamic_cast<SampleClip*>(recorder->getClip(0));
		QCOMPARE(take->startPosition().getTicks(), bar);
		QCOMPARE(take->takes().size(), qsizetype{3});
		QCOMPARE(take->activeTake(), 2);

		// Each pass heard the impulse at the same place of the loop: each take holds it there
		const auto heardAt = 2 * bar * framesPerTick; // the impulse, on the song's timeline
		for (const auto& file : take->takes())
		{
			const auto peak = peakFrame(PathUtil::toAbsolute(file));
			QVERIFY2(peak >= 0, qPrintable(file));
			const auto recordedAt = bar * framesPerTick + static_cast<double>(peak);
			qInfo("%s: impulse %.2f frames from where it was heard", qPrintable(QFileInfo{file}.fileName()),
				recordedAt - heardAt);
			// The sample playback is a frame late (Sample::play's resampler): the take holds what was heard
			QVERIFY(std::abs(recordedAt - heardAt - 1.0) <= 1.0);
		}
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

		// Play on past bar 3, then stop
		for (int callback = 0; callback < 1000; ++callback) { device->callback(256); }
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
		auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
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
