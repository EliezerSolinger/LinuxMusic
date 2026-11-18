/*
 * DrumMachinesTest.cpp - the Drum Machines instrument: its kits and how its pieces play
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

#include <QtTest>

#include "AudioDummy.h"
#include "AudioEngine.h"
#include "ComboBoxModel.h"
#include "ConfigManager.h"
#include "Engine.h"
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Note.h"
#include "PluginFactory.h"
#include "Song.h"
#include "Track.h"

using namespace lmms;

class DrumMachinesTest : public QObject
{
	Q_OBJECT

	//! A track with the instrument, its kit @p kit (by its index); none if it is not built here
	static auto setUp(int kit) -> InstrumentTrack*
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		if (!track || !track->loadInstrument("drummachines")) { return nullptr; }
		auto* kits = track->instrument()->findChild<ComboBoxModel*>();
		if (!kits) { return nullptr; }
		kits->setValue(static_cast<float>(kit));
		// (the kit is synthesized on the main thread, queued)
		QCoreApplication::processEvents();
		return track;
	}

	//! @p notes, each at its time (in ticks) from the start, rendered for @p seconds: the left channel
	static auto play(InstrumentTrack* track, const std::vector<std::pair<int, int>>& notes, double seconds)
		-> std::vector<float>
	{
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		for (const auto& [key, tick] : notes) { clip->addNote(Note{TimePos{12}, TimePos{tick}, key}); }
		auto* song = Engine::getSong();
		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		auto left = std::vector<float>{};
		const auto frames = static_cast<std::size_t>(Engine::audioEngine()->outputSampleRate() * seconds);
		while (left.size() < frames)
		{
			for (const auto& frame : Engine::audioEngine()->renderNextPeriod()) { left.push_back(frame.left()); }
		}
		song->stop();
		return left;
	}

	static auto energy(const std::vector<float>& samples, std::size_t from, std::size_t to) -> double
	{
		auto sum = 0.0;
		for (auto i = from; i < std::min(to, samples.size()); ++i) { sum += samples[i] * samples[i]; }
		return sum;
	}

private slots:
	void initTestCase()
	{
		ConfigManager::inst()->setValue("audioengine", "audiodev", AudioDummy::name());
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase() { Engine::destroy(); }

	void everyKit_data()
	{
		QTest::addColumn<int>("kit");
		for (int kit = 0; kit < 9; ++kit) { QTest::newRow(qPrintable(QString::number(kit))) << kit; }
	}

	//! Every kit loads (every piece's DrumSynth file read) and its pieces sound, finite: its notes from the
	//! General MIDI kick to the last variation
	void everyKit()
	{
		QFETCH(int, kit);
		if (getPluginFactory()->pluginInfo("drummachines").isNull()) { QSKIP("not built here"); }
		QTest::failOnWarning(QRegularExpression{"Failed to load sample"});
		auto* track = setUp(kit);
		QVERIFY(track);
		auto notes = std::vector<std::pair<int, int>>{};
		for (int key = 35; key <= 90; ++key) { notes.emplace_back(key, (key - 35) * 12); }
		const auto sound = play(track, notes, 7.0);
		auto loudest = 0.f;
		for (const auto sample : sound)
		{
			QVERIFY(std::isfinite(sample));
			loudest = std::max(loudest, std::abs(sample));
		}
		QVERIFY2(loudest > 0.05f, qPrintable(QString{"loudest %1"}.arg(loudest)));
	}

	//! A closed hi-hat chokes the open one that rings, as on the machines (the TR-808's)
	void closedHatChokesOpenHat()
	{
		if (getPluginFactory()->pluginInfo("drummachines").isNull()) { QSKIP("not built here"); }
		const auto rate = Engine::audioEngine()->outputSampleRate();
		auto* alone = setUp(4);
		QVERIFY(alone);
		const auto ringing = play(alone, {{46, 0}}, 1.0);
		auto* choked = setUp(4);
		QVERIFY(choked);
		const auto stopped = play(choked, {{46, 0}, {42, 4}}, 1.0);
		// From after the closed hat's own sound (it starts 4 ticks in, at 140 BPM 0.04 s, and lasts some tens of
		// milliseconds) to the open hat's end (a quarter of a second)
		auto end = ringing.size();
		while (end > 0 && std::abs(ringing[end - 1]) < 1e-4f) { --end; }
		const auto from = static_cast<std::size_t>(rate * 0.12);
		const auto to = end;
		QVERIFY2(to > from + rate / 20, qPrintable(QString{"the open hat ends at %1 s"}.arg(double(end) / rate)));
		const auto name = QString{"open alone %1, choked %2"}.arg(energy(ringing, from, to))
			.arg(energy(stopped, from, to));
		QVERIFY2(energy(ringing, from, to) > 0, qPrintable(name));
		QVERIFY2(energy(stopped, from, to) < energy(ringing, from, to) * 0.01, qPrintable(name));
	}
};

QTEST_GUILESS_MAIN(DrumMachinesTest)
#include "DrumMachinesTest.moc"
