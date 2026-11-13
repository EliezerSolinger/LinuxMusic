/*
 * GainStagingTest.cpp
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

#include <QDomDocument>
#include <QObject>
#include <QScopeGuard>
#include <QTemporaryDir>
#include <QtTest>
#include <sndfile.h>

#include "AudioEngine.h"
#include "Engine.h"
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "NotePlayHandle.h"
#include "Mixer.h"
#include "SampleClip.h"
#include "SampleTrack.h"
#include "Song.h"
#include "lmms_math.h"

using namespace lmms;

namespace
{

constexpr auto Level = 0.25f; //!< the test sine's peak, -12 dBFS

void writeSine(const QString& path, int channels, int rate, int frames)
{
	auto info = SF_INFO{};
	info.samplerate = rate;
	info.channels = channels;
	info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
	auto* file = sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
	QVERIFY(file);
	auto samples = std::vector<float>(static_cast<std::size_t>(frames * channels));
	for (int f = 0; f < frames; ++f)
	{
		const auto value = Level * static_cast<float>(std::sin(2 * std::numbers::pi * 1000.0 * f / rate));
		for (int c = 0; c < channels; ++c) { samples[f * channels + c] = value; }
	}
	sf_writef_float(file, samples.data(), frames);
	sf_close(file);
}

//! Plays the song from its start and @returns the level of the master's output (left, right) relative to the
//! test sine's, in dB, measured over its second quarter second
auto playedLevel() -> std::pair<float, float>
{
	auto* song = Engine::getSong();
	auto* engine = Engine::audioEngine();
	song->playSong();
	auto sum = std::array<double, 2>{};
	auto frames = 0;
	const auto rate = static_cast<int>(engine->outputSampleRate());
	auto played = 0;
	while (played < rate / 2)
	{
		const auto period = engine->renderNextPeriod();
		for (const auto& frame : period)
		{
			if (played >= rate / 4)
			{
				sum[0] += frame.left() * frame.left();
				sum[1] += frame.right() * frame.right();
				++frames;
			}
			++played;
		}
	}
	song->stop();
	const auto reference = Level / std::sqrt(2.0);
	return {static_cast<float>(20 * std::log10(std::sqrt(sum[0] / frames) / reference)),
		static_cast<float>(20 * std::log10(std::sqrt(sum[1] / frames) / reference))};
}

} // namespace

/**
 * Unity gain from end to end: a sound played by a track with every control at its default comes out of the master
 * at the level it went in (the user's notes of 13/11 suspect a -6 dB somewhere).
 */
class GainStagingTest : public QObject
{
	Q_OBJECT

private:
	QTemporaryDir m_directory;

private slots:
	void initTestCase()
	{
		QVERIFY(m_directory.isValid());
		NotePlayHandleManager::init(); // as main() does
		// The build's plugins (next to the test programs' folder)
		qputenv("LMMS_PLUGIN_DIR", QFile::encodeName(QCoreApplication::applicationDirPath() + "/../plugins"));
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
		NotePlayHandleManager::free();
	}

	void init()
	{
		Engine::getSong()->createNewProject();
	}

	void aSampleClipPlaysAtUnity_data()
	{
		QTest::addColumn<bool>("analog");
		QTest::addColumn<int>("channels");
		QTest::newRow("digital, stereo") << false << 2;
		QTest::newRow("digital, mono") << false << 1;
		QTest::newRow("analog, stereo") << true << 2;
		QTest::newRow("analog, mono") << true << 1;
	}

	void aSampleClipPlaysAtUnity()
	{
		QFETCH(bool, analog);
		QFETCH(int, channels);
		auto* song = Engine::getSong();
		Engine::mixer()->analogModel()->setValue(analog);
		const auto path = m_directory.filePath(QString{"sine%1.wav"}.arg(channels));
		writeSine(path, channels, static_cast<int>(Engine::audioEngine()->outputSampleRate()),
			static_cast<int>(Engine::audioEngine()->outputSampleRate()));

		auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		Engine::mixer()->setUpTrackChannels();
		auto* clip = new SampleClip{track};
		clip->setSampleFile(path);

		const auto [left, right] = playedLevel();
		qInfo("sample clip: %s, %d channel(s): L %+.2f dB, R %+.2f dB", analog ? "analog" : "digital", channels, left, right);
		QVERIFY2(std::abs(left) < 0.5f && std::abs(right) < 0.5f, qPrintable(QString{"%1 %2"}.arg(left).arg(right)));
	}

	void aTrackGoesUpToTwelveDecibels()
	{
		auto* song = Engine::getSong();
		const auto path = m_directory.filePath("loud.wav");
		writeSine(path, 2, static_cast<int>(Engine::audioEngine()->outputSampleRate()),
			static_cast<int>(Engine::audioEngine()->outputSampleRate()));
		auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		Engine::mixer()->setUpTrackChannels();
		auto* clip = new SampleClip{track};
		clip->setSampleFile(path);

		// Its whole range loads (the top once wrapped around in an 8-bit volume_t)
		auto* volume = track->volumeModel();
		QVERIFY(std::abs(20.f * std::log10(volume->maxValue() / 100.f) - 12.f) < 0.1f);
		volume->setInitValue(150.f);
		QCOMPARE(volume->value(), 150.f);
		volume->setValue(volume->maxValue());
		const auto [left, right] = playedLevel();
		QVERIFY2(std::abs(left - 12.04f) < 0.1f && std::abs(right - 12.04f) < 0.1f,
			qPrintable(QString{"%1 %2"}.arg(left).arg(right)));
	}

	void anInstrumentPlaysAtUnity_data()
	{
		QTest::addColumn<bool>("analog");
		QTest::newRow("digital") << false;
		QTest::newRow("analog") << true;
	}

	void anInstrumentPlaysAtUnity()
	{
		QFETCH(bool, analog);
		auto* song = Engine::getSong();
		Engine::mixer()->analogModel()->setValue(analog);
		const auto path = m_directory.filePath("afp.wav");
		writeSine(path, 2, static_cast<int>(Engine::audioEngine()->outputSampleRate()),
			static_cast<int>(Engine::audioEngine()->outputSampleRate()));

		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		Engine::mixer()->setUpTrackChannels();
		auto* instrument = track->loadInstrument("audiofileprocessor");
		QVERIFY(instrument);
		QCOMPARE(QString{instrument->descriptor()->name}, QString{"audiofileprocessor"});
		auto document = QDomDocument{};
		auto element = document.createElement("audiofileprocessor");
		element.setAttribute("src", path);
		element.setAttribute("amp", 100);
		element.setAttribute("sframe", 0);
		element.setAttribute("eframe", 1);
		element.setAttribute("lframe", 0);
		element.setAttribute("looped", 0);
		element.setAttribute("reversed", 0);
		element.setAttribute("stutter", 0);
		element.setAttribute("interp", 0);
		instrument->restoreState(element);

		// A note at the default volume, at the sample's own pitch (A4 plays it as it is)
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		clip->addNote(Note{TimePos{192}, TimePos{0}, DefaultKey}, false);

		const auto [left, right] = playedLevel();
		qInfo("instrument: %s: L %+.2f dB, R %+.2f dB", analog ? "analog" : "digital", left, right);
		QVERIFY2(std::abs(left) < 0.5f && std::abs(right) < 0.5f, qPrintable(QString{"%1 %2"}.arg(left).arg(right)));
	}
};

QTEST_GUILESS_MAIN(GainStagingTest)
#include "GainStagingTest.moc"
