/*
 * BuiltinPluginsTest.cpp
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
#include <memory>
#include <atomic>
#include <thread>
#include <chrono>
#include <random>
#include <vector>

#include <QDomDocument>
#include <QTemporaryDir>
#include <QtTest>

#include <sndfile.h>

#include "AudioDummy.h"
#include "AudioEngine.h"
#include "AutomatableModel.h"
#include "ConfigManager.h"
#include "Effect.h"
#include "EffectChain.h"
#include "Engine.h"
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Mixer.h"
#include "Note.h"
#include "PluginFactory.h"
#include "PluginInstance.h"
#include "Song.h"
#include "Track.h"

using namespace lmms;

/**
 * Every built-in instrument and effect (phase 5 of docs/plano-refatoracao.md): made, playing notes, with its
 * parameters at their defaults, all at their minimum, all at their maximum and at random (fixed seeds), saved and
 * loaded, and at another sample rate. Nothing may crash, and the output must stay finite. Built with ASan/UBSan
 * too, nothing may touch memory it should not.
 */
class BuiltinPluginsTest : public QObject
{
	Q_OBJECT

	QTemporaryDir m_directory;

	//! Third-party hosts and players that need files of their own: not built-in sound
	static auto isHost(const QString& name) -> bool
	{
		return name == "ladspaeffect" || name == "lv2effect" || name == "lv2instrument" || name == "vsteffect"
			|| name == "vestige" || name == "carlarack" || name == "carlapatchbay" || name == "gigplayer"
			|| name == "patman";
	}

	void writeWav(const QString& path)
	{
		auto info = SF_INFO{};
		info.samplerate = static_cast<int>(Engine::audioEngine()->outputSampleRate());
		info.channels = 2;
		info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
		auto* file = sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
		QVERIFY(file);
		auto samples = std::vector<float>(44100 * 2);
		for (std::size_t i = 0; i < samples.size(); ++i)
		{
			samples[i] = static_cast<float>(std::sin(static_cast<double>(i) * 0.03));
		}
		sf_writef_float(file, samples.data(), static_cast<sf_count_t>(samples.size() / 2));
		sf_close(file);
	}

	//! Plays @p periods periods of the song; false (and why) when a frame is not finite
	static auto playsFinite(int periods, QString& why) -> bool
	{
		auto* song = Engine::getSong();
		auto* engine = Engine::audioEngine();
		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		auto finite = true;
		for (int period = 0; period < periods && finite; ++period)
		{
			for (const auto& frame : engine->renderNextPeriod())
			{
				if (!std::isfinite(frame[0]) || !std::isfinite(frame[1]))
				{
					why = QString{"not finite in period %1"}.arg(period);
					finite = false;
					break;
				}
			}
		}
		song->stop();
		return finite;
	}

	//! The plugin's parameters: its automatable models, its controls' included
	static auto parametersOf(Plugin* plugin) -> std::vector<AutomatableModel*>
	{
		auto models = std::vector<AutomatableModel*>{};
		for (auto* model : plugin->findChildren<AutomatableModel*>()) { models.push_back(model); }
		return models;
	}

	//! Notes over three octaves, short and long, for the instrument on @p track
	static void addNotes(InstrumentTrack* track)
	{
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		for (int note = 0; note < 24; ++note)
		{
			const auto length = note % 3 == 0 ? 96 : 12;
			clip->addNote(Note{TimePos{length}, TimePos{note * 16}, DefaultKey - 12 + (note * 5) % 36});
		}
	}

	//! The track playing what this row tests, and the plugin under test
	struct Setup
	{
		InstrumentTrack* track = nullptr;
		Plugin* plugin = nullptr;
	};

