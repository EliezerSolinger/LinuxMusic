/*
 * ConfigManagerTest.cpp
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

#include <QFile>
#include <QObject>
#include <QTemporaryDir>
#include <QtTest>

#include "ConfigManager.h"

using namespace lmms;

class ConfigManagerTest : public QObject
{
	Q_OBJECT

	QTemporaryDir m_directory;

	auto configFile(const QByteArray& root, const QByteArray& content) -> QString
	{
		const auto path = m_directory.filePath("rc.xml");
		auto file = QFile{path};
		file.open(QIODevice::WriteOnly | QIODevice::Truncate);
		file.write("<?xml version=\"1.0\"?>\n<!DOCTYPE linuxmusic-config-file>\n<linuxmusic " + root + ">\n"
			+ content + "</linuxmusic>\n");
		return path;
	}

private slots:
	void initTestCase()
	{
		QVERIFY(m_directory.isValid());
	}

	//! A value not there is null, so the upgrades tell it from an empty one
	void aMissingValueIsNull()
	{
		auto* config = ConfigManager::inst();
		QVERIFY(config->value("nosuchsection", "nosuchvalue").isNull());
		QCOMPARE(config->value("nosuchsection", "nosuchvalue", "fallback"), QString{"fallback"});
		config->setValue("test", "empty", "");
		QVERIFY(!config->value("test", "empty").isNull());
		QVERIFY(config->value("test", "empty").isEmpty());
	}

	//! The upgrade from 1.2's "mixer" leaves the audio engine's settings alone when there was no "mixer"
	void theUpgradeKeepsTheAudioSettings()
	{
		auto* config = ConfigManager::inst();
		config->loadConfigFile(configFile("version=\"1.2.2\" configversion=\"2\"",
			"<audioengine audiodev=\"SDL (Simple DirectMedia Layer)\" mididev=\"Dummy MIDI\" samplerate=\"48000\"/>\n"));
		QCOMPARE(config->value("audioengine", "audiodev"), QString{"SDL (Simple DirectMedia Layer)"});
		QCOMPARE(config->value("audioengine", "mididev"), QString{"Dummy MIDI"});
		QCOMPARE(config->value("audioengine", "samplerate"), QString{"48000"});
	}

	//! ... and moves what was in it (from before 1.1.90: all the upgrades)
	void theUpgradeMovesTheMixerSettings()
	{
		auto* config = ConfigManager::inst();
		config->loadConfigFile(configFile("version=\"1.1.0\"",
			"<mixer audiodev=\"PulseAudio (bad latency!)\" framesperaudiobuffer=\"512\"/>\n"));
		QCOMPARE(config->value("audioengine", "audiodev"), QString{"PulseAudio"});
		QCOMPARE(config->value("audioengine", "framesperaudiobuffer"), QString{"512"});
		QVERIFY(config->value("mixer", "audiodev").isNull());
	}

	//! A value with lines (e.g. the suspect plugins, one a line) comes back from the file as it was
	void aValueWithLinesIsKept()
	{
		auto* config = ConfigManager::inst();
		const auto path = configFile("version=\"1.3.0\" configversion=\"3\"", "");
		config->loadConfigFile(path);
		config->setValue("plugins", "suspects", "ladspaeffect|file=a b.so\nvestige|file=Synth.dll");
		config->saveConfigFile();
		config->setValue("plugins", "suspects", "");
		config->loadConfigFile(path);
		QCOMPARE(config->value("plugins", "suspects"), QString{"ladspaeffect|file=a b.so\nvestige|file=Synth.dll"});
	}
};

QTEST_GUILESS_MAIN(ConfigManagerTest)
#include "ConfigManagerTest.moc"
