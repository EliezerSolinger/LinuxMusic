/*
 * UserNotice.h
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

#ifndef LMMS_USER_NOTICE_H
#define LMMS_USER_NOTICE_H

#include <QString>

#include <functional>
#include <memory>

#include "lmms_export.h"

namespace lmms
{

/**
 * What the core tells the user: a file it could not open, a plugin it could not load, a device that went away. The
 * core never opens a window itself (it runs without one when rendering, importing or exporting from the command
 * line, and in the tests): it hands the notice here, and the GUI, when there is one, installs who shows it.
 * A notice always goes to the log as well; without a GUI, a question gets the answer given for that case.
 *
 * Any thread may notify (not the audio thread's processing, which must not allocate): the notice reaches the
 * presenter on the main thread. A question is answered on the main thread only; asked from another thread, or with
 * no presenter, it gets the answer given for when nobody can answer.
 */
namespace UserNotice
{

enum class Level
{
	Hint, //!< In passing, without stopping the user (the GUI shows it for a few seconds)
	Information,
	Warning,
	Error
};

struct Notice
{
	Level level = Level::Information;
	QString title;
	QString text;
};

//! Shows a notice to the user (runs on the main thread)
using Presenter = std::function<void(const Notice&)>;
//! Asks the user a yes/no question; @returns the answer (runs on the main thread)
using Asker = std::function<bool(const Notice&)>;

//! Tells the user @p text; returns at once (from another thread, the notice is shown when the main thread can)
LMMS_EXPORT void notify(Level level, const QString& title, const QString& text);
inline void hint(const QString& title, const QString& text) { notify(Level::Hint, title, text); }
inline void information(const QString& title, const QString& text) { notify(Level::Information, title, text); }
inline void warning(const QString& title, const QString& text) { notify(Level::Warning, title, text); }
inline void error(const QString& title, const QString& text) { notify(Level::Error, title, text); }

/**
 * Asks the user a yes/no question and waits for the answer; @returns @p answerWithoutUser at once when nobody can
 * answer (no GUI, or asked off the main thread)
 */
LMMS_EXPORT auto ask(const QString& title, const QString& text, bool answerWithoutUser) -> bool;

//! Whether a user can see notices and answer questions (a presenter is installed)
LMMS_EXPORT auto hasUser() -> bool;

//! Installs who shows notices and asks questions (the GUI, at its start); empty ones go back to the log
LMMS_EXPORT void setPresenter(Presenter presenter, Asker asker);


//! What shows a Progress (the GUI's dialog)
class ProgressView
{
public:
	virtual ~ProgressView() = default;
	virtual void setMaximum(int maximum) = 0;
	virtual void setValue(int value) = 0;
	virtual void setText(const QString& text) = 0;
	//! Whether the user cancelled; lets the GUI handle its events meanwhile
	virtual auto cancelled() -> bool = 0;
};

//! Makes the view of a progress (runs on the main thread)
using ProgressViewFactory
	= std::function<std::unique_ptr<ProgressView>(const QString& title, const QString& text, int maximum)>;

//! Installs who shows progress (the GUI, at its start); an empty one shows none
LMMS_EXPORT void setProgressViewFactory(ProgressViewFactory factory);

/**
 * A long task's progress (loading a project, importing a file), shown to the user while it lives, with a way to
 * cancel. Without a GUI, or off the main thread, nobody sees it and it is never cancelled.
 */
class LMMS_EXPORT Progress
{
public:
	Progress(const QString& title, const QString& text, int maximum);
	~Progress();
	Progress(const Progress&) = delete;
	auto operator=(const Progress&) -> Progress& = delete;

	void setMaximum(int maximum);
	void setValue(int value);
	auto value() const -> int { return m_value; }
	void setText(const QString& text);
	//! Whether the user cancelled the task
	auto cancelled() -> bool;

private:
	std::unique_ptr<ProgressView> m_view;
	int m_value = 0;
};

} // namespace UserNotice

} // namespace lmms

#endif // LMMS_USER_NOTICE_H
