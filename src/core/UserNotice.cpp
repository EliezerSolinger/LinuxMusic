/*
 * UserNotice.cpp
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

#include "UserNotice.h"
#include "ErrorLog.h"

#include <QCoreApplication>
#include <QDebug>
#include <QMetaObject>
#include <QThread>

#include <mutex>

namespace lmms::UserNotice
{

namespace
{

std::mutex s_mutex;
Presenter s_presenter;
Asker s_asker;
ProgressViewFactory s_progressViewFactory;

auto onMainThread() -> bool
{
	const auto* app = QCoreApplication::instance();
	return app && QThread::currentThread() == app->thread();
}

void log(const Notice& notice)
{
	const auto line = notice.title.isEmpty() ? notice.text : notice.title + ": " + notice.text;
	switch (notice.level)
	{
	case Level::Hint:
	case Level::Information: qInfo().noquote() << line; break;
	// (warnings and errors into the error log, which also prints them)
	case Level::Warning:
		ErrorLog::reportText(ErrorLog::Severity::Warning, "UserNotice", line);
		break;
	case Level::Error:
		ErrorLog::reportText(ErrorLog::Severity::Error, "UserNotice", line);
		break;
	}
}

//! On the main thread: the presenter installed now, or the log
void present(const Notice& notice)
{
	auto presenter = Presenter{};
	{
		const auto lock = std::lock_guard{s_mutex};
		presenter = s_presenter;
	}
	if (presenter) { presenter(notice); }
}

} // namespace


void notify(Level level, const QString& title, const QString& text)
{
	auto notice = Notice{level, title, text};
	// (always in the log too: what the user was told is then there when something goes wrong)
	log(notice);
	if (!hasUser()) { return; }
	if (onMainThread()) { present(notice); return; }
	// To the main thread; the presenter is looked up there, so that one removed meanwhile is not called
	QMetaObject::invokeMethod(QCoreApplication::instance(), [notice = std::move(notice)] { present(notice); },
		Qt::QueuedConnection);
}


auto ask(const QString& title, const QString& text, bool answerWithoutUser) -> bool
{
	auto asker = Asker{};
	{
		const auto lock = std::lock_guard{s_mutex};
		asker = s_asker;
	}
	if (!asker || !onMainThread()) { return answerWithoutUser; }
	return asker(Notice{Level::Information, title, text});
}


auto hasUser() -> bool
{
	const auto lock = std::lock_guard{s_mutex};
	return static_cast<bool>(s_presenter);
}


void setPresenter(Presenter presenter, Asker asker)
{
	const auto lock = std::lock_guard{s_mutex};
	s_presenter = std::move(presenter);
	s_asker = std::move(asker);
}


void setProgressViewFactory(ProgressViewFactory factory)
{
	const auto lock = std::lock_guard{s_mutex};
	s_progressViewFactory = std::move(factory);
}


Progress::Progress(const QString& title, const QString& text, int maximum)
{
	auto factory = ProgressViewFactory{};
	{
		const auto lock = std::lock_guard{s_mutex};
		factory = s_progressViewFactory;
	}
	if (factory && onMainThread()) { m_view = factory(title, text, maximum); }
}


Progress::~Progress() = default;


void Progress::setMaximum(int maximum)
{
	if (m_view) { m_view->setMaximum(maximum); }
}


void Progress::setValue(int value)
{
	m_value = value;
	if (m_view) { m_view->setValue(value); }
}


void Progress::setText(const QString& text)
{
	if (m_view) { m_view->setText(text); }
}


auto Progress::cancelled() -> bool
{
	return m_view && m_view->cancelled();
}

} // namespace lmms::UserNotice
