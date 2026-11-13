/*
 * MetronomeTest.cpp
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

#include <QObject>
#include <QtTest>

#include "AudioEngine.h"
#include "ConfigManager.h"
#include "Engine.h"
#include "Metronome.h"
#include "Song.h"

using namespace lmms;

class MetronomeTest : public QObject
{
	Q_OBJECT
private:
	//! The loudest the song is over a bar from its start, with the metronome on
	auto loudestOverABar() -> float
	{
		auto* song = Engine::getSong();
		auto* engine = Engine::audioEngine();
		// (half a second stopped: a click still sounding from before ends)
		for (int period = 0; period < 100; ++period) { engine->renderNextPeriod(); }
		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		const auto frames = static_cast<int>(TimePos::ticksPerBar() * Engine::framesPerTick());
		auto loudest = 0.f;
		for (int done = 0; done < frames + 2048; done += engine->framesPerPeriod())
		{
			for (const auto& frame : engine->renderNextPeriod()) { loudest = std::max(loudest, std::abs(frame.left())); }
		}
		song->stop();
		return loudest;
	}

	//! The loudest the first beat's click is
	auto firstClick() -> float
	{
		auto* song = Engine::getSong();
		auto* engine = Engine::audioEngine();
		// (half a second stopped: a click still sounding from before ends)
		for (int period = 0; period < 100; ++period) { engine->renderNextPeriod(); }
		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		const auto frames = static_cast<int>(TimePos::ticksPerBar() / 4 * Engine::framesPerTick() * 0.9);
		auto loudest = 0.f;
		for (int done = 0; done < frames; done += engine->framesPerPeriod())
		{
			for (const auto& frame : engine->renderNextPeriod()) { loudest = std::max(loudest, std::abs(frame.left())); }
		}
		song->stop();
		return loudest;
	}

	void setting(const char* name, const QString& value)
	{
		ConfigManager::inst()->setValue("metronome", name, value);
		Engine::getSong()->metronome().loadSettings();
	}

private slots:
	void initTestCase()
	{
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
		Engine::getSong()->createNewProject();
		Engine::getSong()->metronome().setActive(true);
	}

	void cleanupTestCase()
	{
		Engine::getSong()->metronome().setActive(false);
		Engine::destroy();
	}

	void itClicksAtItsVolume()
	{
		setting("onlyrecording", "0");
		setting("accent", "1");
		setting("volume", "0");
		const auto unity = loudestOverABar();
		QVERIFY(unity > 0.05f);
		setting("volume", "-6.0206");
		const auto half = loudestOverABar();
		QVERIFY(std::abs(half / unity - 0.5f) < 0.01f);
		setting("volume", "0");
	}

	void onlyWhileRecordingItIsSilentWhilePlaying()
	{
		setting("onlyrecording", "1");
		QCOMPARE(loudestOverABar(), 0.f);
		setting("onlyrecording", "0");
		QVERIFY(loudestOverABar() > 0.05f);
	}

	void withoutAccentTheFirstBeatClicksAsTheOthers()
	{
		setting("accent", "1");
		const auto accented = firstClick();
		setting("accent", "0");
		const auto plain = firstClick();
		QVERIFY(accented > 0.f && plain > 0.f);
		QVERIFY(std::abs(accented - plain) > 1e-3f); // (the two clicks are different sounds)
		setting("accent", "1");
	}
};

QTEST_GUILESS_MAIN(MetronomeTest)
#include "MetronomeTest.moc"
