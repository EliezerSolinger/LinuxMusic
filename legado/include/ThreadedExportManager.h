/*
 * ThreadedExportManager.h - exports files in .flac format on an other thread
 *
 * Copyright (c) 2024 - 2025 szeli1
 *
 * This file is part of LMMS - https://lmms.io
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

#ifndef LMMS_THREADED_EXPORT_MANAGER_H
#define LMMS_THREADED_EXPORT_MANAGER_H

#include <condition_variable>
#include <deque>
#include <memory>
#include <mutex>
#include <thread>

#include <QString>

namespace lmms
{

class SampleBuffer;

typedef void (*callbackFn)(void*);

/**
 * Writes sample buffers to FLAC files on a thread of its own, in the order they were asked for. The thread starts
 * with the first request and waits for more; stopExporting() (and the destructor) writes what is still queued and
 * then ends it.
 */
class ThreadedExportManager
{
public:
	ThreadedExportManager() = default;
	~ThreadedExportManager();

	ThreadedExportManager(const ThreadedExportManager&) = delete;
	ThreadedExportManager& operator=(const ThreadedExportManager&) = delete;

	//! @param outputLocationAndName: should include path and name, could include ".flac"
	//! @param callbackFunction called on the export thread once the file is written (or failed)
	void startExporting(const QString& outputLocationAndName, std::shared_ptr<const SampleBuffer> buffer,
		callbackFn callbackFunction = nullptr, void* callbackObject = nullptr);
	//! Writes what is queued and ends the thread (a later request starts it again)
	void stopExporting();

private:
	struct Request
	{
		QString path;
		std::shared_ptr<const SampleBuffer> buffer;
		callbackFn callback;
		void* callbackObject;
	};

	void run();
	static void write(const Request& request);

	std::mutex m_mutex;
	std::condition_variable m_requested;
	std::deque<Request> m_requests; //!< Under m_mutex
	bool m_stopping = false; //!< Under m_mutex
	std::thread m_thread;
};

} // namespace lmms

#endif // LMMS_THREADED_EXPORT_MANAGER_H