	auto setUp(const QString& name, bool instrument, bool withNotes = true) -> Setup
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		if (!track) { return {}; }
		if (!instrument)
		{
			if (!track->loadInstrument("tripleoscillator")) { return {}; }
		}
		else if (!track->loadInstrument(name)) { return {}; }
		if (instrument && (name == "audiofileprocessor" || name == "slicert"))
		{
			// A sample to play
			const auto file = m_directory.filePath("note.wav");
			writeWav(file);
			auto document = QDomDocument{};
			auto saved = document.createElement("saved");
			track->saveState(document, saved);
			auto settings = saved.firstChildElement();
			auto plugin = settings.elementsByTagName("instrument").item(0).firstChildElement();
			plugin.setAttribute("src", file);
			track->restoreState(settings);
		}
		if (instrument && name == "sf2player")
		{
			// A soundfont to play: an AVL drum kit
			track->instrument()->loadFile(QT_TESTCASE_SOURCEDIR "/../data/avldrums/Black_Pearl_4_LV2.sf2");
		}
		auto* mixer = Engine::mixer();
		const auto channelIndex = mixer->createChannel();
		track->mixerChannelModel()->setValue(channelIndex);
		auto* plugin = static_cast<Plugin*>(track->instrument());
		if (!instrument)
		{
			auto* chain = &mixer->mixerChannel(channelIndex)->m_fxChain;
			chain->appendEffect(std::make_unique<PluginInstance>(Plugin::Type::Effect, name, chain, nullptr));
			plugin = chain->effects().back()->plugin();
		}
		mixer->setUpTrackChannels();
		if (withNotes) { addNotes(track); }
		return {track, plugin};
	}

	void exercise(const QString& name, bool instrument)
	{
		auto setup = setUp(name, instrument);
		if (!setup.plugin) { QSKIP("not built here"); }
		const auto parameters = parametersOf(setup.plugin);
		qInfo("%s: %zu parameters", qPrintable(name), parameters.size());
		auto why = QString{};

		QVERIFY2(playsFinite(60, why), qPrintable("defaults: " + why));

		for (auto* model : parameters) { model->setValue(model->minValue<float>()); }
		QVERIFY2(playsFinite(60, why), qPrintable("all at their minimum: " + why));

		for (auto* model : parameters) { model->setValue(model->maxValue<float>()); }
		QVERIFY2(playsFinite(60, why), qPrintable("all at their maximum: " + why));

		for (const auto seed : {1u, 2u, 3u})
		{
			auto random = std::mt19937{seed};
			for (auto* model : parameters)
			{
				auto between = std::uniform_real_distribution<float>{model->minValue<float>(), model->maxValue<float>()};
				model->setValue(between(random));
			}
			QVERIFY2(playsFinite(60, why), qPrintable(QString{"random (seed %1): "}.arg(seed) + why));
		}

		// Turned while it plays, as the user turns a knob: the main thread sets the parameters while a thread renders
		// (under TSan: what process() reads, the knobs' slots must not write meanwhile)
		{
			auto* song = Engine::getSong();
			song->setPlayPos(0, Song::PlayMode::Song);
			song->playSong();
			auto done = std::atomic<bool>{false};
			auto finite = std::atomic<bool>{true};
			auto audio = std::thread{[&] {
				for (int period = 0; period < 80; ++period)
				{
					for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
					{
						if (!std::isfinite(frame[0]) || !std::isfinite(frame[1])) { finite = false; }
					}
				}
				done = true;
			}};
			auto random = std::mt19937{7};
			while (!done)
			{
				for (auto* model : parameters)
				{
					auto between = std::uniform_real_distribution<float>{model->minValue<float>(), model->maxValue<float>()};
					model->setValue(between(random));
				}
				QCoreApplication::processEvents();
				std::this_thread::sleep_for(std::chrono::milliseconds{1});
			}
			audio.join();
			song->stop();
			QVERIFY2(finite, "turned while it plays: not finite");
		}

		// Saved and loaded (as a project, or undo, does), then played
		auto document = QDomDocument{};
		auto saved = document.createElement("saved");
		setup.track->saveState(document, saved);
		setup.track->restoreState(saved.firstChildElement());
		QVERIFY2(playsFinite(40, why), qPrintable("saved and loaded: " + why));

		// At another sample rate, and back
		auto* engine = Engine::audioEngine();
		const auto rate = engine->outputSampleRate();
		engine->reopenAudioDevice(96000);
		engine->stopProcessing();
		QCOMPARE(engine->outputSampleRate(), sample_rate_t{96000});
		QVERIFY2(playsFinite(40, why), qPrintable("at 96 kHz: " + why));
		engine->reopenAudioDevice(192000);
		engine->stopProcessing();
		QVERIFY2(playsFinite(40, why), qPrintable("at 192 kHz: " + why));
		// Down from there: buffers sized for the higher rate shrink, and what indexes them must stay inside
		engine->reopenAudioDevice(rate);
		engine->stopProcessing();
		QVERIFY2(playsFinite(20, why), qPrintable("back at the first rate: " + why));
	}

	static void addRows(Plugin::Type type)
	{
		QTest::addColumn<QString>("plugin");
		auto names = QStringList{};
		for (const auto* descriptor : getPluginFactory()->descriptors(type))
		{
			const auto name = QString::fromUtf8(descriptor->name);
			if (!isHost(name)) { names << name; }
		}
		names.sort();
		for (const auto& name : names) { QTest::newRow(qPrintable(name)) << name; }
	}

