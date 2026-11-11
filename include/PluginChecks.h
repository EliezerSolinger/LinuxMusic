/*
 * PluginChecks.h - which thread is which, and checks of the plugin API's threading and state rules
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

#ifndef LMMS_PLUGIN_CHECKS_H
#define LMMS_PLUGIN_CHECKS_H

#include <functional>
#include <string_view>

#include "lmms_export.h"

namespace lmms
{

/**
 * The two threads of the plugin API, as in CLAP:
 *
 * - The main thread is Qt's main thread.
 * - The audio thread is symbolic: whichever thread of the audio engine processes the plugin in a period. Only one
 *   thread at a time is a plugin's audio thread. While the audio engine is paused (see
 *   AudioEngine::requestChangeInModel()), the host may act as a plugin's audio thread with an AudioThreadScope.
 */
namespace ThreadCheck
{

LMMS_EXPORT auto isMainThread() -> bool;
LMMS_EXPORT auto isAudioThread() -> bool;

//! Marks the current thread as an audio thread for its lifetime (or until the scope ends)
class LMMS_EXPORT AudioThreadScope
{
public:
	AudioThreadScope();
	~AudioThreadScope();

	AudioThreadScope(const AudioThreadScope&) = delete;
	auto operator=(const AudioThreadScope&) -> AudioThreadScope& = delete;

private:
	bool m_wasAudioThread;
};

//! Marks the current thread as an audio thread for good (the audio engine's worker threads)
LMMS_EXPORT void markAsAudioThread();

} // namespace ThreadCheck


/**
 * Checks of the plugin API's rules: on which thread and in which state each method may be called.
 *
 * Violations go to a handler. By default, they are checked in debug builds, where the handler aborts; with the
 * environment variable LMMS_PLUGIN_CHECKS=1 they are checked in any build and reported.
 */
namespace PluginChecks
{

using Handler = std::function<void(std::string_view message)>;

LMMS_EXPORT auto enabled() -> bool;
LMMS_EXPORT void setEnabled(bool enabled);

//! Replaces the handler (e.g. in tests); an empty handler restores the default one
LMMS_EXPORT void setViolationHandler(Handler handler);

//! Reports a violation if checks are enabled
LMMS_EXPORT void violation(std::string_view message);

//! Reports @p message unless @p condition holds
inline void require(bool condition, std::string_view message)
{
	if (!condition && enabled()) { violation(message); }
}

} // namespace PluginChecks

} // namespace lmms

#endif // LMMS_PLUGIN_CHECKS_H
