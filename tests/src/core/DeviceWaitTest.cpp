/*
 * DeviceWaitTest.cpp
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

#include <QObject>
#include <QtTest>

#include <chrono>
#include <thread>
#include <vector>

#include "AudioEngine.h"
#include "Engine.h"
#include "Song.h"

using namespace lmms;

//! The device's thread waits for a change of the model a bounded while, then gives silence (and the song waits)
class DeviceWaitTest : public QObject
{
	Q_OBJECT

	//! The device asking for @p periods periods, on a thread of its own; @returns how long it took
	static auto deviceRenders(int periods, std::vector<float>& out) -> std::chrono::duration<double>
	{
		auto* engine = Engine::audioEngine();
		const auto frames = static_cast<std::size_t>(periods) * engine->framesPerPeriod();
		out.assign(frames * 2, 1.f);
		auto took = std::chrono::duration<double>{};
		auto device = std::thread{[&] {
			const auto start = std::chrono::steady_clock::now();
			engine->renderNextBuffer(InterleavedBufferView<float>{out.data(), 2, static_cast<f_cnt_t>(frames)});
			took = std::chrono::steady_clock::now() - start;
		}};
		device.join();
		return took;
	}

	static auto periodSeconds() -> double
	{
		return static_cast<double>(Engine::audioEngine()->framesPerPeriod()) / Engine::audioEngine()->outputSampleRate();
	}

private slots:
	void initTestCase()
	{
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
		Engine::getSong()->createNewProject();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	//! Held for the whole buffer: the first period waits half a period, the others not at all; all silence
	void aLongChangeGivesSilenceInTime()
	{
		auto* engine = Engine::audioEngine();
		auto out = std::vector<float>{};
		const auto skippedBefore = engine->periodsSkipped();
		const auto renderedBefore = engine->periodsRendered();
		auto took = std::chrono::duration<double>{};
		{
			const auto guard = engine->requestChangesGuard();
			took = deviceRenders(4, out);
		}
		QCOMPARE(engine->periodsSkipped() - skippedBefore, std::int64_t{4});
		QCOMPARE(engine->periodsRendered() - renderedBefore, std::int64_t{0});
		for (const auto sample : out) { QCOMPARE(sample, 0.f); }
		// Well within the four periods' time the device has (half a period waited, and a margin)
		QVERIFY2(took.count() < 2 * periodSeconds(), qPrintable(QString::number(took.count())));

		// Once the change ends, the periods are rendered again
		deviceRenders(2, out);
		QCOMPARE(engine->periodsRendered() - renderedBefore, std::int64_t{2});
		QCOMPARE(engine->periodsSkipped() - skippedBefore, std::int64_t{4});
	}

	//! A short change is waited for: nothing is skipped
	void aShortChangeIsWaitedFor()
	{
		auto* engine = Engine::audioEngine();
		const auto skippedBefore = engine->periodsSkipped();
		const auto renderedBefore = engine->periodsRendered();
		auto out = std::vector<float>{};
		engine->requestChangeInModel();
		auto device = std::thread{[&] { deviceRenders(1, out); }};
		// A tenth of a period, then the change ends
		std::this_thread::sleep_for(std::chrono::duration<double>{periodSeconds() / 10});
		engine->doneChangeInModel();
		device.join();
		QCOMPARE(engine->periodsSkipped() - skippedBefore, std::int64_t{0});
		QCOMPARE(engine->periodsRendered() - renderedBefore, std::int64_t{1});
	}
};

QTEST_GUILESS_MAIN(DeviceWaitTest)
#include "DeviceWaitTest.moc"