private slots:
	void initTestCase()
	{
		QVERIFY(m_directory.isValid());
		// The sample rate changes on a device of no hardware (one that converts would keep its own rate)
		ConfigManager::inst()->setValue("audioengine", "audiodev", AudioDummy::name());
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	void instruments_data() { addRows(Plugin::Type::Instrument); }

	void instruments()
	{
		QFETCH(QString, plugin);
		exercise(plugin, true);
	}

	/**
	 * A note starts from silence: no jump from nothing to its first sample (a click on every note, as the Kicker
	 * made with its attack at 0). With the defaults, as a new instrument comes.
	 */
	void notesStartFromSilence_data() { addRows(Plugin::Type::Instrument); }

	void notesStartFromSilence()
	{
		QFETCH(QString, plugin);
		auto setup = setUp(plugin, true, false);
		if (!setup.plugin) { QSKIP("not built here"); }
		auto* clip = dynamic_cast<MidiClip*>(setup.track->createClip(TimePos{0}));
		// (a drum kit: its kick)
		clip->addNote(Note{TimePos{96}, TimePos{48}, plugin == "avldrums" || plugin == "sf2player" ? 36 : DefaultKey});
		auto* engine = Engine::audioEngine();
		auto* song = Engine::getSong();
		auto out = std::vector<float>{};
		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		const auto rate = static_cast<std::size_t>(engine->outputSampleRate());
		while (out.size() < rate * 2)
		{
			for (const auto& frame : engine->renderNextPeriod()) { out.push_back(frame[0] + frame[1]); }
		}
		song->stop();
		auto start = std::size_t{1};
		while (start < out.size() && std::abs(out[start]) < 1e-4f) { ++start; }
		QVERIFY2(start < out.size(), "silent");
		// How far the note gets in its first two frames (0.05 ms), against its peak in the first 10 ms: a sine at
		// this pitch from zero gets to an eighth of its peak; a click, from silence to most of it
		auto peak = 0.f;
		for (auto f = start; f < std::min(out.size(), start + rate / 100); ++f) { peak = std::max(peak, std::abs(out[f])); }
		const auto jump = std::max(std::abs(out[start]), std::abs(out[start + 1])) / peak;
		qInfo("%s: in two frames, %.3f of the peak", qPrintable(plugin), jump);
		QVERIFY2(jump < 0.25f, qPrintable(QString{"the note jumps from silence to %1 of its peak"}.arg(jump)));
	}

	void effects_data() { addRows(Plugin::Type::Effect); }

	void effects()
	{
		QFETCH(QString, plugin);
		exercise(plugin, false);
	}
};

QTEST_GUILESS_MAIN(BuiltinPluginsTest)
#include "BuiltinPluginsTest.moc"
