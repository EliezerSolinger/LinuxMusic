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

#include <algorithm>
#include <array>
#include <cmath>
#include <functional>
#include <vector>

#include <QDataStream>
#include <QDirIterator>
#include <QFile>
#include <QTemporaryDir>
#include <QtTest>

#include "AudioDummy.h"
#include "AutomationClip.h"
#include "AudioEngine.h"
#include "ConfigManager.h"
#include "DataFile.h"
#include "Engine.h"
#include "EffectChain.h"
#include "PluginInstance.h"
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Mixer.h"
#include "Note.h"
#include "PathUtil.h"
#include "SampleFrame.h"
#include "SampleBuffer.h"
#include "Sample.h"
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

	//! Writes @p frames (stereo, -1 to 1) as a 16 bit WAV file at @p sampleRate
	static void writeWav(const QString& path, const std::vector<std::array<float, 2>>& frames, int sampleRate)
	{
		auto file = QFile{path};
		QVERIFY(file.open(QIODevice::WriteOnly));
		auto stream = QDataStream{&file};
		stream.setByteOrder(QDataStream::LittleEndian);
		const auto dataBytes = static_cast<quint32>(frames.size() * 4);
		stream.writeRawData("RIFF", 4);
		stream << quint32{36 + dataBytes};
		stream.writeRawData("WAVEfmt ", 8);
		stream << quint32{16} << quint16{1} << quint16{2} << quint32(sampleRate) << quint32(sampleRate * 4)
			<< quint16{4} << quint16{16};
		stream.writeRawData("data", 4);
		stream << dataBytes;
		for (const auto& frame : frames)
		{
			for (const auto sample : frame) { stream << static_cast<qint16>(std::clamp(sample, -1.f, 1.f) * 32767); }
		}
	}

	//! A drum loop: a kick on beats 1 and 3, a snare on 2 and 4, a hi-hat on every eighth, @p bars bars at @p bpm
	//! and @p tail seconds more (the hats go on)
	//! (@p fourOnTheFloor: a kick on every beat and nothing else)
	static auto drumLoop(double bpm, int bars, double tail, int sampleRate, bool fourOnTheFloor = false)
		-> std::vector<std::array<float, 2>>
	{
		const auto beat = 60.0 / bpm;
		const auto length = static_cast<std::size_t>((bars * 4 * beat + tail) * sampleRate);
		auto frames = std::vector<std::array<float, 2>>(length, {0.f, 0.f});
		auto noise = std::uint32_t{12345};
		const auto random = [&noise] { noise = noise * 1664525u + 1013904223u; return (noise >> 8) / 8388608.f - 1.f; };
		const auto add = [&](double at, double seconds, auto&& sound) {
			const auto first = static_cast<std::size_t>(at * sampleRate);
			for (auto i = std::size_t{0}; i < static_cast<std::size_t>(seconds * sampleRate) && first + i < length; ++i)
			{
				const auto value = sound(static_cast<double>(i) / sampleRate);
				frames[first + i][0] += value;
				frames[first + i][1] += value;
			}
		};
		for (auto eighth = 0; eighth * beat / 2 < length / static_cast<double>(sampleRate); ++eighth)
		{
			const auto at = eighth * beat / 2;
			const auto beatInBar = (eighth / 2) % 4;
			if (fourOnTheFloor)
			{
				if (eighth % 2 == 0)
				{
					add(at, 0.25, [](double t) { return 0.8 * std::sin(2 * M_PI * 55 * t - 30 * std::exp(-t * 30)) * std::exp(-t * 12); });
				}
				continue;
			}
			if (eighth % 2 == 0 && (beatInBar == 0 || beatInBar == 2))
			{
				add(at, 0.25, [](double t) { return 0.8 * std::sin(2 * M_PI * 55 * t - 30 * std::exp(-t * 30)) * std::exp(-t * 12); });
			}
			if (eighth % 2 == 0 && (beatInBar == 1 || beatInBar == 3))
			{
				add(at, 0.15, [&](double t) { return 0.5 * random() * std::exp(-t * 25); });
			}
			add(at, 0.04, [&](double t) { return 0.2 * random() * std::exp(-t * 120); });
		}
		return frames;
	}

	//! Loads @p file into a new SlicerT; @returns its saved settings
	static auto slicerWith(const QString& file) -> QDomElement
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		if (!track || !track->loadInstrument("slicert")) { return {}; }
		track->instrument()->loadFile(file);
		auto state = DataFile{DataFile::Type::InstrumentTrackSettings};
		track->saveState(state, state.content());
		return state.content().firstChildElement().elementsByTagName("instrument").item(0).firstChildElement();
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

	/**
	 * #1207, #7852: the instruments as they come played at very different levels (Nescaline about 28 dB over
	 * ZynAddSubFX). A note (A4) of each new instrument, its loudness at its loudest moment: all about the same
	 * (LINUXMUSIC_PRINT_LEVELS prints them). Only new instruments: projects and presets keep what they saved.
	 */
	void newInstrumentsPlayAtSimilarLevels()
	{
		auto levels = std::vector<std::pair<QString, double>>{};
		for (const auto* name : {"tripleoscillator", "bitinvader", "kicker", "lb302", "monstro", "nes", "organic",
			"sfxr", "sid", "vibedstrings", "watsyn", "xpressive", "freeboy", "malletsstk", "opulenz", "zynaddsubfx"})
		{
			const auto played = play(name, nullptr, 180, name == QString{"kicker"} ? DefaultKey - 24 : DefaultKey);
			if (played.samples.empty()) { continue; }
			// The note's second (it starts at the first frame)
			const auto rate = static_cast<std::size_t>(Engine::audioEngine()->outputSampleRate());
			// Its loudest moment: the RMS of 50 ms windows (a drum or a bell is loud at its start only)
			const auto window = 2 * rate / 20;
			auto rms = 0.0;
			for (std::size_t start = 0; start + window <= std::min(played.samples.size(), 2 * rate); start += window / 2)
			{
				auto sum = 0.0;
				for (auto i = start; i < start + window; ++i) { sum += played.samples[i] * played.samples[i]; }
				rms = std::max(rms, std::sqrt(sum / static_cast<double>(window)));
			}
			levels.emplace_back(name, 20 * std::log10(std::max(rms, 1e-9)));
		}
		auto lowest = 0.0, highest = -200.0;
		for (const auto& [name, db] : levels)
		{
			if (qEnvironmentVariableIsSet("LINUXMUSIC_PRINT_LEVELS")) { qInfo("%-18s %6.1f dB", qPrintable(name), db); }
			lowest = std::min(lowest, db);
			highest = std::max(highest, db);
		}
		QVERIFY(!levels.empty());
		if (qEnvironmentVariableIsSet("LINUXMUSIC_PRINT_LEVELS")) { qInfo("spread %.1f dB", highest - lowest); }
		// About -12 dB each at its loudest, within a few dB (it was 30 dB from the quietest to the loudest)
		for (const auto& [name, db] : levels)
		{
			QVERIFY2(db > -18 && db < -8, qPrintable(QString{"%1 at %2 dB"}.arg(name).arg(db, 0, 'f', 1)));
		}
		QVERIFY2(highest - lowest < 8, qPrintable(QString{"%1 dB between them"}.arg(highest - lowest, 0, 'f', 1)));
	}

	/**
	 * #8043: FreeBoy's outputs did not go where they said: the channel buttons were labelled the other way round
	 * (each saying SO2 (Left) went right), and the two output levels acted as one (the emulator has one volume).
	 */
	void freeboyOutputsGoWhereTheySay()
	{
		const auto sides = [&](const std::function<void(QDomElement&)>& change) {
			const auto played = play("freeboy", change, 150);
			auto left = 0.0, right = 0.0;
			for (std::size_t i = 0; i + 1 < played.samples.size(); i += 2)
			{
				left += played.samples[i] * played.samples[i];
				right += played.samples[i + 1] * played.samples[i + 1];
			}
			return std::pair{std::sqrt(left), std::sqrt(right)};
		};
		// The right output's level only (SO1): nothing on the left
		const auto [l1, r1] = sides([](QDomElement& e) { e.setAttribute("so1vol", 7); e.setAttribute("so2vol", 0); });
		QVERIFY(r1 > 0 && l1 < r1 * 1e-3);
		// And the left's only (SO2)
		const auto [l2, r2] = sides([](QDomElement& e) { e.setAttribute("so1vol", 0); e.setAttribute("so2vol", 7); });
		QVERIFY(l2 > 0 && r2 < l2 * 1e-3);
		// Channel 1 alone, to SO1 (Right) (its model saved as "ch1so2"): mostly right
		const auto [l3, r3] = sides([](QDomElement& e) {
			for (const auto* attribute : {"ch1so1", "ch2so1", "ch3so1", "ch4so1", "ch2so2", "ch3so2", "ch4so2"})
			{
				e.setAttribute(attribute, 0);
			}
			e.setAttribute("ch1so2", 1);
		});
		QVERIFY2(r3 > 5 * l3, qPrintable(QString{"left %1, right %2"}.arg(l3).arg(r3)));
	}

	/**
	 * #5855: a sample's start, end and loop point clicked (a jump from silence, to silence, or from the loop's end to
	 * its start). With the microfades (Sample::setDeclick(), on in new AudioFileProcessors) a sample that is all at
	 * full level starts from near silence, ends near it, and wraps its loop without a jump.
	 */
	void sampleMicrofades()
	{
		const auto rate = static_cast<int>(Engine::audioEngine()->outputSampleRate());
		// A ramp: every frame differs from the next by little, but the loop's end and start are far apart
		auto data = std::vector<SampleFrame>(static_cast<std::size_t>(rate));
		for (std::size_t i = 0; i < data.size(); ++i)
		{
			const auto value = 0.2f + 0.6f * static_cast<float>(i) / static_cast<float>(data.size());
			data[i] = SampleFrame{value, value};
		}
		auto sample = Sample{std::make_shared<const SampleBuffer>(std::move(data), rate)};
		const auto loopStart = rate / 4, loopEnd = rate * 3 / 4;
		sample.setAllPointFrames(0, rate, loopStart, loopEnd);
		const auto largestJump = [&](bool declick, Sample::Loop loop, std::vector<SampleFrame>& out) {
			sample.setDeclick(declick);
			auto state = Sample::PlaybackState{};
			out.assign(static_cast<std::size_t>(rate * 2), SampleFrame{});
			sample.play(out.data(), &state, out.size(), loop, 1.0);
			auto jump = std::abs(out[0][0]); // (from the silence before)
			for (std::size_t i = 1; i < out.size(); ++i) { jump = std::max(jump, std::abs(out[i][0] - out[i - 1][0])); }
			return jump;
		};
		auto out = std::vector<SampleFrame>{};
		// Without them: the start (0.2) and the wrap (0.6) are jumps
		QVERIFY(largestJump(false, Sample::Loop::On, out) > 0.1f);
		// With them: no jump bigger than a few steps of the ramp
		QVERIFY2(largestJump(true, Sample::Loop::On, out) < 0.01f, qPrintable(QString::number(largestJump(true, Sample::Loop::On, out))));
		QVERIFY2(largestJump(true, Sample::Loop::Off, out) < 0.01f, qPrintable(QString::number(largestJump(true, Sample::Loop::Off, out))));
	}

	//! #7980: LB302's resonance goes to 1.25, and its automation stopped at 1
	void lb302ResonanceAutomationReachesItsTop()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QVERIFY(track->loadInstrument("lb302"));
		AutomatableModel* resonance = nullptr;
		for (auto* model : track->instrument()->findChildren<AutomatableModel*>())
		{
			if (model->displayName() == "VCF Resonance") { resonance = model; }
		}
		QVERIFY(resonance);
		auto* automationTrack = Track::create(Track::Type::Automation, song);
		auto* clip = dynamic_cast<AutomationClip*>(automationTrack->createClip(TimePos{0}));
		clip->addObject(resonance);
		clip->putValue(TimePos{0}, 1.25f, false);
		clip->putValue(TimePos{TimePos::ticksPerBar()}, 1.25f, false);
		QCOMPARE(clip->valueAt(TimePos{TimePos::ticksPerBar() / 2}), 1.25f);
		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		for (int period = 0; period < 20; ++period) { Engine::audioEngine()->renderNextPeriod(); }
		qInfo("resonance while playing: %g", resonance->value<float>());
		QVERIFY(resonance->value<float>() > 1.2f);
		song->stop();
	}

	//! #3046: Monstro's pulse at its narrowest was shorter than a sample at high notes: missed, or not, as the phase
	//! fell (silent or wrong notes); and a modulated width could not go under 25%
	void monstroNarrowestPulseSounds()
	{
		for (const int key : {DefaultKey, DefaultKey + 12, DefaultKey + 24, DefaultKey + 31})
		{
			const auto played = play("monstro", [](QDomElement& monstro) {
				monstro.setAttribute("o1pw", 0.25);
				monstro.setAttribute("o1vol", 100);
				monstro.setAttribute("o2vol", 0);
				monstro.setAttribute("o3vol", 0);
			}, 60, key);
			// What it plays, its mean (a constant) taken out
			auto mean = 0.0;
			for (const auto sample : played.samples) { mean += sample; }
			mean /= static_cast<double>(played.samples.size());
			auto sum = 0.0;
			for (const auto sample : played.samples) { sum += (sample - mean) * (sample - mean); }
			const auto rms = std::sqrt(sum / static_cast<double>(played.samples.size()));
			QVERIFY2(rms > 1e-3, qPrintable(QString{"key %1: %2"}.arg(key).arg(rms)));
		}
		// Modulated (a touch of LFO), at its narrowest: the pulse is high a small part of the time, not a quarter
		const auto played = play("monstro", [](QDomElement& monstro) {
			monstro.setAttribute("o1pw", 0.25);
			monstro.setAttribute("o1vol", 100);
			monstro.setAttribute("o2vol", 0);
			monstro.setAttribute("o3vol", 0);
			monstro.setAttribute("w1l1", 0.001);
		}, 60, DefaultKey - 24);
		auto highest = 0.f;
		for (const auto sample : played.samples) { highest = std::max(highest, sample); }
		auto high = 0;
		for (const auto sample : played.samples) { high += sample > highest * 0.5f; }
		const auto duty = static_cast<double>(high) / static_cast<double>(played.samples.size());
		QVERIFY2(duty < 0.05, qPrintable(QString{"high %1% of the time"}.arg(duty * 100)));
	}

	//! #8163: a patch Patman could not load was only printed to the console; the user is told now
	void patmanSaysWhenAPatchDoesNotLoad()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		if (!track->loadInstrument("patman")) { QSKIP("not built here"); }
		song->errorSummary(); // (what was collected before goes)
		track->instrument()->loadFile("/nowhere/missing.pat");
		QVERIFY(song->hasErrors());
		QVERIFY(song->errorSummary().contains("missing.pat"));
	}

	/**
	 * #7839: a factory preset (Organ_leslie, with a LADSPA effect) warned when loaded. Every factory preset loads
	 * with no error, its instrument and its effects there.
	 */
	void factoryPresetsLoadCleanly()
	{
		auto* song = Engine::getSong();
		auto presets = QStringList{};
		QDirIterator it{QT_TESTCASE_SOURCEDIR "/../data/presets", {"*.xpf"}, QDir::Files, QDirIterator::Subdirectories};
		// (the Sf2 Player's all read the same 14 MB soundfont: three of them are enough)
		auto sf2Presets = 0;
		while (it.hasNext())
		{
			const auto preset = it.next();
			if (preset.contains("/Sf2Player/") && ++sf2Presets > 3) { continue; }
			presets << preset;
		}
		QVERIFY(presets.size() > 50);
		for (const auto& preset : presets)
		{
			song->createNewProject();
			const auto before = song->errorSummary();
			auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
			auto dataFile = DataFile{preset};
			track->loadPreset(dataFile.content().toElement());
			const auto errors = song->errorSummary().mid(before.size());
			QVERIFY2(errors.isEmpty(), qPrintable(preset + ": " + errors));
			QVERIFY2(track->instrument(), qPrintable(preset));
			for (const auto& effect : track->audioBusHandle()->effects()->effects())
			{
				QVERIFY2(effect->plugin(), qPrintable(preset + ": an effect did not load"));
			}
		}
	}

	/**
	 * #7573: the banded waveguide presets (uniform bar, tuned bar, glass, Tibetan bowl) gave only a quiet buzz: the
	 * voice's frequency was set each period (which clears its delays), and bowed they were silent (about -80 dB:
	 * the bow's controls were outside the range where the bars sing). Every preset rings, tonal and audible; bowed,
	 * it goes on while the note is held; plucked (an old project's "strike") too
	 */
	void malletsPresetsRing()
	{
		struct Case { int preset; bool bowed; };
		auto cases = std::vector<Case>{};
		for (int preset = 0; preset < 14; ++preset) { cases.push_back({preset, true}); }
		for (int preset = 10; preset < 14; ++preset) { cases.push_back({preset, false}); }
		for (const auto& [preset, bowed] : cases)
		{
			const auto name = QString{"preset %1%2"}.arg(preset).arg(bowed ? "" : " plucked");
			const auto played = play("malletsstk", [preset, bowed](QDomElement& mallets) {
				mallets.setAttribute("preset", preset);
				mallets.setAttribute("strike", bowed ? 1 : 0);
			}, 400);
			QVERIFY2(played.finite, qPrintable(name));
			QVERIFY2(played.loudest > 0.1f && played.loudest < 4.f,
				qPrintable(QString{"%1: loudest %2"}.arg(name).arg(played.loudest)));

			// Tonal: the best normalized autocorrelation of the left channel, over lags of 20 to 400 frames
			auto left = std::vector<float>{};
			for (auto i = std::size_t{4000}; i < std::min(played.samples.size(), std::size_t{4000 + 2 * 8192}); i += 2)
			{
				left.push_back(played.samples[i]);
			}
			auto energy = 0.0;
			for (const auto sample : left) { energy += sample * sample; }
			auto periodicity = 0.0;
			for (std::size_t lag = 20; lag < 400; ++lag)
			{
				auto correlation = 0.0;
				for (std::size_t i = 0; i + lag < left.size(); ++i) { correlation += left[i] * left[i + lag]; }
				periodicity = std::max(periodicity, correlation / energy);
			}
			QVERIFY2(periodicity > 0.6, qPrintable(QString{"%1: periodicity %2"}.arg(name).arg(periodicity)));

			// Bowed bars sing on (the last quarter of a held note as loud as a third of the loudest)
			if (bowed && preset >= 10)
			{
				auto lastQuarter = 0.f;
				for (auto i = played.samples.size() * 3 / 4; i < played.samples.size(); ++i)
				{
					lastQuarter = std::max(lastQuarter, std::abs(played.samples[i]));
				}
				QVERIFY2(lastQuarter > played.loudest / 3,
					qPrintable(QString{"%1: %2 at the end, %3 at most"}.arg(name).arg(lastQuarter).arg(played.loudest)));
			}
		}
	}

	/**
	 * #8102: SlicerT's tempo was the sample's length taken as a bar, doubled or halved into 100 to 200 BPM: right
	 * only for loops of whole bars. It finds the beat: whole bars or not, it is the tempo played (87 and 174 are the
	 * same beat, and either is right)
	 */
	void slicerTFindsTheTempo()
	{
		const auto dir = QTemporaryDir{};
		QVERIFY(dir.isValid());
		for (const auto bpm : {87.0, 100.0, 125.0, 140.0, 160.0, 174.0})
		{
			for (const auto& [bars, tail] : {std::pair{2, 0.0}, std::pair{4, 0.0}, std::pair{2, 0.37}, std::pair{3, 0.11}})
			{
				const auto path = dir.filePath(QString{"loop-%1-%2-%3.wav"}.arg(bpm).arg(bars).arg(tail));
				writeWav(path, drumLoop(bpm, bars, tail, 44100), 44100);
				const auto settings = slicerWith(path);
				QVERIFY(!settings.isNull());
				const auto found = settings.attribute("origBPM").toDouble();
				const auto name = QString{"%1 BPM, %2 bars and %3 s: %4"}.arg(bpm).arg(bars).arg(tail).arg(found);
				const auto right = std::abs(found - bpm) <= 1
					|| (bpm == 87.0 && std::abs(found - 174) <= 1) || (bpm == 174.0 && std::abs(found - 87) <= 1);
				QVERIFY2(right, qPrintable(name));
			}
		}
		for (const auto bpm : {118.0, 128.0, 135.0})
		{
			const auto path = dir.filePath(QString{"kicks-%1.wav"}.arg(bpm));
			writeWav(path, drumLoop(bpm, 4, 0.2, 48000, true), 48000);
			const auto found = slicerWith(path).attribute("origBPM").toDouble();
			QVERIFY2(std::abs(found - bpm) <= 1, qPrintable(QString{"%1 BPM, four on the floor: %2"}.arg(bpm).arg(found)));
		}

		// No beat (a held tone): the length's estimate, as before (a bar of 2.4 s: 100 BPM)
		const auto path = dir.filePath("tone.wav");
		auto tone = std::vector<std::array<float, 2>>(2.4 * 44100);
		for (auto i = std::size_t{0}; i < tone.size(); ++i)
		{
			const auto value = static_cast<float>(0.4 * std::sin(2 * M_PI * 220 * i / 44100.0));
			tone[i] = {value, value};
		}
		writeWav(path, tone, 44100);
		QCOMPARE(slicerWith(path).attribute("origBPM").toInt(), 100);
	}

	//! The beats that come with the program say their tempo in their names: SlicerT finds it (or its double or half:
	//! the same beat)
	void slicerTFindsTheFactoryBeatsTempo()
	{
		auto it = QDirIterator{QT_TESTCASE_SOURCEDIR "/../data/samples/beats", {"*BPM.ogg"}, QDir::Files};
		auto count = 0;
		while (it.hasNext())
		{
			const auto path = it.next();
			const auto name = QFileInfo{path}.completeBaseName();
			const auto bpm = name.section(" - ", 1).section(' ', 0, 0).toDouble();
			QVERIFY2(bpm > 0, qPrintable(name));
			const auto found = slicerWith(path).attribute("origBPM").toDouble();
			qInfo("%s: %g", qPrintable(name), found);
			const auto right = std::abs(found - bpm) <= 1 || std::abs(found - bpm * 2) <= 2
				|| std::abs(found - bpm / 2) <= 1;
			QVERIFY2(right, qPrintable(QString{"%1: %2"}.arg(name).arg(found)));
			++count;
		}
		QVERIFY(count >= 10);
	}

	//! #7364: SlicerT's slices can loop (round or back and forth) while their note is held; played once, by default
	void slicerTLoopsSlices()
	{
		const auto dir = QTemporaryDir{};
		QVERIFY(dir.isValid());
		const auto path = dir.filePath("tone.wav");
		auto tone = std::vector<std::array<float, 2>>(22050);
		for (auto i = std::size_t{0}; i < tone.size(); ++i)
		{
			const auto value = static_cast<float>(0.5 * std::sin(2 * M_PI * 441 * i / 44100.0));
			tone[i] = {value, value};
		}
		writeWav(path, tone, 44100);

		for (const auto loopMode : {0, 1, 2})
		{
			const auto played = play("slicert", [&](QDomElement& slicer) {
				slicer.setAttribute("src", path);
				slicer.setAttribute("totalSlices", 2);
				slicer.setAttribute("slice_0", 0);
				slicer.setAttribute("slice_1", 1);
				slicer.setAttribute("loopMode", loopMode);
			}, 400);
			QVERIFY(played.finite);
			// A second into the note (two bars long, from the start), well after the sample's half second
			const auto noteStart = std::size_t{0};
			auto late = 0.f;
			for (auto i = 2 * (noteStart + 44100); i < std::min(played.samples.size(), 2 * (noteStart + 66150)); ++i)
			{
				late = std::max(late, std::abs(played.samples[i]));
			}
			auto early = 0.f;
			for (auto i = 2 * (noteStart + 4410); i < std::min(played.samples.size(), 2 * (noteStart + 17640)); ++i)
			{
				early = std::max(early, std::abs(played.samples[i]));
			}
			const auto name = QString{"loop mode %1: %2 early, %3 late"}.arg(loopMode).arg(early).arg(late);
			QVERIFY2(early > 0.3f, qPrintable(name));
			QVERIFY2(loopMode == 0 ? late == 0.f : late > 0.3f, qPrintable(name));
		}
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
