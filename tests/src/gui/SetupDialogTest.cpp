/*
 * SetupDialogTest.cpp
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

#include <QCheckBox>
#include <QComboBox>
#include <QLabel>
#include <QSignalSpy>
#include <QSlider>
#include <QtTest>

#include "AudioBuffer.h"
#include "AudioDummy.h"
#include "AudioEngine.h"
#include "ConfigManager.h"
#include "Engine.h"
#include "SetupDialog.h"

using namespace lmms;
using namespace lmms::gui;

/**
 * The settings (design 08): sections with a search, each setting applied as it changes (written at once, and the
 * parts of the program that keep it told), the device opening again once after a run of changes, and the ones that
 * need a restart saying so.
 */
class SetupDialogTest : public QObject
{
	Q_OBJECT

	//! The control of the setting named @p name (as its row shows it)
	template<class Control>
	static auto control(SetupDialog& dialog, const QString& name) -> Control*
	{
		for (auto* label : dialog.findChildren<QLabel*>("settingsName"))
		{
			if (label->text() != name) { continue; }
			// (the row: the label's column's parent)
			auto* row = label->parentWidget()->objectName() == "settingsRow" ? label->parentWidget()
				: label->parentWidget()->parentWidget();
			if (auto* found = row->findChild<Control*>()) { return found; }
		}
		return nullptr;
	}

private slots:
	void initTestCase()
	{
		ConfigManager::inst()->setValue("audioengine", "audiodev", AudioDummy::name());
		ConfigManager::inst()->setValue("app", "language", "en");
		Engine::init(true);
	}

	void cleanupTestCase() { Engine::destroy(); }

	void sectionsAndTheTabsTheyReplace()
	{
		auto dialog = SetupDialog{SetupDialog::ConfigTab::AudioSettings};
		QCOMPARE(dialog.sectionTitles().size(), 9);
		QCOMPARE(dialog.currentSection(), QString{"Audio"});
		auto shortcuts = SetupDialog{SetupDialog::ConfigTab::ShortcutsSettings};
		QCOMPARE(shortcuts.currentSection(), QString{"Keyboard and shortcuts"});
	}

	void theSearchFindsSettingsWhereverTheyAre()
	{
		auto dialog = SetupDialog{};
		dialog.search("buffer");
		QCOMPARE(dialog.sectionTitles(true), QStringList{"Audio"});
		QCOMPARE(dialog.currentSection(), QString{"Audio"});
		// Without case or accents, and by a section's title
		dialog.search("METRÔNOME");
		QCOMPARE(dialog.sectionTitles(true), QStringList{"Recording"});
		dialog.search("folders");
		QVERIFY(dialog.sectionTitles(true).contains("Folders"));
		dialog.search("nothing like this");
		QVERIFY(dialog.sectionTitles(true).isEmpty());
		dialog.search("");
		QCOMPARE(dialog.sectionTitles(true).size(), 9);
	}

	void aSettingAppliesAsItChanges()
	{
		auto dialog = SetupDialog{};
		auto* compress = control<QCheckBox>(dialog, "Compress project files");
		QVERIFY(compress);
		auto spy = QSignalSpy{ConfigManager::inst(), &ConfigManager::valueChanged};
		const bool was = compress->isChecked();
		compress->setChecked(!was);
		// Written at once (no Save), and those who keep it told
		QCOMPARE(ConfigManager::inst()->value("app", "nommpz"), QString{was ? "1" : "0"});
		QVERIFY(!spy.isEmpty());
		compress->setChecked(was);
		QVERIFY(dialog.restartNeeded().isEmpty());
	}

	void aSettingNeverSetBeforeIsHeardToo()
	{
		auto spy = QSignalSpy{ConfigManager::inst(), &ConfigManager::valueChanged};
		ConfigManager::inst()->setValue("test", "never-set-before", "1");
		QCOMPARE(spy.count(), 1);
	}

	void theEngineFollowsItsSettingsLive()
	{
		auto dialog = SetupDialog{};
		auto* sanitize = control<QCheckBox>(dialog, "Mix sanitization");
		auto* keepRunning = control<QCheckBox>(dialog, "Effects keep running without input");
		QVERIFY(sanitize && keepRunning);
		sanitize->setChecked(false);
		QVERIFY(!Engine::audioEngine()->sanitizationEnabled());
		sanitize->setChecked(true);
		QVERIFY(Engine::audioEngine()->sanitizationEnabled());
		// Auto-quit (the buffers tracking silence) follows "keep running" at once, for the effects already there
		keepRunning->setChecked(false);
		QVERIFY(AudioBuffer::silenceTrackingWanted());
		keepRunning->setChecked(true);
		QVERIFY(!AudioBuffer::silenceTrackingWanted());
	}

	void theDeviceOpensAgainOnceAfterARunOfChanges()
	{
		auto dialog = SetupDialog{SetupDialog::ConfigTab::AudioSettings};
		auto* slider = dialog.findChild<QSlider*>();
		QVERIFY(slider);
		auto* engine = Engine::audioEngine();
		const auto before = engine->framesPerAudioBuffer();
		auto opened = QSignalSpy{engine, &AudioEngine::sampleRateChanged};
		// Dragged: 64, 128, 512 frames (the slider counts in 32s)
		for (const int value : {2, 4, 16}) { slider->setValue(value); }
		QCOMPARE(engine->framesPerAudioBuffer(), before); // (not yet)
		QTRY_COMPARE(engine->framesPerAudioBuffer(), f_cnt_t{512});
		QCOMPARE(opened.count(), 1);
		slider->setValue(static_cast<int>(before / 32));
		dialog.accept(); // (what waits applies on leaving)
		QCOMPARE(engine->framesPerAudioBuffer(), before);
	}

	void theLanguageAsksForARestart()
	{
		auto dialog = SetupDialog{};
		auto* language = control<QComboBox>(dialog, "Language");
		QVERIFY(language);
		if (language->count() < 2) { QSKIP("one language here"); }
		language->setCurrentIndex(language->currentIndex() == 0 ? 1 : 0);
		QCOMPARE(dialog.restartNeeded(), QStringList{"The language"});
		ConfigManager::inst()->setValue("app", "language", "en");
	}
};

QTEST_MAIN(SetupDialogTest)
#include "SetupDialogTest.moc"
