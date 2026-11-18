/*
 * LadspaIsolationTest.cpp
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

#include <algorithm>
#include <cmath>
#include <vector>

#include <QDomDocument>
#include <QtTest>

#include "AudioEngine.h"
#include "AutomatableModel.h"
#include "EffectChain.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Mixer.h"
#include "Note.h"
#include "PluginGuard.h"
#include "PluginInstance.h"
#include "Song.h"
#include "Track.h"

using namespace lmms;

//! A LADSPA effect run in a process of its own (phase 6.3 of docs/plano-refatoracao.md) sounds as it does in the
//! program, and can be moved there and back
class LadspaIsolationTest : public QObject
{
	Q_OBJECT

	//! A track of notes into a mixer channel with the LADSPA effect @p label (from library @p file); the effect
	auto setUp(const QString& file, const QString& label, bool isolated) -> Plugin*
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		if (!track || !track->loadInstrument("tripleoscillator")) { return nullptr; }
		auto* mixer = Engine::mixer();
		const auto channelIndex = mixer->createChannel();
		track->mixerChannelModel()->setValue(channelIndex);
		auto key = Plugin::Descriptor::SubPluginFeatures::Key{};
		key.attributes["file"] = file;
		key.attributes["plugin"] = label;
		if (isolated) { key.attributes["isolated"] = "1"; }
		auto* chain = &mixer->mixerChannel(channelIndex)->m_fxChain;
		chain->appendEffect(std::make_unique<PluginInstance>(Plugin::Type::Effect, "ladspaeffect", chain, &key));
		mixer->setUpTrackChannels();
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		for (int note = 0; note < 16; ++note)
		{
			clip->addNote(Note{TimePos{24}, TimePos{note * 12}, DefaultKey - 12 + (note * 5) % 24});
		}
		return chain->effects().back()->plugin();
	}

	static auto render(int periods) -> std::vector<float>
	{
		auto* song = Engine::getSong();
		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		auto samples = std::vector<float>{};
		for (int period = 0; period < periods; ++period)
		{
			for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
			{
				samples.push_back(frame[0]);
				samples.push_back(frame[1]);
			}
		}
		song->stop();
		return samples;
	}

	static auto largestDifference(const std::vector<float>& a, const std::vector<float>& b) -> float
	{
		auto largest = 0.f;
		for (std::size_t i = 0; i < std::min(a.size(), b.size()); ++i) { largest = std::max(largest, std::abs(a[i] - b[i])); }
		return largest;
	}

	static auto loudness(const std::vector<float>& samples) -> float
	{
		auto sum = 0.0;
		for (const auto sample : samples) { sum += static_cast<double>(sample) * sample; }
		return samples.empty() ? 0.f : static_cast<float>(std::sqrt(sum / static_cast<double>(samples.size())));
	}

private slots:
	void initTestCase()
	{
		// (the test's plugin that crashes on demand, see tests/src/helpers/CrashingLadspa.c)
		qputenv("LADSPA_PATH", LMMS_TEST_LADSPA_DIR);
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	void isolatedItSoundsTheSame_data()
	{
		QTest::addColumn<QString>("file");
		QTest::addColumn<QString>("label");
		// A mono plugin (two instances, one per channel) and stereo ones, with state that carries on (filters, a
		// delay line)
		QTest::newRow("tap_tubewarmth") << "tap_tubewarmth" << "tap_tubewarmth";
		QTest::newRow("tap_echo") << "tap_echo" << "tap_stereo_echo";
		QTest::newRow("tap_reverb") << "tap_reverb" << "tap_reverb";
	}

	void isolatedItSoundsTheSame()
	{
		QFETCH(QString, file);
		QFETCH(QString, label);
		// (a third render, the same as this one, shows the comparison is fair)
		if (!setUp(file, label, false)) { QSKIP("not built here"); }
		const auto inProgram = render(300);
		if (loudness(inProgram) < 1e-4f) { QSKIP("the plugin is not here (silent)"); }
		setUp(file, label, false);
		QCOMPARE(largestDifference(inProgram, render(300)), 0.f);

		QVERIFY(setUp(file, label, true));
		const auto inItsProcess = render(300);
		QCOMPARE(inItsProcess.size(), inProgram.size());
		QVERIFY2(loudness(inItsProcess) > 1e-4f, "isolated, it was silent");
		const auto difference = largestDifference(inProgram, inItsProcess);
		qInfo("%s: largest difference %g", qPrintable(label), static_cast<double>(difference));
		QVERIFY2(difference < 1e-6f, qPrintable(QString{"it sounds different isolated (%1)"}.arg(difference)));
	}

	//! Moved into a process of its own while the project is open, and back: it goes on sounding the same
	void itMovesThereAndBack()
	{
		auto* effect = setUp("tap_tubewarmth", "tap_tubewarmth", false);
		if (!effect) { QSKIP("not built here"); }
		const auto before = render(200);
		if (loudness(before) < 1e-4f) { QSKIP("the plugin is not here (silent)"); }

		Engine::audioEngine()->takeLongestChangeHold();
		QVERIFY(QMetaObject::invokeMethod(effect, "setIsolated", Q_ARG(bool, true)));
		// The process starts and loads the plugin without the audio engine's lock (see PluginInstance::reactivate())
		const auto movingHeld = Engine::audioEngine()->takeLongestChangeHold();
		qInfo("moving it to its process held the change lock %.2f ms", movingHeld / 1000.0);
		QVERIFY2(movingHeld < 2'000, "moving it to its process held the audio engine's lock");
		const auto there = render(200);
		QVERIFY(QMetaObject::invokeMethod(effect, "setIsolated", Q_ARG(bool, false)));
		const auto backHeld = Engine::audioEngine()->takeLongestChangeHold();
		qInfo("bringing it back held the change lock %.2f ms", backHeld / 1000.0);
		QVERIFY2(backHeld < 2'000, "bringing it back held the audio engine's lock");
		const auto back = render(200);

		// (made again each time, so from the same state)
		QVERIFY2(largestDifference(before, there) < 1e-6f, "different after moving it to its process");
		QVERIFY2(largestDifference(before, back) < 1e-6f, "different after bringing it back");
	}

	//! Isolated is kept with the project: saved, and loaded again
	void isolatedIsSavedAndLoaded()
	{
		auto* effect = setUp("tap_tubewarmth", "tap_tubewarmth", true);
		if (!effect) { QSKIP("not built here"); }
		QVERIFY(effect->property("isolated").toBool());
		auto document = QDomDocument{};
		auto saved = document.createElement("saved");
		document.appendChild(saved);
		effect->saveState(document, saved);

		auto* loaded = setUp("tap_tubewarmth", "tap_tubewarmth", false);
		QVERIFY(!loaded->property("isolated").toBool());
		loaded->restoreState(saved.firstChildElement());
		// (it changes once the loading is over: making the plugin again replaces the controls being loaded)
		QTRY_VERIFY_WITH_TIMEOUT(loaded->property("isolated").toBool(), 5000);
		QVERIFY2(loudness(render(50)) > 1e-4f, "loaded isolated, it did not play");
	}

	//! Isolated, a plugin that crashes takes down its own process, not the program: it goes silent, the dry signal
	//! still passes, and it is a suspect
	void aCrashStaysInItsProcess()
	{
		auto* effect = setUp("CrashingLadspa", "crash_test", true);
		if (!effect) { QSKIP("the crashing test plugin is not here"); }
		const auto identity = PluginGuard::identity(*effect);
		PluginGuard::clearSuspect(identity);
		const auto working = render(50);
		QVERIFY2(loudness(working) > 1e-4f, "isolated, the pass-through plugin was silent");

		// Its "Crash" control up (each of the models a LADSPA control has for it): the next period crashes its process
		auto crashes = 0;
		for (auto* model : effect->findChildren<AutomatableModel*>())
		{
			if (model->displayName() != "Crash") { continue; }
			model->setValue(model->maxValue<float>());
			++crashes;
		}
		QVERIFY(crashes > 0);
		render(200);
		QTRY_VERIFY_WITH_TIMEOUT(PluginGuard::isSuspect(identity), 5000);
		// (still here, and rendering: silent from the crash on)
		const auto later = render(50);
		QCOMPARE(later.size(), working.size());
		QVERIFY2(loudness(later) < 1e-6f, "after its process crashed, it was not silent");

		// A suspect now: asked for in the program, it comes isolated (and so it cannot take the program down)
		auto* again = setUp("CrashingLadspa", "crash_test", false);
		QVERIFY(again);
		QVERIFY(again->property("isolated").toBool());
		QVERIFY2(loudness(render(50)) > 1e-4f, "the suspect, isolated, did not play");
		PluginGuard::clearSuspect(identity);
	}
};

QTEST_GUILESS_MAIN(LadspaIsolationTest)
#include "LadspaIsolationTest.moc"
