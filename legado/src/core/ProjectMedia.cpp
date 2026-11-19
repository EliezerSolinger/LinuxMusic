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
#include "PeakCache.h"
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


namespace
{

auto readList(const QString& folder) -> QStringList
{
	auto file = QFile{folder + '/' + ProjectMedia::GeneratedList};
	if (!file.open(QIODevice::ReadOnly | QIODevice::Text)) { return {}; }
	auto names = QStringList{};
	for (const auto& line : QString::fromUtf8(file.readAll()).split('\n', Qt::SkipEmptyParts))
	{
		// (names only: nothing outside the folder is ever listed)
		const auto name = line.trimmed();
		if (!name.isEmpty() && !name.contains('/') && !name.contains('\\') && name != "." && name != ".."
			&& !names.contains(name))
		{
			names << name;
		}
	}
	return names;
}

auto writeList(const QString& folder, const QStringList& names) -> bool
{
	const auto path = folder + '/' + ProjectMedia::GeneratedList;
	if (names.isEmpty()) { return !QFileInfo::exists(path) || QFile::remove(path); }
	auto file = QFile{path};
	if (!file.open(QIODevice::WriteOnly | QIODevice::Truncate | QIODevice::Text)) { return false; }
	return file.write((names.join('\n') + '\n').toUtf8()) > 0;
}

auto absoluteOf(const QString& path) -> QString
{
	return QFileInfo{path}.absoluteFilePath();
}

auto moveToTrash(const QString& path) -> bool
{
	const auto trash = qEnvironmentVariable("LINUXMUSIC_TRASH_DIR");
	if (trash.isEmpty()) { return QFile::moveToTrash(path); }
	if (!QDir{}.mkpath(trash)) { return false; }
	const auto info = QFileInfo{path};
	auto target = trash + '/' + info.fileName();
	for (int n = 2; QFileInfo::exists(target); ++n) { target = QString{"%1/%2 %3"}.arg(trash, info.fileName()).arg(n); }
	return QFile::rename(path, target);
}

} // namespace


void ProjectMedia::noteGenerated(const QString& path)
{
	const auto info = QFileInfo{path};
	const auto folder = info.absolutePath();
	auto names = readList(folder);
	if (names.contains(info.fileName())) { return; }
	if (!QDir{}.mkpath(folder)) { return; }
	names << info.fileName();
	writeList(folder, names);
}


auto ProjectMedia::isGenerated(const QString& path) -> bool
{
	const auto info = QFileInfo{path};
	return readList(info.absolutePath()).contains(info.fileName());
}


auto ProjectMedia::generatedIn(const QString& folder) -> QStringList
{
	auto files = QStringList{};
	for (const auto& name : readList(folder))
	{
		const auto path = absoluteOf(folder + '/' + name);
		if (QFileInfo{path}.isFile()) { files << path; }
	}
	return files;
}


auto ProjectMedia::absolutePath(const QString& path, const QString& projectFile) -> QString
{
	const auto local = PathUtil::basePrefix(PathUtil::Base::LocalDir);
	if (path.startsWith(local))
	{
		if (projectFile.isEmpty()) { return {}; }
		return absoluteOf(QFileInfo{projectFile}.absolutePath() + '/' + path.mid(local.size()));
	}
	return absoluteOf(PathUtil::toAbsolute(path));
}


auto ProjectMedia::referencedFiles(const DataFile& file, const QString& projectFile) -> QSet<QString>
{
	auto files = QSet<QString>{};
	for (const auto& [tag, attributes] : DataFile::ELEMENTS_WITH_RESOURCES)
	{
		const auto elements = file.elementsByTagName(tag);
		for (int i = 0; i < elements.size(); ++i)
		{
			const auto element = elements.at(i).toElement();
			for (const auto& attribute : attributes)
			{
				if (const auto value = element.attribute(attribute); !value.isEmpty())
				{
					if (auto path = absolutePath(value, projectFile); !path.isEmpty()) { files.insert(path); }
				}
			}
		}
	}
	return files;
}


auto ProjectMedia::cleanOrphans(const QStringList& folders, const QSet<QString>& keep) -> QStringList
{
	auto moved = QStringList{};
	auto done = QSet<QString>{};
	for (const auto& folderName : folders)
	{
		const auto folder = absoluteOf(folderName);
		if (folder.isEmpty() || done.contains(folder) || !QFileInfo{folder}.isDir()) { continue; }
		done.insert(folder);

		auto names = readList(folder);
		auto left = QStringList{};
		for (const auto& name : names)
		{
			const auto path = absoluteOf(folder + '/' + name);
			if (!QFileInfo::exists(path)) { continue; } // (gone already: dropped from the list)
			if (keep.contains(path)) { left << name; continue; }
			// To the trash, not deleted: another project may still use it. Where there is no trash it stays.
			// (LINUXMUSIC_TRASH_DIR gives another folder for it, for tests)
			if (moveToTrash(path))
			{
				moved << path;
				QFile::remove(PeakCache::peakFileOf(path));
			}
			else { left << name; }
		}
		if (left != names) { writeList(folder, left); }

		// An unsaved project's folder with nothing left goes too
		if (QFileInfo{folder}.fileName().startsWith(UnsavedPrefix) && left.isEmpty())
		{
			QDir{}.rmdir(folder); // (only if empty)
		}
	}
	return moved;
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
				// (a copy of a file the program made is one too: cleaned when orphaned)
				if (isGenerated(path)) { noteGenerated(target); }
				gathered[path] = local + mediaName + '/' + QFileInfo{target}.fileName();
				element.setAttribute(attribute, gathered[path]);
			}
		}
	}
	return problems;
}

} // namespace lmms
