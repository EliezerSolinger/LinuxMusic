/*
 * UserNoticeTest.cpp
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

#include <QObject>
#include <QScopeGuard>
#include <QtTest>

#include <thread>
#include <vector>

#include "UserNotice.h"

using namespace lmms;

class UserNoticeTest : public QObject
{
	Q_OBJECT

private slots:
	void cleanup()
	{
		UserNotice::setPresenter({}, {});
		UserNotice::setProgressViewFactory({});
	}

	//! Without a GUI, a notice goes to the log and a question gets the answer given for that case
	void withoutUserNoticesAreLoggedAndQuestionsGetTheirDefault()
	{
		QVERIFY(!UserNotice::hasUser());
		UserNotice::warning("Title", "Logged, not shown");
		QCOMPARE(UserNotice::ask("Title", "Load it?", false), false);
		QCOMPARE(UserNotice::ask("Title", "Load it?", true), true);
	}

	void aNoticeOnTheMainThreadIsShownAtOnce()
	{
		auto shown = std::vector<UserNotice::Notice>{};
		UserNotice::setPresenter([&](const UserNotice::Notice& notice) { shown.push_back(notice); }, {});
		QVERIFY(UserNotice::hasUser());
		UserNotice::error("Could not open file", "No permission");
		QCOMPARE(shown.size(), std::size_t{1});
		QCOMPARE(shown[0].level, UserNotice::Level::Error);
		QCOMPARE(shown[0].title, QString{"Could not open file"});
		QCOMPARE(shown[0].text, QString{"No permission"});
	}

	//! From another thread, the notice reaches the presenter on the main thread, when it handles its events
	void aNoticeFromAnotherThreadIsShownOnTheMainThread()
	{
		auto shownOn = std::vector<QThread*>{};
		UserNotice::setPresenter([&](const UserNotice::Notice&) { shownOn.push_back(QThread::currentThread()); }, {});
		auto worker = std::thread{[] { UserNotice::information("From a worker", "Text"); }};
		worker.join();
		QVERIFY(shownOn.empty());
		QTRY_COMPARE(shownOn.size(), std::size_t{1});
		QCOMPARE(shownOn[0], QCoreApplication::instance()->thread());
	}

	//! A presenter removed before the main thread gets to a notice is not called: the notice goes to the log
	void aNoticePendingWhenThePresenterGoesIsLogged()
	{
		auto calls = 0;
		UserNotice::setPresenter([&](const UserNotice::Notice&) { ++calls; }, {});
		auto worker = std::thread{[] { UserNotice::information("From a worker", "Text"); }};
		worker.join();
		UserNotice::setPresenter({}, {});
		QCoreApplication::processEvents();
		QCOMPARE(calls, 0);
	}

	void aQuestionOnTheMainThreadIsAsked()
	{
		auto asked = QString{};
		UserNotice::setPresenter([](const UserNotice::Notice&) {},
			[&](const UserNotice::Notice& notice) { asked = notice.text; return true; });
		QCOMPARE(UserNotice::ask("Plugin", "Load it anyway?", false), true);
		QCOMPARE(asked, QString{"Load it anyway?"});
	}

	//! Off the main thread nobody can answer (a window would open on the wrong thread): the default, at once
	void aQuestionFromAnotherThreadGetsItsDefault()
	{
		auto asked = 0;
		UserNotice::setPresenter([](const UserNotice::Notice&) {}, [&](const UserNotice::Notice&) { ++asked; return true; });
		auto answer = true;
		auto worker = std::thread{[&] { answer = UserNotice::ask("Plugin", "Load it anyway?", false); }};
		worker.join();
		QCOMPARE(answer, false);
		QCOMPARE(asked, 0);
	}

	//! Without a GUI, a progress counts but nobody sees it, and it is never cancelled
	void withoutUserProgressIsNeverCancelled()
	{
		auto progress = UserNotice::Progress{"Please wait...", "Loading project...", 3};
		progress.setValue(2);
		progress.setText("Loading Track 2");
		QCOMPARE(progress.value(), 2);
		QVERIFY(!progress.cancelled());
	}

	//! With a GUI, the view shows what the progress says, says when the user cancelled, and goes with it
	void aProgressIsShownWhileItLives()
	{
		struct Seen
		{
			QString title, text;
			int maximum = 0, value = -1;
			bool cancel = false, gone = false;
		} seen;
		struct View : UserNotice::ProgressView
		{
			Seen& seen;
			explicit View(Seen& seen) : seen{seen} {}
			~View() override { seen.gone = true; }
			void setMaximum(int maximum) override { seen.maximum = maximum; }
			void setValue(int value) override { seen.value = value; }
			void setText(const QString& text) override { seen.text = text; }
			auto cancelled() -> bool override { return seen.cancel; }
		};
		UserNotice::setProgressViewFactory([&](const QString& title, const QString& text, int maximum) {
			seen.title = title;
			seen.text = text;
			seen.maximum = maximum;
			return std::make_unique<View>(seen);
		});
		{
			auto progress = UserNotice::Progress{"Please wait...", "Importing MIDI-file...", 2};
			QCOMPARE(seen.title, QString{"Please wait..."});
			QCOMPARE(seen.text, QString{"Importing MIDI-file..."});
			QCOMPARE(seen.maximum, 2);
			progress.setMaximum(10);
			progress.setValue(4);
			progress.setText("Track 3");
			QCOMPARE(seen.maximum, 10);
			QCOMPARE(seen.value, 4);
			QCOMPARE(seen.text, QString{"Track 3"});
			QVERIFY(!progress.cancelled());
			seen.cancel = true;
			QVERIFY(progress.cancelled());
			QVERIFY(!seen.gone);
		}
		QVERIFY(seen.gone);
	}

	//! Off the main thread no window can open: nobody sees the progress
	void aProgressOffTheMainThreadIsNotShown()
	{
		auto made = 0;
		UserNotice::setProgressViewFactory([&](const QString&, const QString&, int) {
			++made;
			return std::unique_ptr<UserNotice::ProgressView>{};
		});
		auto worker = std::thread{[] {
			auto progress = UserNotice::Progress{"Please wait...", "Loading", 1};
			progress.setValue(1);
		}};
		worker.join();
		QCOMPARE(made, 0);
	}
};

QTEST_GUILESS_MAIN(UserNoticeTest)
#include "UserNoticeTest.moc"
