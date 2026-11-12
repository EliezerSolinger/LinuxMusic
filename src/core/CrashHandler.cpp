/*
 * CrashHandler.cpp - writes a report when the program crashes
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

#include "CrashHandler.h"

#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>

#include "lmmsconfig.h"
#include "lmmsversion.h"

#if defined(LMMS_BUILD_LINUX) || defined(LMMS_BUILD_APPLE)
#define LMMS_CRASH_HANDLER_POSIX
#include <csignal>
#include <cstring>
#include <execinfo.h>
#include <fcntl.h>
#include <unistd.h>
#endif

namespace lmms::CrashHandler
{

#ifdef LMMS_CRASH_HANDLER_POSIX

namespace
{

int s_reportFile = -1; //!< opened when installed, written only on a crash
char s_reportPath[4096] = {};
constexpr int Signals[] = {SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGABRT};

void write(const char* text)
{
	// Only async-signal-safe calls here
	if (s_reportFile >= 0) { (void)::write(s_reportFile, text, std::strlen(text)); }
}

void handler(int signal)
{
	if (s_reportFile >= 0)
	{
		write("LinuxMusic " LMMS_VERSION " crashed: ");
		switch (signal)
		{
		case SIGSEGV: write("SIGSEGV (invalid memory access)"); break;
		case SIGBUS: write("SIGBUS (invalid memory access)"); break;
		case SIGILL: write("SIGILL (illegal instruction)"); break;
		case SIGFPE: write("SIGFPE (arithmetic error)"); break;
		case SIGABRT: write("SIGABRT (aborted)"); break;
		default: write("a signal"); break;
		}
		write("\n\nStack of the crashing thread:\n");
		void* frames[128];
		const int count = backtrace(frames, 128);
		backtrace_symbols_fd(frames, count, s_reportFile);
		write("\nThe last autosaved state of the project is offered at the next start.\n");
		fsync(s_reportFile);
		close(s_reportFile);
		s_reportFile = -1;
	}

	// Then end as the signal would have: the handler was reset to the default (SA_RESETHAND)
	raise(signal);
}

} // namespace




void install(const QString& reportPath)
{
	// The sanitizers report crashes themselves, better
#if defined(__SANITIZE_ADDRESS__) || defined(__SANITIZE_THREAD__)
	return;
#elif defined(__has_feature)
#	if __has_feature(address_sanitizer) || __has_feature(thread_sanitizer) || __has_feature(memory_sanitizer)
	return;
#	endif
#endif
	QDir{}.mkpath(QFileInfo{reportPath}.absolutePath());
	const auto path = QFile::encodeName(reportPath);
	std::strncpy(s_reportPath, path.constData(), sizeof(s_reportPath) - 1);

	// The report file is created now (nothing may allocate in the handler) and removed on a clean exit, by
	// takeLastReport() at the next start if it stayed empty
	s_reportFile = open(s_reportPath, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0644);
	if (s_reportFile < 0) { return; }

	// A stack of its own, so that a stack overflow can be reported
	static char alternateStack[256 * 1024];
	auto stack = stack_t{};
	stack.ss_sp = alternateStack;
	stack.ss_size = sizeof(alternateStack);
	sigaltstack(&stack, nullptr);

	// Load the unwinder now: its first use may allocate
	void* warmUp[1];
	backtrace(warmUp, 1);

	struct sigaction action = {};
	action.sa_handler = handler;
	action.sa_flags = SA_ONSTACK | SA_RESETHAND;
	sigemptyset(&action.sa_mask);
	for (const auto signal : Signals) { sigaction(signal, &action, nullptr); }
}

#else

void install(const QString&)
{
}

#endif




auto takeLastReport(const QString& reportPath) -> QString
{
	const auto report = QFileInfo{reportPath};
	if (!report.exists()) { return {}; }
	if (report.size() == 0)
	{
		QFile::remove(reportPath);
		return {};
	}

	// Kept with the time it was found, so that the next run starts with an empty report
	const auto kept = report.absolutePath() + '/' + report.completeBaseName() + '-'
		+ QDateTime::currentDateTime().toString("yyyyMMdd-HHmmss") + '.' + report.suffix();
	return QFile::rename(reportPath, kept) ? kept : reportPath;
}

} // namespace lmms::CrashHandler
