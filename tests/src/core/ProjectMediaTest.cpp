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
#include <QTemporaryDir>
#include <QtTest>
#include <sndfile.h>

#include "AudioEngine.h"
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
	const auto clips = file.elementsByTagName("sampleclip");
	for (int i = 0; i < clips.size(); ++i) { sources << clips.at(i).toElement().attribute("src"); }
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

		// Opened, it plays the copies
		song->loadProject(projectFile);
		auto* loaded = dynamic_cast<SampleTrack*>(song->tracks().front());
		QVERIFY(loaded);
		auto* loadedClip = dynamic_cast<SampleClip*>(loaded->getClip(0));
		QCOMPARE(PathUtil::toAbsolute(loadedClip->sampleFile()), QFileInfo{projectDir + "/song_media/other.wav"}.absoluteFilePath());

		// Saved in another folder: the files come along
		const auto copyDir = m_directory.filePath("copy");
		QVERIFY(QDir{}.mkpath(copyDir));
		QVERIFY(song->guiSaveProjectAs(copyDir + "/moved.mmp"));
		QVERIFY(QFileInfo::exists(copyDir + "/moved_media/hit.wav"));
		QVERIFY(QFileInfo::exists(copyDir + "/moved_media/other.wav"));
		QCOMPARE(clipSources(copyDir + "/moved.mmp"), QStringList{"local:moved_media/other.wav"});

		// A different file of the same name is not overwritten: the copy gets a number
		writeWav(elsewhere + "/hit2.wav", 0.75f);
		QVERIFY(QDir{}.mkpath(m_directory.filePath("other")));
		QVERIFY(QFile::copy(elsewhere + "/hit2.wav", m_directory.filePath("other/hit.wav")));
		auto* third = new SampleClip{loaded};
		third->setSampleFile(m_directory.filePath("other/hit.wav"));
		QVERIFY(song->guiSaveProjectAs(copyDir + "/moved.mmp"));
		QVERIFY(QFileInfo::exists(copyDir + "/moved_media/hit-2.wav"));
		song->createNewProject();
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
