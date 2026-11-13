/*
 * TempoMapTest.cpp
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
#include <QtTest>

#include "AudioEngine.h"
#include "AutomationClip.h"
#include "Engine.h"
#include "Song.h"
#include "TempoMap.h"
#include "TimePos.h"

using namespace lmms;

class TempoMapTest : public QObject
{
	Q_OBJECT

private:
	//! Plays the song from its start until @p tick, period by period
	static void playUntil(tick_t tick)
	{
		auto* song = Engine::getSong();
		while (song->getPlayPos(Song::PlayMode::Song).getTicks() < tick) { Engine::audioEngine()->renderNextPeriod(); }
	}

private slots:
	void initTestCase()
	{
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	void cleanup()
	{
		auto* song = Engine::getSong();
		song->stop();
		song->tempoMap().clear();
		song->tempoModel().setValue(DefaultTempo);
	}

	void tempoJumpsAndRamps()
	{
		auto map = TempoMap{};
		QVERIFY(map.isEmpty());
		map.setMarker({192, 100, true}, 120); // a ramp from bar 2 ...
		map.setMarker({384, 140, false}, 120); // ... to bar 3, then 140
		QCOMPARE(map.markers().size(), std::size_t{3}); // with the one at the start
		QCOMPARE(map.tempoAt(0), 120.0);
		QCOMPARE(map.tempoAt(191), 120.0);
		QCOMPARE(map.tempoAt(192), 100.0);
		QCOMPARE(map.tempoAt(288), 120.0); // half way up the ramp
		QCOMPARE(map.tempoAt(384), 140.0);
		QCOMPARE(map.tempoAt(10000), 140.0);

		// Moving and removing; the start marker stays, and alone it is no map
		map.moveMarker(384, 480);
		QCOMPARE(map.tempoAt(432), 100.0 + 40.0 * (432 - 192) / (480 - 192)); // the ramp is longer now
		map.moveMarker(0, 50);
		QCOMPARE(map.markers().front().tick, 0);
		map.removeMarker(480);
		map.removeMarker(192);
		QVERIFY(map.isEmpty());
	}

	void savedAndLoaded()
	{
		auto map = TempoMap{};
		map.setMarker({96, 90.5, true}, 120);
		map.setMarker({960, 180, false}, 120);
		auto document = QDomDocument{};
		auto element = document.createElement("tempomap");
		document.appendChild(element);
		map.saveSettings(document, element);

		auto loaded = TempoMap{};
		loaded.loadSettings(element);
		QVERIFY(loaded.markers() == map.markers());

		// Nonsense from a damaged file stays within range
		auto damaged = document.createElement("tempomap");
		auto marker = document.createElement("marker");
		marker.setAttribute("tick", -5);
		marker.setAttribute("bpm", "nan");
		damaged.appendChild(marker);
		loaded.loadSettings(damaged);
		QCOMPARE(loaded.markers().front().tick, 0);
		QCOMPARE(loaded.markers().front().bpm, static_cast<double>(DefaultTempo));
	}

	void playbackFollowsTheMap()
	{
		auto* song = Engine::getSong();
		const auto bar = TimePos::ticksPerBar();
		song->tempoModel().setValue(120);
		song->tempoMap().setMarker({2 * bar, 90, false}, 120);
		song->tempoMap().setMarker({4 * bar, 60, true}, 120);
		song->tempoMap().setMarker({6 * bar, 160, false}, 120);

		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		playUntil(bar);
		QCOMPARE(song->getTempo(), bpm_t{120});
		playUntil(3 * bar);
		QCOMPARE(song->getTempo(), bpm_t{90});
		QCOMPARE(Engine::framesPerTick(), Engine::framesPerTick(Engine::audioEngine()->outputSampleRate()));
		playUntil(5 * bar);
		QVERIFY(std::abs(song->getTempo() - 110) <= 1); // half way up the ramp from 60 to 160
		playUntil(6 * bar + 10);
		QCOMPARE(song->getTempo(), bpm_t{160});
	}

	void tempoAutomationBecomesMarkers()
	{
		auto* song = Engine::getSong();
		auto* clip = AutomationClip::globalAutomationClip(&song->tempoModel());
		clip->setProgressionType(AutomationClip::ProgressionType::Discrete);
		clip->putValue(TimePos{0}, 100);
		clip->putValue(TimePos{192}, 150);
		QVERIFY(song->tempoMap().takeOverAutomation(&song->tempoModel(), 120));

		const auto& markers = song->tempoMap().markers();
		QCOMPARE(markers.size(), std::size_t{2});
		QCOMPARE(markers[0].tick, 0);
		QCOMPARE(markers[0].bpm, 100.0);
		QCOMPARE(markers[1].tick, 192);
		QCOMPARE(markers[1].bpm, 150.0);
		QVERIFY(!markers[0].ramp);
		QVERIFY(clip->getTimeMap().isEmpty()); // the automation no longer changes the tempo
	}
};

QTEST_GUILESS_MAIN(TempoMapTest)
#include "TempoMapTest.moc"
