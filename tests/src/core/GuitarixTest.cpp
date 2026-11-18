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
#include "EffectChain.h"
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
