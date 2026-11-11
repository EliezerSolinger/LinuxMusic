/*
 * PluginChecks.cpp - which thread is which, and checks of the plugin API's threading and state rules
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

#include "PluginChecks.h"

#include <atomic>
#include <cstdlib>
#include <mutex>
#include <thread>

#include <QCoreApplication>
#include <QDebug>
#include <QThread>

namespace lmms
{

namespace
{

thread_local bool s_audioThread = false;

//! The thread that loaded the program: the main thread, also before and after the QCoreApplication exists
const auto s_startThread = std::this_thread::get_id();

auto checksEnabledByDefault() -> bool
{
	if (const auto env = std::getenv("LMMS_PLUGIN_CHECKS")) { return env[0] == '1'; }
#ifdef NDEBUG
	return false;
#else
	return true;
#endif
}

std::atomic<bool> s_enabled = checksEnabledByDefault();
std::mutex s_handlerMutex;
PluginChecks::Handler s_handler;

} // namespace


auto ThreadCheck::isMainThread() -> bool
{
	const auto app = QCoreApplication::instance();
	return app ? QThread::currentThread() == app->thread() : std::this_thread::get_id() == s_startThread;
}

auto ThreadCheck::isAudioThread() -> bool
{
	return s_audioThread;
}

ThreadCheck::AudioThreadScope::AudioThreadScope()
	: m_wasAudioThread{s_audioThread}
{
	s_audioThread = true;
}

ThreadCheck::AudioThreadScope::~AudioThreadScope()
{
	s_audioThread = m_wasAudioThread;
}

void ThreadCheck::markAsAudioThread()
{
	s_audioThread = true;
}


auto PluginChecks::enabled() -> bool
{
	return s_enabled.load(std::memory_order_relaxed);
}

void PluginChecks::setEnabled(bool enabled)
{
	s_enabled.store(enabled, std::memory_order_relaxed);
}

void PluginChecks::setViolationHandler(Handler handler)
{
	const auto lock = std::lock_guard{s_handlerMutex};
	s_handler = std::move(handler);
}

void PluginChecks::violation(std::string_view message)
{
	{
		const auto lock = std::lock_guard{s_handlerMutex};
		if (s_handler)
		{
			s_handler(message);
			return;
		}
	}

	qCritical().noquote() << "Plugin API violation:" << QString::fromUtf8(message.data(), message.size());
#ifndef NDEBUG
	std::abort();
#endif
}

} // namespace lmms
