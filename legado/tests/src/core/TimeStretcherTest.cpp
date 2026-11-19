/*
 * TimeStretcherTest.cpp
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
#include <numbers>
#include <vector>

#include <QDir>
#include <QObject>
#include <QScopeGuard>
#include <QTemporaryDir>
#include <QtTest>
#include <sndfile.h>

#include "AudioEngine.h"
#include "Engine.h"
#include "PathUtil.h"
#include "SampleClip.h"
#include "SampleTrack.h"
#include "Song.h"
#include "TimeStretcher.h"

using namespace lmms;

namespace
{

constexpr auto Rate = 48000;

//! Writes a stereo float WAV: a sine of @p hz for the first @p soundFrames frames, then silence up to @p frames
void writeSine(const QString& path, int frames, int soundFrames, double hz, int rate = Rate)
{
	auto info = SF_INFO{};
	info.samplerate = rate;
	info.channels = 2;
	info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
	auto* file = sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
	QVERIFY(file);
	auto samples = std::vector<float>(static_cast<std::size_t>(frames) * 2, 0.f);
	for (int f = 0; f < soundFrames; ++f)
	{
		const auto value = static_cast<float>(0.5 * std::sin(2 * std::numbers::pi * hz * f / rate));
		samples[f * 2] = samples[f * 2 + 1] = value;
	}
	sf_writef_float(file, samples.data(), frames);
	sf_close(file);
}

struct Audio
{
	int rate = 0;
	std::vector<float> left;
};

auto readLeft(const QString& path) -> Audio
{
	auto info = SF_INFO{};
	auto* file = sf_open(QFile::encodeName(path).constData(), SFM_READ, &info);
	if (!file) { return {}; }
	auto samples = std::vector<float>(static_cast<std::size_t>(info.frames * info.channels));
	sf_readf_float(file, samples.data(), info.frames);
	sf_close(file);
	auto audio = Audio{info.samplerate, {}};
	for (sf_count_t f = 0; f < info.frames; ++f) { audio.left.push_back(samples[f * info.channels]); }
	return audio;
}

//! @returns the frequency of the signal between @p from and @p to, from its rising zero crossings
auto frequency(const std::vector<float>& samples, std::size_t from, std::size_t to, int rate) -> double
{
	auto first = -1.0;
	auto last = -1.0;
	auto crossings = 0;
	for (auto f = from + 1; f < to; ++f)
	{
		if (samples[f - 1] < 0.f && samples[f] >= 0.f)
		{
			// Where it crosses, between the two frames
			const auto at = static_cast<double>(f - 1) + samples[f - 1] / (samples[f - 1] - samples[f]);
			if (first < 0) { first = at; } else { ++crossings; }
			last = at;
		}
	}
	return crossings > 0 ? crossings * rate / (last - first) : 0.0;
}

//! @returns the first frame after @p from where the level (RMS over 10 ms) falls under -40 dBFS
auto endOfSound(const std::vector<float>& samples, std::size_t from) -> std::size_t
{
	constexpr auto Window = std::size_t{480};
	for (auto f = from; f + Window < samples.size(); f += 48)
	{
		auto sum = 0.0;
		for (auto i = f; i < f + Window; ++i) { sum += samples[i] * samples[i]; }
		if (std::sqrt(sum / Window) < 0.01) { return f; }
	}
	return samples.size();
}

const auto NoProgress = [](double) {};

} // namespace

class TimeStretcherTest : public QObject
{
	Q_OBJECT

private:
	QTemporaryDir m_directory;

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

	void aSlowerTempoMakesItLongerAtTheSamePitch()
	{
		const auto source = m_directory.filePath("sine.wav");
		const auto target = m_directory.filePath("st100_sine.wav");
		writeSine(source, 96000, 96000, 440.0);

		auto cancel = std::atomic<bool>{false};
		auto lastProgress = 0.0;
		auto progressWentBack = false;
		const auto result = TimeStretcher::render(source, target, {{0, 0.0}, {384, 96000.0}}, {{0, 0.0}, {384, 120000.0}},
			Rate, cancel, [&](double progress) {
				progressWentBack = progressWentBack || progress < lastProgress;
				lastProgress = progress;
			});
		QCOMPARE(result.status, TimeStretcher::Status::Done);
		QVERIFY(!progressWentBack);
		QCOMPARE(lastProgress, 1.0);

		const auto audio = readLeft(target);
		QCOMPARE(audio.rate, Rate);
		QCOMPARE(audio.left.size(), std::size_t{120000});
		QVERIFY(!QFileInfo::exists(target + ".part"));

		const auto hz = frequency(audio.left, 20000, 100000, Rate);
		if (TimeStretcher::canKeepPitch()) { QVERIFY2(std::abs(hz - 440.0) < 2.0, qPrintable(QString::number(hz))); }
		else { QVERIFY2(std::abs(hz - 440.0 * 96000 / 120000) < 2.0, qPrintable(QString::number(hz))); }
	}

	void everyPointLandsWhereTheNewTimingPutsIt()
	{
		// Sound for the first half, silence for the second: the first half is stretched by 1.5, the second squeezed
		const auto source = m_directory.filePath("half.wav");
		const auto target = m_directory.filePath("st_half.wav");
		writeSine(source, 96000, 48000, 330.0);

		auto cancel = std::atomic<bool>{false};
		const auto result = TimeStretcher::render(source, target, {{0, 0.0}, {192, 48000.0}, {384, 96000.0}},
			{{0, 0.0}, {192, 72000.0}, {384, 100000.0}}, Rate, cancel, NoProgress);
		QCOMPARE(result.status, TimeStretcher::Status::Done);

		const auto audio = readLeft(target);
		QCOMPARE(audio.left.size(), std::size_t{100000});
		// The sound ends where the point now is, give or take a window of the stretcher (~20 ms)
		const auto end = endOfSound(audio.left, 1000);
		QVERIFY2(std::abs(static_cast<double>(end) - 72000.0) < 1000.0, qPrintable(QString::number(end)));
	}

	void theSongsRateIsTheRenderingsRate()
	{
		// A recording made at 44.1 kHz, rendered for a song at 48 kHz at the same tempo: same length in time and pitch
		const auto source = m_directory.filePath("sine44.wav");
		const auto target = m_directory.filePath("st_sine44.wav");
		writeSine(source, 88200, 88200, 440.0, 44100);

		auto cancel = std::atomic<bool>{false};
		const auto result = TimeStretcher::render(source, target, {{0, 0.0}, {384, 88200.0}}, {{0, 0.0}, {384, 96000.0}},
			Rate, cancel, NoProgress);
		QCOMPARE(result.status, TimeStretcher::Status::Done);

		const auto audio = readLeft(target);
		QCOMPARE(audio.rate, Rate);
		QCOMPARE(audio.left.size(), std::size_t{96000});
		if (TimeStretcher::canKeepPitch())
		{
			const auto hz = frequency(audio.left, 10000, 86000, Rate);
			QVERIFY2(std::abs(hz - 440.0) < 2.0, qPrintable(QString::number(hz)));
		}
	}

	void aCancelledRenderLeavesNothing()
	{
		const auto source = m_directory.filePath("cancel.wav");
		const auto target = m_directory.filePath("st_cancel.wav");
		writeSine(source, 96000, 96000, 440.0);

		auto cancel = std::atomic<bool>{false};
		const auto result = TimeStretcher::render(source, target, {{0, 0.0}, {384, 96000.0}}, {{0, 0.0}, {384, 80000.0}},
			Rate, cancel, [&](double progress) { if (progress > 0.1) { cancel = true; } });
		QCOMPARE(result.status, TimeStretcher::Status::Cancelled);
		QVERIFY(!QFileInfo::exists(target));
		QVERIFY(!QFileInfo::exists(target + ".part"));
	}

	void badInputFails()
	{
		auto cancel = std::atomic<bool>{false};
		const auto target = m_directory.filePath("st_bad.wav");
		QCOMPARE(TimeStretcher::render(m_directory.filePath("missing.wav"), target, {{0, 0.0}, {384, 100.0}},
			{{0, 0.0}, {384, 200.0}}, Rate, cancel, NoProgress).status, TimeStretcher::Status::Failed);

		const auto source = m_directory.filePath("bad.wav");
		writeSine(source, 1000, 1000, 440.0);
		// The ticks of the two timings must be the same points
		QCOMPARE(TimeStretcher::render(source, target, {{0, 0.0}, {384, 1000.0}}, {{0, 0.0}, {192, 1000.0}}, Rate,
			cancel, NoProgress).status, TimeStretcher::Status::Failed);
		QVERIFY(!QFileInfo::exists(target));
	}

	void aTempoChangeMakesAJobAndTheOldTempoUndoesIt()
	{
		auto* song = Engine::getSong();
		song->tempoModel().setValue(120);
		const auto rate = Engine::audioEngine()->outputSampleRate();
		const auto source = m_directory.filePath("take.wav");
		writeSine(source, static_cast<int>(rate * 2), static_cast<int>(rate * 2), 440.0, static_cast<int>(rate));

		auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		const auto cleanUp = qScopeGuard([&] {
			const auto guard = Engine::audioEngine()->requestChangesGuard();
			Track::destroy(track);
			song->tempoModel().setValue(120);
		});
		auto* clip = new SampleClip{track};
		clip->setSampleFile(source);
		clip->setOriginalTiming(song->timingOf(0, rate * 2.0), source);

		// At the tempo it was played at there is nothing to do
		QVERIFY(song->stretchJobs().empty());

		// Slower: one job, from the original, to a longer timing with the same points
		song->tempoModel().setValue(100);
		auto jobs = song->stretchJobs();
		QCOMPARE(jobs.size(), std::size_t{1});
		QCOMPARE(jobs[0].clip, clip);
		QCOMPARE(PathUtil::toAbsolute(jobs[0].source), source);
		QCOMPARE(jobs[0].from.size(), jobs[0].to.size());
		QVERIFY(std::abs(jobs[0].to.back().frame - rate * 2.0 * 120 / 100) < 2.0);
		QVERIFY(QFileInfo{jobs[0].target}.fileName().startsWith("st100_"));

		// Rendered and played: no more jobs at this tempo
		auto cancel = std::atomic<bool>{false};
		QCOMPARE(TimeStretcher::render(jobs[0].source.isEmpty() ? source : PathUtil::toAbsolute(jobs[0].source),
			jobs[0].target, jobs[0].from, jobs[0].to, rate, cancel, NoProgress).status, TimeStretcher::Status::Done);
		clip->setRendering(jobs[0].target, jobs[0].to);
		QVERIFY(song->stretchJobs().empty());
		QCOMPARE(PathUtil::toAbsolute(clip->sampleFile()), jobs[0].target);

		// The second part of a split clip (later in the song, later in the recording) shares the rendering
		song->tempoModel().setValue(90);
		auto* part = new SampleClip{track};
		part->setSampleFile(clip->sampleFile());
		part->setOriginalTiming(clip->originalTiming(), source);
		part->movePosition(TimePos{192});
		part->setAutoResize(false);
		part->changeLength(TimePos{96});
		part->setStartTimeOffset(TimePos{-192});
		jobs = song->stretchJobs();
		QCOMPARE(jobs.size(), std::size_t{2});
		QCOMPARE(jobs[0].target, jobs[1].target);
		QCOMPARE(jobs[0].to, jobs[1].to);

		// A rendered part plays, from its start, the frame its tick has in the rendering
		QCOMPARE(TimeStretcher::render(source, jobs[0].target, jobs[0].from, jobs[0].to, rate, cancel, NoProgress).status,
			TimeStretcher::Status::Done);
		for (const auto& job : jobs) { Song::applyStretch(job); }
		QVERIFY(song->stretchJobs().empty());
		QVERIFY(std::abs(static_cast<double>(part->fileFrameAt(192)) - song->framesBetween(0, 192)) <= 1.0);

		// Back to the tempo it was played at: the original again, without a job
		song->tempoModel().setValue(120);
		QVERIFY(song->stretchJobs().empty());
		QCOMPARE(PathUtil::toAbsolute(clip->sampleFile()), source);
	}
};

QTEST_GUILESS_MAIN(TimeStretcherTest)
#include "TimeStretcherTest.moc"
