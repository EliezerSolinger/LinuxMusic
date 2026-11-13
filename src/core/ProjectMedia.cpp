/*
 * ProjectMedia.cpp - keeps the files a project uses in the project's folder
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

#include "ProjectMedia.h"

#include <map>

#include <QCoreApplication>
#include <QCryptographicHash>
#include <QDir>
#include <QFile>
#include <QFileInfo>

#include "ConfigManager.h"
#include "DataFile.h"
#include "PathUtil.h"

namespace lmms
{

namespace
{

auto tr(const char* text) -> QString
{
	return QCoreApplication::translate("ProjectMedia", text);
}

auto contentsHash(const QString& path) -> QByteArray
{
	auto file = QFile{path};
	if (!file.open(QIODevice::ReadOnly)) { return {}; }
	auto hash = QCryptographicHash{QCryptographicHash::Sha256};
	hash.addData(&file);
	return hash.result();
}

auto sameContents(const QString& a, const QString& b) -> bool
{
	if (QFileInfo{a}.size() != QFileInfo{b}.size()) { return false; }
	return contentsHash(a) == contentsHash(b);
}

//! @p path relative to @p directory, if it is inside it
auto inside(const QString& path, const QString& directory) -> std::optional<QString>
{
	const auto relative = QDir{directory}.relativeFilePath(path);
	if (relative.startsWith("..") || QDir::isAbsolutePath(relative)) { return std::nullopt; }
	return relative;
}

} // namespace


auto ProjectMedia::isFactoryFile(const QString& path) -> bool
{
	const auto data = QFileInfo{ConfigManager::inst()->dataDir()}.canonicalFilePath();
	const auto file = QFileInfo{path}.canonicalFilePath();
	return !data.isEmpty() && !file.isEmpty() && inside(file, data).has_value();
}


auto ProjectMedia::gather(DataFile& file, const QString& projectFile) -> QStringList
{
	auto problems = QStringList{};
	const auto project = QFileInfo{projectFile};
	const auto projectDir = project.absolutePath();
	const auto mediaName = project.completeBaseName() + "_media";
	const auto mediaDir = projectDir + '/' + mediaName;
	const auto local = PathUtil::basePrefix(PathUtil::Base::LocalDir);

	// Each file once, wherever it is referred to from
	auto gathered = std::map<QString, QString>{}; // its absolute path -> its reference now

	for (const auto& [tag, attributes] : DataFile::ELEMENTS_WITH_RESOURCES)
	{
		const auto elements = file.elementsByTagName(tag);
		for (int i = 0; i < elements.size(); ++i)
		{
			auto element = elements.at(i).toElement();
			for (const auto& attribute : attributes)
			{
				const auto value = element.attribute(attribute);
				if (value.isEmpty()) { continue; }

				const auto path = QFileInfo{PathUtil::toAbsolute(value)}.absoluteFilePath();
				if (const auto done = gathered.find(path); done != gathered.end())
				{
					element.setAttribute(attribute, done->second);
					continue;
				}
				if (!QFileInfo{path}.isFile())
				{
					problems << tr("Missing: %1").arg(value);
					continue;
				}
				// The program's own files stay where they are, as they were referred to
				if (isFactoryFile(path)) { continue; }

				// In the project's folder already: relative to it
				if (const auto relative = inside(path, projectDir))
				{
					gathered[path] = local + *relative;
					element.setAttribute(attribute, gathered[path]);
					continue;
				}

				// Elsewhere: copied into the media folder (a file of the same name there is used if it is the same)
				if (!QDir{}.mkpath(mediaDir))
				{
					problems << tr("The folder %1 could not be created.").arg(mediaDir);
					continue;
				}
				const auto info = QFileInfo{path};
				auto target = QString{};
				for (int n = 1;; ++n)
				{
					const auto name = n == 1 ? info.fileName()
						: QString{"%1-%2.%3"}.arg(info.completeBaseName()).arg(n).arg(info.suffix());
					target = mediaDir + '/' + name;
					if (!QFileInfo::exists(target) || sameContents(path, target)) { break; }
				}
				if (!QFileInfo::exists(target) && !QFile::copy(path, target))
				{
					problems << tr("%1 could not be copied into %2.").arg(path, mediaDir);
					continue;
				}
				gathered[path] = local + mediaName + '/' + QFileInfo{target}.fileName();
				element.setAttribute(attribute, gathered[path]);
			}
		}
	}
	return problems;
}

} // namespace lmms
