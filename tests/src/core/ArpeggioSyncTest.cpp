/*
 * ArpeggioSyncTest.cpp
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

#include <vector>

#include <QtTest>

#include "AudioEngine.h"
#include "DataFile.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Note.h"
#include "SampleFrame.h"
#include "Song.h"
#include "Track.h"

using namespace lmms;

/**
 * An arpeggio in Sync or Sort mode follows the first note of the chord (InstrumentFunctionArpeggio::processNote()),
 * while each note of the chord may be played on another of the audio engine's threads, before or after it. It read
 * the first note's frame counter, which that note moves on only when it is played: so the steps came a period late
 * or not, as the threads happened to run. It reads the frames the first note had when the period began
 * (NotePlayHandle::framesPlayedAtPeriodStart()): the same render every time.
 */
class ArpeggioSyncTest : public QObject
{
	Q_OBJECT

	//! A chord of four notes, arpeggiated in @p mode (1: Sort, 2: Sync), rendered for @p periods
	static auto render(int mode, int periods) -> std::vector<float>
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		track->loadInstrument("tripleoscillator");

		// The arpeggio on, through the track's saved state (its models are its own)
		auto state = DataFile{DataFile::Type::InstrumentTrackSettings};
		track->saveState(state, state.content());
		auto arpeggiator = state.content().firstChildElement().elementsByTagName("arpeggiator").item(0).toElement();
		arpeggiator.setAttribute("arp-enabled", 1);
		arpeggiator.setAttribute("arpmode", mode);
		arpeggiator.setAttribute("arptime", 37); // ms: steps that do not fall on period boundaries
		track->restoreState(state.content().firstChildElement());

		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		for (const auto key : {DefaultKey, DefaultKey + 4, DefaultKey + 7, DefaultKey + 11})
		{
			clip->addNote(Note{TimePos{4 * TimePos::ticksPerBar()}, TimePos{0}, key});
		}

		song->playSong();
		auto out = std::vector<float>{};
		for (int period = 0; period < periods; ++period)
		{
			for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
			{
				out.push_back(frame.left());
				out.push_back(frame.right());
			}
		}
		song->stop();
		return out;
	}

private slots:
	void initTestCase()
	{
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase() { Engine::destroy(); }

	void theSameEveryTime_data()
	{
		QTest::addColumn<int>("mode");
		QTest::newRow("Sort") << 1;
		QTest::newRow("Sync") << 2;
	}

	void theSameEveryTime()
	{
		QFETCH(int, mode);
		const auto first = render(mode, 400);
		auto loudest = 0.f;
		for (const auto sample : first) { loudest = std::max(loudest, std::abs(sample)); }
		QVERIFY2(loudest > 1e-3f, "the arpeggio did not sound");
		for (int run = 1; run < 6; ++run)
		{
			const auto again = render(mode, 400);
			QCOMPARE(again.size(), first.size());
			auto firstDifference = std::size_t{0};
			while (firstDifference < first.size() && first[firstDifference] == again[firstDifference]) { ++firstDifference; }
			QVERIFY2(firstDifference == first.size(),
				qPrintable(QString{"run %1 differs from the first at sample %2"}.arg(run).arg(firstDifference / 2)));
		}
	}
};

QTEST_GUILESS_MAIN(ArpeggioSyncTest)
#include "ArpeggioSyncTest.moc"
