/*
 * Autosave.cpp
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

#include "Autosave.h"

#include <utility>

#include <QDir>
#include <QFileInfo>
#include <QGuiApplication>
#include <QSaveFile>
#include <QTextStream>

#include "RemotePluginBase.h"
#include "Song.h"

namespace lmms
{

Autosave::Autosave(Song& song, QString path, QObject* parent, int quietMs, int longestWaitMs)
	: QObject{parent}
	, m_song{song}
	, m_path{std::move(path)}
{
	m_quietTimer.setSingleShot(true);
	m_quietTimer.setInterval(quietMs);
	m_longestTimer.setSingleShot(true);
	m_longestTimer.setInterval(longestWaitMs);
	connect(&m_quietTimer, &QTimer::timeout, this, &Autosave::takeSnapshot);
	connect(&m_longestTimer, &QTimer::timeout, this, &Autosave::takeSnapshot);
	connect(&m_song, &Song::changed, this, &Autosave::changed);
	m_writer = std::thread{[this] { writerLoop(); }};
}




Autosave::~Autosave()
{
	{
		const auto lock = std::lock_guard{m_mutex};
		m_quit = true;
	}
	m_wake.notify_all();
	m_writer.join();
}




void Autosave::changed()
{
	// Once the changes pause; while they go on, at the longest a while after the first
	m_quietTimer.start();
	if (!m_longestTimer.isActive()) { m_longestTimer.start(); }
}




void Autosave::saveNow()
{
	takeSnapshot();
}




auto Autosave::canSnapshot() const -> bool
{
	// Not in the middle of something: a project loading or exporting, a plugin waited for, a drag of the mouse
	if (m_song.isLoadingProject() || m_song.isExporting() || RemotePluginBase::isMainThreadWaiting()) { return false; }
	if (qobject_cast<QGuiApplication*>(QCoreApplication::instance()) && QGuiApplication::mouseButtons() != Qt::NoButton)
	{
		return false;
	}
	return true;
}




void Autosave::takeSnapshot()
{
	if (!canSnapshot())
	{
		m_quietTimer.start(); // again after a pause
		return;
	}
	m_quietTimer.stop();
	m_longestTimer.stop();

	auto xml = QString{};
	{
		auto stream = QTextStream{&xml};
		m_song.serializeProject().write(stream);
	}
	{
		const auto lock = std::lock_guard{m_mutex};
		m_pending = xml.toUtf8();
	}
	m_wake.notify_all();
}




void Autosave::flush()
{
	auto lock = std::unique_lock{m_mutex};
	m_idle.wait(lock, [this] { return !m_pending && !m_writing; });
}




void Autosave::writerLoop()
{
	while (true)
	{
		auto bytes = QByteArray{};
		{
			auto lock = std::unique_lock{m_mutex};
			m_wake.wait(lock, [this] { return m_quit || m_pending; });
			if (!m_pending)
			{
				m_idle.notify_all();
				return; // quitting with nothing left
			}
			bytes = std::move(*m_pending);
			m_pending.reset();
			m_writing = true;
		}

		// Atomically: written to a temporary file, renamed over the last snapshot
		QDir{}.mkpath(QFileInfo{m_path}.absolutePath());
		auto file = QSaveFile{m_path};
		const bool written = file.open(QIODevice::WriteOnly) && file.write(bytes) == bytes.size() && file.commit();
		const auto error = written ? QString{} : file.errorString();

		{
			const auto lock = std::lock_guard{m_mutex};
			m_writing = false;
		}
		m_idle.notify_all();
		QMetaObject::invokeMethod(this, [this, written, error] {
			if (written) { emit saved(); }
			else { emit failed(error); }
		}, Qt::QueuedConnection);
	}
}

} // namespace lmms
