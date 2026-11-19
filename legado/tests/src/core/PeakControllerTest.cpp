/*
 * PeakControllerTest.cpp
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
#include <atomic>
#include <chrono>
#include <thread>
#include <vector>

#include <QPointer>
#include <QTemporaryDir>
#include <QtTest>

#include "AudioEngine.h"
#include "ControllerConnection.h"
#include "DataFile.h"
#include "EffectChain.h"
#include "Engine.h"
#include "Mixer.h"
#include "PeakController.h"
#include "PluginInstance.h"
#include "Song.h"

using namespace lmms;
using namespace std::chrono_literals;

/**
 * The Peak Controller effect owns its controller (phase 4 of docs/plano-refatoracao.md): whichever goes first, the
 * other goes with it once, and the audio thread, which reads the effect through the controller, never sees it gone.
 */
class PeakControllerTest : public QObject
{
	Q_OBJECT

	static auto chain() -> EffectChain& { return Engine::mixer()->mixerChannel(1)->m_fxChain; }

	static auto inSong(const Controller* controller) -> bool
	{
		const auto& controllers = Engine::getSong()->controllers();
		return std::find(controllers.begin(), controllers.end(), controller) != controllers.end();
	}

	//! A Peak Controller effect on mixer channel 1, and its controller
	static auto addPeakEffect() -> std::pair<Effect*, QPointer<Controller>>
	{
		chain().appendEffect(std::make_unique<PluginInstance>(Plugin::Type::Effect, "peakcontrollereffect", &chain(),
			nullptr));
		auto* effect = chain().effects().back()->effect();
		auto* controller = static_cast<Controller*>(nullptr);
		for (auto* c : Engine::getSong()->controllers())
		{
			if (c->type() == Controller::ControllerType::Peak) { controller = c; }
		}
		return {effect, QPointer<Controller>{controller}};
	}

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
					std::this_thread::sleep_for(1ms);
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

private slots:
	void initTestCase()
	{
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase() { Engine::destroy(); }

	void init()
	{
		Engine::getSong()->createNewProject();
		Engine::mixer()->createChannel();
	}

	//! The effect is deleted: its controller leaves the song and is deleted once
	void theControllerGoesWithItsEffect()
	{
		auto [effect, controller] = addPeakEffect();
		QVERIFY(controller);
		QVERIFY(inSong(controller));
		chain().removeEffect(effect);
		QVERIFY(!controller);
	}

	//! The controller is taken out of the song (the controller rack): its effect leaves the chain and is deleted once
	void theEffectGoesWithItsController()
	{
		auto [effect, controller] = addPeakEffect();
		QVERIFY(controller);
		const auto effects = chain().effects().size();
		Engine::getSong()->removeController(controller);
		QVERIFY(!controller);
		QCOMPARE(chain().effects().size(), effects - 1);
	}

	//! A project that lists the effect but not its controller (as a preview of it): the effect still owns the
	//! controller, and deletes it when it goes (it used to delete itself, from the effect's destroyed() signal)
	void aControllerTheSongDoesNotListGoesWithItsEffect()
	{
		addPeakEffect();
		const auto dir = QTemporaryDir{};
		const auto path = dir.filePath("peak.mmp");
		auto project = Engine::getSong()->serializeProject();
		auto controllers = project.content().firstChildElement("controllers");
		QVERIFY(!controllers.isNull());
		project.content().removeChild(controllers);
		QVERIFY(project.writeFile(path));

		Engine::getSong()->loadProject(path);
		auto* effect = chain().effects().empty() ? nullptr : chain().effects().back()->effect();
		QVERIFY(effect);
		const auto peaks = std::count_if(Engine::getSong()->controllers().begin(), Engine::getSong()->controllers().end(),
			[](const Controller* c) { return c->type() == Controller::ControllerType::Peak; });
		QCOMPARE(peaks, 0);
		chain().removeEffect(effect); // (ASan: nothing left behind, nothing deleted twice)
	}

	//! The effect is deleted while the song plays and a knob follows its controller: the audio thread reads the
	//! effect through the controller, and is told before the effect goes
	void deletedWhilePlaying()
	{
		for (int round = 0; round < 20; ++round)
		{
			auto [effect, controller] = addPeakEffect();
			auto& volume = Engine::mixer()->mixerChannel(1)->m_volumeModel;
			volume.setControllerConnection(std::make_unique<ControllerConnection>(controller.data()));
			Engine::getSong()->playSong();
			{
				const auto device = Device{};
				std::this_thread::sleep_for(10ms);
				chain().removeEffect(effect);
				std::this_thread::sleep_for(10ms);
			}
			Engine::getSong()->stop();
			QVERIFY(!controller);
		}
	}
};

QTEST_GUILESS_MAIN(PeakControllerTest)
#include "PeakControllerTest.moc"
