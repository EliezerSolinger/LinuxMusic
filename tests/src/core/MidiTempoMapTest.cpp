/*
 * MidiTempoMapTest.cpp
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

#include "AudioEngine.h"
#include "Engine.h"
#include "ImportFilter.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Note.h"
#include "Song.h"
#include "TempoMap.h"

using namespace lmms;

//! The tempo map in MIDI files: written by the export, read back by the import
class MidiTempoMapTest : public QObject
{
	Q_OBJECT

	QTemporaryDir m_directory;

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

	void tempoAndTimeSignaturesGoThroughAMidiFile()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QVERIFY(track);
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		clip->addNote(Note{TimePos{48}, TimePos{0}, 60});
		song->tempoModel().setValue(120);
		// 4/4 at 120, from bar 2 3/4 at 90, from the next bar 7/8
		song->tempoMap().setMarkers({{0, 120, false}, {384, 90, false, TempoMap::Meter{3, 4}},
			{528, 90, false, TempoMap::Meter{7, 8}}});

		const auto path = m_directory.filePath("tempo.mid");
		song->exportProjectMidi(path.toStdString());
		QVERIFY(QFileInfo{path}.size() > 0);

		song->createNewProject();
		QVERIFY(song->tempoMap().isEmpty());
		ImportFilter::import(path, song);

		QCOMPARE(song->getTimeSigModel().getNumerator(), 4);
		QCOMPARE(song->getTimeSigModel().getDenominator(), 4);
		QCOMPARE(song->getTempo(), bpm_t{120});
		const auto& markers = song->tempoMap().markers();
		QCOMPARE(markers.size(), std::size_t{3});
		QCOMPARE(markers[1].tick, 384);
		QVERIFY(std::abs(markers[1].bpm - 90) < 0.01);
		QCOMPARE(markers[1].meter, (std::optional<TempoMap::Meter>{TempoMap::Meter{3, 4}}));
		QCOMPARE(markers[2].tick, 528);
		QCOMPARE(markers[2].meter, (std::optional<TempoMap::Meter>{TempoMap::Meter{7, 8}}));

		// The time signatures are the song's bars now, no longer automation tracks
		for (const auto* imported : song->tracks()) { QVERIFY(imported->type() != Track::Type::Automation); }
		QCOMPARE(song->tempoMap().barAt(528).index, 3);
	}
};

QTEST_MAIN(MidiTempoMapTest)
#include "MidiTempoMapTest.moc"
