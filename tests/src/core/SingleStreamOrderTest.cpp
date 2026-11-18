/*
 * SingleStreamOrderTest.cpp
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
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Note.h"
#include "SampleFrame.h"
#include "Song.h"
#include "Track.h"

using namespace lmms;

/**
 * A single-streamed instrument (one stream for all its notes: SF2, AVL Drums, OpulenZ, LB302...) gets the notes that
 * start together from several of the audio engine's threads, in the order the threads happen to run. Whatever it
 * does with them (which voice each gets, which one a monophonic synth keeps) must not depend on that order: the same
 * chords, rendered again and again, the same bit for bit. Each one also as a project brings it (restored from its
 * settings): an instrument's destructor once removed the play handle of the one made to take its place.
 */
class SingleStreamOrderTest : public QObject
{
	Q_OBJECT

	static auto render(const QString& instrument, int periods) -> std::vector<float>
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		track->loadInstrument(instrument.section(' ', 0, 0));
		if (instrument.startsWith("sf2player"))
		{
			// A soundfont here: an AVL drum kit (its one preset is bank 0, program 0, the instrument's default)
			const auto font = QString{QT_TESTCASE_SOURCEDIR "/../data/avldrums/Black_Pearl_4_LV2.sf2"};
			if (instrument.endsWith("restored"))
			{
				// As a project brings it: the track restored with the soundfont in its settings
				auto state = DataFile{DataFile::Type::InstrumentTrackSettings};
				track->saveState(state, state.content());
				state.content().firstChildElement().elementsByTagName("instrument").item(0)
					.firstChildElement("sf2player").setAttribute("src", font);
				track->restoreState(state.content().firstChildElement());
			}
			else { track->instrument()->loadFile(font); }
		}
		else if (instrument.endsWith("restored"))
		{
			// As a project brings it: the track saved and restored, the instrument made again from its settings
			auto state = DataFile{DataFile::Type::InstrumentTrackSettings};
			track->saveState(state, state.content());
			track->restoreState(state.content().firstChildElement());
		}
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		// Chords of six notes, each starting together, a few in a row, some overlapping
		for (int chord = 0; chord < 6; ++chord)
		{
			for (const auto offset : {0, 3, 7, 10, 14, 17})
			{
				clip->addNote(Note{TimePos{24 + chord * 6}, TimePos{chord * 16}, DefaultKey - 12 + chord * 2 + offset});
			}
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
		QTest::addColumn<QString>("instrument");
		for (const auto* name : {"sf2player", "sf2player restored", "avldrums", "avldrums restored", "opulenz",
			"opulenz restored", "lb302", "lb302 restored"})
		{
			QTest::newRow(name) << QString{name};
		}
	}

	void theSameEveryTime()
	{
		QFETCH(QString, instrument);
		const auto first = render(instrument, 300);
		auto loudest = 0.f;
		for (const auto sample : first) { loudest = std::max(loudest, std::abs(sample)); }
		QVERIFY2(loudest > 1e-4f, "silent");
		for (int run = 1; run < 8; ++run)
		{
			const auto again = render(instrument, 300);
			QCOMPARE(again.size(), first.size());
			auto at = std::size_t{0};
			while (at < first.size() && first[at] == again[at]) { ++at; }
			QVERIFY2(at == first.size(), qPrintable(QString{"run %1 differs at sample %2"}.arg(run).arg(at / 2)));
		}
	}
};

QTEST_GUILESS_MAIN(SingleStreamOrderTest)
#include "SingleStreamOrderTest.moc"
