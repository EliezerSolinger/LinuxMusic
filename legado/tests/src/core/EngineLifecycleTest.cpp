/*
 * EngineLifecycleTest.cpp
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
#include <thread>
#include <vector>

#include <QtTest>

#include "AudioEngine.h"
#include "AutomationClip.h"
#include "EffectChain.h"
#include "Engine.h"
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Mixer.h"
#include "Note.h"
#include "PatternStore.h"
#include "PluginInstance.h"
#include "Song.h"
#include "Track.h"

using namespace lmms;
using namespace std::chrono_literals;

/**
 * The engine's life (phase 8 of docs/plano-refatoracao.md): made, given a project that plays (instruments with their
 * files, effects, automation, patterns, a mixer channel), taken down while it plays, and made again, twice. Run under
 * ASan too: nothing taken down may be used afterwards, by any thread.
 */
class EngineLifecycleTest : public QObject
{
	Q_OBJECT

	//! Plays as a device does, while it lives
	class Device
	{
	public:
		Device()
		{
			m_thread = std::thread{[this] {
				auto* engine = Engine::audioEngine();
				auto buffer = std::vector<float>(2 * engine->framesPerPeriod());
				while (!m_stop.load())
				{
					engine->renderNextBuffer(InterleavedBufferView<float>{buffer.data(), 2, engine->framesPerPeriod()});
					for (const auto sample : buffer) { m_loudest = std::max(m_loudest.load(), std::abs(sample)); }
					std::this_thread::sleep_for(2ms);
				}
			}};
		}

		~Device() { stop(); }

		void stop()
		{
			m_stop = true;
			if (m_thread.joinable()) { m_thread.join(); }
		}

		auto loudest() const -> float { return m_loudest.load(); }

	private:
		std::atomic<bool> m_stop = false;
		std::atomic<float> m_loudest = 0.f;
		std::thread m_thread;
	};

	static auto data(const char* path) -> QString { return QString{QT_TESTCASE_SOURCEDIR "/../data/"} + path; }

	static auto newInstrumentTrack(const char* instrument, TrackContainer* container) -> InstrumentTrack*
	{
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, container));
		track->loadInstrument(instrument);
		auto* clip = dynamic_cast<MidiClip*>(track->numOfClips() > 0 ? track->getClip(0) : track->createClip(TimePos{0}));
		for (int note = 0; note < 8; ++note)
		{
			clip->addNote(Note{TimePos{24}, TimePos{note * 24}, DefaultKey + note % 5});
		}
		return track;
	}

	//! A project with a bit of everything that holds on to the engine, playing
	static void makeAProjectThatPlays()
	{
		auto* song = Engine::getSong();
		song->createNewProject();

		newInstrumentTrack("tripleoscillator", song);
		auto* sf2 = newInstrumentTrack("sf2player", song);
		sf2->instrument()->loadFile(data("avldrums/Black_Pearl_4_LV2.sf2"));
		auto* sampler = newInstrumentTrack("audiofileprocessor", song);
		sampler->instrument()->loadFile(data("samples/misc/hit01.ogg"));
		auto* slicer = newInstrumentTrack("slicert", song);
		slicer->instrument()->loadFile(data("samples/misc/snaph01.ogg"));

		// Effects on a mixer channel of its own, its volume automated
		auto* mixer = Engine::mixer();
		const auto index = mixer->createChannel();
		auto* channel = mixer->mixerChannel(index);
		for (const auto* effect : {"amplifier", "stereoenhancer", "bassbooster"})
		{
			channel->m_fxChain.appendEffect(
				std::make_unique<PluginInstance>(Plugin::Type::Effect, effect, &channel->m_fxChain, nullptr));
		}
		auto* automation = Track::create(Track::Type::Automation, song);
		auto* clip = dynamic_cast<AutomationClip*>(automation->createClip(TimePos{0}));
		clip->addObject(&channel->m_volumeModel);
		clip->putValue(TimePos{0}, 0.5f, false);
		clip->putValue(TimePos{TimePos::ticksPerBar()}, 1.f, false);

		// A pattern, with an instrument in the pattern editor
		Track::create(Track::Type::Pattern, song);
		newInstrumentTrack("kicker", Engine::patternStore());

		song->getTimeline(Song::PlayMode::Song).setLoopPoints(TimePos{0}, TimePos{2 * TimePos::ticksPerBar()});
		song->getTimeline(Song::PlayMode::Song).setLoopEnabled(true);
		song->playSong();
	}

private slots:
	//! Made, playing a project, taken down while it plays; twice: the second life is as the first
	void itLivesTwice()
	{
		for (int life = 0; life < 2; ++life)
		{
			Engine::init(true);
			Engine::audioEngine()->stopProcessing();
			makeAProjectThatPlays();
			auto device = Device{};
			std::this_thread::sleep_for(300ms);
			QCoreApplication::processEvents();
			QVERIFY2(device.loudest() > 1e-3f, qPrintable(QString{"life %1: the project did not sound"}.arg(life)));

			// Taken down while the song plays (the device stops first, as Engine::destroy() stops a real one)
			device.stop();
			QVERIFY(Engine::getSong()->isPlaying());
			Engine::destroy();

			QVERIFY(Engine::audioEngine() == nullptr);
			QVERIFY(Engine::getSong() == nullptr);
		}
	}
};

QTEST_GUILESS_MAIN(EngineLifecycleTest)
#include "EngineLifecycleTest.moc"
