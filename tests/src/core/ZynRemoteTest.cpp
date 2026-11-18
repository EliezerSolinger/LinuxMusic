/*
 * ZynRemoteTest.cpp
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

#include <future>

#include <QtTest>

#include "AudioEngine.h"
#include "AudioProcessor.h"
#include "Engine.h"
#include "PluginGuard.h"
#include "RemotePlugin.h"
#include "RemoteZynAddSubFx.h"
#include "SharedMemory.h"

using namespace lmms;
using namespace std::chrono_literals;

/**
 * ZynAddSubFX in its own process (RemoteZynAddSubFx), as it runs while its window is shown: the program's own
 * messages to it. Its message loop holds the master's mutex while it handles a message, a std::mutex: a handler that
 * took it again stopped the process for good (the master volume and the shut up did), and the instrument went
 * silent. The tests use ZynAddSubFX in-process, which locks once: only this runs the process
 */
class ZynRemoteTest : public QObject
{
	Q_OBJECT

	static constexpr auto Identity = "remotezynaddsubfx-test";

	//! process(), as the audio thread calls it: whether the process answered
	static auto processes(RemotePlugin& plugin) -> bool
	{
		auto result = std::async(std::launch::async, [&plugin] { return plugin.process(); });
		if (result.wait_for(RemotePlugin::ProcessTimeout + 3s) != std::future_status::ready) { return false; }
		return result.get();
	}

	static void send(RemotePlugin& plugin, const RemotePluginBase::message& message)
	{
		plugin.lock();
		plugin.sendMessage(message);
		plugin.unlock();
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

	void answersTheProgramsMessages()
	{
		auto plugin = RemotePlugin{};
		plugin.setGuardIdentity(Identity);
		if (plugin.init("RemoteZynAddSubFx", false)) { QSKIP("the RemoteZynAddSubFx process is not built here"); }
		plugin.lock();
		plugin.waitForInitDone(false);
		plugin.unlock();
		QVERIFY(!plugin.failed());
		send(plugin, RemotePluginBase::message{IdBufferSizeInformation}.addInt(256));
		plugin.updateSampleRate(44100);
		// Its outputs in shared memory, as the host makes them (no inputs, two outputs of a period)
		auto buffers = SharedMemory<float[]>{};
		buffers.create(2 * 256);
		plugin.setSharedAudioBuffer(AudioProcessor::SharedBufferInfo{buffers.key(), 2, 2, 256});
		QVERIFY(processes(plugin));

		// The messages a new instrument and a project's rest send (see ZynAddSubFxInstrument): it still processes
		send(plugin, RemotePluginBase::message{IdZasfSetMasterVolume}.addInt(127));
		QVERIFY2(processes(plugin), "stopped after the master volume");
		send(plugin, RemotePluginBase::message{IdZasfShutUp});
		QVERIFY2(processes(plugin), "stopped after the shut up");
		send(plugin, RemotePluginBase::message{IdZasfSetPitchWheelBendRange}.addInt(2));
		QVERIFY2(processes(plugin), "stopped after the bend range");
		QVERIFY(!plugin.stoppedResponding());
	}
};

QTEST_GUILESS_MAIN(ZynRemoteTest)
#include "ZynRemoteTest.moc"
