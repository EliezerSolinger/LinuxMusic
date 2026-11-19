/*
 * ErrorLog.cpp
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

#include "ErrorLog.h"

#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QStringList>

#include <array>
#include <atomic>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <cstdio>
#include <cstring>
#include <limits>
#include <mutex>
#include <thread>
#include <utility>

namespace lmms::ErrorLog
{

namespace
{

//! A bounded queue of many writers and one reader (Dmitry Vyukov's): writers never wait, nor allocate
constexpr std::uint64_t Capacity = 1024;

struct Entry
{
	std::atomic<std::uint64_t> sequence{0};
	Severity severity = Severity::Warning;
	const char* where = "";
	const char* what = "";
	double value = std::numeric_limits<double>::quiet_NaN(); //!< NaN: none
	std::int64_t time = 0; //!< milliseconds since the epoch
};

struct Queue
{
	Queue()
	{
		for (std::uint64_t i = 0; i < Capacity; ++i) { entries[i].sequence.store(i, std::memory_order_relaxed); }
	}
	std::array<Entry, Capacity> entries;
	alignas(64) std::atomic<std::uint64_t> writePosition{0};
	alignas(64) std::uint64_t readPosition = 0; //!< (the reader's, under s_writeMutex)
};

Queue s_queue;
std::atomic<std::uint64_t> s_reported{0};
std::atomic<std::uint64_t> s_dropped{0};
std::uint64_t s_droppedWritten = 0; //!< (under s_writeMutex)

//! Serializes reading the queue and writing out
std::mutex s_writeMutex;
QString s_file;               //!< (under s_writeMutex) empty: the standard error only
QStringList s_waiting;        //!< texts reported before start(), for the file (under s_writeMutex)
constexpr qsizetype MaxWaiting = 1000;
constexpr qint64 MaxFileSize = qint64{1} << 20;

std::mutex s_threadMutex;
std::condition_variable s_wake;
std::thread s_thread;
bool s_stopping = false;

//! Stops the log's thread when the program ends without stop() (a std::thread destroyed while it runs ends the
//! program); declared after it, so destroyed before it
struct StopAtExit
{
	~StopAtExit() { stop(); }
};

auto now() noexcept -> std::int64_t
{
	return std::chrono::duration_cast<std::chrono::milliseconds>(
		std::chrono::system_clock::now().time_since_epoch()).count();
}

auto severityName(Severity severity) -> const char*
{
	return severity == Severity::Error ? "error" : "warning";
}

auto line(std::int64_t time, Severity severity, const char* where, const QString& what) -> QString
{
	return QStringLiteral("%1 [%2] %3: %4")
		.arg(QDateTime::fromMSecsSinceEpoch(time).toString(QStringLiteral("yyyy-MM-dd hh:mm:ss.zzz")),
			QString::fromLatin1(severityName(severity)), QString::fromUtf8(where), what);
}

void push(Severity severity, const char* where, const char* what, double value) noexcept
{
	s_reported.fetch_add(1, std::memory_order_relaxed);
	auto position = s_queue.writePosition.load(std::memory_order_relaxed);
	for (;;)
	{
		auto& entry = s_queue.entries[position % Capacity];
		const auto sequence = entry.sequence.load(std::memory_order_acquire);
		const auto difference = static_cast<std::int64_t>(sequence) - static_cast<std::int64_t>(position);
		if (difference == 0)
		{
			if (s_queue.writePosition.compare_exchange_weak(position, position + 1, std::memory_order_relaxed))
			{
				entry.severity = severity;
				entry.where = where ? where : "";
				entry.what = what ? what : "";
				entry.value = value;
				entry.time = now();
				entry.sequence.store(position + 1, std::memory_order_release);
				return;
			}
		}
		else if (difference < 0)
		{
			// Full: the log's thread is behind; counted, never waited for
			s_dropped.fetch_add(1, std::memory_order_relaxed);
			return;
		}
		else { position = s_queue.writePosition.load(std::memory_order_relaxed); }
	}
}

//! Appends @p lines to the file (making its folder, keeping one older file past MaxFileSize) and the standard error
void writeOut(const QStringList& lines)
{
	if (lines.isEmpty()) { return; }
	for (const auto& text : lines) { std::fprintf(stderr, "%s\n", qUtf8Printable(text)); }
	if (s_file.isEmpty()) { return; }
	QDir{}.mkpath(QFileInfo{s_file}.absolutePath());
	if (QFileInfo{s_file}.size() > MaxFileSize)
	{
		QFile::remove(s_file + ".1");
		QFile::rename(s_file, s_file + ".1");
	}
	auto file = QFile{s_file};
	if (!file.open(QIODevice::WriteOnly | QIODevice::Append | QIODevice::Text)) { return; }
	for (const auto& text : lines) { file.write((text + '\n').toUtf8()); }
}

//! Takes the queue's entries as lines, the same ones in a row joined (under s_writeMutex)
auto takeEntries() -> QStringList
{
	auto lines = QStringList{};
	const Entry* previous = nullptr;
	auto previousTime = std::int64_t{0};
	auto repeated = 0;
	auto severity = Severity::Warning;
	const char* where = nullptr;
	const char* what = nullptr;
	auto value = 0.0;
	const auto close = [&] {
		if (!previous) { return; }
		auto text = QString::fromUtf8(what);
		if (!std::isnan(value)) { text += QStringLiteral(" (%1)").arg(value); }
		if (repeated > 1) { text += QStringLiteral(" (repeated %1 times)").arg(repeated); }
		lines.append(line(previousTime, severity, where, text));
	};
	for (;;)
	{
		auto& entry = s_queue.entries[s_queue.readPosition % Capacity];
		if (entry.sequence.load(std::memory_order_acquire) != s_queue.readPosition + 1) { break; }
		const auto same = previous && entry.severity == severity && std::strcmp(entry.where, where) == 0
			&& std::strcmp(entry.what, what) == 0 && (entry.value == value || (std::isnan(entry.value) && std::isnan(value)));
		if (same) { ++repeated; }
		else
		{
			close();
			previous = &entry;
			previousTime = entry.time;
			repeated = 1;
			severity = entry.severity;
			where = entry.where;
			what = entry.what;
			value = entry.value;
		}
		entry.sequence.store(s_queue.readPosition + Capacity, std::memory_order_release);
		++s_queue.readPosition;
	}
	close();
	const auto dropped = s_dropped.load(std::memory_order_relaxed);
	if (dropped != s_droppedWritten)
	{
		lines.append(line(now(), Severity::Warning, "ErrorLog",
			QStringLiteral("%1 entries lost: reported faster than written").arg(dropped - s_droppedWritten)));
		s_droppedWritten = dropped;
	}
	return lines;
}

} // namespace


void report(Severity severity, const char* where, const char* what) noexcept
{
	push(severity, where, what, std::numeric_limits<double>::quiet_NaN());
}

void report(Severity severity, const char* where, const char* what, double value) noexcept
{
	push(severity, where, what, value);
}

void reportText(Severity severity, const char* where, const QString& what)
{
	s_reported.fetch_add(1, std::memory_order_relaxed);
	const auto lock = std::lock_guard{s_writeMutex};
	// (after what was reported before it, in order)
	auto lines = takeEntries();
	lines.append(line(now(), severity, where, what));
	if (s_file.isEmpty() && s_waiting.size() < MaxWaiting) { s_waiting.append(lines); }
	writeOut(lines);
}

void flush()
{
	const auto lock = std::lock_guard{s_writeMutex};
	const auto lines = takeEntries();
	if (s_file.isEmpty() && s_waiting.size() < MaxWaiting) { s_waiting.append(lines); }
	writeOut(lines);
}

void start(const QString& file)
{
	{
		const auto lock = std::lock_guard{s_writeMutex};
		s_file = file;
		// What was reported before, to the file now (it went to the standard error already)
		if (!s_file.isEmpty() && !s_waiting.isEmpty())
		{
			const auto waiting = std::exchange(s_waiting, {});
			QDir{}.mkpath(QFileInfo{s_file}.absolutePath());
			auto out = QFile{s_file};
			if (out.open(QIODevice::WriteOnly | QIODevice::Append | QIODevice::Text))
			{
				for (const auto& text : waiting) { out.write((text + '\n').toUtf8()); }
			}
		}
	}
	const auto lock = std::lock_guard{s_threadMutex};
	if (s_thread.joinable()) { return; }
	s_stopping = false;
	s_thread = std::thread{[] {
		auto lock = std::unique_lock{s_threadMutex};
		while (!s_stopping)
		{
			s_wake.wait_for(lock, std::chrono::milliseconds{200});
			lock.unlock();
			flush();
			lock.lock();
		}
	}};
}

void stop()
{
	{
		const auto lock = std::lock_guard{s_threadMutex};
		s_stopping = true;
	}
	s_wake.notify_all();
	if (s_thread.joinable()) { s_thread.join(); }
	flush();
	const auto lock = std::lock_guard{s_writeMutex};
	s_file.clear();
}

auto reportedCount() noexcept -> std::uint64_t
{
	return s_reported.load(std::memory_order_relaxed);
}

auto droppedCount() noexcept -> std::uint64_t
{
	return s_dropped.load(std::memory_order_relaxed);
}

namespace
{
const StopAtExit s_stopAtExit;
}

auto fileIn(const QString& workingDir) -> QString
{
	return QDir{workingDir}.filePath(QStringLiteral("logs/errors.log"));
}

} // namespace lmms::ErrorLog
