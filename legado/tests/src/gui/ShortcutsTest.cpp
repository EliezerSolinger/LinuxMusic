/*
 * ShortcutsTest.cpp
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

#include <QKeyEvent>
#include <QObject>
#include <QtTest>

#include "ConfigManager.h"
#include "Shortcuts.h"

using namespace lmms;
using namespace lmms::gui;

class ShortcutsTest : public QObject
{
	Q_OBJECT

private slots:
	void initTestCase()
	{
		ConfigManager::inst();
	}

	void defaultsChangeAndComeBack()
	{
		QCOMPARE(Shortcuts::key("record"), QKeySequence{Qt::Key_R});
		const auto r = QKeyEvent{QEvent::KeyPress, Qt::Key_R, Qt::NoModifier};
		const auto shiftR = QKeyEvent{QEvent::KeyPress, Qt::Key_R, Qt::ShiftModifier};
		QVERIFY(Shortcuts::matches("record", &r));
		QVERIFY(!Shortcuts::matches("record", &shiftR));
		QVERIFY(!Shortcuts::matches("cycle", &r));

		// Changed: the new key, kept in the configuration
		Shortcuts::setKey("record", QKeySequence{Qt::SHIFT | Qt::Key_R});
		QCOMPARE(ConfigManager::inst()->value("shortcuts", "record"), QString{"Shift+R"});
		QVERIFY(Shortcuts::matches("record", &shiftR));
		QVERIFY(!Shortcuts::matches("record", &r));

		// Back to the default: nothing kept
		Shortcuts::setKey("record", QKeySequence{Qt::Key_R});
		QVERIFY(ConfigManager::inst()->value("shortcuts", "record").isEmpty());
		QVERIFY(Shortcuts::matches("record", &r));

		// Unknown actions match nothing; every action has a text and a default
		QVERIFY(!Shortcuts::matches("nothing", &r));
		for (const auto& action : Shortcuts::actions())
		{
			QVERIFY(!Shortcuts::text(action).isEmpty());
			QVERIFY(!action.fallback.isEmpty());
		}
	}
};

QTEST_MAIN(ShortcutsTest)
#include "ShortcutsTest.moc"
