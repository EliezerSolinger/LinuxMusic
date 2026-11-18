/*
 * InstrumentFixesTest.cpp
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
#include <functional>
#include <vector>

#include <QtTest>

#include "AudioDummy.h"
#include "AudioEngine.h"
#include "ConfigManager.h"
#include "DataFile.h"
#include "Engine.h"
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Mixer.h"
#include "Note.h"
#include "SampleFrame.h"
#include "Song.h"
#include "Track.h"

using namespace lmms;

/**
 * The problems users reported with the built-in instruments (docs/pesquisas/sintetizadores-reclamacoes.md), each
 * played the way it went wrong, so that it stays fixed.
 */
class InstrumentFixesTest : public QObject
{
	Q_OBJECT

	struct Played
	{
		std::vector<float> samples;
		float loudest = 0.f;
		bool finite = true;
	};

	//! @p instrument on a new track, its settings changed by @p change (as saved), a note played for @p periods
	static auto play(const QString& instrument, const std::function<void(QDomElement&)>& change, int periods = 150,
		int key = DefaultKey) -> Played
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		if (!track || !track->loadInstrument(instrument)) { return {}; }
		if (change)
		{
			auto state = DataFile{DataFile::Type::InstrumentTrackSettings};
			track->saveState(state, state.content());
			auto element = state.content().firstChildElement().elementsByTagName("instrument").item(0)
				.firstChildElement();
			change(element);
			track->restoreState(state.content().firstChildElement());
		}
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		clip->addNote(Note{TimePos{2 * TimePos::ticksPerBar()}, TimePos{0}, key});
		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		auto played = Played{};
		for (int period = 0; period < periods; ++period)
		{
			for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
			{
				for (const auto sample : {frame.left(), frame.right()})
				{
					played.finite = played.finite && std::isfinite(sample);
					played.loudest = std::max(played.loudest, std::abs(sample));
					played.samples.push_back(sample);
				}
			}
		}
		song->stop();
		return played;
	}

private slots:
	void initTestCase()
	{
		// (the rate changes on a device of no hardware)
		ConfigManager::inst()->setValue("audioengine", "audiodev", AudioDummy::name());
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase() { Engine::destroy(); }

	//! #7221: at 192 kHz with a string of length 1, the oversampling was 0 (an integer division): no sound
	void vibedAtHighRates()
	{
		auto* engine = Engine::audioEngine();
		for (const auto rate : {sample_rate_t{192000}, sample_rate_t{96000}, sample_rate_t{44100}})
		{
			engine->reopenAudioDevice(rate);
			engine->stopProcessing();
			const auto played = play("vibedstrings", nullptr);
			QVERIFY2(played.finite, qPrintable(QString::number(rate)));
			QVERIFY2(played.loudest > 1e-3f, qPrintable(QString{"silent at %1 Hz"}.arg(rate)));
		}
	}

	/**
	 * #7234: the same song exported twice in a row came out different (the LB302 went on from the phase and the
	 * filter it had). An export starts from rest (Mixer::discardProcessingState(), each plugin's reset()): brought
	 * to rest and played from the start, again after playing something else, it is the same to the bit.
	 */
	void theSameEachTimeFromRest_data()
	{
		QTest::addColumn<QString>("instrument");
		for (const auto* name : {"lb302", "opulenz", "kicker", "tripleoscillator", "monstro", "sid", "zynaddsubfx",
			"avldrums", "sf2player"})
		{
			QTest::newRow(name) << QString{name};
		}
	}

	void theSameEachTimeFromRest()
	{
		QFETCH(QString, instrument);
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		if (!track || !track->loadInstrument(instrument)) { QSKIP("not built here"); }
		if (instrument == "sf2player")
		{
			// (a soundfont: an AVL drum kit)
			track->instrument()->loadFile(QT_TESTCASE_SOURCEDIR "/../data/avldrums/Black_Pearl_4_LV2.sf2");
		}
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		for (int note = 0; note < 8; ++note)
		{
			clip->addNote(Note{TimePos{12}, TimePos{note * 12}, DefaultKey - 12 + note * 3});
		}
		const auto render = [&] {
			Engine::mixer()->discardProcessingState();
			song->setPlayPos(0, Song::PlayMode::Song);
			song->playSong();
			auto out = std::vector<float>{};
			for (int period = 0; period < 200; ++period)
			{
				for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
				{
					out.push_back(frame.left());
					out.push_back(frame.right());
				}
			}
			song->stop();
			return out;
		};
		// As the program runs: the instrument has played before the first export too (an instrument that never
		// played needs no reset)
		const auto playSome = [&](int periods) {
			song->setPlayPos(0, Song::PlayMode::Song);
			song->playSong();
			for (int period = 0; period < periods; ++period) { Engine::audioEngine()->renderNextPeriod(); }
			song->stop();
		};
		playSome(61);
		const auto first = render();
		auto loudest = 0.f;
		for (const auto sample : first) { loudest = std::max(loudest, std::abs(sample)); }
		QVERIFY2(loudest > 1e-4f, "silent");
		// (some more played in between)
		playSome(37);
		const auto again = render();
		QCOMPARE(again.size(), first.size());
		auto at = std::size_t{0};
		while (at < first.size() && first[at] == again[at]) { ++at; }
		QVERIFY2(at == first.size(), qPrintable(QString{"differs at frame %1"}.arg(at / 2)));
	}

	//! #5877: a phase past an int's range was converted to an int (undefined; it crashed)
	void xpressiveWithHugeValues()
	{
		const auto played = play("xpressive", [](QDomElement& xpressive) {
			xpressive.setAttribute("O1", "sinew(1e12*t*f)+squarew(-1e15*t)+trianglew(1e30*t)");
			xpressive.setAttribute("O2", "saww(1e20*t)");
		});
		QVERIFY(played.finite);
	}
};

QTEST_GUILESS_MAIN(InstrumentFixesTest)
#include "InstrumentFixesTest.moc"
