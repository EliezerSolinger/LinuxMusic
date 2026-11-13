/*
 * SampleTrackTest.cpp
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

#include <QObject>
#include <QTemporaryDir>
#include <QtTest>

#include <algorithm>
#include <cmath>
#include <vector>

#include <sndfile.h>

#include "AudioEngine.h"
#include "Engine.h"
#include "Mixer.h"
#include "SampleClip.h"
#include "SampleTrack.h"
#include "Song.h"

using namespace lmms;

class SampleTrackTest : public QObject
{
	Q_OBJECT

	QTemporaryDir m_directory;

	//! A stereo file: @p silentFrames of silence, then @p loudFrames at 0.5
	auto writeHalfSilent(int silentFrames, int loudFrames) -> QString
	{
		const auto path = m_directory.filePath("half.wav");
		auto info = SF_INFO{};
		info.samplerate = static_cast<int>(Engine::audioEngine()->outputSampleRate());
		info.channels = 2;
		info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
		auto* file = sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
		auto samples = std::vector<float>(static_cast<std::size_t>(silentFrames + loudFrames) * 2, 0.f);
		std::fill(samples.begin() + 2 * silentFrames, samples.end(), 0.5f);
		sf_writef_float(file, samples.data(), silentFrames + loudFrames);
		sf_close(file);
		return path;
	}

	static auto loudest(std::span<const SampleFrame> period) -> float
	{
		auto peak = 0.f;
		for (const auto& frame : period) { peak = std::max({peak, std::abs(frame[0]), std::abs(frame[1])}); }
		return peak;
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

	//! The play head jumps while the song plays: the clip plays from the new place in the very next period (it went
	//! on from the old one until the main thread delivered a signal)
	void aJumpPlaysTheClipFromTheNewPlace()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		const auto bar = TimePos::ticksPerBar();
		const auto framesPerBar = static_cast<int>(bar * Engine::framesPerTick());
		const auto file = writeHalfSilent(framesPerBar, framesPerBar);
		auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		auto* clip = dynamic_cast<SampleClip*>(track->createClip(TimePos{0}));
		clip->setSampleFile(file);
		clip->changeLength(TimePos{2 * bar});
		Engine::mixer()->setUpTrackChannels();

		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		for (int period = 0; period < 4; ++period)
		{
			QCOMPARE(loudest(Engine::audioEngine()->renderNextPeriod()), 0.f);
		}

		// Into the loud half
		song->setPlayPos(bar + bar / 2, Song::PlayMode::Song);
		Engine::audioEngine()->renderNextPeriod();
		const auto afterJump = loudest(Engine::audioEngine()->renderNextPeriod());
		song->stop();
		QVERIFY2(afterJump > 0.1f, qPrintable(QString::number(afterJump)));
	}
};

QTEST_GUILESS_MAIN(SampleTrackTest)
#include "SampleTrackTest.moc"
