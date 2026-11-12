/*
 * TestEnvironment.cpp - keeps the tests away from the user's configuration and folders
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

#include <QTemporaryDir>
#include <QtGlobal>

#include "ConfigManager.h"

namespace
{

/**
 * Linked into every test program: before main(), points the configuration at a fresh directory of its own, which is
 * removed at exit. Without it, the tests read and overwrote the build's configuration file, and scanned the user's
 * working directory (in Documents, where on macOS each newly built test program waited for the privacy prompt).
 */
struct IsolatedConfiguration
{
	IsolatedConfiguration()
	{
		if (!directory.isValid()) { qFatal("Could not create a directory for the tests' configuration"); }
		qputenv(lmms::IsolatedConfigVariable, directory.path().toLocal8Bit());
	}

	QTemporaryDir directory;
};

const IsolatedConfiguration s_isolatedConfiguration;

} // namespace
