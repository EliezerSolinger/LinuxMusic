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
#include <QProcess>
#include <QStandardPaths>
#include <QTemporaryDir>
#include <QtTest>

#include "AudioDummy.h"
#include "AutomationClip.h"
#include "AudioEngine.h"
#include "ComboBoxModel.h"
#include "ConfigManager.h"
#include "ControllerConnection.h"
#include "DataFile.h"
#include "Engine.h"
#include "EffectChain.h"
#include "PluginInstance.h"
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "LfoController.h"
#include "MidiClient.h"
#include "MidiClip.h"
#include "Mixer.h"
#include "Note.h"
#include "PathUtil.h"
#include "PresetPreviewPlayHandle.h"
#include "SampleFrame.h"
#include "SampleBuffer.h"
#include "Sample.h"
#include "Song.h"
#include "Track.h"
#include "base64.h"
#include "../helpers/Spectrum.h"

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
		const auto changeInstrument = [&change](QDomElement& track) {
			auto element = track.elementsByTagName("instrument").item(0).firstChildElement();
			change(element);
		};
		return playTrack(instrument, change ? changeInstrument : std::function<void(QDomElement&)>{}, periods, key);
	}

	//! As play(), @p change given the whole track's settings
	static auto playTrack(const QString& instrument, const std::function<void(QDomElement&)>& change,
		int periods = 150, int key = DefaultKey) -> Played
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		if (!track || !track->loadInstrument(instrument)) { return {}; }
		if (change)
		{
			auto state = DataFile{DataFile::Type::InstrumentTrackSettings};
			track->saveState(state, state.content());
			auto element = state.content().firstChildElement();
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

	//! A GIG file in @p dir of a 440 Hz tone on A4 (pitch-tracked over the keyboard; its own envelope: none), made
	//! with libgig's wav2gig; empty without it
	static auto makeGig(const QTemporaryDir& dir) -> QString
	{
		const auto wav2gig = QStandardPaths::findExecutable("wav2gig");
		if (wav2gig.isEmpty()) { return {}; }
		auto tone = std::vector<std::array<float, 2>>(44100 * 3);
		for (std::size_t i = 0; i < tone.size(); ++i)
		{
			const auto value = static_cast<float>(0.5 * std::sin(2 * M_PI * 440 * i / 44100.0));
			tone[i] = {value, value};
		}
		QDir{dir.path()}.mkpath("wav");
		writeWav(dir.filePath("wav/Tone - Sine - 127 - 69 - A4.wav"), tone, 44100);
		const auto gig = dir.filePath("tone.gig");
		QProcess::execute(wav2gig, {"-f", gig, dir.filePath("wav")});
		return QFileInfo::exists(gig) ? gig : QString{};
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

	//! An LFO controller of the song, around @p base by @p amount, @p speed fast, starting at @p phase degrees
	static auto lfo(float base, float speed, float amount, float phase = 0.f) -> LfoController*
	{
		auto* song = Engine::getSong();
		auto* controller = new LfoController(song);
		song->addController(controller);
		for (auto* model : controller->findChildren<FloatModel*>())
		{
			if (model->displayName() == "Base value") { model->setValue(base); }
			if (model->displayName() == "Oscillator speed") { model->setValue(speed); }
			if (model->displayName() == "Oscillator amount") { model->setValue(amount); }
			if (model->displayName() == "Oscillator phase") { model->setValue(phase); }
		}
		return controller;
	}

	//! A track with @p instrument and a bar of short notes, then a long one
	static auto trackPlaying(const QString& instrument) -> InstrumentTrack*
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		if (!track || !track->loadInstrument(instrument)) { return nullptr; }
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		for (int i = 0; i < 16; ++i) { clip->addNote(Note{TimePos{24}, TimePos{i * 24}, 57 + i % 5}); }
		clip->addNote(Note{TimePos{4 * 192}, TimePos{0}, 45});
		return track;
	}

	//! @p periods rendered: false if a sample is not a number
	static auto renderFinite(int periods) -> bool
	{
		auto finite = true;
		for (int i = 0; i < periods; ++i)
		{
			for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
			{
				finite = finite && std::isfinite(frame.left()) && std::isfinite(frame.right());
			}
		}
		return finite;
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
		// The Mallets' presets are struck: set by their peak (about 6 dB under full scale at the base velocity, so that
		// a hard hit does not go past it, see velocityAsTheOtherInstruments()), their loudness lower than the others'
		const auto percussive = [](const QString& name) { return name == "malletsstk"; };
		auto lowest = 0.0, highest = -200.0;
		for (const auto& [name, db] : levels)
		{
			if (qEnvironmentVariableIsSet("LINUXMUSIC_PRINT_LEVELS")) { qInfo("%-18s %6.1f dB", qPrintable(name), db); }
			if (percussive(name)) { continue; }
			lowest = std::min(lowest, db);
			highest = std::max(highest, db);
		}
		QVERIFY(!levels.empty());
		if (qEnvironmentVariableIsSet("LINUXMUSIC_PRINT_LEVELS")) { qInfo("spread %.1f dB", highest - lowest); }
		// About -12 dB each at its loudest, within a few dB (it was 30 dB from the quietest to the loudest)
		for (const auto& [name, db] : levels)
		{
			const auto low = percussive(name) ? -24 : -18;
			QVERIFY2(db > low && db < -8, qPrintable(QString{"%1 at %2 dB"}.arg(name).arg(db, 0, 'f', 1)));
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

	//! #6098: Organic's saws and squares folded their high harmonics back down (it did not use the Oscillator's band
	//! limited tables): a high saw has (almost) nothing between its harmonics
	void organicIsBandLimited()
	{
		for (const auto wave : {1, 2})
		{
			const auto played = play("organic", [wave](QDomElement& organic) {
				for (int i = 0; i < 8; ++i)
				{
					organic.setAttribute(QString{"wavetype%1"}.arg(i), wave);
					organic.setAttribute(QString{"vol%1"}.arg(i), i == 0 ? 100 : 0);
					organic.setAttribute(QString{"newharmonic%1"}.arg(i), 2);
					organic.setAttribute(QString{"newdetune%1"}.arg(i), 0);
				}
			}, 200, DefaultKey + 39);
			const auto frequency = 440.0 * std::exp2(39 / 12.0);
			const auto folded = tests::aliasing(played.samples, 4096, frequency, Engine::audioEngine()->outputSampleRate(), 2);
			QVERIFY2(folded < -60, qPrintable(QString{"wave %1: %2 dB folded"}.arg(wave).arg(folded)));
		}
	}

	//! #6098: BitInvader played its points as they were, folding high notes' harmonics back down. Band limited in a
	//! new instrument (held or joined points alike); a project from before it plays as it did
	void bitInvaderIsBandLimited()
	{
		auto saw = std::array<float, 200>{};
		for (std::size_t i = 0; i < saw.size(); ++i) { saw[i] = -0.5f + static_cast<float>(i) / saw.size(); }
		auto shape = QString{};
		base64::encode(reinterpret_cast<const char*>(saw.data()), sizeof(saw), shape);
		const auto frequency = 440.0 * std::exp2(39 / 12.0);
		for (const auto interpolation : {0, 1})
		{
			for (const auto antiAliasing : {-1, 0, 1}) // -1: from before it
			{
				const auto played = play("bitinvader", [&](QDomElement& bitInvader) {
					bitInvader.setAttribute("sampleShape", shape);
					bitInvader.setAttribute("sampleLength", 200);
					bitInvader.setAttribute("interpolation", interpolation);
					if (antiAliasing >= 0) { bitInvader.setAttribute("antialias", antiAliasing); }
					else { bitInvader.removeAttribute("antialias"); }
				}, 200, DefaultKey + 39);
				QVERIFY(played.finite && played.loudest > 0.1f);
				const auto folded = tests::aliasing(played.samples, 4096, frequency, Engine::audioEngine()->outputSampleRate(), 2);
				const auto name = QString{"interpolation %1, anti-aliasing %2: %3 dB folded"}.arg(interpolation).arg(antiAliasing).arg(folded);
				QVERIFY2(antiAliasing == 1 ? folded < -80 : folded > -40, qPrintable(name));
			}
		}
	}

	//! #6098: Watsyn's oversampled waves folded high notes' harmonics back down. Band limited in a new instrument;
	//! a project from before it plays as it did
	void watsynIsBandLimited()
	{
		auto saw = std::array<float, 220>{};
		for (std::size_t i = 0; i < saw.size(); ++i) { saw[i] = -0.5f + static_cast<float>(i) / saw.size(); }
		auto shape = QString{};
		base64::encode(reinterpret_cast<const char*>(saw.data()), sizeof(saw), shape);
		const auto frequency = 440.0 * std::exp2(27 / 12.0);
		for (const auto antiAliasing : {-1, 0, 1}) // -1: from before it
		{
			const auto played = play("watsyn", [&](QDomElement& watsyn) {
				// A1 alone (the A side, all of it), at the note's pitch
				for (const auto* wave : {"a1_wave", "a2_wave", "b1_wave", "b2_wave"}) { watsyn.setAttribute(wave, shape); }
				watsyn.setAttribute("a1_vol", 100);
				for (const auto* volume : {"a2_vol", "b1_vol", "b2_vol"}) { watsyn.setAttribute(volume, 0); }
				watsyn.setAttribute("a1_mult", 8);
				watsyn.setAttribute("a1_ltune", 0);
				watsyn.setAttribute("a1_rtune", 0);
				watsyn.setAttribute("abmix", -100);
				watsyn.setAttribute("amod", 0);
				watsyn.setAttribute("envAmt", 0);
				if (antiAliasing >= 0) { watsyn.setAttribute("antialias", antiAliasing); }
				else { watsyn.removeAttribute("antialias"); }
			}, 200, DefaultKey + 27);
			QVERIFY(played.finite && played.loudest > 0.05f);
			const auto folded = tests::aliasing(played.samples, 4096, frequency, Engine::audioEngine()->outputSampleRate(), 2);
			const auto name = QString{"anti-aliasing %1: %2 dB folded"}.arg(antiAliasing).arg(folded);
			QVERIFY2(antiAliasing == 1 ? folded < -80 : folded > -40, qPrintable(name));
		}
	}

	//! #6098: with phase or frequency modulation, the TripleOscillator took its band limited table by the note's
	//! pitch, while the modulation pushes the wave's pitch past it: harmonics folded back down (-10 dB with this phase
	//! modulation). It takes the table of the highest pitch the wave reaches in the period, and the sidebands'
	void tripleOscillatorModulationIsBandLimited()
	{
		for (const auto algo : {0, 4}) // phase, frequency modulation
		{
			const auto played = play("tripleoscillator", [algo](QDomElement& triple) {
				triple.setAttribute("wavetype0", 2); // a saw
				triple.setAttribute("vol0", 100);
				triple.setAttribute("wavetype1", 0); // modulated by a sine of the same pitch
				// (frequency modulation by a sine 3% loud already pushes the pitch 1.3 kHz each way; much louder, the
				// sidebands of a sine itself reach past half the rate, which no table removes)
				triple.setAttribute("vol1", algo == 0 ? 40 : 3);
				triple.setAttribute("vol2", 0);
				triple.setAttribute("modalgo1", algo);
				triple.setAttribute("modalgo2", 2);
				for (int i = 0; i < 3; ++i)
				{
					triple.setAttribute(QString{"coarse%1"}.arg(i), 0);
					triple.setAttribute(QString{"finel%1"}.arg(i), 0);
					triple.setAttribute(QString{"finer%1"}.arg(i), 0);
					triple.setAttribute(QString{"useWaveTable%1"}.arg(i + 1), 1);
				}
			}, 200, DefaultKey + 24);
			const auto frequency = 440.0 * std::exp2(24 / 12.0);
			const auto folded = tests::aliasing(played.samples, 4096, frequency, Engine::audioEngine()->outputSampleRate(), 2);
			QVERIFY2(folded < -60, qPrintable(QString{"modulation %1: %2 dB folded"}.arg(algo).arg(folded)));
		}
	}

	//! #6098: Monstro's first oscillator, a pulse, stepped from 1 to -1 with only a sample of 0 between: its harmonics
	//! past half the rate folded back down (-22 dB on a 1 kHz note, -14 on a 7 kHz one). It is two band limited saws
	//! now, as clean as the other oscillators, and as loud as it was
	void monstroPulseIsBandLimited()
	{
		for (const auto key : {15, 27, 39})
		{
			const auto played = play("monstro", [](QDomElement& monstro) {
				for (int i = 1; i <= 3; ++i) { monstro.setAttribute(QString{"o%1vol"}.arg(i), i == 1 ? 100 : 0); }
			}, 200, DefaultKey + key);
			const auto frequency = 440.0 * std::exp2(key / 12.0);
			const auto folded = tests::aliasing(played.samples, 4096, frequency, Engine::audioEngine()->outputSampleRate(), 2);
			QVERIFY2(folded < -60, qPrintable(QString{"%1 Hz: %2 dB folded"}.arg(frequency).arg(folded)));
			QVERIFY2(played.loudest > 0.9f && played.loudest < 1.2f, qPrintable(QString::number(played.loudest)));
		}
	}

	/**
	 * #3233: the soundfont and GIG players play one stream, which the track's envelopes cannot shape: the ENV tab
	 * did nothing. They give their voices its volume envelope now, as their own: a slow attack starts quiet; with the
	 * envelope off (its amount at 0, as in a new track) they sound as the file says
	 */
	void samplersTakeTheVolumeEnvelope()
	{
		const auto dir = QTemporaryDir{};
		QVERIFY(dir.isValid());
		const auto gig = makeGig(dir);

		for (const auto* instrument : {"sf2player", "gigplayer"})
		{
			if (QString{instrument} == "gigplayer" && gig.isEmpty()) { qInfo("no wav2gig here: GIG not tried"); continue; }
			auto starts = std::array<float, 2>{};
			for (const auto amount : {0, 1})
			{
				const auto played = playTrack(instrument, [&](QDomElement& track) {
					if (QString{instrument} == "gigplayer")
					{
						auto plugin = track.elementsByTagName("instrument").item(0).firstChildElement();
						plugin.setAttribute("src", gig);
					}
					auto volume = track.elementsByTagName("elvol").item(0).toElement();
					QVERIFY(!volume.isNull());
					volume.setAttribute("amt", amount);
					volume.setAttribute("pdel", 0);
					volume.setAttribute("att", 0.4); // 0.8 s
					volume.setAttribute("hold", 0);
					volume.setAttribute("dec", 0);
					volume.setAttribute("sustain", 1);
				}, 400);
				QVERIFY(played.finite);
				const auto loudest = [&](double from, double to) {
					auto result = 0.f;
					for (auto i = static_cast<std::size_t>(from * 44100) * 2; i < std::min(played.samples.size(), static_cast<std::size_t>(to * 44100) * 2); ++i)
					{
						result = std::max(result, std::abs(played.samples[i]));
					}
					return result;
				};
				starts[amount] = loudest(0.0, 0.05);
				QVERIFY2(loudest(0.8, 1.2) > 0.005f, qPrintable(QString{"%1, amount %2: silent"}.arg(instrument).arg(amount)));
			}
			// (a 0.8 s attack: 50 ms in, about a sixteenth as loud as the note from the file)
			QVERIFY2(starts[1] < starts[0] * 0.2f, qPrintable(QString{"%1: %2 at the start with the envelope, %3 without"}
				.arg(instrument).arg(starts[1]).arg(starts[0])));
		}
	}

	//! #3825: the GIG player read its samples from the file on the audio thread; it plays them from memory now,
	//! each voice made in advance. Still at the right pitch (the file made here has one key: MonoModeTest follows a
	//! voice away from it)
	void gigPlaysItsSamples()
	{
		const auto dir = QTemporaryDir{};
		const auto gig = makeGig(dir);
		if (gig.isEmpty()) { QSKIP("no wav2gig here to make a GIG file"); }
		for (const auto key : {DefaultKey})
		{
			const auto played = play("gigplayer", [&gig](QDomElement& plugin) { plugin.setAttribute("src", gig); }, 150, key);
			QVERIFY(played.finite);
			QVERIFY2(played.loudest > 0.1f, qPrintable(QString{"key %1: %2"}.arg(key).arg(played.loudest)));
			// Upward zero crossings of the left channel over the second half
			auto crossings = 0;
			auto first = -1.0;
			auto last = -1.0;
			const auto half = played.samples.size() / 4 * 2;
			for (auto i = half + 2; i < played.samples.size(); i += 2)
			{
				const auto a = played.samples[i - 2];
				const auto b = played.samples[i];
				if (a < 0 && b >= 0)
				{
					const auto at = (i - 2) / 2 + a / (a - b);
					if (first < 0) { first = at; } else { ++crossings; }
					last = at;
				}
			}
			const auto frequency = crossings / (last - first) * 44100.0;
			const auto expected = 440.0 * std::exp2((key - DefaultKey) / 12.0);
			QVERIFY2(std::abs(frequency - expected) < expected * 0.01,
				qPrintable(QString{"key %1: %2 Hz, %3 expected"}.arg(key).arg(frequency).arg(expected)));
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

	/**
	 * #2149: LFOs moving the AudioFileProcessor's start, end and loop points across each other crashed it (a stack
	 * overflow: each point's knob moved the others to keep them in order, and with a controller the knob does not
	 * take the value set, so they moved each other without end). The points are put in order for the sample instead.
	 */
	void audioFileProcessorPointsCrossedByControllers()
	{
		auto* track = trackPlaying("audiofileprocessor");
		QVERIFY(track);
		auto* afp = track->instrument();
		QMetaObject::invokeMethod(afp, "setAudioFile", Q_ARG(QString, QString{"drums/bassdrum01.ogg"}),
			Q_ARG(bool, true));
		auto phase = 0.f;
		for (auto* model : afp->findChildren<FloatModel*>())
		{
			const auto name = model->displayName();
			if (name == "Start of sample" || name == "End of sample" || name == "Loopback point")
			{
				model->setControllerConnection(std::make_unique<ControllerConnection>(lfo(0.5f, 13.f, 1.f, phase)));
				phase += 120.f;
			}
		}
		auto* song = Engine::getSong();
		for (const auto mode : {0, 1, 2})
		{
			for (auto* model : afp->findChildren<IntModel*>())
			{
				if (model->displayName() == "Loop mode") { model->setValue(mode); }
			}
			song->setPlayPos(0, Song::PlayMode::Song);
			song->playSong();
			QVERIFY(renderFinite(400));
			song->stop();
		}
	}

	//! #8271: another instrument chosen while controllers move the track's pitch and the instrument's knobs
	void instrumentChangedWhileControlled_data()
	{
		QTest::addColumn<QString>("first");
		for (const auto* name : {"nes", "tripleoscillator", "sid", "freeboy", "lb302", "opulenz", "monstro"})
		{
			QTest::newRow(name) << QString{name};
		}
	}

	void instrumentChangedWhileControlled()
	{
		QFETCH(QString, first);
		auto* track = trackPlaying(first);
		QVERIFY(track);
		const auto controlAll = [track](float speed) {
			for (auto* model : track->instrument()->findChildren<FloatModel*>())
			{
				model->setControllerConnection(std::make_unique<ControllerConnection>(lfo(0.5f, speed, 1.f)));
			}
		};
		track->pitchModel()->setControllerConnection(std::make_unique<ControllerConnection>(lfo(0.5f, 5.f, 1.f)));
		controlAll(7.f);
		auto* song = Engine::getSong();
		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		QVERIFY(renderFinite(40));
		for (const auto* next : {"nes", "kicker", "tripleoscillator", "nes", "sid"})
		{
			QVERIFY(track->loadInstrument(next));
			controlAll(9.f);
			QVERIFY(renderFinite(30));
		}
		song->stop();
	}

	/**
	 * #6264: a pad hit short (a key hit and let go in the same period: input comes a period at a time) played
	 * nothing: the note was released before it sounded (no release: no frames; an instrument played by MIDI events
	 * did not even get its note on; the LB302, reading notes a period late, released it at once). It sounds for its
	 * first period now, then is released, again and again.
	 */
	void shortHitsSound_data()
	{
		QTest::addColumn<QString>("instrument");
		QTest::addColumn<bool>("mono");
		for (const auto* name : {"tripleoscillator", "kicker", "drummachines", "avldrums", "sf2player", "lb302",
			"opulenz", "zynaddsubfx", "monstro", "sid", "nes", "freeboy", "organic", "bitinvader", "watsyn", "sfxr",
			"vibedstrings", "malletsstk", "xpressive"})
		{
			QTest::newRow(name) << QString{name} << false;
		}
		// (a mono voice, let go before it sounded)
		QTest::newRow("tripleoscillator, mono") << QString{"tripleoscillator"} << true;
		QTest::newRow("sf2player, mono") << QString{"sf2player"} << true;
	}

	void shortHitsSound()
	{
		QFETCH(QString, instrument);
		QFETCH(bool, mono);
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QVERIFY(track && track->loadInstrument(instrument));
		for (auto* model : track->monoMode().findChildren<BoolModel*>())
		{
			if (model->displayName() == "Mono") { model->setValue(mono); }
		}
		QCOMPARE(track->monoMode().enabled(), mono);
		QVERIFY(renderFinite(3));
		auto* client = Engine::audioEngine()->midiClient();
		auto hardest = 0.f;
		for (int hit = 0; hit < 3; ++hit)
		{
			client->queueInEvent(track->midiPort(), MidiEvent{MidiNoteOn, 0, 40, 110});
			client->queueInEvent(track->midiPort(), MidiEvent{MidiNoteOff, 0, 40, 0});
			auto loudest = 0.f;
			for (int period = 0; period < 60; ++period)
			{
				for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
				{
					loudest = std::max(loudest, std::abs(frame.left()));
				}
			}
			QVERIFY2(loudest > 0.02f, qPrintable(QString{"hit %1: %2"}.arg(hit).arg(loudest)));
			hardest = std::max(hardest, loudest);
		}
		// And it ends: the note off reached the instrument after the note on (an instrument taking events with the
		// period got both together, the note off first, and its voice rang on)
		QVERIFY(renderFinite(1000));
		auto left = 0.f;
		for (int period = 0; period < 100; ++period)
		{
			for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
			{
				left = std::max(left, std::abs(frame.left()));
			}
		}
		QVERIFY2(left < hardest / 4, qPrintable(QString{"%1 six seconds later (%2 at the hits)"}.arg(left).arg(hardest)));
	}

	/**
	 * Velocity, as the other instruments: twice the velocity, 6 dB louder, and a hard hit about full scale. The
	 * OpulenZ went 24 dB from the base velocity to the top (the chip's levels in steps), past 0 dBFS with the volume
	 * the new instruments have; the Mallets' struck presets 12 dB for twice the velocity, and most peaked over full
	 * scale already at the base velocity.
	 */
	void velocityAsTheOtherInstruments_data()
	{
		QTest::addColumn<QString>("instrument");
		QTest::addColumn<int>("preset");
		QTest::newRow("tripleoscillator") << QString{"tripleoscillator"} << -1;
		QTest::newRow("opulenz") << QString{"opulenz"} << -1;
		for (const auto preset : {0, 2, 6, 7, 9})
		{
			QTest::newRow(qPrintable(QString{"malletsstk %1"}.arg(preset))) << QString{"malletsstk"} << preset;
		}
	}

	void velocityAsTheOtherInstruments()
	{
		QFETCH(QString, instrument);
		QFETCH(int, preset);
		auto* client = Engine::audioEngine()->midiClient();
		const auto peakAt = [&](int velocity) {
			auto* song = Engine::getSong();
			song->createNewProject();
			auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
			if (!track || !track->loadInstrument(instrument)) { return -1.f; }
			for (auto* model : track->instrument()->findChildren<ComboBoxModel*>())
			{
				if (preset >= 0 && model->displayName() == "Instrument") { model->setValue(preset); }
			}
			renderFinite(3);
			client->queueInEvent(track->midiPort(), MidiEvent{MidiNoteOn, 0, 57, static_cast<std::int16_t>(velocity)});
			auto loudest = 0.f;
			for (int period = 0; period < 120; ++period)
			{
				for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
				{
					loudest = std::max({loudest, std::abs(frame.left()), std::abs(frame.right())});
				}
			}
			client->queueInEvent(track->midiPort(), MidiEvent{MidiNoteOff, 0, 57, 0});
			renderFinite(300);
			return loudest;
		};
		const auto base = peakAt(63);
		const auto top = peakAt(127);
		QVERIFY(base > 0.f && top > 0.f);
		const auto rise = 20 * std::log10(top / base);
		const auto name = QString{"base %1, top %2 (%3 dB)"}.arg(base).arg(top).arg(rise);
		QVERIFY2(rise > 4.5f && rise < 7.5f, qPrintable(name));
		// (the levels are set on average over six octaves, some 6 dB under full scale at the base velocity: a key
		// goes a little over or under it)
		QVERIFY2(top < 1.2f, qPrintable(name));
	}

	//! The pitch of @p samples (interleaved stereo, the left channel) from @p from to @p to: from the time between
	//! its first and last rising zero crossings (each placed between its two samples), the crossings counted
	static auto pitchOf(const std::vector<float>& samples, std::size_t from, std::size_t to, double rate) -> double
	{
		auto first = -1.0, last = -1.0;
		auto crossings = 0;
		for (auto i = from + 1; i < std::min(to, samples.size() / 2); ++i)
		{
			const auto before = samples[2 * (i - 1)], now = samples[2 * i];
			if (before < 0 && now >= 0)
			{
				const auto at = static_cast<double>(i - 1) + before / (before - now);
				if (first < 0) { first = at; }
				last = at;
				++crossings;
			}
		}
		return crossings > 1 ? (crossings - 1) * rate / (last - first) : 0.0;
	}

	/**
	 * #389, #1010: the Sfxr, which runs at 44.1 kHz, took each period's frames at its pitch as whole frames, the
	 * fraction dropped: the notes went flat, the more the lower and the higher the rate (an A2 at 96 kHz by a quarter
	 * of a semitone). The same note at 44.1 and 96 kHz is the same pitch now.
	 */
	void sfxrInTuneAtAnyRate()
	{
		auto* engine = Engine::audioEngine();
		for (const auto key : {33, 45, 60})
		{
			auto pitches = std::vector<double>{};
			for (const auto rate : {sample_rate_t{44100}, sample_rate_t{96000}})
			{
				engine->reopenAudioDevice(rate);
				engine->stopProcessing();
				const auto played = play("sfxr", nullptr, 150, key);
				QVERIFY(played.finite);
				pitches.push_back(pitchOf(played.samples, rate / 50, rate / 4, rate));
			}
			QVERIFY(pitches[0] > 0 && pitches[1] > 0);
			const auto cents = 1200 * std::log2(pitches[1] / pitches[0]);
			QVERIFY2(std::abs(cents) < 3, qPrintable(QString{"key %1: %2 Hz at 44.1 kHz, %3 Hz at 96 kHz (%4 cents)"}
				.arg(key).arg(pitches[0]).arg(pitches[1]).arg(cents)));
		}
		engine->reopenAudioDevice(44100);
		engine->stopProcessing();
	}

	/**
	 * #1644, #6074: previewing presets, one kept what the one before had and it has not: the arpeggio (an older
	 * preset has none), an instrument's settings (it was kept between presets of the same instrument). Each preview
	 * starts from the track as made, with an instrument of its own.
	 */
	void presetPreviewsDoNotLeak()
	{
		// A preset with the arpeggio on and the first oscillator quiet, and an older one with neither saved
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QVERIFY(track && track->loadInstrument("tripleoscillator"));
		// (saved as the program saves a preset)
		auto state = DataFile{DataFile::Type::InstrumentTrackSettings};
		track->savePreset(state, state.content());
		auto settings = state.content();
		auto arpeggio = settings.elementsByTagName("arpeggiator").item(0).toElement();
		QVERIFY(!arpeggio.isNull());
		arpeggio.setAttribute("arp-enabled", 1);
		auto oscillators = settings.elementsByTagName("tripleoscillator").item(0).toElement();
		QVERIFY(!oscillators.isNull());
		oscillators.setAttribute("vol0", 3);
		const auto dir = QTemporaryDir{};
		const auto first = dir.filePath("first.xpf");
		QVERIFY(state.writeFile(first));
		arpeggio.parentNode().removeChild(arpeggio);
		oscillators.removeAttribute("vol0");
		const auto second = dir.filePath("second.xpf");
		QVERIFY(state.writeFile(second));

		const auto oscillatorVolume = [] {
			for (auto* model : PresetPreviewPlayHandle::previewTrack()->instrument()->findChildren<FloatModel*>())
			{
				if (model->displayName() == "Osc 1 volume") { return model->value(); }
			}
			return -1.f;
		};
		delete new PresetPreviewPlayHandle(first, false);
		QVERIFY(PresetPreviewPlayHandle::previewTrack()->isArpeggioEnabled());
		QCOMPARE(oscillatorVolume(), 3.f);
		delete new PresetPreviewPlayHandle(second, false);
		QVERIFY(!PresetPreviewPlayHandle::previewTrack()->isArpeggioEnabled());
		QVERIFY2(oscillatorVolume() != 3.f, qPrintable(QString::number(oscillatorVolume())));
	}

	//! The pitch wheel bends 2 semitones each way on a new track, General MIDI's (it was 1: "pitch bend does nothing");
	//! a project or preset saved without a range keeps the 1 it had
	void newTracksBendTwoSemitones()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QVERIFY(track && track->loadInstrument("tripleoscillator"));
		QCOMPARE(track->pitchRangeModel()->value(), 2);
		auto state = DataFile{DataFile::Type::InstrumentTrackSettings};
		track->savePreset(state, state.content());
		auto settings = state.content().firstChildElement("instrumenttrack");
		QCOMPARE(settings.attribute("pitchrange").toInt(), 2);
		settings.removeAttribute("pitchrange");
		track->loadTrackSpecificSettings(settings);
		QCOMPARE(track->pitchRangeModel()->value(), 1);
	}
};

QTEST_GUILESS_MAIN(InstrumentFixesTest)
#include "InstrumentFixesTest.moc"
