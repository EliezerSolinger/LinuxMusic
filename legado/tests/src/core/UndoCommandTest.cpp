/*
 * UndoCommandTest.cpp
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


#include <memory>

#include <QObject>
#include <QtTest>

#include "AudioEngine.h"
#include "Engine.h"
#include "ProjectJournal.h"
#include "Song.h"
#include "UndoCommand.h"

using namespace lmms;

namespace
{

int s_value = 0;

//! Adds to a number (only the difference is kept)
class AddCommand : public UndoCommand
{
public:
	explicit AddCommand(int amount) : m_amount{amount} {}
	void apply() override
	{
		// (nothing a command changes is journalled again)
		QVERIFY(!Engine::projectJournal()->isJournalling());
		s_value += m_amount;
	}
	void revert() override { s_value -= m_amount; }

private:
	int m_amount;
};

} // namespace

class UndoCommandTest : public QObject
{
	Q_OBJECT
private slots:
	void initTestCase()
	{
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	void init()
	{
		Engine::getSong()->createNewProject();
		Engine::projectJournal()->clearJournal();
		Engine::projectJournal()->setJournalling(true);
		s_value = 0;
	}

	void aCommandIsUndoneAndRedone()
	{
		auto* journal = Engine::projectJournal();
		journal->execute(std::make_unique<AddCommand>(5));
		journal->execute(std::make_unique<AddCommand>(2));
		QCOMPARE(s_value, 7);
		QVERIFY(journal->isJournalling());
		journal->undo();
		QCOMPARE(s_value, 5);
		journal->undo();
		QCOMPARE(s_value, 0);
		QVERIFY(!journal->canUndo());
		journal->redo();
		QCOMPARE(s_value, 5);
		// A new step drops what could be redone
		journal->execute(std::make_unique<AddCommand>(10));
		QVERIFY(!journal->canRedo());
		QCOMPARE(s_value, 15);
	}

	void commandsAndCheckpointsShareOneHistory()
	{
		auto* journal = Engine::projectJournal();
		auto* song = Engine::getSong();
		auto& tempo = song->tempoModel();
		const auto before = tempo.value();

		tempo.setValue(before + 10); // a checkpoint (the model's state)
		journal->execute(std::make_unique<AddCommand>(3));
		{
			// One step of both kinds
			const auto group = ProjectJournal::Group{journal};
			tempo.setValue(before + 20);
			journal->execute(std::make_unique<AddCommand>(4));
		}
		QCOMPARE(s_value, 7);
		journal->undo(); // the group
		QCOMPARE(s_value, 3);
		QCOMPARE(tempo.value(), before + 10);
		journal->undo(); // the command
		QCOMPARE(s_value, 0);
		QCOMPARE(tempo.value(), before + 10);
		journal->undo(); // the checkpoint
		QCOMPARE(tempo.value(), before);
		journal->redo();
		journal->redo();
		journal->redo();
		QCOMPARE(s_value, 7);
		QCOMPARE(tempo.value(), before + 20);
	}

	void withoutJournallingACommandIsOnlyApplied()
	{
		auto* journal = Engine::projectJournal();
		journal->setJournalling(false);
		journal->execute(std::make_unique<AddCommand>(1));
		QCOMPARE(s_value, 1);
		QVERIFY(!journal->canUndo());
		journal->setJournalling(true);
	}
};

QTEST_GUILESS_MAIN(UndoCommandTest)
#include "UndoCommandTest.moc"
