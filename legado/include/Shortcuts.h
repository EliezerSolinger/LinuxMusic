/*
 * Shortcuts.h - the keyboard shortcuts in one place, each changeable in the settings
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

#ifndef LMMS_GUI_SHORTCUTS_H
#define LMMS_GUI_SHORTCUTS_H

#include <vector>

#include <QKeySequence>
#include <QString>

#include "lmms_export.h"

class QKeyEvent;

namespace lmms::gui
{

/**
 * The keyboard shortcuts of the transport and the editors (docs/plano-anotacoes-13-11.md, 9.6), with their defaults
 * after REAPER and Logic, each changeable in the settings (saved in the configuration's "shortcuts" section).
 *
 * The editors keep handling their keys in keyPressEvent(), asking matches(): that way what comes first (an
 * instrument window's keyboard playing notes, a text field) still does.
 */
namespace Shortcuts
{

struct Action
{
	const char* id;			//!< its name in the configuration
	const char* text;		//!< what it does, untranslated (translation context "Shortcuts")
	QKeySequence fallback;	//!< its default key
};

//! Every action, in the order the settings show them
LMMS_EXPORT auto actions() -> const std::vector<Action>&;
//! What the action @p id does, translated
LMMS_EXPORT auto text(const Action& action) -> QString;
//! The key of the action @p id now: the user's, else its default
LMMS_EXPORT auto key(const char* id) -> QKeySequence;
//! Sets the action @p id's key (its default again when @p key is the default)
LMMS_EXPORT void setKey(const char* id, const QKeySequence& key);
//! Whether @p event is the action @p id's key
LMMS_EXPORT auto matches(const char* id, const QKeyEvent* event) -> bool;

} // namespace Shortcuts

} // namespace lmms::gui

#endif // LMMS_GUI_SHORTCUTS_H
