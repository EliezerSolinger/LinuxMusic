/*
 * PluginGuardTest.cpp
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
#include <QScopeGuard>
#include <QTemporaryDir>
#include <QtTest>

#include <thread>

#include "DummyEffect.h"
#include "Engine.h"
#include "AudioEngine.h"
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "Mixer.h"
#include "PluginGuard.h"
#include "PluginInstance.h"
#include "Song.h"

using namespace lmms;

class PluginGuardTest : public QObject
{
	Q_OBJECT

	QTemporaryDir m_directory;

	static auto instrumentName(InstrumentTrack* track) -> QString
	{
		const auto* instrument = track->instrument();
		return instrument && instrument->descriptor() ? QString{instrument->descriptor()->name} : QString{};
	}

	static void clearAllSuspects()
	{
		for (const auto& identity : PluginGuard::suspects()) { PluginGuard::clearSuspect(identity); }
	}

private slots:
	void initTestCase()
	{
		QVERIFY(m_directory.isValid());
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		clearAllSuspects();
		Engine::destroy();
	}

	void init()
	{
		clearAllSuspects();
	}

	//! One of many is told apart by its key's attributes, in the order of their names
	void identityTellsOneOfMany()
	{
		QCOMPARE(PluginGuard::identity("tripleoscillator", {}), QString{"tripleoscillator"});
		auto attributes = QMap<QString, QString>{};
		attributes["plugin"] = "delay_5";
		attributes["file"] = "delay.so";
		const auto identity = PluginGuard::identity("ladspaeffect", attributes);
		QCOMPARE(identity, QString{"ladspaeffect|file=delay.so|plugin=delay_5"});
		QCOMPARE(PluginGuard::displayName(identity), QString{"ladspaeffect (delay.so) (delay_5)"});
	}

	void suspectsAreKeptAndCleared()
	{
		QVERIFY(PluginGuard::suspects().isEmpty());
		PluginGuard::addSuspect("a|file=x y.so");
		PluginGuard::addSuspect("b");
		PluginGuard::addSuspect("b");
		QCOMPARE(PluginGuard::suspects(), (QStringList{"a|file=x y.so", "b"}));
		QVERIFY(PluginGuard::isSuspect("b"));
		PluginGuard::clearSuspect("a|file=x y.so");
		QCOMPARE(PluginGuard::suspects(), QStringList{"b"});
		QVERIFY(!PluginGuard::isSuspect("a|file=x y.so"));
	}

	//! The crash handler's line names the plugin; once is enough
	void aReportMakesSuspects()
	{
		const auto report = QString{"LinuxMusic crashed: SIGSEGV (invalid memory access)\n"}
			+ PluginGuard::ReportMarker + "vestige|file=Synth.dll\n"
			+ PluginGuard::ReportMarker + "vestige|file=Synth.dll\n"
			+ "\nStack of the crashing thread:\n0 something\n";
		QCOMPARE(PluginGuard::takeSuspectsFromReport(report), QStringList{"vestige (Synth.dll)"});
		QCOMPARE(PluginGuard::suspects(), QStringList{"vestige|file=Synth.dll"});
		QVERIFY(PluginGuard::takeSuspectsFromReport("LinuxMusic crashed: SIGABRT (aborted)\n").isEmpty());
	}

	//! Scopes nest, and each thread has its own
	void runningNestsPerThread()
	{
		QCOMPARE(PluginGuard::running(), nullptr);
		{
			const auto outer = PluginGuard::Running{"outer"};
			QCOMPARE(PluginGuard::running(), "outer");
			{
				const auto inner = PluginGuard::Running{"inner"};
				QCOMPARE(PluginGuard::running(), "inner");
				const char* other = "unset";
				std::thread{[&] { other = PluginGuard::running(); }}.join();
				QCOMPARE(other, nullptr);
			}
			QCOMPARE(PluginGuard::running(), "outer");
		}
		QCOMPARE(PluginGuard::running(), nullptr);
	}

	//! A suspect instrument in a project is not loaded, and its settings are saved as they were; allowed, it loads
	void aSuspectInstrumentIsNotLoaded()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QVERIFY(track);
		track->loadInstrument("tripleoscillator");
		QCOMPARE(instrumentName(track), QString{"tripleoscillator"});
		const auto path = m_directory.filePath("suspect.mmp");
		QVERIFY(song->serializeProject().writeFile(path));
		const auto savedInstrument = [&] {
			auto document = song->serializeProject();
			return document.elementsByTagName("tripleoscillator").count();
		};

		PluginGuard::addSuspect("tripleoscillator");
		song->loadProject(path);
		track = dynamic_cast<InstrumentTrack*>(song->tracks().front());
		QVERIFY(track);
		QVERIFY(instrumentName(track) != "tripleoscillator");
		QCOMPARE(savedInstrument(), 1); // its settings, as they were

		// Not loading a project, and without a window to ask: still not loaded
		track->loadInstrument("tripleoscillator");
		QVERIFY(instrumentName(track) != "tripleoscillator");

		PluginGuard::clearSuspect("tripleoscillator");
		song->loadProject(path);
		track = dynamic_cast<InstrumentTrack*>(song->tracks().front());
		QCOMPARE(instrumentName(track), QString{"tripleoscillator"});
	}

	//! A suspect effect becomes a stand-in that keeps what was saved
	void aSuspectEffectKeepsItsSettings()
	{
		auto document = QDomDocument{};
		auto effect = document.createElement("effect");
		effect.setAttribute("name", "amplifier");
		auto settings = document.createElement("AmplifierControls");
		settings.setAttribute("volume", "150");
		effect.appendChild(settings);
		document.appendChild(effect);

		auto& chain = Engine::mixer()->mixerChannel(0)->m_fxChain;
		{
			const auto loaded = chain.loadEffect(effect);
			QVERIFY(loaded->effect());
			QVERIFY(!dynamic_cast<DummyEffect*>(loaded->effect()));
		}
		PluginGuard::addSuspect("amplifier");
		const auto loaded = chain.loadEffect(effect);
		auto* dummy = dynamic_cast<DummyEffect*>(loaded->effect());
		QVERIFY(dummy);
		QCOMPARE(dummy->originalPluginData().attribute("name"), QString{"amplifier"});
	}
};

QTEST_GUILESS_MAIN(PluginGuardTest)
#include "PluginGuardTest.moc"
