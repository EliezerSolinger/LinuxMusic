/*
 * RemotePluginIpcTest.cpp
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
#include <cstdint>
#include <string>

#include <QtTest>

#include "RemotePluginBase.h"
#include "SystemSemaphore.h"

#ifndef SYNC_WITH_SHM_FIFO
#include <sys/socket.h>
#include <unistd.h>
#endif

using namespace lmms;
using namespace std::chrono_literals;

#ifndef SYNC_WITH_SHM_FIFO
namespace
{

//! The host's side of a connection to a plugin process (what RemotePlugin is, without the process)
class Host : public RemotePluginBase
{
public:
	explicit Host(int socket) { m_socket = socket; }
	~Host() override { ::close(m_socket); }
	auto processMessage(const message&) -> bool override { return true; }
	using RemotePluginBase::invalidate;
};

//! The plugin process's side, written by hand: it may send anything
class FakePlugin
{
public:
	explicit FakePlugin(int socket) : m_socket{socket} {}
	~FakePlugin() { close(); }
	void close()
	{
		if (m_socket >= 0) { ::close(m_socket); }
		m_socket = -1;
	}
	void writeInt(std::int32_t value) { (void)::write(m_socket, &value, sizeof(value)); }
	void writeBytes(const std::string& bytes) { (void)::write(m_socket, bytes.data(), bytes.size()); }

private:
	int m_socket;
};

struct Connection
{
	Connection()
	{
		int sockets[2];
		QVERIFY(socketpair(AF_UNIX, SOCK_STREAM, 0, sockets) == 0);
		host = std::make_unique<Host>(sockets[0]);
		plugin = std::make_unique<FakePlugin>(sockets[1]);
	}
	std::unique_ptr<Host> host;
	std::unique_ptr<FakePlugin> plugin;
};

} // namespace
#endif

//! The connection to a plugin process (phase 6 of docs/plano-refatoracao.md): what the other process sends is checked,
//! and waiting for it has a deadline
class RemotePluginIpcTest : public QObject
{
	Q_OBJECT

private slots:
	//! The semaphore of the shared-memory FIFO (Windows) waits at most as long as asked
	void aSemaphoreWaitsAtMostItsTimeout()
	{
		const auto key = "lmms-test-" + std::to_string(QCoreApplication::applicationPid());
		auto semaphore = SystemSemaphore{key, 0u};
		const auto start = std::chrono::steady_clock::now();
		QVERIFY(!semaphore.tryAcquireFor(50ms));
		const auto waited = std::chrono::steady_clock::now() - start;
		QVERIFY(waited >= 45ms);
		QVERIFY(waited < 2s);
		QVERIFY(semaphore.release());
		QVERIFY(semaphore.tryAcquireFor(50ms));
	}

#ifndef SYNC_WITH_SHM_FIFO
	//! A message as the plugin sends it arrives whole
	void aMessageArrives()
	{
		auto connection = Connection{};
		connection.plugin->writeInt(IdUserBase + 3);
		connection.plugin->writeInt(2);
		connection.plugin->writeInt(5);
		connection.plugin->writeBytes("hello");
		connection.plugin->writeInt(0);
		const auto m = connection.host->waitForMessage(RemotePluginBase::message{IdUserBase + 3}, 1s);
		QCOMPARE(m.id, IdUserBase + 3);
		QCOMPARE(m.getString(0), std::string{"hello"});
		QCOMPARE(m.getString(1), std::string{});
		QVERIFY(!connection.host->isInvalid());
	}

	//! A plugin that does not answer: the wait ends at its deadline, the connection still usable
	void aSilentPluginTimesOut()
	{
		auto connection = Connection{};
		const auto start = std::chrono::steady_clock::now();
		const auto m = connection.host->waitForMessage(RemotePluginBase::message{IdProcessingDone}, 100ms);
		const auto waited = std::chrono::steady_clock::now() - start;
		QCOMPARE(m.id, int{IdUndefined});
		QVERIFY(waited >= 90ms);
		QVERIFY(waited < 2s);
		QVERIFY(!connection.host->isInvalid());
	}

	//! A text longer than any message may carry (or negative): broken, nothing allocated for it, no wait for it
	void anImpossibleTextLengthBreaksTheConnection_data()
	{
		QTest::addColumn<int>("length");
		QTest::newRow("huge") << 0x7fffffff;
		QTest::newRow("negative") << -5;
	}

	void anImpossibleTextLengthBreaksTheConnection()
	{
		QFETCH(int, length);
		auto connection = Connection{};
		connection.plugin->writeInt(IdUserBase);
		connection.plugin->writeInt(1);
		connection.plugin->writeInt(length);
		const auto m = connection.host->waitForMessage(RemotePluginBase::message{IdUserBase}, 1s);
		QCOMPARE(m.id, int{IdUndefined});
		QVERIFY(connection.host->isInvalid());
	}

	//! A message of more parts than any may have (or negative): broken
	void anImpossiblePartCountBreaksTheConnection_data()
	{
		QTest::addColumn<int>("parts");
		QTest::newRow("huge") << 0x7fffffff;
		QTest::newRow("negative") << -1;
	}

	void anImpossiblePartCountBreaksTheConnection()
	{
		QFETCH(int, parts);
		auto connection = Connection{};
		connection.plugin->writeInt(IdUserBase);
		connection.plugin->writeInt(parts);
		const auto m = connection.host->waitForMessage(RemotePluginBase::message{IdUserBase}, 1s);
		QCOMPARE(m.id, int{IdUndefined});
		QVERIFY(connection.host->isInvalid());
	}

	//! The plugin process dies in the middle of a message: broken, at once
	void aPluginDyingMidMessageBreaksTheConnection()
	{
		auto connection = Connection{};
		connection.plugin->writeInt(IdUserBase);
		connection.plugin->writeInt(1);
		connection.plugin->writeInt(100);
		connection.plugin->writeBytes("only part of it");
		connection.plugin->close();
		const auto start = std::chrono::steady_clock::now();
		const auto m = connection.host->waitForMessage(RemotePluginBase::message{IdUserBase}, 5s);
		QCOMPARE(m.id, int{IdUndefined});
		QVERIFY(connection.host->isInvalid());
		QVERIFY(std::chrono::steady_clock::now() - start < 2s);
	}

	//! A broken connection is not waited on any more
	void aBrokenConnectionIsNotWaitedOn()
	{
		auto connection = Connection{};
		connection.host->invalidate();
		const auto start = std::chrono::steady_clock::now();
		const auto m = connection.host->waitForMessage(RemotePluginBase::message{IdProcessingDone}, 5s);
		QCOMPARE(m.id, int{IdUndefined});
		QVERIFY(std::chrono::steady_clock::now() - start < 1s);
	}
#endif
};

QTEST_GUILESS_MAIN(RemotePluginIpcTest)
#include "RemotePluginIpcTest.moc"
