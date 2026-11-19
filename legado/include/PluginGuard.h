/*
 * PluginGuard.h
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

#ifndef LMMS_PLUGIN_GUARD_H
#define LMMS_PLUGIN_GUARD_H

#include <QMap>
#include <QString>
#include <QStringList>

#include "lmms_export.h"

namespace lmms
{

class Plugin;

/**
 * The plugins that crashed the program: while a plugin's code runs (loading, processing, restoring its settings),
 * the thread says which plugin it is, and the crash handler writes it in its report. At the next start, that plugin
 * becomes a suspect: it is not loaded again (a project keeps its settings, and says why) until the settings let it.
 */
namespace PluginGuard
{

//! Who a plugin is: its name and, for one of many (a LADSPA or LV2 plugin, a VST file), its key's attributes
LMMS_EXPORT auto identity(const QString& pluginName, const QMap<QString, QString>& attributes) -> QString;
LMMS_EXPORT auto identity(const Plugin& plugin) -> QString;
//! A name to show for an identity
LMMS_EXPORT auto displayName(const QString& identity) -> QString;

LMMS_EXPORT auto suspects() -> QStringList;
LMMS_EXPORT auto isSuspect(const QString& identity) -> bool;
LMMS_EXPORT void addSuspect(const QString& identity);
//! Loads it again from now on
LMMS_EXPORT void clearSuspect(const QString& identity);

//! The line of a crash report that names the plugin running (see the crash handler)
inline constexpr const char* ReportMarker = "Plugin running: ";
//! Makes the plugins a crash report names suspects; @returns their names
LMMS_EXPORT auto takeSuspectsFromReport(const QString& report) -> QStringList;

/**
 * While it lives, the calling thread runs the code of the plugin @p identity (a string that outlives the scope).
 * Scopes nest: the outer plugin is named again when the inner one ends.
 */
class LMMS_EXPORT Running
{
public:
	explicit Running(const char* identity) noexcept;
	~Running() noexcept;
	Running(const Running&) = delete;
	auto operator=(const Running&) -> Running& = delete;

private:
	const char* m_previous;
};

//! The plugin the calling thread runs, or null; safe in a signal handler
LMMS_EXPORT auto running() noexcept -> const char*;

} // namespace PluginGuard

} // namespace lmms

#endif // LMMS_PLUGIN_GUARD_H
