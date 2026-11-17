/*
 * ThreadedExportTest.cpp
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
#include <memory>
#include <vector>

#include <QFileInfo>
#include <QTemporaryDir>
#include <QtTest>

#include "SampleBuffer.h"
#include "SampleFrame.h"
#include "ThreadedExportManager.h"

using namespace lmms;

//! Sample buffers written to FLAC files on the export thread
class ThreadedExportTest : public QObject
{
	Q_OBJECT

	QTemporaryDir m_directory;

	static auto buffer() -> std::shared_ptr<const SampleBuffer>
	{
		auto frames = std::vector<SampleFrame>(4410);
		for (auto i = std::size_t{0}; i < frames.size(); ++i)
		{
			frames[i] = SampleFrame{static_cast<float>(i % 100) / 200.f};
		}
		return std::make_shared<const SampleBuffer>(std::move(frames), 44100);
	}

	static void count(void* written) { ++*static_cast<std::atomic<int>*>(written); }

private slots:
	void initTestCase() { QVERIFY(m_directory.isValid()); }

	//! Each export is written, the ones asked for after the first was done too (they were lost)
	void everyExportIsWritten()
	{
		auto written = std::atomic<int>{0};
		auto manager = ThreadedExportManager{};
		manager.startExporting(m_directory.filePath("first.flac"), buffer(), &count, &written);
		QTRY_COMPARE(written.load(), 1);
		manager.startExporting(m_directory.filePath("second.flac"), buffer(), &count, &written);
		QTRY_COMPARE(written.load(), 2);
		QVERIFY(QFileInfo{m_directory.filePath("first.flac")}.size() > 0);
		QVERIFY(QFileInfo{m_directory.filePath("second.flac")}.size() > 0);
	}

	//! Stopping writes what is still queued, and a later export starts the thread again
	void stoppingWritesWhatIsQueued()
	{
		auto written = std::atomic<int>{0};
		{
			auto manager = ThreadedExportManager{};
			for (auto i = 0; i < 5; ++i)
			{
				manager.startExporting(m_directory.filePath(QString{"queued%1.flac"}.arg(i)), buffer(), &count,
					&written);
			}
			manager.stopExporting();
			QCOMPARE(written.load(), 5);
			manager.startExporting(m_directory.filePath("again.flac"), buffer(), &count, &written);
		}
		// (the destructor stops it too)
		QCOMPARE(written.load(), 6);
	}

	//! A file that cannot be written still calls back, and the thread goes on
	void aFailedExportCallsBack()
	{
		auto written = std::atomic<int>{0};
		auto manager = ThreadedExportManager{};
		manager.startExporting(m_directory.filePath("missing/folder/file.flac"), buffer(), &count, &written);
		manager.startExporting(m_directory.filePath("after.flac"), buffer(), &count, &written);
		manager.stopExporting();
		QCOMPARE(written.load(), 2);
		QVERIFY(QFileInfo{m_directory.filePath("after.flac")}.size() > 0);
	}
};

QTEST_GUILESS_MAIN(ThreadedExportTest)
#include "ThreadedExportTest.moc"
