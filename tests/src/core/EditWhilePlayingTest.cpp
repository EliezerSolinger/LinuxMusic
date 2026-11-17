/*
 * EditWhilePlayingTest.cpp
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

#include <atomic>
#include <chrono>
#include <functional>
#include <thread>
#include <vector>

#include <QtTest>

#include "AudioEngine.h"
#include "AutomationClip.h"
#include "EffectChain.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Mixer.h"
#include "Note.h"
#include "PluginInstance.h"
#include "Song.h"
#include "Track.h"

using namespace lmms;
using namespace std::chrono_literals;

/**
 * Playing and editing at once (phase 7 of docs/plano-refatoracao.md): a thread plays as a sound card does, a period
 * every period's time, giving up on the change lock after half a period (a period of silence: periodsSkipped()),
 * while the main thread edits as the user does. No edit may make it skip a period.
 */
class EditWhilePlayingTest : public QObject
{
	Q_OBJECT

	//! Plays as a device does while it lives
	class Device
	{
	public:
		Device()
		{
			m_thread = std::thread{[this] {
				auto* engine = Engine::audioEngine();
				const auto period = std::chrono::duration<double>{
					static_cast<double>(engine->framesPerPeriod()) / engine->outputSampleRate()};
				auto buffer = std::vector<float>(2 * engine->framesPerPeriod());
				auto next = std::chrono::steady_clock::now();
				while (!m_stop.load())
				{
					engine->renderNextBuffer(InterleavedBufferView<float>{buffer.data(), 2, engine->framesPerPeriod()});
					next += std::chrono::duration_cast<std::chrono::steady_clock::duration>(period);
					std::this_thread::sleep_until(next);
				}
			}};
		}

		~Device()
		{
			m_stop = true;
			m_thread.join();
		}

	private:
		std::atomic<bool> m_stop = false;
		std::thread m_thread;
	};

	static auto newInstrumentTrack(const char* instrument) -> InstrumentTrack*
	{
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
		track->loadInstrument(instrument);
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		for (int note = 0; note < 16; ++note) { clip->addNote(Note{TimePos{24}, TimePos{note * 12}, DefaultKey + note % 7}); }
		return track;
	}

	struct Edit
	{
		const char* name;
		std::function<void(int)> run;
	};

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

	void noEditMakesThePlayingSkipAPeriod()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* playing = newInstrumentTrack("tripleoscillator");
		(void)playing;
		auto* mixer = Engine::mixer();
		const auto channelIndex = mixer->createChannel();
		auto& chain = mixer->mixerChannel(channelIndex)->m_fxChain;
		song->getTimeline(Song::PlayMode::Song).setLoopPoints(TimePos{0}, TimePos{4 * TimePos::ticksPerBar()});
		song->getTimeline(Song::PlayMode::Song).setLoopEnabled(true);
		song->playSong();

