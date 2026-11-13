/*
 * ProjectMediaTest.cpp
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

#include <vector>

#include <QDir>
#include <QDirIterator>
#include <QDomDocument>
#include <QObject>
#include <QLockFile>
#include <QTemporaryDir>
#include <QtTest>
#include <sndfile.h>

#include "AudioEngine.h"
#include "AudioTrack.h"
#include "ConfigManager.h"
#include "DataFile.h"
#include "Engine.h"
#include "PathUtil.h"
#include "ProjectMedia.h"
#include "SampleClip.h"
#include "SampleTrack.h"
#include "Song.h"

using namespace lmms;

namespace
{

void writeWav(const QString& path, float value)
{
	auto info = SF_INFO{};
	info.samplerate = 48000;
	info.channels = 1;
	info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
	auto* file = sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
	QVERIFY(file);
	auto samples = std::vector<float>(4800, value);
	sf_writef_float(file, samples.data(), 4800);
	sf_close(file);
}

//! The "src" of each <sampleclip> of the project file @p path
auto clipSources(const QString& path) -> QStringList
{
	auto file = DataFile{path};
	auto sources = QStringList{};
	for (const auto* tag : {"sampleclip", "audioitem"})
	{
		const auto clips = file.elementsByTagName(tag);
		for (int i = 0; i < clips.size(); ++i) { sources << clips.at(i).toElement().attribute("src"); }
	}
	return sources;
}

} // namespace

class ProjectMediaTest : public QObject
{
	Q_OBJECT

private:
	QTemporaryDir m_directory;

private slots:
	void initTestCase()
	{
		QVERIFY(m_directory.isValid());
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	void aSavedProjectKeepsItsFilesInItsFolder()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		const auto elsewhere = m_directory.filePath("elsewhere");
		QVERIFY(QDir{}.mkpath(elsewhere));
		const auto sample = elsewhere + "/hit.wav";
		const auto other = elsewhere + "/other.wav";
		writeWav(sample, 0.5f);
		writeWav(other, 0.25f);

		auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		auto* clip = new SampleClip{track};
		clip->setSampleFile(sample);
		clip->addTake(other); // a take, the active one

		// Saved: both copied into the project's media folder, referred to relative to the project
		const auto projectDir = m_directory.filePath("project");
		QVERIFY(QDir{}.mkpath(projectDir));
		const auto projectFile = projectDir + "/song.mmp";
		QVERIFY(song->guiSaveProjectAs(projectFile));
		QVERIFY(QFileInfo::exists(projectDir + "/song_media/hit.wav"));
		QVERIFY(QFileInfo::exists(projectDir + "/song_media/other.wav"));
		QCOMPARE(clipSources(projectFile), QStringList{"local:song_media/other.wav"});
		auto saved = DataFile{projectFile};
		const auto takes = saved.elementsByTagName("take");
		QCOMPARE(takes.size(), 2);
		QCOMPARE(takes.at(0).toElement().attribute("src"), QString{"local:song_media/hit.wav"});
		// (The originals stay where they were)
		QVERIFY(QFileInfo::exists(sample));

		// Saved again: nothing more is copied (same files, same contents)
		QVERIFY(song->guiSaveProjectAs(projectFile));
		QCOMPARE(QDir{projectDir + "/song_media"}.entryList(QDir::Files).size(), qsizetype{2});

		// Opened, it plays the copies (a sample track with takes opens as an audio track: a lane for each take)
		song->loadProject(projectFile);
		auto* loaded = dynamic_cast<AudioTrack*>(song->tracks().front());
		QVERIFY(loaded);
		auto* heard = loaded->items().front()->lane() == 0 ? loaded->items().front() : loaded->items().back();
		QCOMPARE(PathUtil::toAbsolute(heard->file()), QFileInfo{projectDir + "/song_media/other.wav"}.absoluteFilePath());

		// Saved in another folder: the files come along
		const auto copyDir = m_directory.filePath("copy");
		QVERIFY(QDir{}.mkpath(copyDir));
		QVERIFY(song->guiSaveProjectAs(copyDir + "/moved.mmp"));
		QVERIFY(QFileInfo::exists(copyDir + "/moved_media/hit.wav"));
		QVERIFY(QFileInfo::exists(copyDir + "/moved_media/other.wav"));
		auto sources = clipSources(copyDir + "/moved.mmp");
		sources.sort();
		QCOMPARE(sources, (QStringList{"local:moved_media/hit.wav", "local:moved_media/other.wav"}));

		// A different file of the same name is not overwritten: the copy gets a number
		writeWav(elsewhere + "/hit2.wav", 0.75f);
		QVERIFY(QDir{}.mkpath(m_directory.filePath("other")));
		QVERIFY(QFile::copy(elsewhere + "/hit2.wav", m_directory.filePath("other/hit.wav")));
		loaded->addItem(m_directory.filePath("other/hit.wav"), TimePos{0});
		QVERIFY(song->guiSaveProjectAs(copyDir + "/moved.mmp"));
		QVERIFY(QFileInfo::exists(copyDir + "/moved_media/hit-2.wav"));
		song->createNewProject();
	}

	void onlyFilesTheProgramMadeAreListed()
	{
		const auto folder = m_directory.filePath("listed");
		QVERIFY(QDir{}.mkpath(folder));
		writeWav(folder + "/take_1.wav", 0.1f);
		writeWav(folder + "/mine.wav", 0.1f);
		ProjectMedia::noteGenerated(folder + "/take_1.wav");
		ProjectMedia::noteGenerated(folder + "/take_1.wav"); // (once)
		QVERIFY(ProjectMedia::isGenerated(folder + "/take_1.wav"));
		QVERIFY(!ProjectMedia::isGenerated(folder + "/mine.wav"));
		QCOMPARE(ProjectMedia::generatedIn(folder), QStringList{QFileInfo{folder + "/take_1.wav"}.absoluteFilePath()});

		// A list that names something outside the folder is not followed
		auto list = QFile{folder + '/' + ProjectMedia::GeneratedList};
		QVERIFY(list.open(QIODevice::Append | QIODevice::Text));
		list.write("../elsewhere.wav\nsub/x.wav\n..\n");
		list.close();
		QCOMPARE(ProjectMedia::generatedIn(folder).size(), qsizetype{1});
	}

	void orphansGoToTheTrashAndNothingElse()
	{
		const auto folder = m_directory.filePath("orphans");
		QVERIFY(QDir{}.mkpath(folder));
		for (const auto* name : {"kept.wav", "orphan.wav", "users.wav"}) { writeWav(folder + '/' + name, 0.1f); }
		ProjectMedia::noteGenerated(folder + "/kept.wav");
		ProjectMedia::noteGenerated(folder + "/orphan.wav");
		ProjectMedia::noteGenerated(folder + "/gone.wav"); // (never made)

		const auto moved = ProjectMedia::cleanOrphans({folder}, {QFileInfo{folder + "/kept.wav"}.absoluteFilePath()});
		QCOMPARE(moved, QStringList{QFileInfo{folder + "/orphan.wav"}.absoluteFilePath()});
		QVERIFY(QFileInfo::exists(folder + "/kept.wav"));
		QVERIFY(!QFileInfo::exists(folder + "/orphan.wav"));
		QVERIFY(QFileInfo::exists(folder + "/users.wav")); // the user's own file is never touched
		QVERIFY(QFileInfo::exists(qEnvironmentVariable("LINUXMUSIC_TRASH_DIR") + "/orphan.wav"));
		QCOMPARE(ProjectMedia::generatedIn(folder), QStringList{QFileInfo{folder + "/kept.wav"}.absoluteFilePath()});

		// An unsaved project's folder left with nothing goes too
		const auto unsaved = m_directory.filePath(QString{ProjectMedia::UnsavedPrefix} + "x");
		QVERIFY(QDir{}.mkpath(unsaved));
		writeWav(unsaved + "/take.wav", 0.1f);
		ProjectMedia::noteGenerated(unsaved + "/take.wav");
		ProjectMedia::cleanOrphans({unsaved}, {});
		QVERIFY(!QFileInfo::exists(unsaved));
	}

	void closingAProjectCleansWhatItsFileDoesNotUse()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		const auto projectDir = m_directory.filePath("cleaned");
		QVERIFY(QDir{}.mkpath(projectDir + "/song_media"));
		const auto used = projectDir + "/song_media/take_1.wav";
		const auto undone = projectDir + "/song_media/take_2.wav";
		writeWav(used, 0.2f);
		writeWav(undone, 0.3f);
		ProjectMedia::noteGenerated(used);
		ProjectMedia::noteGenerated(undone);

		auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		auto* clip = new SampleClip{track};
		clip->setSampleFile(used);
		QVERIFY(song->guiSaveProjectAs(projectDir + "/song.mmp"));
		// The second take is used after the save only, and not saved: discarded with the project
		auto* other = new SampleClip{track};
		other->setSampleFile(undone);

		song->createNewProject();
		QVERIFY(QFileInfo::exists(used));
		QVERIFY(!QFileInfo::exists(undone));

		// Opened and closed again: what it uses stays
		song->loadProject(projectDir + "/song.mmp");
		song->createNewProject();
		QVERIFY(QFileInfo::exists(used));

		// Closed by opening another project (whose file name is set before the one open is cleared): cleaned
		// against its own file, not the other's
		song->loadProject(projectDir + "/song.mmp");
		const auto later = projectDir + "/song_media/take_3.wav";
		writeWav(later, 0.4f);
		ProjectMedia::noteGenerated(later);
		auto* laterClip = new SampleClip{dynamic_cast<SampleTrack*>(song->tracks().front())};
		laterClip->setSampleFile(later);
		const auto otherDir = m_directory.filePath("cleaned-other");
		QVERIFY(QDir{}.mkpath(otherDir));
		// (one that cannot be loaded leaves the project open, and nothing is cleaned)
		auto broken = QFile{otherDir + "/broken.mmp"};
		QVERIFY(broken.open(QIODevice::WriteOnly));
		broken.write("not a project");
		broken.close();
		song->loadProject(otherDir + "/broken.mmp");
		QVERIFY(QFileInfo::exists(later));
		song->loadProject(projectDir + "/song.mmp"); // (the same one again: it closes the one open)
		QVERIFY(QFileInfo::exists(used));
		QVERIFY(!QFileInfo::exists(later));
		song->createNewProject();
	}

	void anUnsavedProjectKeepsNothingAndAbandonedFoldersAreCleaned()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		const auto folder = song->mediaDirectory();
		QVERIFY(QFileInfo{folder}.fileName().startsWith(ProjectMedia::UnsavedPrefix));
		QVERIFY(QDir{}.mkpath(folder));
		writeWav(folder + "/take_1.wav", 0.2f);
		ProjectMedia::noteGenerated(folder + "/take_1.wav");
		auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		(new SampleClip{track})->setSampleFile(folder + "/take_1.wav");
		song->createNewProject();
		QVERIFY(!QFileInfo::exists(folder + "/take_1.wav"));
		QVERIFY(!QFileInfo::exists(folder));
		QVERIFY(song->mediaDirectory() != folder); // the next project has a folder of its own

		// Left by a run that crashed: cleaned at start, unless another instance holds it
		const auto abandoned = ConfigManager::inst()->workingDir() + "recordings/" + ProjectMedia::UnsavedPrefix + "old";
		QVERIFY(QDir{}.mkpath(abandoned));
		writeWav(abandoned + "/take_1.wav", 0.2f);
		ProjectMedia::noteGenerated(abandoned + "/take_1.wav");
		{
			auto held = QLockFile{abandoned + "/.lock"};
			QVERIFY(held.tryLock(0));
			song->cleanAbandonedMedia();
			QVERIFY(QFileInfo::exists(abandoned + "/take_1.wav"));
		}
		song->cleanAbandonedMedia();
		QVERIFY(!QFileInfo::exists(abandoned));
	}

	void audioItemsKeepTheirFilesWithTheProject()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		const auto elsewhere = m_directory.filePath("items-elsewhere");
		QVERIFY(QDir{}.mkpath(elsewhere));
		writeWav(elsewhere + "/take_1.wav", 0.5f);
		ProjectMedia::noteGenerated(elsewhere + "/take_1.wav");
		auto* track = dynamic_cast<AudioTrack*>(Track::create(Track::Type::Audio, song));
		QVERIFY(track);
		track->addItem(elsewhere + "/take_1.wav", TimePos{0});

		// Saved: copied into the media folder, and kept when the project closes
		const auto projectDir = m_directory.filePath("items");
		QVERIFY(QDir{}.mkpath(projectDir));
		QVERIFY(song->guiSaveProjectAs(projectDir + "/song.mmp"));
		QVERIFY(QFileInfo::exists(projectDir + "/song_media/take_1.wav"));
		song->loadProject(projectDir + "/song.mmp");
		auto* loaded = dynamic_cast<AudioTrack*>(song->tracks().back());
		QVERIFY(loaded);
		QCOMPARE(PathUtil::toAbsolute(loaded->items().front()->file()),
			QFileInfo{projectDir + "/song_media/take_1.wav"}.absoluteFilePath());
		song->createNewProject();
		QVERIFY(QFileInfo::exists(projectDir + "/song_media/take_1.wav"));
	}

	void factoryFilesStayWhereTheyAre()
	{
		auto factory = QString{};
		for (auto it = QDirIterator{ConfigManager::inst()->factorySamplesDir(), {"*.ogg", "*.wav"},
			QDir::Files, QDirIterator::Subdirectories}; it.hasNext();)
		{
			factory = it.next();
			break;
		}
		if (factory.isEmpty()) { QSKIP("No factory samples in this build"); }
		QVERIFY(ProjectMedia::isFactoryFile(factory));

		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		auto* clip = new SampleClip{track};
		clip->setSampleFile(factory);
		const auto projectDir = m_directory.filePath("factory");
		QVERIFY(QDir{}.mkpath(projectDir));
		QVERIFY(song->guiSaveProjectAs(projectDir + "/song.mmp"));
		QVERIFY(!QFileInfo::exists(projectDir + "/song_media"));
		QVERIFY(!clipSources(projectDir + "/song.mmp").front().startsWith("local:"));
		song->createNewProject();
	}

	void longAudioIsARecordingShortIsASample()
	{
		// The length comes from the header (a recording is not decoded to tell)
		const auto write = [](const QString& path, int frames) {
			auto info = SF_INFO{};
			info.samplerate = 8000;
			info.channels = 1;
			info.format = SF_FORMAT_WAV | SF_FORMAT_PCM_16;
			auto* file = sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
			QVERIFY(file);
			auto samples = std::vector<short>(static_cast<std::size_t>(frames), 0);
			sf_writef_short(file, samples.data(), frames);
			sf_close(file);
		};
		const auto stem = m_directory.filePath("stem.wav");
		const auto hit = m_directory.filePath("hit-short.wav");
		write(stem, 8000 * 31);
		write(hit, 8000 * 2);
		QVERIFY(std::abs(SampleTrack::audioFileSeconds(stem) - 31.0) < 1e-6);
		QVERIFY(SampleTrack::audioFileSeconds(stem) > SampleTrack::LongAudioSeconds);
		QVERIFY(SampleTrack::audioFileSeconds(hit) < SampleTrack::LongAudioSeconds);
		QCOMPARE(SampleTrack::audioFileSeconds(m_directory.filePath("nothing.wav")), 0.0);
	}

	void factoryFilesReferOnlyToFactorySamplesThatExist()
	{
		// Presets, demos and templates: every sample they name is one of the program's (removing samples must
		// take what uses them along)
		const auto data = QDir{ConfigManager::inst()->dataDir()};
		auto missing = QStringList{};
		auto checked = 0;
		for (const auto& folder : {QString{"presets"}, QString{"projects"}})
		{
			for (auto it = QDirIterator{data.filePath(folder), {"*.xpf", "*.mmp", "*.mpt"}, QDir::Files,
				QDirIterator::Subdirectories}; it.hasNext();)
			{
				const auto path = it.next();
				auto file = QFile{path};
				QVERIFY(file.open(QIODevice::ReadOnly));
				auto document = QDomDocument{};
				if (!document.setContent(&file)) { continue; }
				// Every element, depth first
				auto elements = std::vector<QDomElement>{document.documentElement()};
				for (std::size_t n = 0; n < elements.size(); ++n)
				{
					for (auto child = elements[n].firstChildElement(); !child.isNull(); child = child.nextSiblingElement())
					{
						elements.push_back(child);
					}
				}
				for (const auto& element : elements)
				{
					for (const auto& attribute : {QString{"src"}, QString{"userwavefile"}})
					{
						const auto value = element.attribute(attribute);
						if (value.isEmpty() || QDir::isAbsolutePath(value)) { continue; }
						if (!value.contains('.')) { continue; } // (not a file)
						++checked;
						// (with a base's prefix, e.g. factorysample:, or relative to the factory samples)
						const auto path = value.contains(':') ? PathUtil::toAbsolute(value)
							: ConfigManager::inst()->factorySamplesDir() + value;
						if (!QFileInfo::exists(path))
						{
							missing << QString{"%1: %2"}.arg(QFileInfo{path}.fileName(), value);
						}
					}
				}
			}
		}
		QVERIFY2(checked > 0, qPrintable(data.absolutePath()));
		QVERIFY2(missing.isEmpty(), qPrintable(missing.join('\n')));
	}

	void onlyFileReferencesMayBeLocal()
	{
		const auto check = [](const QString& xml) {
			auto file = DataFile{DataFile::Type::SongProject};
			auto document = QDomDocument{};
			document.setContent(xml);
			file.content().appendChild(file.importNode(document.documentElement(), true));
			return file.hasLocalPlugins();
		};
		QVERIFY(!check(R"(<sampleclip src="local:a.wav"><take src="local:b.wav"/><original src="local:c.wav"/></sampleclip>)"));
		QVERIFY(!check(R"(<sf2player src="local:kit.sf2"/>)"));
		// A plugin's library may not be relative to the project, nor another attribute of a resource element
		QVERIFY(check(R"(<vestige plugin="local:evil.so"/>)"));
		QVERIFY(check(R"(<sampleclip src="a.wav" plugin="local:evil.so"/>)"));
	}
};

QTEST_GUILESS_MAIN(ProjectMediaTest)
#include "ProjectMediaTest.moc"
