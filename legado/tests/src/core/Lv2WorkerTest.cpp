/*
 * Lv2WorkerTest.cpp
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

#include <QtTest>

#include "lmmsconfig.h"

#ifdef LMMS_HAVE_LV2

#include <chrono>
#include <cstring>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include "LmmsSemaphore.h"
#include "Lv2Worker.h"

using namespace lmms;

namespace
{

//! A plugin's worker: echoes each request back as a response, and records what it was given
struct FakePlugin
{
	std::mutex lock;
	std::vector<std::string> requests;  //!< (worker thread)
	std::vector<std::string> responses; //!< (the "audio" thread, in emitResponses())
};

LV2_Worker_Status work(LV2_Handle handle, LV2_Worker_Respond_Function respond, LV2_Worker_Respond_Handle respondHandle,
	uint32_t size, const void* data)
{
	auto* plugin = static_cast<FakePlugin*>(handle);
	const auto text = std::string(static_cast<const char*>(data), size);
	{
		const auto guard = std::lock_guard{plugin->lock};
		plugin->requests.push_back(text);
	}
	return respond(respondHandle, size, size ? data : nullptr);
}

LV2_Worker_Status workResponse(LV2_Handle handle, uint32_t size, const void* body)
{
	auto* plugin = static_cast<FakePlugin*>(handle);
	plugin->responses.emplace_back(static_cast<const char*>(body), size);
	return LV2_WORKER_SUCCESS;
}

} // namespace

//! The LV2 worker's two queues: whole messages, in order, those of no data too
class Lv2WorkerTest : public QObject
{
	Q_OBJECT

private slots:
	void requestsAndResponsesArriveWholeAndInOrder()
	{
		auto plugin = FakePlugin{};
		auto interface = LV2_Worker_Interface{work, workResponse, nullptr};
		auto workLock = Semaphore{1};
		auto worker = Lv2Worker{&workLock, true};
		worker.setHandle(&plugin);
		worker.setInterface(&interface);

		const auto messages = std::vector<std::string>{"first", "", "a longer third message", "", "", "last"};
		for (const auto& message : messages)
		{
			QCOMPARE(worker.scheduleWork(static_cast<uint32_t>(message.size()), message.data()), LV2_WORKER_SUCCESS);
		}
		// A size without data is taken as no data (it must not take the next message's bytes)
		QCOMPARE(worker.scheduleWork(5, nullptr), LV2_WORKER_SUCCESS);

		// The "audio" thread collects the responses as they come
		const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds{10};
		while (plugin.responses.size() < messages.size() + 1 && std::chrono::steady_clock::now() < deadline)
		{
			worker.emitResponses();
			std::this_thread::sleep_for(std::chrono::milliseconds{1});
		}
		auto expected = messages;
		expected.emplace_back();
		{
			const auto guard = std::lock_guard{plugin.lock};
			QCOMPARE(plugin.requests, expected);
		}
		QCOMPARE(plugin.responses, expected);
	}
};

QTEST_GUILESS_MAIN(Lv2WorkerTest)
#include "Lv2WorkerTest.moc"

#else

class Lv2WorkerTest : public QObject
{
	Q_OBJECT
private slots:
	void skipped() { QSKIP("built without LV2"); }
};

QTEST_GUILESS_MAIN(Lv2WorkerTest)
#include "Lv2WorkerTest.moc"

#endif