		auto made = std::vector<Track*>{};
		auto* automation = Track::create(Track::Type::Automation, song);
		const auto edits = std::vector<Edit>{
			{"a track with an instrument made", [&](int) { made.push_back(newInstrumentTrack("tripleoscillator")); }},
			{"a track with an instrument deleted", [&](int) {
				if (made.empty()) { return; }
				// As the track's view deletes it (TrackContainerView::deleteTrackView())
				mixer->releaseTrackChannel(made.back());
				Track::destroy(made.back());
				made.pop_back();
			}},
			{"the instrument of a track swapped", [&](int round) {
				playing->loadInstrument(round % 2 ? "bitinvader" : "tripleoscillator");
			}},
			{"an effect added and removed", [&](int) {
				chain.appendEffect(std::make_unique<PluginInstance>(Plugin::Type::Effect, "amplifier", &chain, nullptr));
				chain.removeEffect(chain.effects().back()->effect());
			}},
			{"a mixer channel with a send, made and deleted", [&](int) {
				const auto added = mixer->createChannel();
				mixer->createChannelSend(added, channelIndex, 0.5f);
				mixer->deleteChannel(added);
			}},
			{"notes added and a clip moved", [&](int round) {
				auto* clip = dynamic_cast<MidiClip*>(playing->getClip(0));
				clip->addNote(Note{TimePos{12}, TimePos{round % 64 * 3}, DefaultKey});
				clip->movePosition(TimePos{round % 2 * 48});
			}},
			{"the tempo changed", [&](int round) { song->tempoModel().setValue(100 + round % 60); }},
			{"a clip with notes made and deleted", [&](int round) {
				auto* clip = dynamic_cast<MidiClip*>(playing->createClip(TimePos{round % 4 * TimePos::ticksPerBar()}));
				for (int note = 0; note < 8; ++note) { clip->addNote(Note{TimePos{24}, TimePos{note * 24}, DefaultKey}); }
				playing->deleteClip(clip);
			}},
			{"a pattern made and deleted", [&](int) {
				Track::destroy(Track::create(Track::Type::Pattern, song));
			}},
			{"a mixer channel with effects and automation deleted", [&](int) {
				const auto added = mixer->createChannel();
				auto* channel = mixer->mixerChannel(added);
				for (const auto* effect : {"amplifier", "stereoenhancer", "bassbooster"})
				{
					channel->m_fxChain.appendEffect(
						std::make_unique<PluginInstance>(Plugin::Type::Effect, effect, &channel->m_fxChain, nullptr));
				}
				auto* clip = dynamic_cast<AutomationClip*>(automation->createClip(TimePos{0}));
				clip->addObject(&channel->m_volumeModel);
				clip->putValue(TimePos{0}, 0.5f, false);
				clip->changeLength(TimePos{4 * TimePos::ticksPerBar()});
				std::this_thread::sleep_for(10ms);
				mixer->deleteChannel(added);
				automation->deleteClip(clip);
			}},
			{"an automated track deleted", [&](int) {
				// Another track's clip sets its volume while it plays
				auto* track = newInstrumentTrack("tripleoscillator");
				auto* clip = dynamic_cast<AutomationClip*>(automation->createClip(TimePos{0}));
				clip->addObject(dynamic_cast<InstrumentTrack*>(track)->volumeModel());
				clip->putValue(TimePos{0}, 0.5f, false);
				clip->putValue(TimePos{2 * TimePos::ticksPerBar()}, 1.f, false);
				clip->changeLength(TimePos{4 * TimePos::ticksPerBar()});
				std::this_thread::sleep_for(10ms);
				mixer->releaseTrackChannel(track);
				Track::destroy(track);
				automation->deleteClip(clip);
			}},
		};

		auto skippedInAll = std::int64_t{0};
		{
			const auto device = Device{};
			std::this_thread::sleep_for(100ms);
			for (const auto& edit : edits)
			{
				const auto before = Engine::audioEngine()->periodsSkipped();
				Engine::audioEngine()->takeLongestChangeHold();
				for (int round = 0; round < 12; ++round)
				{
					edit.run(round);
					QCoreApplication::processEvents();
					std::this_thread::sleep_for(3ms);
				}
				std::this_thread::sleep_for(20ms);
				const auto skipped = Engine::audioEngine()->periodsSkipped() - before;
				const auto longest = Engine::audioEngine()->takeLongestChangeHold();
				qInfo("%-45s %3lld periods skipped, the change lock held %.2f ms at most", edit.name,
					static_cast<long long>(skipped), static_cast<double>(longest) / 1000.0);
				skippedInAll += skipped;
			}
		}
		song->stop();
		QCOMPARE(skippedInAll, std::int64_t{0});
	}

	//! The clip the piano roll plays alone (Song::playMidiClip()) is deleted while it plays: the playing stops, and the
	//! audio thread never sees the clip gone (Song::forgetClip())
	void theClipThePianoRollPlaysIsDeleted()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = newInstrumentTrack("tripleoscillator");
		auto* clip = dynamic_cast<MidiClip*>(track->getClip(0));
		song->playMidiClip(clip);
		QCOMPARE(song->playMode(), Song::PlayMode::MidiClip);
		{
			const auto device = Device{};
			std::this_thread::sleep_for(30ms);
			track->deleteClip(clip);
			std::this_thread::sleep_for(30ms);
			QCoreApplication::processEvents();
			std::this_thread::sleep_for(10ms);
		}
		QVERIFY(song->isStopped());
	}
};

QTEST_GUILESS_MAIN(EditWhilePlayingTest)
#include "EditWhilePlayingTest.moc"
