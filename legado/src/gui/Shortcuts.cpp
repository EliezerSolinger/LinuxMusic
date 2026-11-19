/*
 * Shortcuts.cpp - the keyboard shortcuts in one place, each changeable in the settings
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

#include "Shortcuts.h"

#include <cstring>

#include <QCoreApplication>
#include <QKeyEvent>

#include "ConfigManager.h"

namespace lmms::gui::Shortcuts
{

auto actions() -> const std::vector<Action>&
{
	static const auto list = std::vector<Action>{
		{"record", QT_TRANSLATE_NOOP("Shortcuts", "Record"), QKeySequence{Qt::Key_R}},
		{"cycle", QT_TRANSLATE_NOOP("Shortcuts", "Cycle on or off"), QKeySequence{Qt::Key_C}},
		{"split", QT_TRANSLATE_NOOP("Shortcuts", "Split the clips at the playhead"), QKeySequence{Qt::Key_S}},
		{"marker", QT_TRANSLATE_NOOP("Shortcuts", "Add a marker at the playhead"), QKeySequence{Qt::Key_M}},
		{"region", QT_TRANSLATE_NOOP("Shortcuts", "Add a region over the cycle"), QKeySequence{Qt::SHIFT | Qt::Key_R}},
		{"copy", QT_TRANSLATE_NOOP("Shortcuts", "Copy the selected clips"), QKeySequence{QKeySequence::Copy}},
		{"cut", QT_TRANSLATE_NOOP("Shortcuts", "Cut the selected clips"), QKeySequence{QKeySequence::Cut}},
		{"paste", QT_TRANSLATE_NOOP("Shortcuts", "Paste at the playhead"), QKeySequence{QKeySequence::Paste}},
		{"duplicate", QT_TRANSLATE_NOOP("Shortcuts", "Duplicate the selected clips"),
			QKeySequence{Qt::CTRL | Qt::Key_D}},
	};
	return list;
}


auto text(const Action& action) -> QString
{
	return QCoreApplication::translate("Shortcuts", action.text);
}


auto key(const char* id) -> QKeySequence
{
	for (const auto& action : actions())
	{
		if (std::strcmp(action.id, id) != 0) { continue; }
		const auto saved = ConfigManager::inst()->value("shortcuts", id);
		return saved.isEmpty() ? action.fallback : QKeySequence::fromString(saved, QKeySequence::PortableText);
	}
	return {};
}


void setKey(const char* id, const QKeySequence& sequence)
{
	for (const auto& action : actions())
	{
		if (std::strcmp(action.id, id) != 0) { continue; }
		ConfigManager::inst()->setValue("shortcuts", id,
			sequence == action.fallback ? QString{} : sequence.toString(QKeySequence::PortableText));
	}
}


auto matches(const char* id, const QKeyEvent* event) -> bool
{
	const auto sequence = key(id);
	if (sequence.isEmpty()) { return false; }
	// One chord: the key with its modifiers (the keypad's is the same key)
	const auto pressed = QKeyCombination{event->modifiers() & ~Qt::KeypadModifier, static_cast<Qt::Key>(event->key())};
	return sequence[0] == pressed;
}

} // namespace lmms::gui::Shortcuts
