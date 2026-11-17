/*
 * ErrorLogTest.cpp
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

#include <QFile>
#include <QTemporaryDir>
#include <QtTest>

#include <thread>
#include <vector>

#include "ErrorLog.h"

using namespace lmms;

class ErrorLogTest : public QObject
{
	Q_OBJECT

	QTemporaryDir m_directory;

	auto logText() -> QString
	{
		ErrorLog::flush();
		auto file = QFile{ErrorLog::fileIn(m_directory.path())};
		return file.open(QIODevice::ReadOnly) ? QString::fromUtf8(file.readAll()) : QString{};
	}

	//! How many times @p what was reported, from the log's lines ("(repeated N times)" counts N)
	static auto timesIn(const QString& log, const QString& what) -> int
	{
		auto times = 0;
		for (const auto& line : log.split('\n'))
		{
			if (!line.contains(what)) { continue; }
			const auto repeated = QRegularExpression{"\\(repeated (\\d+) times\\)"}.match(line);
			times += repeated.hasMatch() ? repeated.captured(1).toInt() : 1;
		}
		return times;
	}

private slots:
	void initTestCase()
	{
		QVERIFY(m_directory.isValid());
		ErrorLog::start(ErrorLog::fileIn(m_directory.path()));
	}

	void cleanupTestCase()
	{
		ErrorLog::stop();
	}

	void anEntryIsWrittenWithWhereWhatAndValue()
	{
		ErrorLog::report(ErrorLog::Severity::Error, "ErrorLogTest", "a resampling failed", 6);
		const auto log = logText();
		QVERIFY2(log.contains("[error] ErrorLogTest: a resampling failed (6)"), qPrintable(log));
	}

	//! From many threads at once, the audio threads among them: none lost while there is room
	void entriesFromManyThreadsAreAllWritten()
	{
		constexpr auto Threads = 4;
		constexpr auto Each = 200;
		auto threads = std::vector<std::thread>{};
		for (int t = 0; t < Threads; ++t)
		{
			threads.emplace_back([] {
				for (int i = 0; i < Each; ++i) { ErrorLog::report(ErrorLog::Severity::Warning, "thread", "many at once"); }
			});
		}
		// (written meanwhile, as the log's thread does)
		for (int i = 0; i < 20; ++i) { ErrorLog::flush(); }
		for (auto& thread : threads) { thread.join(); }
		const auto dropped = ErrorLog::droppedCount();
		QCOMPARE(timesIn(logText(), "many at once") + static_cast<int>(dropped), Threads * Each);
	}

	//! A text made at the moment comes after what was reported before it
	void textsKeepTheirOrder()
	{
		ErrorLog::report(ErrorLog::Severity::Warning, "order", "first");
		ErrorLog::reportText(ErrorLog::Severity::Warning, "order", QString{"second, %1"}.arg(2));
		const auto log = logText();
		const auto first = log.indexOf("order: first");
		const auto second = log.indexOf("order: second, 2");
		QVERIFY(first >= 0 && second > first);
	}

	//! Faster than the log writes (no flush): the extra entries are counted and the count is written
	void tooManyAtOnceAreCountedNotWaitedFor()
	{
		const auto before = ErrorLog::droppedCount();
		for (int i = 0; i < 5000; ++i) { ErrorLog::report(ErrorLog::Severity::Warning, "flood", "too many"); }
		QVERIFY(ErrorLog::droppedCount() > before);
		QVERIFY(logText().contains("entries lost"));
	}
};

QTEST_GUILESS_MAIN(ErrorLogTest)
#include "ErrorLogTest.moc"
