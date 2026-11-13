/*
 * SongMarkersTest.cpp
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
#include "Engine.h"
#include "ProjectJournal.h"
#include "Song.h"
#include "SongMarkers.h"

using namespace lmms;

class SongMarkersTest : public QObject
{
	Q_OBJECT
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

	void init()
	{
		Engine::getSong()->createNewProject();
		Engine::projectJournal()->setJournalling(true);
	}

	void regionsAreKeptInOrderAndReversedSpansAreTurned()
	{
		auto& markers = Engine::getSong()->markers();
		markers.addRegion(384, 768, "Chorus");
		markers.addRegion(192, 0, "Intro"); // given backwards
		markers.addRegion(500, 500, "Hit"); // empty: a tick at least
		QCOMPARE(markers.regions().size(), std::size_t{3});
		QCOMPARE(markers.regions()[0].name, QString{"Intro"});
		QCOMPARE(markers.regions()[0].begin, 0);
		QCOMPARE(markers.regions()[0].end, 192);
		QCOMPARE(markers.regions()[1].name, QString{"Chorus"});
		QCOMPARE(markers.regions()[2].end, 501);
	}

	void aRegionAtTheSameStartReplacesTheOldOne()
	{
		auto& markers = Engine::getSong()->markers();
		markers.addRegion(0, 192, "A");
		markers.addRegion(0, 384, "B");
		QCOMPARE(markers.regions().size(), std::size_t{1});
		QCOMPARE(markers.regions()[0].name, QString{"B"});
	}

	void theShortestRegionIsTheOneAtATick()
	{
		auto& markers = Engine::getSong()->markers();
		markers.addRegion(0, 1536, "Verse");
		markers.addRegion(384, 576, "Break");
		QCOMPARE(markers.regionAt(100)->name, QString{"Verse"});
		QCOMPARE(markers.regionAt(400)->name, QString{"Break"});
		QCOMPARE(markers.regionAt(576)->name, QString{"Verse"}); // the end is outside
		QVERIFY(!markers.regionAt(1536).has_value());
	}

	void aRegionIsMovedResizedRenamedAndRemoved()
	{
		auto& markers = Engine::getSong()->markers();
		markers.addRegion(192, 384, "Bridge");
		markers.setRegionSpan(192, 768, 1152);
		QCOMPARE(markers.regions()[0].begin, 768);
		QCOMPARE(markers.regions()[0].end, 1152);
		QCOMPARE(markers.regions()[0].name, QString{"Bridge"});
		markers.renameRegion(768, "Solo");
		markers.setRegionColor(768, QColor{"#ff0000"});
		QCOMPARE(markers.regions()[0].name, QString{"Solo"});
		QCOMPARE(markers.regions()[0].color, QColor{"#ff0000"});
		markers.removeRegion(768);
		QVERIFY(markers.regions().empty());
	}

	void regionsAreSavedAndLoaded()
	{
		auto& markers = Engine::getSong()->markers();
		markers.add(0, "Start");
		markers.addRegion(0, 384, "Intro", QColor{"#336699"});
		markers.addRegion(384, 1152, "Verse");

		auto document = QDomDocument{};
		auto element = document.createElement("markers");
		markers.saveSettings(document, element);

		auto loaded = SongMarkers{};
		loaded.loadSettings(element);
		QCOMPARE(loaded.markers().size(), std::size_t{1});
		QCOMPARE(loaded.regions(), markers.regions());

		// A damaged region is left out, a reversed one turned
		auto damaged = document.createElement("markers");
		auto bad = document.createElement("region");
		bad.setAttribute("begin", "x");
		bad.setAttribute("end", "10");
		damaged.appendChild(bad);
		auto reversed = document.createElement("region");
		reversed.setAttribute("begin", "300");
		reversed.setAttribute("end", "100");
		reversed.setAttribute("name", "R");
		damaged.appendChild(reversed);
		loaded.loadSettings(damaged);
		QCOMPARE(loaded.regions().size(), std::size_t{1});
		QCOMPARE(loaded.regions()[0].begin, 100);
		QCOMPARE(loaded.regions()[0].end, 300);
	}

	void regionChangesAreUndone()
	{
		auto& markers = Engine::getSong()->markers();
		markers.addRegion(0, 384, "Intro");
		markers.setRegionSpan(0, 192, 576);
		Engine::projectJournal()->undo();
		QCOMPARE(markers.regions().size(), std::size_t{1});
		QCOMPARE(markers.regions()[0].begin, 0);
		QCOMPARE(markers.regions()[0].end, 384);
		Engine::projectJournal()->undo();
		QVERIFY(markers.regions().empty());
	}
};

QTEST_GUILESS_MAIN(SongMarkersTest)
#include "SongMarkersTest.moc"
