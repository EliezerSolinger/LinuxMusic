/*
 * ProjectMedia.h - keeps the files a project uses in the project's folder
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

#ifndef LMMS_PROJECT_MEDIA_H
#define LMMS_PROJECT_MEDIA_H

#include <QString>
#include <QSet>
#include <QStringList>

#include "lmms_export.h"

namespace lmms
{

class DataFile;

/**
 * The files a project uses live with it (docs/plano-anotacoes-13-11.md, 11.1): when a project is saved, every file
 * it refers to (DataFile::ELEMENTS_WITH_RESOURCES: clips with their takes and originals, AudioFileProcessor, SlicerT,
 * Sf2Player, GigPlayer, Patman) that is not one of the program's own (factory) files is copied into the project's
 * media folder, "<project>_media/" next to the project file, unless it is in the project's folder already; the
 * project then refers to it relative to its folder ("local:"), so that the folder can be moved or copied whole.
 * Saving to another folder brings the files along the same way.
 *
 * The files are copied, not moved: what is open keeps playing the files it has, and the undo history keeps
 * referring to them. A file of the same name already there is used if it has the same contents, else the copy
 * gets a number.
 */
class LMMS_EXPORT ProjectMedia
{
public:
	/**
	 * Gathers the files @p file refers to for the project file @p projectFile, rewriting its references
	 * @returns what went wrong: files missing (left as they were) or that could not be copied
	 */
	static auto gather(DataFile& file, const QString& projectFile) -> QStringList;

	//! @returns whether @p path is one of the program's own files (factory samples, presets, soundfonts...)
	static auto isFactoryFile(const QString& path) -> bool;

	/**
	 * Files the program makes (docs/plano-gravacao.md, "Mídia do projeto"): takes, renderings for the tempo, and
	 * their copies in a media folder. Each folder lists the ones it has in a hidden file, GeneratedList; only
	 * those are ever cleaned, never a file the user put there.
	 */
	static constexpr const char* GeneratedList = ".linuxmusic-generated";

	//! Notes that the program made the file @p path
	static void noteGenerated(const QString& path);
	//! @returns whether the program made @p path
	static auto isGenerated(const QString& path) -> bool;
	//! The files the program made in @p folder (absolute paths, existing)
	static auto generatedIn(const QString& folder) -> QStringList;

	//! The files @p file refers to (absolute paths; "local:" relative to @p projectFile)
	static auto referencedFiles(const DataFile& file, const QString& projectFile) -> QSet<QString>;

	//! @p path as a project refers to it, absolute: "local:" relative to @p projectFile (which may be empty for a
	//! project without a file), the other bases as PathUtil::toAbsolute()
	static auto absolutePath(const QString& path, const QString& projectFile) -> QString;

	/**
	 * Cleans the orphans of @p folders: the files the program made there that @p keep does not have (absolute
	 * paths) go to the system's trash, so a file another project still uses can be taken back. A folder left with
	 * nothing is removed if it is a folder of an unsaved project (UnsavedPrefix).
	 * @returns the files moved to the trash
	 */
	static auto cleanOrphans(const QStringList& folders, const QSet<QString>& keep) -> QStringList;

	//! The media folders of unsaved projects are named so
	static constexpr const char* UnsavedPrefix = "unsaved-";
};

} // namespace lmms

#endif // LMMS_PROJECT_MEDIA_H
