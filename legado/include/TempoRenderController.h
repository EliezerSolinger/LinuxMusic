/*
 * TempoRenderController.h - renders the recordings again when the tempo changes, asking first
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

#ifndef LMMS_GUI_TEMPO_RENDER_CONTROLLER_H
#define LMMS_GUI_TEMPO_RENDER_CONTROLLER_H

#include <vector>

#include <QObject>
#include <QTimer>

#include "Song.h"
#include "TempoMap.h"

class QWidget;

namespace lmms::gui
{

/**
 * The processing screen of a tempo change (docs/plano-gravacao.md, "Time-stretch renderizado").
 *
 * When the tempo or the tempo map changes, or a recording moves, changes take or is restored by undo, and some
 * recordings no longer play at the timing the song asks for (Song::stretchJobs()):
 *   1. a modal says how many items will be rendered and about how long it takes, with Render and Cancel. Cancel
 *      after a change of tempo puts the tempo (and the map) back to the last one confirmed;
 *   2. a thread renders them (TimeStretcher), one by one, from their originals, while the modal shows each item's
 *      progress and can cancel (which also puts the tempo back);
 *   3. only when all are done do the items play their renderings: a cancelled or failed batch changes nothing and
 *      leaves no file.
 *
 * The checks wait for the edits to settle (dragging the tempo makes one check, at the end) and for a recording or
 * an export to finish.
 */
class TempoRenderController : public QObject
{
	Q_OBJECT
public:
	explicit TempoRenderController(QWidget* parent);

	//! Checks now (normally a check waits for the edits to settle)
	void checkNow();

private:
	void tempoEdited();
	void scheduleCheck();
	void check();
	//! The tempo and the map now are the ones the recordings play at
	void confirm();
	//! Puts the tempo and the map back to the last confirmed ones
	void revert();
	//! Renders the jobs with the progress modal; @returns whether all of them were rendered and are played
	auto render(const std::vector<Song::StretchJob>& jobs) -> bool;

	QWidget* m_parent;
	QTimer m_settle;
	bool m_tempoEdited = false; //!< a change of tempo is among what is being checked (Cancel puts it back)
	bool m_busy = false;
	bool m_reverting = false;
	int m_confirmedTempo = 0;
	std::vector<TempoMap::Marker> m_confirmedMarkers;
};

} // namespace lmms::gui

#endif // LMMS_GUI_TEMPO_RENDER_CONTROLLER_H
