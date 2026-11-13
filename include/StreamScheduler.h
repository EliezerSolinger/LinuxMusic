/*
 * StreamScheduler.h
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

#ifndef LMMS_STREAM_SCHEDULER_H
#define LMMS_STREAM_SCHEDULER_H

#include <vector>

#include <QObject>
#include <QTimer>

#include "AudioStream.h"
#include "lmms_export.h"

namespace lmms
{

class Song;

/**
 * Tells the disk thread (DiskReader) what is about to play: from the song's position, the next
 * DiskReader::ReadAheadSeconds of every clip streamed from disk (and, near the end of the cycle, its start), nearest
 * first. It looks at the song from the main thread, a few times a second, so the disk thread never touches the song.
 */
class LMMS_EXPORT StreamScheduler : public QObject
{
	Q_OBJECT
public:
	explicit StreamScheduler(Song* song);

	//! What is about to play from @p tick on, for @p seconds (main thread)
	static auto demandAt(const Song& song, tick_t tick, double seconds) -> std::vector<DiskReader::Demand>;

	//! Tells the disk thread now
	void update();

private:
	Song* m_song;
	QTimer m_timer;
};

} // namespace lmms

#endif // LMMS_STREAM_SCHEDULER_H
