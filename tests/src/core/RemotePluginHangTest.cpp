/*
 * RemotePluginHangTest.cpp
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

#include <chrono>
#include <future>

#include <QtTest>

#include "AudioEngine.h"
#include "AudioProcessor.h"
#include "Engine.h"
#include "PluginGuard.h"
#include "RemotePlugin.h"

using namespace lmms;
using namespace std::chrono_literals;

//! A plugin process (tests/src/helpers/FakeRemotePlugin.cpp, the same protocol as the real ones) that stops
//! answering while processing: the audio thread gives up on it in time, it goes silent, its process is ended, and the
//! plugin is not loaded again until the user allows it (phase 6 of docs/plano-refatoracao.md)
class RemotePluginHangTest : public QObject
{
	Q_OBJECT

	static constexpr auto Identity = "fakeremoteplugin-hang-test";
	static constexpr int IdFakeHangWhenProcessing = IdUserBase + 1;

	//! process(), as the audio thread calls it (from another thread, so that the main thread's events go on)
	static auto processOnAnotherThread(RemotePlugin& plugin) -> std::future<bool>
	{
		return std::async(std::launch::async, [&plugin] { return plugin.process(); });
	}

private slots:
	void initTestCase()
	{
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
		PluginGuard::clearSuspect(Identity);
	}

	void cleanupTestCase()
	{
		PluginGuard::clearSuspect(Identity);
		Engine::destroy();
	}

	void aHungPluginIsGivenUpEndedAndSuspected()
	{
		auto plugin = RemotePlugin{};
		plugin.setGuardIdentity(Identity);
		// (init() says whether it failed)
		QVERIFY(!plugin.init("FakeRemotePlugin", false));
		plugin.lock();
		plugin.waitForInitDone(false);
		plugin.unlock();
		QVERIFY(!plugin.failed());
		QVERIFY(plugin.isRunning());
		plugin.setSharedAudioBuffer(AudioProcessor::SharedBufferInfo{"fake", 2, 2, 256});

		// Answering, it processes
		auto answering = processOnAnotherThread(plugin);
		QTRY_VERIFY_WITH_TIMEOUT(answering.wait_for(0s) == std::future_status::ready, 5000);
		QVERIFY(answering.get());

		// Stuck: given up after the deadline, not after forever
		plugin.lock();
		plugin.sendMessage(RemotePluginBase::message{IdFakeHangWhenProcessing});
		plugin.unlock();
		const auto start = std::chrono::steady_clock::now();
		auto stuck = processOnAnotherThread(plugin);
		QTRY_VERIFY_WITH_TIMEOUT(stuck.wait_for(0s) == std::future_status::ready, 10000);
		QVERIFY(!stuck.get());
		const auto waited = std::chrono::steady_clock::now() - start;
		QVERIFY(waited >= RemotePlugin::ProcessTimeout - 100ms);
		QVERIFY(waited < RemotePlugin::ProcessTimeout + 3s);
		QVERIFY(plugin.stoppedResponding());

		// Silent from now on, at once (no more waiting)
		const auto again = std::chrono::steady_clock::now();
		QVERIFY(!plugin.process());
		QVERIFY(std::chrono::steady_clock::now() - again < 100ms);

		// Its process is ended, and it is a suspect
		QTRY_VERIFY_WITH_TIMEOUT(!plugin.isRunning(), 5000);
		QTRY_VERIFY_WITH_TIMEOUT(PluginGuard::isSuspect(Identity), 5000);
	}
};

QTEST_GUILESS_MAIN(RemotePluginHangTest)
#include "RemotePluginHangTest.moc"
