/*
 * MonoModeTest.cpp
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
#include <vector>

#include <QtTest>

#include "AudioDummy.h"
#include "AudioEngine.h"
#include "ConfigManager.h"
#include "DataFile.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "MidiEvent.h"
#include "Note.h"
#include "NotePlayHandle.h"
#include "Song.h"
#include "Track.h"

using namespace lmms;

//! Upstream's #6516: one note at a time on any instrument (MonoMode)
class MonoModeTest : public QObject
{
	Q_OBJECT

	static constexpr int Beat = 48; //!< ticks
	InstrumentTrack* m_track = nullptr;

	//! A new project with a track of a sine (the TripleOscillator's first oscillator alone), mono as @p mono says
	//! (-1: off; 0: retrigger; 1: legato) with @p glide milliseconds
	void makeTrack(int mono, float glide = 0)
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		m_track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QVERIFY(m_track && m_track->loadInstrument("tripleoscillator"));
		auto state = DataFile{DataFile::Type::InstrumentTrackSettings};
		m_track->saveState(state, state.content());
		auto settings = state.content().firstChildElement();
		auto oscillators = settings.elementsByTagName("instrument").item(0).firstChildElement();
		oscillators.setAttribute("wavetype0", 0);
		oscillators.setAttribute("vol0", 100);
		oscillators.setAttribute("vol1", 0);
		oscillators.setAttribute("vol2", 0);
		auto monoElement = settings.elementsByTagName("monomode").item(0).toElement();
		QVERIFY(!monoElement.isNull());
		monoElement.setAttribute("enabled", mono >= 0 ? 1 : 0);
		monoElement.setAttribute("legato", mono == 1 ? 1 : 0);
		monoElement.setAttribute("glide", glide);
		m_track->restoreState(settings);
	}

	//! A clip with notes (key, start and length in ticks)
	void addNotes(std::initializer_list<std::array<int, 3>> notes)
	{
		auto* clip = dynamic_cast<MidiClip*>(m_track->createClip(TimePos{0}));
		for (const auto& [key, start, length] : notes) { clip->addNote(Note{TimePos{length}, TimePos{start}, key}); }
	}

	//! The track's notes sounding (not let go), without the chord or arpeggio notes
	auto voices() const -> std::vector<const NotePlayHandle*>
	{
		auto found = std::vector<const NotePlayHandle*>{};
		for (auto* handle : Engine::audioEngine()->playHandles())
		{
			if (handle->type() != PlayHandle::Type::NotePlayHandle) { continue; }
			const auto* note = static_cast<const NotePlayHandle*>(handle);
			if (note->instrumentTrack() == m_track && !note->hasParent() && !note->isReleased()) { found.push_back(note); }
		}
		return found;
	}

	struct Moment
	{
		std::vector<const NotePlayHandle*> voices;
		double frequency = 0; //!< of the period's samples, by their upward zero crossings
		std::vector<float> samples; //!< the four periods' left channel
	};

	//! The pitch of @p samples by their autocorrelation (the best lag between 50 Hz and 2 kHz, interpolated)
	static auto pitch(const std::vector<float>& samples) -> double
	{
		const auto rate = static_cast<double>(Engine::audioEngine()->outputSampleRate());
		const auto shortest = static_cast<std::size_t>(rate / 2000);
		const auto longest = std::min(static_cast<std::size_t>(rate / 50), samples.size() / 2);
		auto correlation = std::vector<double>(longest + 2, 0.0);
		for (auto lag = shortest; lag <= longest + 1; ++lag)
		{
			for (std::size_t i = 0; i + lag < samples.size(); ++i) { correlation[lag] += samples[i] * samples[i + lag]; }
			correlation[lag] /= static_cast<double>(samples.size() - lag);
		}
		// (the first peak near the best: not one of its multiples)
		auto best = shortest;
		for (auto lag = shortest; lag <= longest; ++lag) { if (correlation[lag] > correlation[best]) { best = lag; } }
		for (auto lag = shortest + 1; lag < best; ++lag)
		{
			if (correlation[lag] > 0.9 * correlation[best] && correlation[lag] >= correlation[lag - 1]
				&& correlation[lag] >= correlation[lag + 1]) { best = lag; break; }
		}
		const auto a = correlation[best - 1];
		const auto b = correlation[best];
		const auto c = correlation[best + 1];
		const auto shift = (a - c) / (2 * (a - 2 * b + c));
		return rate / (static_cast<double>(best) + (std::isfinite(shift) ? shift : 0.0));
	}

	//! Renders the song until tick @p tick, then one more period: what sounds there
	auto renderUntil(int tick) -> Moment
	{
		auto samples = std::vector<float>{};
		const auto frames = static_cast<f_cnt_t>(tick * Engine::framesPerTick());
		while (m_rendered < frames)
		{
			const auto period = Engine::audioEngine()->renderNextPeriod();
			m_rendered += static_cast<f_cnt_t>(period.size());
		}
		auto moment = Moment{voices()};
		// The frequency over the next four periods (crossings counted exactly, between samples)
		auto previous = 0.f;
		auto first = -1.0;
		auto last = -1.0;
		auto crossings = 0;
		auto index = 0;
		for (int period = 0; period < 4; ++period)
		{
			for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
			{
				const auto sample = frame.left();
				moment.samples.push_back(sample);
				if (index > 0 && previous < 0 && sample >= 0)
				{
					const auto at = index - 1 + previous / (previous - sample);
					if (first < 0) { first = at; } else { ++crossings; }
					last = at;
				}
				previous = sample;
				++index;
			}
			m_rendered += static_cast<f_cnt_t>(Engine::audioEngine()->framesPerPeriod());
		}
		moment.frequency = crossings > 0 ? crossings / (last - first) * Engine::audioEngine()->outputSampleRate() : 0;
		return moment;
	}

	void play()
	{
		m_rendered = 0;
		Engine::getSong()->setPlayPos(0, Song::PlayMode::Song);
		Engine::getSong()->playSong();
	}

	static auto near(double frequency, int key) -> bool
	{
		const auto expected = 440.0 * std::exp2((key - DefaultKey) / 12.0);
		return std::abs(frequency - expected) < expected * 0.01;
	}

	f_cnt_t m_rendered = 0;

private slots:
	void initTestCase()
	{
		ConfigManager::inst()->setValue("audioengine", "audiodev", AudioDummy::name());
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase() { Engine::destroy(); }

	void cleanup()
	{
		Engine::getSong()->stop();
		Engine::getSong()->createNewProject();
	}

	//! Polyphonic as before when off: two notes that overlap are two voices
	void offPlaysEveryNote()
	{
		makeTrack(-1);
		addNotes({{DefaultKey, 0, 2 * Beat}, {DefaultKey + 3, Beat, 2 * Beat}});
		play();
		QCOMPARE(renderUntil(Beat + Beat / 2).voices.size(), std::size_t{2});
	}

	//! Legato: a note that overlaps the one playing takes its voice without starting again (the same note), and when
	//! it ends with the first still held, the voice goes back to it
	void legatoSlidesAndReturns()
	{
		makeTrack(1);
		addNotes({{DefaultKey, 0, 4 * Beat}, {DefaultKey + 3, Beat, Beat}});
		play();
		const auto first = renderUntil(Beat / 2);
		QCOMPARE(first.voices.size(), std::size_t{1});
		QVERIFY2(near(first.frequency, DefaultKey), qPrintable(QString::number(first.frequency)));

		const auto second = renderUntil(Beat + Beat / 2);
		QCOMPARE(second.voices.size(), std::size_t{1});
		QCOMPARE(second.voices[0], first.voices[0]);
		QVERIFY2(near(second.frequency, DefaultKey + 3), qPrintable(QString::number(second.frequency)));

		const auto back = renderUntil(3 * Beat);
		QCOMPARE(back.voices.size(), std::size_t{1});
		QCOMPARE(back.voices[0], first.voices[0]);
		QVERIFY2(near(back.frequency, DefaultKey), qPrintable(QString::number(back.frequency)));

		QCOMPARE(renderUntil(5 * Beat).voices.size(), std::size_t{0});
	}

	//! A note that starts where the one playing ends (as notes are usually written) slides into it too
	void adjacentNotesAreLegato()
	{
		makeTrack(1);
		addNotes({{DefaultKey, 0, Beat}, {DefaultKey + 5, Beat, Beat}, {DefaultKey + 2, 2 * Beat, Beat}});
		play();
		const auto first = renderUntil(Beat / 2);
		for (const auto& [tick, key] : {std::pair{Beat + Beat / 2, DefaultKey + 5}, std::pair{2 * Beat + Beat / 2, DefaultKey + 2}})
		{
			const auto moment = renderUntil(tick);
			QCOMPARE(moment.voices.size(), std::size_t{1});
			QCOMPARE(moment.voices[0], first.voices[0]);
			QVERIFY2(near(moment.frequency, key), qPrintable(QString::number(moment.frequency)));
		}
		QCOMPARE(renderUntil(4 * Beat).voices.size(), std::size_t{0});
	}

	//! Retrigger: the new note starts a voice of its own and the one playing is let go; still one at a time
	void retriggerStartsAgain()
	{
		makeTrack(0);
		addNotes({{DefaultKey, 0, 2 * Beat}, {DefaultKey + 3, Beat, 2 * Beat}});
		play();
		const auto first = renderUntil(Beat / 2);
		const auto second = renderUntil(Beat + Beat / 2);
		QCOMPARE(second.voices.size(), std::size_t{1});
		QVERIFY(second.voices[0] != first.voices[0]);
		QVERIFY2(near(second.frequency, DefaultKey + 3), qPrintable(QString::number(second.frequency)));
	}

	//! A glide slides the pitch from the note playing to the next over its time
	void glideSlidesThePitch()
	{
		makeTrack(1, 200);
		addNotes({{DefaultKey, 0, 2 * Beat}, {DefaultKey + 12, Beat, 2 * Beat}});
		play();
		renderUntil(Beat / 2);
		// (a beat at 140 BPM is 429 ms: 50 ms into the glide, a quarter of the octave up; then there)
		const auto gliding = renderUntil(Beat + Beat / 9);
		const auto low = 440.0 * std::exp2(0.1);
		const auto high = 440.0 * std::exp2(0.5);
		QVERIFY2(gliding.frequency > low && gliding.frequency < high, qPrintable(QString::number(gliding.frequency)));
		const auto arrived = renderUntil(2 * Beat);
		QVERIFY2(near(arrived.frequency, DefaultKey + 12), qPrintable(QString::number(arrived.frequency)));
	}

	//! A sampler that sets its voices' pitch itself (the Sf2Player) follows the legato and the glide too
	void soundfontFollows()
	{
		makeTrack(1, 0);
		QVERIFY(m_track->loadInstrument("sf2player"));
		addNotes({{DefaultKey - 12, 0, 4 * Beat}, {DefaultKey - 5, Beat, Beat}});
		play();
		const auto first = renderUntil(Beat / 2);
		QCOMPARE(first.voices.size(), std::size_t{1});
		QVERIFY2(near(pitch(first.samples), DefaultKey - 12), qPrintable(QString::number(pitch(first.samples))));
		const auto second = renderUntil(Beat + Beat / 2);
		QCOMPARE(second.voices.size(), std::size_t{1});
		QVERIFY2(near(pitch(second.samples), DefaultKey - 5), qPrintable(QString::number(pitch(second.samples))));
		const auto back = renderUntil(2 * Beat + Beat / 2);
		QVERIFY2(near(pitch(back.samples), DefaultKey - 12), qPrintable(QString::number(pitch(back.samples))));
	}

	//! Keys from a device or the piano: the last one held plays; let go, the voice goes back to one still held
	void keysHeld()
	{
		makeTrack(1);
		Engine::getSong()->stop();
		m_rendered = 0;
		// (a note enters the engine with the next period)
		m_track->processInEvent(MidiEvent{MidiNoteOn, 0, DefaultKey, 100});
		Engine::audioEngine()->renderNextPeriod();
		const auto first = renderUntil(0);
		QCOMPARE(first.voices.size(), std::size_t{1});
		QVERIFY2(near(first.frequency, DefaultKey), qPrintable(QString::number(first.frequency)));

		m_track->processInEvent(MidiEvent{MidiNoteOn, 0, DefaultKey + 7, 100});
		Engine::audioEngine()->renderNextPeriod();
		const auto second = renderUntil(0);
		QCOMPARE(second.voices.size(), std::size_t{1});
		QCOMPARE(second.voices[0], first.voices[0]);
		QVERIFY2(near(second.frequency, DefaultKey + 7), qPrintable(QString::number(second.frequency)));

		m_track->processInEvent(MidiEvent{MidiNoteOff, 0, DefaultKey + 7, 0});
		Engine::audioEngine()->renderNextPeriod();
		const auto back = renderUntil(0);
		QCOMPARE(back.voices.size(), std::size_t{1});
		QVERIFY2(near(back.frequency, DefaultKey), qPrintable(QString::number(back.frequency)));

		m_track->processInEvent(MidiEvent{MidiNoteOff, 0, DefaultKey, 0});
		Engine::audioEngine()->renderNextPeriod();
		QCOMPARE(renderUntil(0).voices.size(), std::size_t{0});
	}

	//! Saved with the track; settings from before it (no element) leave it off
	void savedAndOffInOlderSettings()
	{
		makeTrack(1, 150);
		auto state = DataFile{DataFile::Type::InstrumentTrackSettings};
		m_track->saveState(state, state.content());
		auto settings = state.content().firstChildElement();
		const auto mono = settings.elementsByTagName("monomode").item(0).toElement();
		QCOMPARE(mono.attribute("enabled").toInt(), 1);
		QCOMPARE(mono.attribute("glide").toFloat(), 150.f);

		auto older = settings.cloneNode(true).toElement();
		auto monoInOlder = older.elementsByTagName("monomode").item(0);
		monoInOlder.parentNode().removeChild(monoInOlder);
		QVERIFY(older.elementsByTagName("monomode").isEmpty());
		m_track->restoreState(older);
		QVERIFY(!m_track->monoMode().enabled());
	}
};

QTEST_GUILESS_MAIN(MonoModeTest)
#include "MonoModeTest.moc"
