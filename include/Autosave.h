/*
 * Autosave.h
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

#ifndef LMMS_AUTOSAVE_H
#define LMMS_AUTOSAVE_H

#include <condition_variable>
#include <mutex>
#include <optional>
#include <thread>

#include <QByteArray>
#include <QObject>
#include <QString>
#include <QTimer>

#include "lmms_export.h"

namespace lmms
{

class Song;

/**
 * Keeps the project for a recovery: a snapshot after each change, once the changes pause for a moment (or at the
 * longest after a while of changes without pause), serialized on the main thread and written by a thread of its
 * own, atomically (a temporary file renamed over the last one), so that a crash leaves a whole snapshot.
 *
 * A recording in progress is in the snapshot: its takes are recovered from their files (see
 * Song::recoverInterruptedTakes()).
 */
class LMMS_EXPORT Autosave : public QObject
{
	Q_OBJECT
public:
	//! The pause after a change before its snapshot
	static constexpr int QuietMs = 2000;
	//! The longest changes without a pause wait for a snapshot
	static constexpr int LongestWaitMs = 30000;

	//! @p quietMs and @p longestWaitMs: QuietMs and LongestWaitMs, but for tests
	Autosave(Song& song, QString path, QObject* parent = nullptr, int quietMs = QuietMs,
		int longestWaitMs = LongestWaitMs);
	//! Writes what is pending, and stops its thread
	~Autosave() override;

	auto path() const -> const QString& { return m_path; }

	//! Takes a snapshot now, if the project can be saved now (else a little later), and writes it [main thread]
	void saveNow();
	//! Waits until the snapshots taken are written
	void flush();

signals:
	//! A snapshot was written (on the main thread)
	void saved();
	void failed(const QString& why);

private:
	void changed();
	void takeSnapshot();
	auto canSnapshot() const -> bool;
	void writerLoop();

	Song& m_song;
	const QString m_path;
	QTimer m_quietTimer;
	QTimer m_longestTimer;

	std::mutex m_mutex;
	std::condition_variable m_wake;
	std::condition_variable m_idle;
	std::optional<QByteArray> m_pending; //!< the latest snapshot not written yet (an earlier one is replaced)
	bool m_writing = false;
	bool m_quit = false;
	std::thread m_writer;
};

} // namespace lmms

#endif // LMMS_AUTOSAVE_H
