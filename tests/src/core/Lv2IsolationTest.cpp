/*
 * Lv2IsolationTest.cpp
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
#include "AudioProcessor.h"
#include "AutomatableModel.h"
#include "Effect.h"
#include "EffectChain.h"
#include "EffectControls.h"
#include "Lv2ControlBase.h"
#include "Lv2Proc.h"
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

#ifndef LMMS_TEST_LV2_DIR
#define LMMS_TEST_LV2_DIR ""
#endif

/**
 * An LV2 effect run in a process of its own (phase 6.3 of docs/plano-refatoracao.md) sounds as it does in the program,
 * and can be moved there and back. The test plugin (tests/src/helpers/TestLv2) is mono (so it runs twice), applies
 * its gain through the worker, scales it by a CV input, reports a latency, reads an event input, and crashes on demand.
 */
class Lv2IsolationTest : public QObject
{
	Q_OBJECT

	static constexpr auto Uri = "urn:linuxmusic:test:isolation";

	auto setUp(bool isolated) -> Plugin*
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		if (!track || !track->loadInstrument("tripleoscillator")) { return nullptr; }
		auto* mixer = Engine::mixer();
		const auto channelIndex = mixer->createChannel();
		track->mixerChannelModel()->setValue(channelIndex);
		auto key = Plugin::Descriptor::SubPluginFeatures::Key{};
		key.attributes["uri"] = Uri;
		if (isolated) { key.attributes["isolated"] = "1"; }
		auto* chain = &mixer->mixerChannel(channelIndex)->m_fxChain;
		auto instance = std::make_unique<PluginInstance>(Plugin::Type::Effect, "lv2effect", chain, &key);
		if (!instance->effect()) { return nullptr; }
		chain->appendEffect(std::move(instance));
		mixer->setUpTrackChannels();
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		for (int note = 0; note < 16; ++note)
		{
			clip->addNote(Note{TimePos{24}, TimePos{note * 12}, DefaultKey - 12 + (note * 5) % 24});
		}
		return chain->effects().empty() ? nullptr : chain->effects().back()->plugin();
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

	//! The control at port @p symbol of every instance (they are linked) to @p value
	static void setControl(Plugin* effect, const QString& symbol, float value)
	{
		auto* controls = dynamic_cast<Lv2ControlBase*>(dynamic_cast<Effect*>(effect)->controls());
		QVERIFY(controls);
		auto found = false;
		for (std::size_t which = 0; which < controls->procCount(); ++which)
		{
			if (auto* model = controls->control(which)->modelAtPort(symbol)) { model->setValue(value); found = true; }
		}
		QVERIFY2(found, qPrintable("no control " + symbol));
	}

	bool m_available = false;

private slots:
	void initTestCase()
	{
		// (the test's plugin, see tests/src/helpers/TestLv2)
		qputenv("LV2_PATH", LMMS_TEST_LV2_DIR);
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
		m_available = setUp(false) != nullptr;
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	//! Isolated, it sounds the same, to the bit: the worker's gain, the CV input, two instances
	void isolatedItSoundsTheSame()
	{
		if (!m_available) { QSKIP("the LV2 test plugin is not here"); }
		auto* inProgram = setUp(false);
		setControl(inProgram, "gain", 0.6f);
		const auto here = render(300);
		QVERIFY2(loudness(here) > 1e-4f, "in the program it was silent");
		QCOMPARE(dynamic_cast<AudioProcessor*>(inProgram)->latency(), f_cnt_t{32});

		auto* isolated = setUp(true);
		QVERIFY(isolated->property("isolated").toBool());
		setControl(isolated, "gain", 0.6f);
		const auto there = render(300);
		QCOMPARE(dynamic_cast<AudioProcessor*>(isolated)->latency(), f_cnt_t{32});
		const auto difference = largestDifference(here, there);
		qInfo("largest difference %g", static_cast<double>(difference));
		QVERIFY2(difference < 1e-6f, qPrintable(QString{"it sounds different isolated (%1)"}.arg(difference)));
	}

	//! The controls reach the plugin's process: a different gain is a different sound
	void theControlsReachItsProcess()
	{
		if (!m_available) { QSKIP("the LV2 test plugin is not here"); }
		auto* isolated = setUp(true);
		setControl(isolated, "gain", 1.f);
		const auto loud = loudness(render(200));
		setUp(true);
		auto* again = setUp(true);
		setControl(again, "gain", 0.25f);
		const auto quiet = loudness(render(200));
		QVERIFY2(quiet < loud * 0.5f, qPrintable(QString{"gain 0.25: %1, gain 1: %2"}.arg(quiet).arg(loud)));
	}

	//! Moved into a process of its own while in use, and back: it goes on sounding the same
	void itMovesThereAndBack()
	{
		if (!m_available) { QSKIP("the LV2 test plugin is not here"); }
		auto* effect = setUp(false);
		const auto before = render(200);
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
		QVERIFY2(largestDifference(before, there) < 1e-6f, "different after moving it to its process");
		QVERIFY2(largestDifference(before, back) < 1e-6f, "different after bringing it back");
	}

	//! Isolated is kept with the project
	void isolatedIsSavedAndLoaded()
	{
		if (!m_available) { QSKIP("the LV2 test plugin is not here"); }
		auto* effect = setUp(true);
		auto document = QDomDocument{};
		auto saved = document.createElement("saved");
		document.appendChild(saved);
		effect->saveState(document, saved);
		auto* loaded = setUp(false);
		loaded->restoreState(saved.firstChildElement());
		QTRY_VERIFY_WITH_TIMEOUT(loaded->property("isolated").toBool(), 5000);
		QVERIFY2(loudness(render(50)) > 1e-4f, "loaded isolated, it did not play");
	}

	//! Isolated, a plugin that crashes takes down its own process, not the program; then it is a suspect, and comes
	//! isolated when asked for in the program
	void aCrashStaysInItsProcess()
	{
		if (!m_available) { QSKIP("the LV2 test plugin is not here"); }
		auto* effect = setUp(true);
		const auto identity = PluginGuard::identity(*effect);
		PluginGuard::clearSuspect(identity);
		QVERIFY2(loudness(render(50)) > 1e-4f, "isolated, it was silent");
		setControl(effect, "crash", 1.f);
		render(200);
		QTRY_VERIFY_WITH_TIMEOUT(PluginGuard::isSuspect(identity), 5000);
		QVERIFY2(loudness(render(50)) < 1e-6f, "after its process crashed, it was not silent");

		auto* again = setUp(false);
		QVERIFY(again);
		QVERIFY(again->property("isolated").toBool());
		QVERIFY2(loudness(render(50)) > 1e-4f, "the suspect, isolated, did not play");
		PluginGuard::clearSuspect(identity);
	}
};

QTEST_GUILESS_MAIN(Lv2IsolationTest)
#include "Lv2IsolationTest.moc"
