/*
 * UndoCommand.h
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

#ifndef LMMS_UNDO_COMMAND_H
#define LMMS_UNDO_COMMAND_H

#include <QString>

#include "lmms_export.h"

namespace lmms
{

/**
 * A step of the undo history that keeps only what changed (docs/plano-gravacao.md, "Desfazer por comandos"), for the
 * new parts of the program (the audio track and its items), where the old journal keeps whole states.
 *
 * A command is made, applied once (ProjectJournal::execute()), and then reverted and applied again as the user undoes
 * and redoes. It lives in the same history as the old checkpoints: a step of it, or part of a group. It must not
 * hold pointers to objects that may be deleted and made again: it finds them by the journal's ids, and does nothing
 * when they are gone.
 */
class LMMS_EXPORT UndoCommand
{
public:
	virtual ~UndoCommand() = default;

	//! Does (and redoes) the change
	virtual void apply() = 0;
	//! Undoes it
	virtual void revert() = 0;
	//! What it does, for the menus ("Move item")
	virtual auto text() const -> QString { return {}; }
};

} // namespace lmms

#endif // LMMS_UNDO_COMMAND_H
