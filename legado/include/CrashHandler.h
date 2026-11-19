/*
 * CrashHandler.h - writes a report when the program crashes
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

#ifndef LMMS_CRASH_HANDLER_H
#define LMMS_CRASH_HANDLER_H

#include <QString>

#include "lmms_export.h"

namespace lmms::CrashHandler
{

/**
 * Stage 0 (docs/plano-gravacao.md, "Estabilidade"): on a crash (SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGABRT), writes the
 * signal, the program's version and the stack of the crashing thread to @p reportPath, then lets the system end the
 * program as it would have (so that its own crash reporting still works). The handler runs on a stack of its own,
 * so that a stack overflow is reported too, and only calls what is safe in a signal handler: the file is opened
 * when the handler is installed. The autosaved project (recover.mmp) is offered at the next start as before.
 *
 * Does nothing where the platform has no POSIX signals and backtraces.
 */
LMMS_EXPORT void install(const QString& reportPath);

/**
 * @returns the report of a crash of the last run, renamed with the time it was found, or an empty string if the
 *          last run did not crash (a report is only left when it holds a crash)
 */
LMMS_EXPORT auto takeLastReport(const QString& reportPath) -> QString;

} // namespace lmms::CrashHandler

#endif // LMMS_CRASH_HANDLER_H
