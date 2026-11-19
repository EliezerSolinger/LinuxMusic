/*
 * AutosaveTest.cpp
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

#include <QDomDocument>
#include <QFile>
#include <QObject>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QtEndian>
#include <QtTest>

#include <vector>

#include <sndfile.h>

#include "AudioEngine.h"
#include "AudioTrack.h"
#include "Autosave.h"
#include "Engine.h"
#include "Song.h"
#include "WavRepair.h"

using namespace lmms;

class AutosaveTest : public QObject
{
	Q_OBJECT

	QTemporaryDir m_directory;

	//! A take's file as a crash leaves it: @p frames written, the header last updated after @p headerFrames
	auto interruptedTake(const QString& name, int frames, int headerFrames) -> QString
	{
		const auto path = m_directory.filePath(name);
		auto info = SF_INFO{};
		info.samplerate = static_cast<int>(Engine::audioEngine()->outputSampleRate());
		info.channels = 2;
		info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
		auto* file = sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
		auto samples = std::vector<float>(static_cast<std::size_t>(frames) * 2, 0.25f);
		sf_writef_float(file, samples.data(), frames);
		sf_close(file);

		// The header as it was when it was last updated
		auto raw = QFile{path};
		raw.open(QIODevice::ReadWrite);
		const auto all = raw.readAll();
		const auto data = all.indexOf("data");
		const auto blockAlign = 2 * 4;
		uchar bytes[4];
		qToLittleEndian<quint32>(static_cast<quint32>(headerFrames * blockAlign), bytes);
		raw.seek(data + 4);
		raw.write(reinterpret_cast<const char*>(bytes), 4);
		qToLittleEndian<quint32>(static_cast<quint32>(data + 8 + headerFrames * blockAlign - 8), bytes);
		raw.seek(4);
		raw.write(reinterpret_cast<const char*>(bytes), 4);
		return path;
	}

	static auto framesIn(const QString& path) -> sf_count_t
	{
		auto info = SF_INFO{};
		auto* file = sf_open(QFile::encodeName(path).constData(), SFM_READ, &info);
		if (!file) { return -1; }
		sf_close(file);
		return info.frames;
	}

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

	//! A snapshot comes once the changes pause, written whole; changes that never pause get one too
	void aSnapshotAfterTheChangesPause()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		const auto path = m_directory.filePath("recover.mmp");
		auto autosave = Autosave{*song, path, nullptr, 200, 1000};
		auto saved = QSignalSpy{&autosave, &Autosave::saved};

		Track::create(Track::Type::Audio, song);
		song->setModified();
		QTest::qWait(100);
		QVERIFY(!QFile::exists(path)); // not yet: the changes may go on
		QVERIFY(saved.wait(2000));
		QVERIFY(QFile::exists(path));
		const auto first = QFileInfo{path}.lastModified();

		// Changes every 100 ms, which never pause for 200: still one within the longest wait
		saved.clear();
		auto elapsed = QElapsedTimer{};
		elapsed.start();
		while (saved.isEmpty() && elapsed.elapsed() < 3000)
		{
			song->setModified();
			QTest::qWait(100);
		}
		QVERIFY(!saved.isEmpty());
		QVERIFY(elapsed.elapsed() >= 900);
		autosave.flush();

		// Whole: it loads, with the track
		auto document = QDomDocument{};
		auto file = QFile{path};
		QVERIFY(file.open(QIODevice::ReadOnly));
		QVERIFY(document.setContent(&file));
		QVERIFY(document.elementsByTagName("track").count() >= 1);
		QVERIFY(QFileInfo{path}.lastModified() >= first);
	}

	//! A take's file whose header says less than it holds: completed to what it holds
	void anInterruptedTakesFileIsCompleted()
	{
		const auto path = interruptedTake("interrupted.wav", 12345, 4096);
		QCOMPARE(framesIn(path), sf_count_t{4096});
		QCOMPARE(WavRepair::completeLength(path), std::int64_t{12345});
		QCOMPARE(framesIn(path), sf_count_t{12345});
		// Already whole: unchanged
		QCOMPARE(WavRepair::completeLength(path), std::int64_t{12345});
		// Not a WAV file
		const auto other = m_directory.filePath("not.wav");
		auto text = QFile{other};
		QVERIFY(text.open(QIODevice::WriteOnly));
		text.write("not a wave file at all");
		text.close();
		QCOMPARE(WavRepair::completeLength(other), std::int64_t{-1});
	}

	//! A snapshot taken while recording, loaded: the takes come back on their track, as much as their files hold
	void aRecordingInProgressIsRecovered()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		Track::create(Track::Type::Audio, song);
		auto document = song->serializeProject();
		const auto take = interruptedTake("take.wav", 20000, 256);

		// As the snapshot of a recording writes it (Song::serializeProject())
		auto recording = document.createElement("recordinginprogress");
		auto takeElement = document.createElement("take");
		takeElement.setAttribute("track", 0);
		auto segment = document.createElement("segment");
		segment.setAttribute("path", take);
		segment.setAttribute("start", 2 * TimePos::ticksPerBar());
		takeElement.appendChild(segment);
		recording.appendChild(takeElement);
		document.content().appendChild(recording);
		const auto path = m_directory.filePath("recovered.mmp");
		QVERIFY(document.writeFile(path));

		song->loadProject(path);
		auto* track = dynamic_cast<AudioTrack*>(song->tracks().front());
		QVERIFY(track);
		QCOMPARE(track->numOfClips(), 1);
		QCOMPARE(track->getClip(0)->startPosition().getTicks(), 2 * TimePos::ticksPerBar());
		QCOMPARE(framesIn(take), sf_count_t{20000});
	}
};

QTEST_MAIN(AutosaveTest)
#include "AutosaveTest.moc"
