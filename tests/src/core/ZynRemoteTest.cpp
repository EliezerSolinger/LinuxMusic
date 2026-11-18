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
#include <thread>
#include <vector>

#include <QtTest>

#include "AudioEngine.h"
#include "AudioProcessor.h"
#include "Engine.h"
#include "MidiEvent.h"
#include "PluginGuard.h"
#include "RemotePlugin.h"
#include "RemoteZynAddSubFx.h"
#include "SharedMemory.h"
#include "SharedMemoryPointer.h"

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

	//! A note's sound from a new process: @p blocks blocks, with a pause between them as the program has (a period)
	static auto renderNote(int blocks) -> std::vector<float>
	{
		auto plugin = RemotePlugin{};
		plugin.setGuardIdentity(Identity);
		if (plugin.init("RemoteZynAddSubFx", false)) { return {}; }
		plugin.lock();
		plugin.waitForInitDone(false);
		plugin.unlock();
		send(plugin, RemotePluginBase::message{IdBufferSizeInformation}.addInt(256));
		plugin.updateSampleRate(44100);
		auto buffers = SharedMemory<std::byte[]>{};
		buffers.create(2 * sizeof(PointerOffset) + 2 * 256 * sizeof(float));
		auto* offsets = reinterpret_cast<PointerOffset*>(buffers.get());
		auto* channels = reinterpret_cast<float*>(buffers.get() + 2 * sizeof(PointerOffset));
		for (int ch = 0; ch < 2; ++ch) { assignPointer(offsets[ch], channels + ch * 256); }
		plugin.setSharedAudioBuffer(AudioProcessor::SharedBufferInfo{buffers.key(), 2, 2, 256});
		send(plugin, RemotePluginBase::message{IdZasfShutUp});

		plugin.lock();
		plugin.processMidiEvent(MidiEvent{MidiNoteOn, 0, 69, 100}, 0);
		plugin.unlock();
		auto sound = std::vector<float>{};
		for (int block = 0; block < blocks; ++block)
		{
			if (!processes(plugin)) { return {}; }
			sound.insert(sound.end(), channels, channels + 256);
			std::this_thread::sleep_for(std::chrono::milliseconds{6});
		}
		return sound;
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
		// Its outputs in shared memory, as the host makes them (see AudioBuffer): a table of where each channel's
		// buffer is, then the buffers (no inputs, two outputs of a period)
		auto buffers = SharedMemory<std::byte[]>{};
		buffers.create(2 * sizeof(PointerOffset) + 2 * 256 * sizeof(float));
		auto* offsets = reinterpret_cast<PointerOffset*>(buffers.get());
		auto* channels = reinterpret_cast<float*>(buffers.get() + 2 * sizeof(PointerOffset));
		for (int ch = 0; ch < 2; ++ch) { assignPointer(offsets[ch], channels + ch * 256); }
		plugin.setSharedAudioBuffer(AudioProcessor::SharedBufferInfo{buffers.key(), 2, 2, 256});
		QVERIFY(processes(plugin));

		// The messages a new instrument and a project's rest send (see ZynAddSubFxInstrument): it still processes
		send(plugin, RemotePluginBase::message{IdZasfSetMasterVolume}.addInt(127));
		QVERIFY2(processes(plugin), "stopped after the master volume");
		send(plugin, RemotePluginBase::message{IdZasfShutUp});
		QVERIFY2(processes(plugin), "stopped after the shut up");
		send(plugin, RemotePluginBase::message{IdZasfSetPortamentoTime}.addInt(40));
		QVERIFY2(processes(plugin), "stopped after the portamento time");
		send(plugin, RemotePluginBase::message{IdZasfSetPitchWheelBendRange}.addInt(2));
		QVERIFY2(processes(plugin), "stopped after the bend range");
		QVERIFY(!plugin.stoppedResponding());
	}
	//! Only the program drives its sound: the same note sounds the same in another process, whatever the timing
	void soundsTheSameEachTime()
	{
		const auto first = renderNote(60);
		if (first.empty()) { QSKIP("the RemoteZynAddSubFx process is not built here"); }
		const auto second = renderNote(60);
		QCOMPARE(second.size(), first.size());
		auto loudest = 0.f;
		auto differences = 0;
		for (std::size_t i = 0; i < first.size(); ++i)
		{
			loudest = std::max(loudest, std::abs(first[i]));
			if (first[i] != second[i]) { ++differences; }
		}
		QVERIFY2(loudest > 0.001f, qPrintable(QString::number(loudest)));
		QVERIFY2(differences == 0, qPrintable(QString{"%1 samples differ"}.arg(differences)));
	}
};

QTEST_GUILESS_MAIN(ZynRemoteTest)
#include "ZynRemoteTest.moc"
