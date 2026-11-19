/*
 * PluginGuard.cpp
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

#include "PluginGuard.h"

#include "ConfigManager.h"
#include "Plugin.h"

namespace lmms::PluginGuard
{

namespace
{

constexpr auto Section = "plugins";
constexpr auto Key = "suspects";
constexpr auto Separator = '\n';

thread_local const char* s_running = nullptr;

void store(const QStringList& list)
{
	ConfigManager::inst()->setValue(Section, Key, list.join(Separator));
}

} // namespace


auto identity(const QString& pluginName, const QMap<QString, QString>& attributes) -> QString
{
	auto result = pluginName;
	// The attributes that tell which one (a QMap keeps them in order of their names)
	for (auto it = attributes.begin(); it != attributes.end(); ++it)
	{
		// (how it runs is not which one it is: a plugin isolated or not is the same plugin)
		if (it.key() == "isolated") { continue; }
		result += '|' + it.key() + '=' + it.value();
	}
	return result;
}


auto identity(const Plugin& plugin) -> QString
{
	const auto* descriptor = plugin.descriptor();
	return identity(descriptor ? QString{descriptor->name} : QString{}, plugin.key().attributes);
}


auto displayName(const QString& identity) -> QString
{
	const auto parts = identity.split('|');
	auto name = parts.value(0);
	for (const auto& part : parts.mid(1))
	{
		const auto value = part.section('=', 1);
		if (!value.isEmpty()) { name += " (" + value + ")"; }
	}
	return name;
}


auto suspects() -> QStringList
{
	return ConfigManager::inst()->value(Section, Key).split(Separator, Qt::SkipEmptyParts);
}


auto isSuspect(const QString& identity) -> bool
{
	return suspects().contains(identity);
}


void addSuspect(const QString& identity)
{
	auto list = suspects();
	if (!list.contains(identity))
	{
		list << identity;
		store(list);
	}
}


void clearSuspect(const QString& identity)
{
	auto list = suspects();
	if (list.removeAll(identity) > 0) { store(list); }
}


auto takeSuspectsFromReport(const QString& report) -> QStringList
{
	auto names = QStringList{};
	for (const auto& line : report.split('\n'))
	{
		if (!line.startsWith(ReportMarker)) { continue; }
		const auto identity = line.mid(static_cast<int>(qstrlen(ReportMarker))).trimmed();
		if (identity.isEmpty()) { continue; }
		addSuspect(identity);
		if (!names.contains(displayName(identity))) { names << displayName(identity); }
	}
	return names;
}


Running::Running(const char* identity) noexcept
	: m_previous{s_running}
{
	s_running = identity;
}


Running::~Running() noexcept
{
	s_running = m_previous;
}


auto running() noexcept -> const char*
{
	return s_running;
}

} // namespace lmms::PluginGuard
