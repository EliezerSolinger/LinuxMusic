/*
 * LfoControllerTest.cpp
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
#include <QtTest>

#include <memory>
#include <vector>

#include "AudioEngine.h"
#include "Engine.h"
#include "LfoController.h"
#include "Song.h"

using namespace lmms;

class LfoControllerTest : public QObject
{
	Q_OBJECT

	static auto values(LfoController& lfo) -> std::vector<float>
	{
		const auto* buffer = lfo.valueBuffer();
		return {buffer->values(), buffer->values() + buffer->length()};
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

	//! After a jump, the LFO is where it would be had the song started there (it went on from the old place until
	//! the main thread delivered a signal)
	void aJumpPutsTheLfoWhereTheSongIs()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto lfo = std::make_unique<LfoController>(song);
		const auto there = 3 * TimePos::ticksPerBar() + 17;

		// Started there
		song->setPlayPos(there, Song::PlayMode::Song);
		song->playSong();
		Engine::audioEngine()->renderNextPeriod();
		const auto startedThere = values(*lfo);
		song->stop();

		// Played from the start, then moved there
		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		for (int period = 0; period < 10; ++period)
		{
			Engine::audioEngine()->renderNextPeriod();
			values(*lfo);
		}
		song->setPlayPos(there, Song::PlayMode::Song);
		Engine::audioEngine()->renderNextPeriod();
		const auto movedThere = values(*lfo);
		song->stop();

		QCOMPARE(movedThere.size(), startedThere.size());
		for (std::size_t frame = 0; frame < movedThere.size(); ++frame)
		{
			QVERIFY2(std::abs(movedThere[frame] - startedThere[frame]) < 1e-4f, qPrintable(QString{"frame %1: %2 and %3"}
				.arg(frame).arg(movedThere[frame]).arg(startedThere[frame])));
		}
	}
};

QTEST_GUILESS_MAIN(LfoControllerTest)
#include "LfoControllerTest.moc"
