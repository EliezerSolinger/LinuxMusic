/*
 * ErrorLog.h
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

#ifndef LMMS_ERROR_LOG_H
#define LMMS_ERROR_LOG_H

#include <QString>

#include <cstdint>

#include "lmms_export.h"

namespace lmms::ErrorLog
{

/**
 * The program's error log, as audio engines keep one: what went wrong without stopping anything (a damaged file
 * read as far as it could be, a resampling that failed and played silence, a plugin's message dropped) is written
 * to <working directory>/logs/errors.log and to the standard error, with the time, where and what.
 *
 * report() may be called from any thread, the audio threads too: it takes no lock and allocates nothing, so its
 * texts must be string literals (or live as long as the program); a value (an error code, a size, a ratio) goes
 * with them. A thread of the log's own writes the entries out a little later; the same entry repeated at once is
 * written once, with how many times. If the audio threads report faster than it writes, the extra entries are
 * counted and the count is written.
 */

enum class Severity
{
	Warning,
	Error
};

//! Reports from any thread, the audio threads too (see above); @p where and @p what must outlive the program
LMMS_EXPORT void report(Severity severity, const char* where, const char* what) noexcept;
LMMS_EXPORT void report(Severity severity, const char* where, const char* what, double value) noexcept;

//! Reports a text made at the moment (not from the audio threads: it allocates)
LMMS_EXPORT void reportText(Severity severity, const char* where, const QString& what);

//! Starts writing the entries to @p file (and to the standard error), from a thread of its own; before, they wait
LMMS_EXPORT void start(const QString& file);
//! Writes what is left and stops (the entries reported afterwards wait for a start())
LMMS_EXPORT void stop();
//! Writes the entries reported so far now (the log's thread does it by itself every 200 ms)
LMMS_EXPORT void flush();

//! The entries reported since the program started, and those lost because the audio threads reported too fast
LMMS_EXPORT auto reportedCount() noexcept -> std::uint64_t;
LMMS_EXPORT auto droppedCount() noexcept -> std::uint64_t;

//! The log file of the working directory @p workingDir
LMMS_EXPORT auto fileIn(const QString& workingDir) -> QString;

} // namespace lmms::ErrorLog

#endif // LMMS_ERROR_LOG_H
