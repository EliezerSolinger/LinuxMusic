/*
 * ThreadStressTest.cpp
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
#include <QtTest>

#include <atomic>
#include <cmath>
#include <thread>

#include "AudioEngine.h"
#include "ControllerConnection.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "LfoController.h"
#include "MidiClip.h"
#include "Mixer.h"
#include "Piano.h"
#include "PluginInstance.h"
#include "Song.h"

using namespace lmms;

/**
 * The audio thread renders while another thread does what the GUI does: turns knobs, moves the loop and the
 * play head, reads the position, the controllers, the piano's keys and the notes counted. Nothing may crash or
 * break the output; with a ThreadSanitizer build (-DWANT_DEBUG_TSAN=ON), nothing may race.
 */
class ThreadStressTest : public QObject
{
	Q_OBJECT

private slots:
	void initTestCase()
	{
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	void theGuiAndTheAudioThreadAtOnce()
	{
		auto* song = Engine::getSong();
		song->createNewProject();

		// An instrument playing chords, its volume moved by an LFO, through a channel with an effect
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QVERIFY(track->loadInstrument("tripleoscillator"));
		auto document = QDomDocument{};
		auto saved = document.createElement("saved");
		track->saveState(document, saved);
		auto settings = saved.firstChildElement();
		settings.elementsByTagName("chordcreator").item(0).toElement().setAttribute("chord-enabled", "1");
		track->restoreState(settings);
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		for (int note = 0; note < 32; ++note)
		{
			clip->addNote(Note{TimePos{36}, TimePos{note * 24}, DefaultKey + note % 7});
		}
		auto* mixer = Engine::mixer();
		const auto channelIndex = mixer->createChannel();
		track->mixerChannelModel()->setValue(channelIndex);
		auto* channel = mixer->mixerChannel(channelIndex);
		channel->m_fxChain.appendEffect(
			std::make_unique<PluginInstance>(Plugin::Type::Effect, "amplifier", &channel->m_fxChain, nullptr));
		mixer->setUpTrackChannels();
		auto* lfo = new LfoController(song);
		song->addController(lfo);
		track->volumeModel()->setControllerConnection(new ControllerConnection(lfo));

		auto& timeline = song->getTimeline(Song::PlayMode::Song);
		timeline.setLoopPoints(TimePos{0}, TimePos{4 * TimePos::ticksPerBar()});
		timeline.setLoopEnabled(true);
		song->playSong();

		// The audio thread
		auto stop = std::atomic<bool>{false};
		auto periods = std::atomic<int>{0};
		auto finite = std::atomic<bool>{true};
		auto audio = std::thread{[&] {
			while (!stop.load())
			{
				for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
				{
					if (!std::isfinite(frame[0]) || !std::isfinite(frame[1])) { finite = false; }
				}
				++periods;
			}
		}};

		// The GUI's thread meanwhile
		auto sink = 0.0;
		for (int round = 0; periods.load() < 400; ++round)
		{
			track->pitchModel()->setValue(static_cast<float>(round % 200 - 100));
			channel->m_volumeModel.setValue(0.5f + 0.5f * static_cast<float>(round % 2));
			lfo->currentValue(0);
			sink += track->volumeModel()->value() + lfo->currentValue(0);
			sink += timeline.ticks() + timeline.getElapsedSeconds();
			sink += track->pianoModel()->isKeyPressed(DefaultKey + round % 7) ? 1 : 0;
			sink += track->notesStarted() - track->notesEnded();
			if (round % 50 == 0)
			{
				timeline.setLoopPoints(TimePos{(round / 50) % 2 * TimePos::ticksPerBar()},
					TimePos{4 * TimePos::ticksPerBar()});
			}
			if (round % 300 == 0) { song->setPlayPos(round % 3 * 48, Song::PlayMode::Song); }
			std::this_thread::yield();
		}
		stop = true;
		audio.join();
		song->stop();

		QVERIFY(finite.load());
		QVERIFY(std::isfinite(sink));
		QVERIFY(track->notesStarted() > 0);
	}
};

QTEST_GUILESS_MAIN(ThreadStressTest)
#include "ThreadStressTest.moc"
