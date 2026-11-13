/*
 * TransportTest.cpp
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
#include <QScopeGuard>
#include <QtTest>

#include <cmath>

#include "AudioEngine.h"
#include "ConfigManager.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "Song.h"
#include "TempoMap.h"
#include "Transport.h"

using namespace lmms;

class TransportTest : public QObject
{
	Q_OBJECT

	static void render(int periods)
	{
		for (int period = 0; period < periods; ++period) { Engine::audioEngine()->renderNextPeriod(); }
	}

	static auto framesAt(tick_t tick) -> std::int64_t
	{
		auto* song = Engine::getSong();
		return std::llround(song->tempoMap().framesBetween(0, tick, song->getTempo(),
			Engine::audioEngine()->outputSampleRate()));
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

	void init()
	{
		Engine::getSong()->createNewProject();
	}

	//! From period to period, the frame goes on by a period
	void framesCountOnWithThePeriods()
	{
		auto* song = Engine::getSong();
		auto& transport = song->transport();
		const auto fpp = static_cast<std::int64_t>(Engine::audioEngine()->framesPerPeriod());
		song->setPlayPos(0, Song::PlayMode::Song);
		transport.play();
		QCOMPARE(transport.state(), Transport::State::Playing);
		render(10);
		// The frame of the last period played (the tenth) starts at nine periods
		QCOMPARE(transport.frame(), 9 * fpp);
		transport.stop();
		QCOMPARE(transport.state(), Transport::State::Stopped);
	}

	//! Started elsewhere, the frame is counted along the tempo map, as the engine plays each tick
	void aJumpCountsAgainAlongTheTempoMap()
	{
		auto* song = Engine::getSong();
		auto& transport = song->transport();
		const auto fpp = static_cast<std::int64_t>(Engine::audioEngine()->framesPerPeriod());
		auto marker = TempoMap::Marker{};
		marker.tick = 2 * TimePos::ticksPerBar();
		marker.bpm = 60;
		song->tempoMap().setMarker(marker, 140);
		const auto cleanUp = qScopeGuard([&] { song->tempoMap().clear(); });

		const auto start = 4 * TimePos::ticksPerBar();
		song->setPlayPos(start, Song::PlayMode::Song);
		transport.play();
		render(1);
		QCOMPARE(transport.frame(), framesAt(start));
		// Slower than the song's tempo from bar 3 on: more frames than at a constant tempo
		const auto atStartTempo = start * Engine::audioEngine()->outputSampleRate() * 60.0 * 4 / DefaultTicksPerBar / 140;
		QVERIFY(transport.frame() > static_cast<std::int64_t>(atStartTempo) + 1);
		render(1);
		QCOMPARE(transport.frame(), framesAt(start) + fpp);

		QVERIFY(!transport.jumped());

		// A jump while playing counts again, and is told for that period only (to the VST plugins' sync)
		song->setPlayPos(TimePos::ticksPerBar(), Song::PlayMode::Song);
		render(1);
		QCOMPARE(transport.frame(), framesAt(TimePos::ticksPerBar()));
		QVERIFY(transport.jumped());
		render(1);
		QVERIFY(!transport.jumped());
		transport.stop();
	}

	//! In a cycle, the frame goes back where the cycle starts again
	void theCycleGoesBack()
	{
		auto* song = Engine::getSong();
		auto& transport = song->transport();
		auto& timeline = song->getTimeline(Song::PlayMode::Song);
		timeline.setLoopBegin(TimePos{0});
		timeline.setLoopEnd(TimePos{48});
		timeline.setLoopEnabled(true);
		const auto cleanUp = qScopeGuard([&] { timeline.setLoopEnabled(false); });

		const auto cycleFrames = framesAt(48);
		song->setPlayPos(0, Song::PlayMode::Song);
		transport.play();
		auto highest = std::int64_t{0};
		auto wentBack = false;
		auto previous = std::int64_t{-1};
		for (int period = 0; period < 200; ++period)
		{
			render(1);
			highest = std::max(highest, transport.frame());
			wentBack = wentBack || transport.frame() < previous;
			previous = transport.frame();
		}
		QVERIFY(wentBack);
		QVERIFY2(highest < cycleFrames, qPrintable(QString{"%1 of %2"}.arg(highest).arg(cycleFrames)));
		transport.stop();
	}

	void playPausesAndResumes()
	{
		auto& transport = Engine::getSong()->transport();
		transport.play();
		QCOMPARE(transport.state(), Transport::State::Playing);
		transport.play();
		QCOMPARE(transport.state(), Transport::State::Paused);
		transport.play();
		QCOMPARE(transport.state(), Transport::State::Playing);
		auto stopped = false;
		const auto connection = connect(&transport, &Transport::stopped, this, [&] { stopped = true; });
		transport.stop();
		disconnect(connection);
		QVERIFY(stopped);
		QCOMPARE(transport.state(), Transport::State::Stopped);
	}

	//! Recording from a cursor with a pre-roll: the state says pre-roll until the song reaches the cursor
	void aPreRollEndsInRecording()
	{
		auto* song = Engine::getSong();
		auto& transport = song->transport();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		ConfigManager::inst()->setValue("recording", "prerollbars", "1");
		const auto cleanUp = qScopeGuard([&] {
			ConfigManager::inst()->setValue("recording", "prerollbars", "0");
			transport.stop();
		});

		// Nothing armed and no track to arm: nothing starts
		QCOMPARE(transport.record(nullptr), Transport::RecordResult::NothingArmed);
		QCOMPARE(transport.state(), Transport::State::Stopped);

		// The selected track is armed, and the song starts a bar before the cursor
		const auto cursor = 2 * TimePos::ticksPerBar();
		song->setPlayPos(cursor, Song::PlayMode::Song);
		QCOMPARE(transport.record(track), Transport::RecordResult::Started);
		QVERIFY(track->isRecordArmed());
		QCOMPARE(transport.state(), Transport::State::PreRoll);
		QVERIFY(transport.isRecording());

		for (int period = 0; period < 2000 && song->getPlayPos(Song::PlayMode::Song).getTicks() < cursor; ++period)
		{
			render(1);
		}
		render(1);
		QCOMPARE(transport.state(), Transport::State::Recording);

		// Again: the recording ends, the song plays on
		QCOMPARE(transport.record(nullptr), Transport::RecordResult::Ended);
		QCOMPARE(transport.state(), Transport::State::Playing);
	}
};

QTEST_GUILESS_MAIN(TransportTest)
#include "TransportTest.moc"
