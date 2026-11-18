/*
 * GuitarixTest.cpp
 *
 * This file is part of LinuxMusic, based on LMMS - https://lmms.io
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
#include <complex>
#include <numbers>
#include <vector>

#include <QtTest>

#include "AudioDummy.h"
#include "AudioEngine.h"
#include "AutomatableModel.h"
#include "ConfigManager.h"
#include "DataFile.h"
#include "Effect.h"
#include "EffectChain.h"
#include "EffectControls.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Mixer.h"
#include "Note.h"
#include "PluginFactory.h"
#include "PluginInstance.h"
#include "Song.h"
#include "Track.h"

using namespace lmms;

//! Guitarix's units built in (plugins/Guitarix): listed as the plugin's sub-plugins, and playing as in Guitarix
class GuitarixTest : public QObject
{
	Q_OBJECT

	//! The key of the unit @p unit
	static auto keyOf(const QString& unit) -> Plugin::Descriptor::SubPluginFeatures::Key
	{
		const auto info = getPluginFactory()->pluginInfo("guitarix");
		auto attributes = Plugin::Descriptor::SubPluginFeatures::Key::AttributeMap{};
		attributes["unit"] = unit;
		return Plugin::Descriptor::SubPluginFeatures::Key{info.descriptor, unit, attributes};
	}

	//! A sine at 220 Hz (the TripleOscillator's first oscillator alone) through the unit @p unit on its channel,
	//! @p change setting its parameters (by name); none: no effect. @returns the left channel, a second in
	static auto play(const QString& unit, const std::function<void(const QList<AutomatableModel*>&)>& change)
		-> std::vector<float>
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		if (!track || !track->loadInstrument("tripleoscillator")) { return {}; }
		auto state = DataFile{DataFile::Type::InstrumentTrackSettings};
		track->saveState(state, state.content());
		auto settings = state.content().firstChildElement();
		auto oscillators = settings.elementsByTagName("instrument").item(0).firstChildElement();
		oscillators.setAttribute("wavetype0", 0);
		oscillators.setAttribute("vol0", 5); // (soft, as a guitar: a pedal does not saturate it all at once)
		oscillators.setAttribute("vol1", 0);
		oscillators.setAttribute("vol2", 0);
		track->restoreState(settings);

		auto* mixer = Engine::mixer();
		const auto channel = mixer->createChannel();
		track->mixerChannelModel()->setValue(channel);
		if (!unit.isEmpty())
		{
			auto* chain = &mixer->mixerChannel(channel)->m_fxChain;
			const auto key = keyOf(unit);
			chain->appendEffect(std::make_unique<PluginInstance>(Plugin::Type::Effect, "guitarix", chain, &key));
			auto* plugin = chain->effects().back()->plugin();
			if (!plugin) { return {}; }
			if (change) { change(plugin->findChildren<AutomatableModel*>()); }
		}
		mixer->setUpTrackChannels();
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		clip->addNote(Note{TimePos{4 * TimePos::ticksPerBar()}, TimePos{0}, DefaultKey - 12});
		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		auto left = std::vector<float>{};
		const auto rate = Engine::audioEngine()->outputSampleRate();
		while (left.size() < rate)
		{
			for (const auto& frame : Engine::audioEngine()->renderNextPeriod()) { left.push_back(frame.left()); }
		}
		song->stop();
		return left;
	}

	//! The power of @p samples at @p frequency (a Hann windowed correlation over the second half)
	static auto powerAt(const std::vector<float>& samples, double frequency) -> double
	{
		const auto rate = static_cast<double>(Engine::audioEngine()->outputSampleRate());
		const auto from = samples.size() / 2;
		const auto count = samples.size() - from;
		auto sum = std::complex<double>{};
		for (std::size_t i = 0; i < count; ++i)
		{
			const auto window = 0.5 - 0.5 * std::cos(2 * std::numbers::pi * i / count);
			sum += samples[from + i] * window * std::polar(1.0, -2 * std::numbers::pi * frequency * i / rate);
		}
		return std::norm(sum);
	}

	//! How far from a sine @p samples are: the power of the harmonics 2 to 10 against the fundamental's, in dB
	static auto distortion(const std::vector<float>& samples) -> double
	{
		auto harmonics = 0.0;
		for (int k = 2; k <= 10; ++k) { harmonics += powerAt(samples, 220.0 * k); }
		return 10 * std::log10(harmonics / powerAt(samples, 220.0));
	}

	static auto loudest(const std::vector<float>& samples) -> float
	{
		auto result = 0.f;
		for (const auto sample : samples) { result = std::max(result, std::abs(sample)); }
		return result;
	}

	static void set(const QList<AutomatableModel*>& models, const QString& name, float value)
	{
		for (auto* model : models)
		{
			if (model->displayName() == name) { model->setValue(value); return; }
		}
		QFAIL(qPrintable("no parameter " + name));
	}

	//! A note through the unit @p unit (its key), its parameters as @p change sets them; the song plays on
	static auto setUpUnit(const QString& unit, bool withNotes = true) -> QList<AutomatableModel*>
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		if (!track || !track->loadInstrument("tripleoscillator")) { return {}; }
		auto* mixer = Engine::mixer();
		const auto channel = mixer->createChannel();
		track->mixerChannelModel()->setValue(channel);
		auto* chain = &mixer->mixerChannel(channel)->m_fxChain;
		const auto key = keyOf(unit);
		chain->appendEffect(std::make_unique<PluginInstance>(Plugin::Type::Effect, "guitarix", chain, &key));
		auto* plugin = chain->effects().back()->plugin();
		if (!plugin) { return {}; }
		mixer->setUpTrackChannels();
		if (withNotes)
		{
			auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
			for (int note = 0; note < 8; ++note)
			{
				clip->addNote(Note{TimePos{24}, TimePos{note * 24}, DefaultKey - 12 + note * 3});
			}
			song->setPlayPos(0, Song::PlayMode::Song);
			song->playSong();
		}
		return plugin->findChildren<AutomatableModel*>();
	}

	//! @p periods periods' samples (both channels)
	static auto renderPeriods(int periods) -> std::vector<float>
	{
		auto samples = std::vector<float>{};
		for (int period = 0; period < periods; ++period)
		{
			for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
			{
				samples.push_back(frame.left());
				samples.push_back(frame.right());
			}
		}
		return samples;
	}

	//! Renders @p periods periods: whether every sample is finite (@p loudest: the loudest)
	static auto rendersFinite(int periods, float& loudest) -> bool
	{
		loudest = 0.f;
		for (int period = 0; period < periods; ++period)
		{
			for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
			{
				if (!std::isfinite(frame.left()) || !std::isfinite(frame.right())) { return false; }
				loudest = std::max({loudest, std::abs(frame.left()), std::abs(frame.right())});
			}
		}
		return true;
	}

private slots:
	void initTestCase()
	{
		ConfigManager::inst()->setValue("audioengine", "audiodev", AudioDummy::name());
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase() { Engine::destroy(); }

	//! Its units are the plugin's sub-plugins, by Guitarix's ids, with Guitarix's names
	void unitsAreSubPlugins()
	{
		const auto info = getPluginFactory()->pluginInfo("guitarix");
		if (info.isNull()) { QSKIP("not built here"); }
		QVERIFY(info.descriptor->subPluginFeatures);
		auto keys = Plugin::Descriptor::SubPluginFeatures::KeyList{};
		info.descriptor->subPluginFeatures->listSubPluginKeys(info.descriptor, keys);
		auto names = QStringList{};
		for (const auto& key : keys) { names << key.attributes.value("unit") + "=" + key.displayName(); }
		QVERIFY2(names.contains("ts9sim=Tube Screamer"), qPrintable(names.join(", ")));
	}

	void everyUnit_data()
	{
		QTest::addColumn<QString>("unit");
		const auto info = getPluginFactory()->pluginInfo("guitarix");
		if (info.isNull()) { return; }
		auto keys = Plugin::Descriptor::SubPluginFeatures::KeyList{};
		info.descriptor->subPluginFeatures->listSubPluginKeys(info.descriptor, keys);
		for (const auto& key : keys)
		{
			const auto unit = key.attributes.value("unit");
			QTest::newRow(qPrintable(unit)) << unit;
		}
	}

	//! Every unit: as it comes, all at their minimum and at their maximum, its sound finite; then brought to rest, it
	//! is silent (an export, a new project: its past let go)
	void everyUnit()
	{
		QFETCH(QString, unit);
		auto models = setUpUnit(unit);
		QVERIFY2(!models.isEmpty(), "not made");
		auto loudest = 0.f;
		QVERIFY2(rendersFinite(60, loudest), "as it comes");
		for (auto* model : models) { model->setValue(model->minValue<float>()); }
		QVERIFY2(rendersFinite(60, loudest), "at the minimum");
		for (auto* model : models) { model->setValue(model->maxValue<float>()); }
		QVERIFY2(rendersFinite(60, loudest), "at the maximum");
		for (auto* model : models) { model->reset(); }
		QVERIFY2(rendersFinite(60, loudest), "back as it came");

		// Brought to rest after two pasts (at the maximum, and as it comes), the same parameters then: the same
		// sound after (an export the same whatever played before; some give a constant at silence, Guitarix's diode
		// tables not being symmetric and Guitarix blocking the DC at the start of its chain only)
		const auto afterRest = [&] {
			Engine::getSong()->stop();
			for (auto* track : Engine::getSong()->tracks()) { track->setMuted(true); }
			Engine::mixer()->discardProcessingState();
			return renderPeriods(8);
		};
		const auto first = afterRest();
		for (auto* track : Engine::getSong()->tracks()) { track->setMuted(false); }
		Engine::getSong()->setPlayPos(0, Song::PlayMode::Song);
		Engine::getSong()->playSong();
		QVERIFY2(rendersFinite(60, loudest), "again");
		const auto second = afterRest();
		QCOMPARE(second.size(), first.size());
		auto difference = 0.f;
		for (std::size_t i = 0; i < first.size(); ++i) { difference = std::max(difference, std::abs(first[i] - second[i])); }
		QVERIFY2(difference < 1e-6f, qPrintable(QString{"after a rest it depends on what played before: %1"}.arg(difference)));
	}

	void selectorsChooseTheirModules_data()
	{
		QTest::addColumn<QString>("unit");
		QTest::addColumn<int>("modules");
		QTest::newRow("crybaby") << "crybaby" << 2;
		QTest::newRow("wah") << "wah" << 10;
		QTest::newRow("amp.tonestack") << "amp.tonestack" << 27;
		QTest::newRow("poweramp") << "poweramp" << 6;
		// (the convolvers: their impulse response changes on the main thread, as their update runs)
		QTest::newRow("cab") << "cab" << 17;
		QTest::newRow("pre") << "pre" << 10;
		// (the amp head's amps: its 19 tube models, "---" the last)
		QTest::newRow("ampstack") << "ampstack" << 19;
	}

	//! A unit that is a choice among others (Guitarix's ModuleSelectorFromList; a convolver's impulse responses):
	//! each of its modules, chosen, sounds (finite), and not all the same
	void selectorsChooseTheirModules()
	{
		if (getPluginFactory()->pluginInfo("guitarix").isNull()) { QSKIP("not built here"); }
		QFETCH(QString, unit);
		QFETCH(int, modules);
		const auto models = setUpUnit(unit);
		QVERIFY2(!models.isEmpty(), "not made");
		auto* selector = static_cast<AutomatableModel*>(nullptr);
		for (auto* model : models) { if (model->displayName() == "select") { selector = model; } }
		QVERIFY(selector);
		QCOMPARE(selector->maxValue<int>() - selector->minValue<int>() + 1, modules);
		auto sounds = std::vector<std::vector<float>>{};
		for (int module = 0; module < modules; ++module)
		{
			selector->setValue(static_cast<float>(module));
			// (a convolver's update: queued, then on its timer, 200 ms)
			QTest::qWait(450);
			renderPeriods(4);
			auto sound = renderPeriods(8);
			for (const auto sample : sound) { QVERIFY2(std::isfinite(sample), qPrintable(QString::number(module))); }
			sounds.push_back(std::move(sound));
		}
		auto differ = false;
		for (std::size_t m = 1; m < sounds.size() && !differ; ++m)
		{
			for (std::size_t i = 0; i < sounds[m].size() && i < sounds[0].size(); ++i)
			{
				if (std::abs(sounds[m][i] - sounds[0][i]) > 1e-4f) { differ = true; break; }
			}
		}
		QVERIFY2(differ, "every module sounds the same");
	}

	//! A mono unit runs in two instances, one for each channel; the Vibe's LFO keeps its state in its file (Guitarix
	//! runs one instance): each instance has its own, so the same input on both channels comes out the same on both
	//! (were it shared, it would run twice as fast, the channels apart)
	void vibeChannelsAlike()
	{
		if (getPluginFactory()->pluginInfo("guitarix").isNull()) { QSKIP("not built here"); }
		const auto models = setUpUnit("univibe_mono");
		QVERIFY2(!models.isEmpty(), "not made");
		const auto samples = renderPeriods(80);
		auto loudest = 0.f;
		auto difference = 0.f;
		for (std::size_t i = 0; i + 1 < samples.size(); i += 2)
		{
			loudest = std::max(loudest, std::abs(samples[i]));
			difference = std::max(difference, std::abs(samples[i] - samples[i + 1]));
		}
		QVERIFY2(loudest > 0.01f, "silent");
		QVERIFY2(difference < 1e-6f, qPrintable(QString{"left and right differ: %1"}.arg(difference)));
	}

	//! What a unit measures (its meters' levels: Guitarix's output parameters) is not a control: no knob, no
	//! automation, not saved; what it is set by is
	void outputsAreNotControls()
	{
		if (getPluginFactory()->pluginInfo("guitarix").isNull()) { QSKIP("not built here"); }
		const auto models = setUpUnit("mbc");
		QVERIFY2(!models.isEmpty(), "not made");
		auto names = QStringList{};
		for (const auto* model : models) { names << model->displayName(); }
		QVERIFY2(names.contains("Ratio1") && !names.contains("V1"), qPrintable(names.join(", ")));

		// (the unit's effect: on the channel it was put on, wherever setting up the tracks' channels left it)
		auto* mixer = Engine::mixer();
		auto* effect = static_cast<Effect*>(nullptr);
		for (mix_ch_t channel = 0; channel < mixer->numChannels() && !effect; ++channel)
		{
			for (const auto& instance : mixer->mixerChannel(channel)->m_fxChain.effects())
			{
				if (auto* candidate = dynamic_cast<Effect*>(instance->plugin());
					candidate && candidate->descriptor()->name == QStringLiteral("guitarix"))
				{
					effect = candidate;
				}
			}
		}
		QVERIFY(effect && effect->controls());
		auto document = QDomDocument{};
		auto parent = document.createElement("effect");
		const auto element = effect->controls()->saveState(document, parent);
		QVERIFY(element.hasAttribute("mbc.Ratio1"));
		QVERIFY(!element.hasAttribute("mbc.v1"));
		QVERIFY(effect->controls()->controlCount() > 0);
		// (each control saved by its id, an attribute or, logarithmic, an element; the rest is the object's own)
		auto saved = 0;
		for (int i = 0; i < element.attributes().count(); ++i)
		{
			if (element.attributes().item(i).nodeName().startsWith("mbc.")) { ++saved; }
		}
		for (auto child = element.firstChildElement(); !child.isNull(); child = child.nextSiblingElement())
		{
			if (child.tagName().startsWith("mbc.")) { ++saved; }
		}
		QVERIFY(element.firstChildElement("mbc.v1").isNull());
		QCOMPARE(saved, effect->controls()->controlCount());
	}

	//! The Tube Screamer (Guitarix's ts9sim, run at 96 kHz as there): a sine comes out with harmonics, more with
	//! more drive; its level knob is its output level, in dB
	void tubeScreamer()
	{
		if (getPluginFactory()->pluginInfo("guitarix").isNull()) { QSKIP("not built here"); }
		const auto clean = play({}, nullptr);
		QVERIFY(!clean.empty());
		const auto low = play("ts9sim", [](const auto& models) { set(models, "Drive", 0.1f); set(models, "Level", -6.f); });
		const auto high = play("ts9sim", [](const auto& models) { set(models, "Drive", 1.f); set(models, "Level", -6.f); });
		const auto quieter = play("ts9sim", [](const auto& models) { set(models, "Drive", 1.f); set(models, "Level", -18.f); });
		QVERIFY(!low.empty() && !high.empty() && !quieter.empty());

		const auto name = QString{"distortion: clean %1 dB, drive 0.1 %2 dB, drive 1 %3 dB"}
			.arg(distortion(clean)).arg(distortion(low)).arg(distortion(high));
		QVERIFY2(distortion(clean) < -60, qPrintable(name));
		QVERIFY2(distortion(low) > -40, qPrintable(name));
		QVERIFY2(distortion(high) > distortion(low) + 3, qPrintable(name));
		// 12 dB less level: a quarter as loud (the same distortion before it)
		const auto ratio = loudest(quieter) / loudest(high);
		QVERIFY2(std::abs(ratio - 0.251f) < 0.03f, qPrintable(QString{"level: %1"}.arg(ratio)));
		for (const auto sample : high) { QVERIFY(std::isfinite(sample)); }
	}
};

QTEST_GUILESS_MAIN(GuitarixTest)
#include "GuitarixTest.moc"
