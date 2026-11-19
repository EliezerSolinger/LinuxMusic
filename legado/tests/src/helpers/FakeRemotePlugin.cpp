/*
 * FakeRemotePlugin.cpp - a plugin process for the tests: it answers as a real one, and can be told to stop
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
#include <cstdlib>
#include <thread>

#include "RemotePluginClient.h"

using namespace lmms;

//! What the test tells it (see RemotePluginHangTest)
enum FakeMessages
{
	IdFakeHangWhenProcessing = IdUserBase + 1, //!< from now on, it never finishes a period
};

class FakeRemotePlugin : public RemotePluginClient
{
public:
#ifdef SYNC_WITH_SHM_FIFO
	FakeRemotePlugin(const std::string& in, const std::string& out) : RemotePluginClient(in, out)
#else
	explicit FakeRemotePlugin(const char* socketPath) : RemotePluginClient(socketPath)
#endif
	{
		setInputOutputCount(0, 2);
		sendMessage(IdInitDone);
		waitForMessage(IdInitDone);
	}

	void process(const float* const*, float* const*) override {}

	auto processMessage(const message& m) -> bool override
	{
		switch (m.id)
		{
		case IdFakeHangWhenProcessing:
			m_hang = true;
			return true;
		case IdChangeSharedMemoryKey:
			// (no audio here: the test's buffers do not exist)
			return true;
		case IdStartProcessing:
			if (m_hang)
			{
				// Stuck, as a plugin in an endless loop: the host has to give up and end this process
				while (true) { std::this_thread::sleep_for(std::chrono::seconds{1}); }
			}
			sendMessage(IdProcessingDone);
			return true;
		default:
			return RemotePluginClient::processMessage(m);
		}
	}

	void run()
	{
		message m;
		while ((m = receiveMessage()).id != IdQuit && !isInvalid()) { processMessage(m); }
	}

private:
	bool m_hang = false;
};

int main(int argc, char** argv)
{
#ifdef SYNC_WITH_SHM_FIFO
	if (argc < 3) { return EXIT_FAILURE; }
	auto plugin = FakeRemotePlugin{argv[1], argv[2]};
#else
	if (argc < 2) { return EXIT_FAILURE; }
	auto plugin = FakeRemotePlugin{argv[1]};
#endif
	plugin.run();
	return EXIT_SUCCESS;
}
