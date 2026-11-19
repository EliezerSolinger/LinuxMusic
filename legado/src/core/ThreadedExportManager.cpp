/*
 * ThreadedExportManager.cpp - exports files in .flac format on an other thread
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

#include "ThreadedExportManager.h"

#include "ErrorLog.h"
#include "FlacExporter.h"
#include "SampleBuffer.h"

namespace lmms
{

ThreadedExportManager::~ThreadedExportManager()
{
	stopExporting();
}




void ThreadedExportManager::startExporting(const QString& outputLocationAndName,
	std::shared_ptr<const SampleBuffer> buffer, callbackFn callbackFunction, void* callbackObject)
{
	if (!buffer) { return; }
	{
		const auto lock = std::lock_guard{m_mutex};
		m_requests.push_back({outputLocationAndName, std::move(buffer), callbackFunction, callbackObject});
		if (!m_thread.joinable())
		{
			m_stopping = false;
			m_thread = std::thread{&ThreadedExportManager::run, this};
		}
	}
	m_requested.notify_one();
}




void ThreadedExportManager::stopExporting()
{
	{
		const auto lock = std::lock_guard{m_mutex};
		if (!m_thread.joinable()) { return; }
		m_stopping = true;
	}
	m_requested.notify_one();
	m_thread.join();
	m_thread = std::thread{};
}




void ThreadedExportManager::run()
{
	auto lock = std::unique_lock{m_mutex};
	while (true)
	{
		m_requested.wait(lock, [this] { return !m_requests.empty() || m_stopping; });
		// (what is queued is written before stopping)
		if (m_requests.empty()) { return; }
		const auto request = std::move(m_requests.front());
		m_requests.pop_front();

		lock.unlock();
		write(request);
		lock.lock();
	}
}




void ThreadedExportManager::write(const Request& request)
{
	// (its file is closed before the callback)
	{
		auto exporter = FlacExporter{static_cast<int>(request.buffer->sampleRate()), 24, request.path};
		if (exporter.getIsSuccesful()) { exporter.writeThisBuffer(request.buffer->data(), request.buffer->size()); }
		else
		{
			ErrorLog::reportText(ErrorLog::Severity::Error, "ThreadedExportManager",
				QString{"could not write %1"}.arg(request.path));
		}
	}
	if (request.callback) { request.callback(request.callbackObject); }
}

} // namespace lmms
