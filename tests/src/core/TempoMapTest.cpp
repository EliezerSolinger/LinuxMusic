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
#include <QScopeGuard>
#include <QtTest>

#include "AudioEngine.h"
#include "AutomationClip.h"
#include "Engine.h"
#include "Song.h"
#include "ProjectJournal.h"
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

	void namedMarkersAreEditedSavedAndUndone()
	{
		auto* song = Engine::getSong();
		auto& markers = song->markers();
		markers.clear();
		const auto journal = Engine::projectJournal();
		journal->clearJournal();
		journal->setJournalling(true);

		markers.add(384, "Chorus", QColor{Qt::red});
		markers.add(0, "Intro");
		QCOMPARE(markers.markers().size(), std::size_t{2});
		QCOMPARE(markers.markers().front().name, QString{"Intro"}); // in order
		QCOMPARE(markers.next(0)->name, QString{"Chorus"});
		QCOMPARE(markers.previous(384)->name, QString{"Intro"});
		markers.rename(384, "Refrão");
		markers.move(384, 768);
		QVERIFY(!markers.at(384));
		QCOMPARE(markers.at(768)->name, QString{"Refrão"});
		QCOMPARE(markers.at(768)->color, QColor{Qt::red});

		// Saved and loaded
		auto document = QDomDocument{};
		auto root = document.createElement("song");
		document.appendChild(root);
		const auto element = markers.saveState(document, root);
		const auto saved = markers.markers();
		markers.clear();
		markers.restoreState(element);
		QCOMPARE(markers.markers(), saved);

		// Each edit is an undo step
		markers.remove(0);
		QVERIFY(!markers.at(0));
		journal->undo();
		QVERIFY(markers.at(0));
		journal->undo(); // (the restore above is not an edit: this undoes the move)
		QVERIFY(markers.at(384));
		journal->setJournalling(false);
		markers.clear();
	}

	void editsOfTheMapAreUndone()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* journal = Engine::projectJournal();
		journal->setJournalling(true);
		auto& map = song->tempoMap();
		auto before = map.markers();
		map.setMarker({4 * 192, 90.0, false}, song->getTempo());
		map.recordChange(before);
		before = map.markers();
		map.moveMarker(4 * 192, 8 * 192);
		map.recordChange(before);
		QCOMPARE(map.markers().back().tick, 8 * 192);
		journal->undo();
		QCOMPARE(map.markers().back().tick, 4 * 192);
		journal->undo();
		QVERIFY(map.markers().empty());
		journal->redo();
		QCOMPARE(map.markers().back().bpm, 90.0);
	}

	//! Without changes of the time signature, the bars are the song's, as before
	void barsInTheSongsTimeSignature()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		const auto& map = song->tempoMap();
		QVERIFY(!map.hasMeterChanges());
		QCOMPARE(map.barAt(0).index, 0);
		QCOMPARE(map.barAt(191).index, 0);
		QCOMPARE(map.barAt(192).index, 1);
		QCOMPARE(map.barAt(1000).start, 5 * 192);
		QCOMPARE(map.barStart(7), 7 * 192);

		// The song's time signature is the one at the start
		song->getTimeSigModel().setNumerator(3);
		const auto restore = qScopeGuard([&] { song->getTimeSigModel().setNumerator(4); });
		QCOMPARE(map.startMeter(), (TempoMap::Meter{3, 4}));
		QCOMPARE(map.barAt(144).index, 1);
		QCOMPARE(map.barStart(2), 288);
		QCOMPARE(map.barAt(150).length, 144);
	}

	//! A change of the time signature starts a bar; one it cuts short ends there
	void aTimeSignatureChangeStartsABar()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto& map = song->tempoMap();
		// 4/4 for two bars, then 3/4
		map.setMeter(2 * 192, TempoMap::Meter{3, 4}, song->getTempo());
		QVERIFY(map.hasMeterChanges());
		QCOMPARE(map.barAt(383).index, 1);
		QCOMPARE(map.barAt(383).length, 192);
		const auto third = map.barAt(384);
		QCOMPARE(third.index, 2);
		QCOMPARE(third.start, 384);
		QCOMPARE(third.length, 144);
		QCOMPARE(third.meter, (TempoMap::Meter{3, 4}));
		QCOMPARE(map.barAt(384 + 144).index, 3);
		QCOMPARE(map.barStart(3), 384 + 144);
		QCOMPARE(map.barStart(1), 192);
		QCOMPARE(map.meterAt(100), (TempoMap::Meter{4, 4}));

		// In the middle of a bar: that bar ends there
		map.setMeter(300, TempoMap::Meter{6, 8}, song->getTempo());
		const auto cut = map.barAt(250);
		QCOMPARE(cut.index, 1);
		QCOMPARE(cut.start, 192);
		QCOMPARE(cut.length, 108);
		QCOMPARE(map.barAt(300).index, 2);
		QCOMPARE(map.barAt(300).length, 84); // 6/8 cut short by the 3/4 at 384
		QCOMPARE(map.barStart(3), 384);

		// The bars one by one
		auto starts = std::vector<tick_t>{};
		map.forEachBar(0, 700, [&](const TempoMap::Bar& bar) { starts.push_back(bar.start); });
		QCOMPARE(starts, (std::vector<tick_t>{0, 192, 300, 384, 528, 672}));

		// Taken away: the time signature before goes on
		map.setMeter(300, std::nullopt, song->getTempo());
		QCOMPARE(map.barAt(300).index, 1);
	}

	//! A marker made for a time signature keeps the tempo as it was, in a ramp too
	void aTimeSignatureKeepsTheTempo()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto& map = song->tempoMap();
		map.setMarkers({{0, 100, true}, {8 * 192, 180, false}});
		auto before = std::vector<double>{};
		for (tick_t tick = 0; tick < 10 * 192; tick += 37) { before.push_back(map.tempoAt(tick)); }
		map.setMeter(3 * 192, TempoMap::Meter{7, 8}, song->getTempo());
		QCOMPARE(map.markers().size(), std::size_t{3});
		auto index = std::size_t{0};
		for (tick_t tick = 0; tick < 10 * 192; tick += 37)
		{
			QVERIFY2(std::abs(map.tempoAt(tick) - before[index++]) < 1e-9, qPrintable(QString::number(tick)));
		}
	}

	//! Saved and loaded; one at the start, or one not valid, is left out
	void timeSignaturesAreSavedAndLoaded()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto& map = song->tempoMap();
		map.setMarkers({{0, 120, false, TempoMap::Meter{5, 4}}, {384, 120, false, TempoMap::Meter{6, 8}},
			{768, 140, false, TempoMap::Meter{3, 5}}});
		QVERIFY(!map.markers()[0].meter);
		QVERIFY(!map.markers()[2].meter);
		auto document = QDomDocument{};
		auto element = document.createElement("tempomap");
		map.saveSettings(document, element);
		map.clear();
		QVERIFY(!map.hasMeterChanges());
		map.loadSettings(element);
		QCOMPARE(map.markers().size(), std::size_t{3});
		QCOMPARE(map.markers()[1].meter, (std::optional<TempoMap::Meter>{TempoMap::Meter{6, 8}}));
		QCOMPARE(map.barAt(384).meter, (TempoMap::Meter{6, 8}));
	}
};

QTEST_GUILESS_MAIN(TempoMapTest)
#include "TempoMapTest.moc"
