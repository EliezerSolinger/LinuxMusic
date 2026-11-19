/*
 * SetupDialog.cpp - the settings
 *
 * Copyright (c) 2005-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "SetupDialog.h"

#include <algorithm>

#include <QAbstractButton>
#include <QAbstractSlider>
#include <QButtonGroup>
#include <QCheckBox>
#include <QComboBox>
#include <QCoreApplication>
#include <QDoubleSpinBox>
#include <QElapsedTimer>
#include <QHBoxLayout>
#include <QHeaderView>
#include <QKeySequenceEdit>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMessageBox>
#include <QPushButton>
#include <QScrollArea>
#include <QSlider>
#include <QSpinBox>
#include <QStackedWidget>
#include <QTableWidget>
#include <QThread>
#include <QVBoxLayout>

#include "AudioDevice.h"
#include "AudioEngine.h"
#include "ConfigManager.h"
#include "embed.h"
#include "Engine.h"
#include "FileDialog.h"
#include "GuiApplication.h"
#include "LatencyCalibration.h"
#include "MainWindow.h"
#include "MidiClient.h"
#include "MidiSetupWidget.h"
#include "PluginGuard.h"
#include "ProjectJournal.h"
#include "SampleTrack.h"
#include "Shortcuts.h"
#include "Song.h"
#include "TimeLineWidget.h"

// Platform-specific audio-interface classes.
#include "AudioAlsa.h"
#include "AudioAlsaSetupWidget.h"
#include "AudioCoreAudioSetupWidget.h"
#include "AudioDummySetupWidget.h"
#include "AudioJackSetupWidget.h"
#include "AudioPortAudioSetupWidget.h"
#include "AudioPulseAudioSetupWidget.h"
#include "AudioSdlSetupWidget.h"
#include "AudioSndioSetupWidget.h"
#include "AudioDummy.h"
#include "AudioJack.h"
#include "AudioCoreAudio.h"
#include "AudioPortAudio.h"
#include "AudioPulseAudio.h"
#include "AudioSdl.h"
#include "AudioSndio.h"

// Platform-specific midi-interface classes.
#include "MidiAlsaRaw.h"
#include "MidiAlsaSeq.h"
#include "MidiApple.h"
#include "MidiDummy.h"
#include "MidiJack.h"
#include "MidiSndio.h"
#include "MidiWinMM.h"


namespace lmms::gui
{

namespace
{

constexpr int BufferSizeResolution = 32;

//! How the search compares: without case or accents ("Áudio" is found by "audio")
auto folded(const QString& text) -> QString
{
	auto result = QString{};
	for (const auto c : text.normalized(QString::NormalizationForm_D))
	{
		if (c.category() != QChar::Mark_NonSpacing) { result += c.toLower(); }
	}
	return result;
}

//! The texts a control offers (a combo box's choices, a check box's text), for the search
auto textsOf(QWidget* widget) -> QString
{
	auto texts = QStringList{};
	for (auto* box : widget->findChildren<QComboBox*>())
	{
		for (int i = 0; i < box->count(); ++i) { texts << box->itemText(i); }
	}
	if (auto* box = qobject_cast<QComboBox*>(widget))
	{
		for (int i = 0; i < box->count(); ++i) { texts << box->itemText(i); }
	}
	for (auto* button : widget->findChildren<QAbstractButton*>()) { texts << button->text(); }
	if (auto* button = qobject_cast<QAbstractButton*>(widget)) { texts << button->text(); }
	for (auto* label : widget->findChildren<QLabel*>()) { texts << label->text(); }
	return texts.join(' ');
}

} // namespace




SetupDialog::SetupDialog(ConfigTab tabToOpen, QWidget* page) :
	QDialog(page)
{
	setObjectName("settings");
	setWindowIcon(embed::getIconPixmap("setup_general"));
	setWindowTitle(tr("Settings"));
	if (!page)
	{
		setWindowFlag(Qt::WindowContextHelpButtonHint, false);
		setModal(true);
	}

	Engine::projectJournal()->setJournalling(false);

	m_audioReopen.setSingleShot(true);
	m_audioReopen.setInterval(400);
	connect(&m_audioReopen, &QTimer::timeout, this, &SetupDialog::reopenAudioDevice);
	m_midiReopen.setSingleShot(true);
	m_midiReopen.setInterval(400);
	connect(&m_midiReopen, &QTimer::timeout, this, &SetupDialog::reopenMidiClient);
	m_saveConfig.setSingleShot(true);
	m_saveConfig.setInterval(500);
	connect(&m_saveConfig, &QTimer::timeout, this, [] { ConfigManager::inst()->saveConfigFile(); });

	auto* layout = new QHBoxLayout(this);
	layout->setContentsMargins(0, 0, 0, 0);
	layout->setSpacing(0);

	// The side: the search over the list of sections
	auto* side = new QWidget(this);
	side->setObjectName("settingsSide");
	side->setFixedWidth(220);
	auto* sideLayout = new QVBoxLayout(side);
	sideLayout->setContentsMargins(10, 12, 10, 12);
	sideLayout->setSpacing(10);
	m_search = new QLineEdit(side);
	m_search->setObjectName("settingsSearch");
	m_search->setPlaceholderText(tr("Search settings…"));
	m_search->setClearButtonEnabled(true);
	sideLayout->addWidget(m_search);
	m_sectionList = new QListWidget(side);
	m_sectionList->setObjectName("settingsSections");
	m_sectionList->setFrameShape(QFrame::NoFrame);
	m_sectionList->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
	sideLayout->addWidget(m_sectionList, 1);
	layout->addWidget(side);

	// The section shown, with the strip offering a restart under it
	auto* main = new QWidget(this);
	main->setObjectName("settingsMain");
	auto* mainLayout = new QVBoxLayout(main);
	mainLayout->setContentsMargins(0, 0, 0, 0);
	mainLayout->setSpacing(0);
	m_pages = new QStackedWidget(main);
	mainLayout->addWidget(m_pages, 1);

	m_restartStrip = new QFrame(main);
	m_restartStrip->setObjectName("settingsRestartStrip");
	auto* stripLayout = new QHBoxLayout(m_restartStrip);
	stripLayout->setContentsMargins(14, 8, 14, 8);
	m_restartText = new QLabel(m_restartStrip);
	m_restartText->setWordWrap(true);
	stripLayout->addWidget(m_restartText, 1);
	auto* later = new QPushButton(tr("Later"), m_restartStrip);
	later->setObjectName("settingsRestartLater");
	auto* restart = new QPushButton(tr("Restart now"), m_restartStrip);
	restart->setObjectName("settingsRestartNow");
	stripLayout->addWidget(later);
	stripLayout->addWidget(restart);
	connect(later, &QPushButton::clicked, m_restartStrip, &QWidget::hide);
	connect(restart, &QPushButton::clicked, this, &SetupDialog::restartNow);
	m_restartStrip->hide();
	auto* stripMargin = new QHBoxLayout;
	stripMargin->setContentsMargins(24, 0, 24, 16);
	stripMargin->addWidget(m_restartStrip);
	mainLayout->addLayout(stripMargin);
	layout->addWidget(main, 1);

	buildGeneral();
	buildAudio();
	buildMidi();
	buildRecording();
	buildPlugins();
	buildFolders();
	buildAppearance();
	buildShortcuts();
	buildAdvanced();

	connect(m_sectionList, &QListWidget::currentRowChanged, m_pages, &QStackedWidget::setCurrentIndex);
	connect(m_search, &QLineEdit::textChanged, this, &SetupDialog::search);

	// The tabs it had, to their sections
	const auto first = [&] {
		switch (tabToOpen)
		{
		case ConfigTab::PerformanceSettings: return 8;
		case ConfigTab::AudioSettings: return 1;
		case ConfigTab::MidiSettings: return 2;
		case ConfigTab::PathsSettings: return 5;
		case ConfigTab::ShortcutsSettings: return 7;
		case ConfigTab::GeneralSettings: break;
		}
		return 0;
	}();
	m_sectionList->setCurrentRow(first);

	if (page)
	{
		// A plain widget inside the page (a QDialog makes itself a window of its own even with a parent)
		setWindowFlags(Qt::Widget);
		setAutoFillBackground(true);
	}
	else { resize(980, 640); }

	m_building = false;
	show();
}




SetupDialog::~SetupDialog()
{
	// What still waited applies now (the page goes as soon as it is left)
	if (m_audioReopen.isActive()) { reopenAudioDevice(); }
	if (m_midiReopen.isActive()) { reopenMidiClient(); }
	if (m_saveConfig.isActive()) { ConfigManager::inst()->saveConfigFile(); }
	Engine::projectJournal()->setJournalling(true);
}




void SetupDialog::accept()
{
	if (m_audioReopen.isActive()) { reopenAudioDevice(); }
	if (m_midiReopen.isActive()) { reopenMidiClient(); }
	if (m_saveConfig.isActive())
	{
		m_saveConfig.stop();
		ConfigManager::inst()->saveConfigFile();
	}
	QDialog::accept();
}




void SetupDialog::reject()
{
	// Nothing to undo: each setting applied as it changed
	accept();
}




auto SetupDialog::addSection(const QString& title, const QString& subtitle) -> int
{
	auto section = Section{};
	section.title = title;

	auto* scroll = new QScrollArea(m_pages);
	scroll->setObjectName("settingsPage");
	scroll->setWidgetResizable(true);
	scroll->setFrameShape(QFrame::NoFrame);
	scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
	auto* content = new QWidget(scroll);
	content->setObjectName("settingsPageContent");
	auto* layout = new QVBoxLayout(content);
	layout->setContentsMargins(24, 18, 24, 18);
	layout->setSpacing(0);
	auto* heading = new QLabel(title, content);
	heading->setObjectName("settingsTitle");
	layout->addWidget(heading);
	if (!subtitle.isEmpty())
	{
		auto* line = new QLabel(subtitle, content);
		line->setObjectName("settingsSubtitle");
		line->setWordWrap(true);
		layout->addWidget(line);
	}
	layout->addSpacing(14);
	section.rows = new QVBoxLayout;
	section.rows->setSpacing(0);
	layout->addLayout(section.rows);
	layout->addStretch(1);
	scroll->setWidget(content);
	section.page = scroll;
	m_pages->addWidget(scroll);

	section.item = new QListWidgetItem(title, m_sectionList);
	m_sections.push_back(section);
	return static_cast<int>(m_sections.size()) - 1;
}




auto SetupDialog::tagFor(Applies applies, QWidget* parent) -> QLabel*
{
	auto* tag = new QLabel(applies == Applies::Now ? tr("applies at once") : tr("asks for a restart"), parent);
	tag->setObjectName(applies == Applies::Now ? "settingsTagNow" : "settingsTagRestart");
	tag->setToolTip(applies == Applies::Now ? tr("Changing it takes effect right away")
		: tr("Changing it takes effect after the program starts again"));
	return tag;
}




auto SetupDialog::addRow(int section, const QString& label, Applies applies, QWidget* control, const QString& hint)
	-> QWidget*
{
	auto* row = new QWidget(m_sections[section].page);
	row->setObjectName("settingsRow");
	auto* layout = new QHBoxLayout(row);
	layout->setContentsMargins(0, 9, 0, 9);
	layout->setSpacing(12);

	// The name and what it takes, in a column of their own; the control, and the line after it (under it when
	// the page is narrow: see fitRows())
	auto* nameColumn = new QWidget(row);
	nameColumn->setFixedWidth(250);
	auto* nameLayout = new QHBoxLayout(nameColumn);
	nameLayout->setContentsMargins(0, 0, 0, 0);
	nameLayout->setSpacing(6);
	auto* name = new QLabel(label, nameColumn);
	name->setObjectName("settingsName");
	name->setWordWrap(true);
	nameLayout->addWidget(name);
	nameLayout->addWidget(tagFor(applies, nameColumn), 0, Qt::AlignVCenter);
	nameLayout->addStretch(1);
	layout->addWidget(nameColumn, 0, Qt::AlignVCenter);

	auto* body = new QBoxLayout(QBoxLayout::LeftToRight);
	body->setSpacing(12);
	control->setParent(row);
	body->addWidget(control, 0, Qt::AlignVCenter | Qt::AlignLeft);
	if (!hint.isEmpty())
	{
		auto* after = new QLabel(hint, row);
		after->setObjectName("settingsHint");
		after->setWordWrap(true);
		body->addWidget(after, 1, Qt::AlignVCenter);
	}
	else { body->addStretch(1); }
	layout->addLayout(body, 1);

	m_sections[section].rows->addWidget(row);
	m_rows.push_back({row, section, folded(label + ' ' + hint + ' ' + textsOf(control)), body});
	return row;
}




auto SetupDialog::addBlock(int section, const QString& label, Applies applies, QWidget* block, const QString& hint)
	-> QWidget*
{
	auto* row = new QWidget(m_sections[section].page);
	row->setObjectName("settingsRow");
	auto* layout = new QVBoxLayout(row);
	layout->setContentsMargins(0, 9, 0, 9);
	layout->setSpacing(6);
	auto* top = new QHBoxLayout;
	top->setSpacing(6);
	auto* name = new QLabel(label, row);
	name->setObjectName("settingsName");
	top->addWidget(name);
	top->addWidget(tagFor(applies, row));
	if (!hint.isEmpty())
	{
		auto* after = new QLabel(hint, row);
		after->setObjectName("settingsHint");
		after->setWordWrap(true);
		top->addSpacing(12);
		top->addWidget(after, 1);
	}
	else { top->addStretch(1); }
	layout->addLayout(top);
	block->setParent(row);
	layout->addWidget(block);
	m_sections[section].rows->addWidget(row);
	m_rows.push_back({row, section, folded(label + ' ' + hint + ' ' + textsOf(block))});
	return row;
}




void SetupDialog::set(const QString& cls, const QString& attribute, const QString& value)
{
	if (m_building) { return; }
	ConfigManager::inst()->setValue(cls, attribute, value);
	m_saveConfig.start();
}




void SetupDialog::restartFor(const QString& what)
{
	if (m_building) { return; }
	if (!m_restartNeeded.contains(what)) { m_restartNeeded << what; }
	m_restartText->setText(tr("%1: changes after the program restarts.").arg(m_restartNeeded.join(", ")));
	m_restartStrip->show();
}




auto SetupDialog::checkSetting(const QString& cls, const QString& attribute, bool fallback, bool inverted)
	-> QWidget*
{
	auto* box = new QCheckBox;
	const auto saved = ConfigManager::inst()->value(cls, attribute, fallback != inverted ? "1" : "0");
	box->setChecked((saved.toInt() != 0) != inverted);
	connect(box, &QCheckBox::toggled, this, [this, cls, attribute, inverted](bool on) {
		set(cls, attribute, on != inverted ? "1" : "0");
	});
	return box;
}




auto SetupDialog::pathSetting(const QString& value, const QString& caption, bool file, const QString& filter,
	std::function<void(const QString&)> save) -> QWidget*
{
	auto* holder = new QWidget;
	holder->setMinimumWidth(380);
	auto* layout = new QHBoxLayout(holder);
	layout->setContentsMargins(0, 0, 0, 0);
	layout->setSpacing(6);
	auto* edit = new QLineEdit(QDir::toNativeSeparators(value), holder);
	layout->addWidget(edit, 1);
	auto* choose = new QPushButton(embed::getIconPixmap("project_open", 16, 16), "", holder);
	choose->setFixedSize(28, 24);
	choose->setToolTip(caption);
	layout->addWidget(choose);
	// Saved when the typing ends (each letter would be a folder that does not exist), or when chosen
	connect(edit, &QLineEdit::editingFinished, this, [this, edit, save] {
		if (m_building) { return; }
		save(QDir::fromNativeSeparators(edit->text()));
		m_saveConfig.start();
	});
	connect(choose, &QPushButton::clicked, this, [this, edit, caption, file, filter, save] {
		const auto chosen = file ? FileDialog::getOpenFileName(this, caption, edit->text(), filter)
			: FileDialog::getExistingDirectory(this, caption, edit->text());
		if (chosen.isEmpty()) { return; }
		edit->setText(QDir::toNativeSeparators(chosen));
		save(chosen);
		m_saveConfig.start();
	});
	return holder;
}




// Geral
void SetupDialog::buildGeneral()
{
	const auto s = addSection(tr("General"), tr("Projects and how the program behaves."));

	addRow(s, tr("Reopen the last project on startup"), Applies::Now, checkSetting("app", "openlastproject", false));
	addRow(s, tr("Compress project files"), Applies::Now, checkSetting("app", "nommpz", true, true),
		tr("Saved as .mmpz (smaller); off: .mmp, plain XML"));
	addRow(s, tr("Keep a backup when saving"), Applies::Now, checkSetting("app", "disablebackup", true, true),
		tr("The previous version is kept beside the project"));
	addRow(s, tr("Autosave"), Applies::Now, [this] {
		auto* box = checkSetting("ui", "enableautosave", true);
		// Started or stopped now (see MainWindow::updateAutosave())
		connect(static_cast<QCheckBox*>(box), &QCheckBox::toggled, this, [] {
			if (getGUI() && getGUI()->mainWindow()) { getGUI()->mainWindow()->updateAutosave(); }
		});
		return box;
	}(), tr("Two seconds after each change, also while playing or recording; after a crash it is offered when the "
		"program starts"));
	addRow(s, tr("New projects use the analog mixer"), Applies::Now, checkSetting("mixer", "newprojectanalog", false),
		tr("Off: the digital one"));

	auto* longAudio = new QSpinBox;
	longAudio->setRange(1, 3600);
	longAudio->setSuffix(tr(" s"));
	longAudio->setValue(static_cast<int>(SampleTrack::longAudioSeconds()));
	connect(longAudio, &QSpinBox::valueChanged, this,
		[this](int seconds) { set("ui", "longaudioseconds", QString::number(seconds)); });
	addRow(s, tr("Long audio files become audio tracks"), Applies::Now, longAudio,
		tr("Longer than this, an audio file dropped in the project is an audio track; shorter, a sample"));

	addRow(s, tr("Previews play to the end"), Applies::Now, checkSetting("ui", "letpreviewsfinish", false),
		tr("A sample previewed in the browser goes on when the mouse button is released"));
	addRow(s, tr("Ask before deleting a track"), Applies::Now, checkSetting("ui", "trackdeletionwarning", true));
	addRow(s, tr("Ask before deleting a mixer channel in use"), Applies::Now,
		checkSetting("ui", "mixerchanneldeletionwarning", true));

	// The language: the translations found, by their own names
	auto* language = new QComboBox;
	auto current = ConfigManager::inst()->value("app", "language");
	auto codes = QStringList{};
	for (auto fileName : QDir{ConfigManager::inst()->localeDir()}.entryList({"*.qm"}))
	{
		fileName.truncate(fileName.lastIndexOf('.'));
		const auto locale = QLocale{fileName};
		if (locale.language() == QLocale::C) { continue; }
		auto name = locale.nativeLanguageName();
		if (name.isEmpty()) { name = QLocale::languageToString(locale.language()); }
		codes << fileName;
		language->addItem(name, fileName);
	}
	if (current.isEmpty())
	{
		// Unset: the system's, if there is a translation of it
		const auto system = QLocale::system().name().left(2);
		current = codes.contains(system) ? system : "en";
	}
	language->setCurrentIndex(std::max(0, language->findData(current)));
	connect(language, &QComboBox::currentIndexChanged, this, [this, language] {
		set("app", "language", language->currentData().toString());
		restartFor(tr("The language"));
	});
	addRow(s, tr("Language"), Applies::Restart, language);
}




// Áudio
void SetupDialog::buildAudio()
{
	const auto s = addSection(tr("Audio"), tr("Everything here applies as it changes: the device opens again by "
		"itself."));

	// The drivers built here, each with its own settings
	auto* drivers = new QWidget;
	auto* driversLayout = new QVBoxLayout(drivers);
	driversLayout->setContentsMargins(0, 0, 0, 0);
	const auto add = [&](const QString& name, AudioDeviceSetupWidget* widget) {
		m_audioSetupWidgets[name] = widget;
		widget->setParent(drivers);
		widget->setFlat(true);
		widget->setTitle(QString{});
		widget->hide();
		driversLayout->addWidget(widget);
	};
#ifdef LMMS_HAVE_JACK
	add(AudioJack::name(), new AudioJackSetupWidget(drivers));
#endif
#ifdef LMMS_HAVE_ALSA
	add(AudioAlsa::name(), new AudioAlsaSetupWidget(drivers));
#endif
#ifdef LMMS_HAVE_PULSEAUDIO
	add(AudioPulseAudio::name(), new AudioPulseAudioSetupWidget(drivers));
#endif
#ifdef LMMS_BUILD_APPLE
	add(AudioCoreAudio::name(), new AudioCoreAudioSetupWidget(drivers));
#endif
#ifdef LMMS_HAVE_PORTAUDIO
	add(AudioPortAudio::name(), new AudioPortAudioSetupWidget(drivers));
#endif
#ifdef LMMS_HAVE_SDL
	add(AudioSdl::name(), new AudioSdlSetupWidget(drivers));
#endif
#ifdef LMMS_HAVE_SNDIO
	add(AudioSndio::name(), new AudioSndioSetupWidget(drivers));
#endif
	add(AudioDummy::name(), new AudioDummySetupWidget(drivers));

	m_audioInterfaces = new QComboBox;
	m_audioInterfaces->setMinimumWidth(260);
	auto names = QMap<QString, QString>{}; // (sorted by what is shown)
	for (auto it = m_audioSetupWidgets.begin(); it != m_audioSetupWidgets.end(); ++it)
	{
		names[AudioDeviceSetupWidget::tr(it.key().toUtf8())] = it.key();
	}
	for (auto it = names.begin(); it != names.end(); ++it) { m_audioInterfaces->addItem(it.key(), it.value()); }

	// No driver saved (or one not built here): the one in use
	auto driver = ConfigManager::inst()->value("audioengine", "audiodev");
	if (!m_audioSetupWidgets.contains(driver))
	{
		driver = Engine::audioEngine()->audioDevName();
		ConfigManager::inst()->setValue("audioengine", "audiodev", driver);
	}
	m_audioInterfaces->setCurrentIndex(std::max(0, m_audioInterfaces->findData(driver)));
	showAudioInterface(driver);
	connect(m_audioInterfaces, &QComboBox::currentIndexChanged, this, [this] {
		showAudioInterface(m_audioInterfaces->currentData().toString());
		audioSettingChanged();
	});
	addRow(s, tr("Audio system"), Applies::Now, m_audioInterfaces, tr("JACK, PortAudio, SDL... as the system has them"));
	m_audioDeviceRow = addBlock(s, tr("Device"), Applies::Now, drivers);
	showAudioInterface(driver);

	// A driver's own settings: each change opens the device again (a text when its typing ends)
	for (auto* widget : drivers->findChildren<QWidget*>())
	{
		if (auto* box = qobject_cast<QComboBox*>(widget))
		{
			connect(box, &QComboBox::currentIndexChanged, this, &SetupDialog::audioSettingChanged);
		}
		else if (auto* spin = qobject_cast<QSpinBox*>(widget))
		{
			connect(spin, &QSpinBox::valueChanged, this, &SetupDialog::audioSettingChanged);
		}
		else if (auto* edit = qobject_cast<QLineEdit*>(widget); edit && !qobject_cast<QSpinBox*>(edit->parent()))
		{
			connect(edit, &QLineEdit::editingFinished, this, &SetupDialog::audioSettingChanged);
		}
		else if (auto* button = qobject_cast<QAbstractButton*>(widget); button && button->isCheckable())
		{
			connect(button, &QAbstractButton::toggled, this, &SetupDialog::audioSettingChanged);
		}
	}

	// The sample rate: one of the rates the program runs at
	auto* rates = new QWidget;
	rates->setObjectName("settingsRates");
	auto* ratesLayout = new QHBoxLayout(rates);
	ratesLayout->setContentsMargins(0, 0, 0, 0);
	ratesLayout->setSpacing(4);
	m_sampleRates = new QButtonGroup(this);
	m_sampleRates->setExclusive(true);
	const auto savedRate = ConfigManager::inst()->value("audioengine", "samplerate").toInt();
	for (const auto rate : SUPPORTED_SAMPLERATES)
	{
		auto* button = new QPushButton(QLocale{}.toString(rate / 1000.0, 'g', 4), rates);
		button->setObjectName("settingsRate");
		button->setCheckable(true);
		button->setChecked(rate == savedRate || (savedRate <= 0 && rate == SUPPORTED_SAMPLERATES.front()));
		m_sampleRates->addButton(button, rate);
		ratesLayout->addWidget(button);
	}
	if (!m_sampleRates->checkedButton()) { m_sampleRates->buttons().front()->setChecked(true); }
	ratesLayout->addWidget(new QLabel(tr("kHz"), rates));
	connect(m_sampleRates, &QButtonGroup::idClicked, this, [this](int rate) {
		set("audioengine", "samplerate", QString::number(rate));
		updateBufferLabel();
		audioSettingChanged();
	});
	addRow(s, tr("Sample rate"), Applies::Now, rates, tr("New projects; a project opens at its own rate"));

	// The buffer, with the latency the device has with it
	auto* buffer = new QWidget;
	auto* bufferLayout = new QHBoxLayout(buffer);
	bufferLayout->setContentsMargins(0, 0, 0, 0);
	bufferLayout->setSpacing(10);
	m_bufferSlider = new QSlider(Qt::Horizontal, buffer);
	m_bufferSlider->setMinimumWidth(180);
	m_bufferSlider->setRange(1, MAXIMUM_BUFFER_SIZE / BufferSizeResolution);
	m_bufferSlider->setPageStep(8);
	m_bufferSlider->setValue(ConfigManager::inst()->value("audioengine", "framesperaudiobuffer",
		QString::number(DEFAULT_BUFFER_SIZE)).toInt() / BufferSizeResolution);
	m_bufferLabel = new QLabel(buffer);
	m_bufferLabel->setObjectName("settingsValue");
	m_bufferLabel->setMinimumWidth(90);
	auto* reset = new QPushButton(embed::getIconPixmap("reload"), "", buffer);
	reset->setFixedSize(28, 24);
	reset->setToolTip(tr("Back to the default (%1)").arg(DEFAULT_BUFFER_SIZE));
	bufferLayout->addWidget(m_bufferSlider);
	bufferLayout->addWidget(m_bufferLabel);
	bufferLayout->addWidget(reset);
	connect(reset, &QPushButton::clicked, this,
		[this] { m_bufferSlider->setValue(DEFAULT_BUFFER_SIZE / BufferSizeResolution); });
	connect(m_bufferSlider, &QSlider::valueChanged, this, [this](int value) {
		// Above the default's step, in steps of it (32, 64... then 256, 512...)
		const int step = DEFAULT_BUFFER_SIZE / BufferSizeResolution;
		if (value > step && value % step)
		{
			const int rest = value % step;
			m_bufferSlider->setValue(rest < step / 2 ? value - rest : value + step - rest);
			return;
		}
		set("audioengine", "framesperaudiobuffer", QString::number(value * BufferSizeResolution));
		updateBufferLabel();
		audioSettingChanged();
	});
	m_latencyLabel = new QLabel;
	m_latencyLabel->setObjectName("settingsHint");
	m_latencyLabel->setWordWrap(true);
	addRow(s, tr("Buffer size"), Applies::Now, buffer);
	// (the latency in the line after the buffer, as a hint, changing with it)
	m_rows.back().body->insertWidget(1, m_latencyLabel, 1);
	m_bufferWarning = new QLabel;
	m_bufferWarning->setObjectName("settingsWarning");
	m_bufferWarning->setWordWrap(true);
	m_sections[s].rows->addWidget(m_bufferWarning);
	updateBufferLabel();
	updateLatencyLabel();
	connect(Engine::audioEngine(), &AudioEngine::sampleRateChanged, this, &SetupDialog::updateLatencyLabel);

	m_audioStatus = new QLabel;
	m_audioStatus->setObjectName("settingsWarning");
	m_audioStatus->setWordWrap(true);
	m_audioStatus->hide();
	m_sections[s].rows->addWidget(m_audioStatus);
}




void SetupDialog::showAudioInterface(const QString& name)
{
	for (auto it = m_audioSetupWidgets.begin(); it != m_audioSetupWidgets.end(); ++it)
	{
		it.value()->setVisible(it.key() == name);
	}
	// (a driver with nothing of its own to set: no empty block)
	if (m_audioDeviceRow)
	{
		const auto* widget = m_audioSetupWidgets.value(name);
		m_audioDeviceRow->setVisible(widget && !widget->findChildren<QWidget*>().isEmpty());
	}
}




void SetupDialog::audioSettingChanged()
{
	if (m_building) { return; }
	m_audioReopen.start();
}




void SetupDialog::reopenAudioDevice()
{
	m_audioReopen.stop();
	// The driver, its own settings (each widget saves its own), the rate and the buffer: the device opens with them
	ConfigManager::inst()->setValue("audioengine", "audiodev", m_audioInterfaces->currentData().toString());
	for (auto* widget : m_audioSetupWidgets) { widget->saveSettings(); }
	ConfigManager::inst()->saveConfigFile();
	const auto told = Engine::audioEngine()->reopenAudioDevice();
	m_audioStatus->setText(told);
	m_audioStatus->setVisible(!told.isEmpty());
	updateLatencyLabel();
}




void SetupDialog::updateBufferLabel()
{
	const auto frames = m_bufferSlider->value() * BufferSizeResolution;
	m_bufferLabel->setText(tr("%1 frames").arg(frames));
	auto warnings = QStringList{};
	// Not a power of 2 under 256 (above it, the program works in periods of 256 and plugins never see more)
	if ((frames & (frames - 1)) != 0 && frames < 256)
	{
		warnings << tr("Not a power of 2 (32, 64, 128, 256): some plugins may not be available.");
	}
	if (frames <= 32) { warnings << tr("32 frames or fewer: some plugins may not be available."); }
	m_bufferWarning->setText(warnings.join(' '));
	m_bufferWarning->setVisible(!warnings.isEmpty());
	updateLatencyLabel();
}




void SetupDialog::updateLatencyLabel()
{
	if (!m_latencyLabel) { return; }
	const auto* engine = Engine::audioEngine();
	const auto rate = static_cast<double>(engine->outputSampleRate());
	const auto frames = m_bufferSlider->value() * BufferSizeResolution;
	const auto* device = engine->audioDev();
	const bool waiting = m_audioReopen.isActive()
		|| frames != static_cast<int>(engine->framesPerAudioBuffer());
	if (device && device->outputLatency() > 0 && !waiting)
	{
		// What the device says it takes, from the program to the output
		m_latencyLabel->setText(tr("%1 ms of output latency (measured by the device)")
			.arg(QLocale{}.toString(1000.0 * device->outputLatency() / rate, 'f', 1)));
	}
	else
	{
		m_latencyLabel->setText(tr("%1 ms of buffer (the device adds its own)")
			.arg(QLocale{}.toString(1000.0 * frames / rate, 'f', 1)));
	}
}




// MIDI
void SetupDialog::buildMidi()
{
	const auto s = addSection(tr("MIDI"), tr("Applies as it changes: the driver opens again, and the tracks keep "
		"their devices."));

	auto* drivers = new QWidget;
	auto* driversLayout = new QVBoxLayout(drivers);
	driversLayout->setContentsMargins(0, 0, 0, 0);
	const auto add = [&](const QString& name, MidiSetupWidget* widget) {
		m_midiSetupWidgets[name] = widget;
		widget->setParent(drivers);
		widget->setFlat(true);
		widget->setTitle(QString{});
		widget->hide();
		driversLayout->addWidget(widget);
	};
#ifdef LMMS_HAVE_ALSA
	add(MidiAlsaSeq::name(), MidiSetupWidget::create<MidiAlsaSeq>(drivers));
	add(MidiAlsaRaw::name(), MidiSetupWidget::create<MidiAlsaRaw>(drivers));
#endif
#ifdef LMMS_HAVE_JACK
	add(MidiJack::name(), MidiSetupWidget::create<MidiJack>(drivers));
#endif
#ifdef LMMS_HAVE_SNDIO
	add(MidiSndio::name(), MidiSetupWidget::create<MidiSndio>(drivers));
#endif
#ifdef LMMS_HAVE_WINMM
	add(MidiWinMM::name(), MidiSetupWidget::create<MidiWinMM>(drivers));
#endif
#ifdef LMMS_BUILD_APPLE
	add(MidiApple::name(), MidiSetupWidget::create<MidiApple>(drivers));
#endif
	add(MidiDummy::name(), MidiSetupWidget::create<MidiDummy>(drivers));

	m_midiInterfaces = new QComboBox;
	m_midiInterfaces->setMinimumWidth(260);
	auto names = QMap<QString, QString>{};
	for (auto it = m_midiSetupWidgets.begin(); it != m_midiSetupWidgets.end(); ++it)
	{
		names[MidiSetupWidget::tr(it.key().toUtf8())] = it.key();
	}
	for (auto it = names.begin(); it != names.end(); ++it) { m_midiInterfaces->addItem(it.key(), it.value()); }
	auto driver = ConfigManager::inst()->value("audioengine", "mididev");
	if (!m_midiSetupWidgets.contains(driver))
	{
		driver = Engine::audioEngine()->midiClientName();
		ConfigManager::inst()->setValue("audioengine", "mididev", driver);
	}
	m_midiInterfaces->setCurrentIndex(std::max(0, m_midiInterfaces->findData(driver)));
	showMidiInterface(driver);
	connect(m_midiInterfaces, &QComboBox::currentIndexChanged, this, [this] {
		showMidiInterface(m_midiInterfaces->currentData().toString());
		midiSettingChanged();
	});
	addRow(s, tr("MIDI system"), Applies::Now, m_midiInterfaces);
	m_midiDeviceRow = addBlock(s, tr("Device"), Applies::Now, drivers);
	showMidiInterface(driver);
	for (auto* edit : drivers->findChildren<QLineEdit*>())
	{
		connect(edit, &QLineEdit::editingFinished, this, &SetupDialog::midiSettingChanged);
	}

	// A controller that follows the selected track
	m_assignableMidiDevices = new QComboBox;
	m_assignableMidiDevices->setMinimumWidth(260);
	const auto fillAssignable = [this] {
		const auto saved = ConfigManager::inst()->value("midi", "midiautoassign");
		const QSignalBlocker blocker{m_assignableMidiDevices};
		m_assignableMidiDevices->clear();
		m_assignableMidiDevices->addItem(tr("None"), "none");
		if (auto* client = Engine::audioEngine()->midiClient(); client && !client->isRaw())
		{
			for (const auto& port : client->readablePorts()) { m_assignableMidiDevices->addItem(port, port); }
		}
		else { m_assignableMidiDevices->addItem(tr("All"), "all"); }
		m_assignableMidiDevices->setCurrentIndex(std::max(0, m_assignableMidiDevices->findData(saved)));
	};
	fillAssignable();
	connect(m_assignableMidiDevices, &QComboBox::currentIndexChanged, this,
		[this] { set("midi", "midiautoassign", m_assignableMidiDevices->currentData().toString()); });
	addRow(s, tr("Controller for the selected track"), Applies::Now, m_assignableMidiDevices,
		tr("This device plays the track selected, whichever it is"));
	// (the devices of the driver opened again)
	connect(&m_midiReopen, &QTimer::timeout, this, fillAssignable);

	addRow(s, tr("Quantize notes recorded in the Piano Roll"), Applies::Now, checkSetting("midi", "autoquantize", false),
		tr("Off: recorded at the finest resolution"));

	m_midiStatus = new QLabel;
	m_midiStatus->setObjectName("settingsWarning");
	m_midiStatus->setWordWrap(true);
	m_midiStatus->hide();
	m_sections[s].rows->addWidget(m_midiStatus);
}




void SetupDialog::showMidiInterface(const QString& name)
{
	for (auto it = m_midiSetupWidgets.begin(); it != m_midiSetupWidgets.end(); ++it)
	{
		it.value()->setVisible(it.key() == name);
	}
	if (m_midiDeviceRow)
	{
		const auto* widget = m_midiSetupWidgets.value(name);
		m_midiDeviceRow->setVisible(widget && !widget->configSection().isEmpty());
	}
}




void SetupDialog::midiSettingChanged()
{
	if (m_building) { return; }
	m_midiReopen.start();
}




void SetupDialog::reopenMidiClient()
{
	m_midiReopen.stop();
	// The driver and its devices' settings, as they were: it opens again only if they changed
	const auto settings = [this] {
		auto values = QStringList{ConfigManager::inst()->value("audioengine", "mididev")};
		for (const auto* widget : m_midiSetupWidgets)
		{
			if (!widget->configSection().isEmpty())
			{
				values << ConfigManager::inst()->value(widget->configSection(), "device");
			}
		}
		return values;
	};
	const auto before = settings();
	ConfigManager::inst()->setValue("audioengine", "mididev", m_midiInterfaces->currentData().toString());
	for (auto* widget : m_midiSetupWidgets) { widget->saveSettings(); }
	ConfigManager::inst()->saveConfigFile();
	if (settings() == before) { return; }
	// The tracks' ports move to it (see AudioEngine::reopenMidiClient())
	const auto told = Engine::audioEngine()->reopenMidiClient();
	m_midiStatus->setText(told);
	m_midiStatus->setVisible(!told.isEmpty());
}




// Gravação
void SetupDialog::buildRecording()
{
	const auto s = addSection(tr("Recording"), tr("Recording audio and MIDI, and the metronome."));

	auto* calibration = new QWidget;
	auto* calibrationLayout = new QHBoxLayout(calibration);
	calibrationLayout->setContentsMargins(0, 0, 0, 0);
	m_latencyOffset = new QSpinBox(calibration);
	m_latencyOffset->setRange(-48000, 48000);
	m_latencyOffset->setSuffix(tr(" frames"));
	m_latencyOffset->setValue(ConfigManager::inst()->value("recording", "latencyoffset", "0").toInt());
	connect(m_latencyOffset, &QSpinBox::valueChanged, this,
		[this](int frames) { set("recording", "latencyoffset", QString::number(frames)); });
	auto* measure = new QPushButton(tr("Measure..."), calibration);
	measure->setToolTip(tr("Plays a pulse and records it back through a cable from an output to an input, and sets "
		"the calibration from where it arrives"));
	connect(measure, &QPushButton::clicked, this, &SetupDialog::measureLatency);
	calibrationLayout->addWidget(m_latencyOffset);
	calibrationLayout->addWidget(measure);
	addRow(s, tr("Latency calibration"), Applies::Now, calibration,
		tr("Added to the latency the device reports, so that recordings line up with what was heard"));

	auto* preRoll = new QComboBox;
	for (const int bars : {0, 1, 2, 4}) { preRoll->addItem(bars == 0 ? tr("Off") : tr("%n bar(s)", "", bars), bars); }
	preRoll->setCurrentIndex(std::max(0, preRoll->findData(
		ConfigManager::inst()->value("recording", "prerollbars", "0").toInt())));
	connect(preRoll, &QComboBox::currentIndexChanged, this,
		[this, preRoll] { set("recording", "prerollbars", preRoll->currentData().toString()); });
	addRow(s, tr("Pre-roll"), Applies::Now, preRoll, tr("Bars played before the recording starts"));
	addRow(s, tr("Count-in"), Applies::Now, checkSetting("recording", "countin", false),
		tr("The metronome clicks during the pre-roll"));
	addRow(s, tr("Takes in lanes"), Applies::Now, checkSetting("recording", "lanes", true),
		tr("Each pass on a lane of its own, the latest on top; off: the new take replaces what it covers"));

	// The metronome (heard while monitoring, never in an export): applied to it as it changes
	const auto metronomeChanged = [] { Engine::getSong()->metronome().loadSettings(); };
	auto* when = new QComboBox;
	when->addItem(tr("While playing and recording"), 0);
	when->addItem(tr("Only while recording"), 1);
	when->setCurrentIndex(ConfigManager::inst()->value("metronome", "onlyrecording", "0").toInt() != 0 ? 1 : 0);
	connect(when, &QComboBox::currentIndexChanged, this, [this, when, metronomeChanged] {
		set("metronome", "onlyrecording", when->currentData().toString());
		metronomeChanged();
	});
	addRow(s, tr("Metronome"), Applies::Now, when);
	auto* volume = new QDoubleSpinBox;
	volume->setRange(-60.0, 12.0);
	volume->setDecimals(1);
	volume->setSuffix(tr(" dB"));
	volume->setValue(ConfigManager::inst()->value("metronome", "volume", "0").toDouble());
	connect(volume, &QDoubleSpinBox::valueChanged, this, [this, metronomeChanged](double db) {
		set("metronome", "volume", QString::number(db));
		metronomeChanged();
	});
	addRow(s, tr("Metronome volume"), Applies::Now, volume);
	auto* accent = checkSetting("metronome", "accent", true);
	connect(static_cast<QCheckBox*>(accent), &QCheckBox::toggled, this, metronomeChanged);
	addRow(s, tr("Accent on the first beat"), Applies::Now, accent);
}




void SetupDialog::measureLatency()
{
	const auto answer = QMessageBox::information(this, tr("Latency calibration"),
		tr("Connect an output of the audio interface to an input with a cable (output 1 to input 1, for example), "
			"and turn the input's gain to a middle level. A short pulse will play and be recorded back."),
		QMessageBox::Ok | QMessageBox::Cancel);
	if (answer != QMessageBox::Ok) { return; }
	// (with the device as the settings have it now)
	if (m_audioReopen.isActive()) { reopenAudioDevice(); }

	auto calibration = LatencyCalibration{};
	calibration.start();
	// (the audio engine plays and listens meanwhile: a second and a half)
	auto waited = QElapsedTimer{};
	waited.start();
	while (!calibration.isDone() && waited.elapsed() < 5000)
	{
		QCoreApplication::processEvents(QEventLoop::ExcludeUserInputEvents, 50);
		QThread::msleep(10);
	}
	const auto result = calibration.result();
	if (!calibration.isDone() || !result.found)
	{
		QMessageBox::warning(this, tr("Latency calibration"), tr("The pulse did not come back. Check the cable from "
			"the output to the input, that the device has inputs, and the input's gain."));
		return;
	}
	m_latencyOffset->setValue(static_cast<int>(std::clamp<long long>(result.offset, m_latencyOffset->minimum(),
		m_latencyOffset->maximum())));
	QMessageBox::information(this, tr("Latency calibration"),
		tr("The pulse came back after %1 frames: the calibration is %2 frames.").arg(result.delay).arg(result.offset));
}




// Plugins
void SetupDialog::buildPlugins()
{
	const auto s = addSection(tr("Plugins"), tr("Plugin windows, and the plugins that were not loaded after a "
		"crash."));

	auto* embedding = new QComboBox;
	const auto methods = ConfigManager::availableVstEmbedMethods();
	embedding->addItem(tr("No embedding"), "none");
	if (methods.contains("qt")) { embedding->addItem(tr("Embed using Qt API"), "qt"); }
	if (methods.contains("win32")) { embedding->addItem(tr("Embed using native Win32 API"), "win32"); }
	if (methods.contains("xembed")) { embedding->addItem(tr("Embed using XEmbed protocol"), "xembed"); }
	embedding->setCurrentIndex(std::max(0, embedding->findData(ConfigManager::inst()->vstEmbedMethod())));
	addRow(s, tr("VST windows"), Applies::Now, embedding, tr("For the plugin windows opened from now on"));
	auto* onTop = addRow(s, tr("Plugin windows on top"), Applies::Now, checkSetting("ui", "vstalwaysontop", false),
		tr("When not embedded"));
	onTop->setVisible(embedding->currentData().toString() == "none");
	connect(embedding, &QComboBox::currentIndexChanged, this, [this, embedding, onTop] {
		set("ui", "vstembedmethod", embedding->currentData().toString());
		onTop->setVisible(embedding->currentData().toString() == "none");
	});

	// The plugins a crash happened in: not loaded until allowed here
	const auto suspects = PluginGuard::suspects();
	if (suspects.isEmpty()) { return; }
	auto* block = new QWidget;
	auto* blockLayout = new QVBoxLayout(block);
	blockLayout->setContentsMargins(0, 0, 0, 0);
	auto* list = new QListWidget(block);
	list->setSelectionMode(QAbstractItemView::ExtendedSelection);
	for (const auto& identity : suspects)
	{
		auto* item = new QListWidgetItem(PluginGuard::displayName(identity), list);
		item->setData(Qt::UserRole, identity);
		item->setToolTip(identity);
	}
	list->setMaximumHeight(list->sizeHintForRow(0) * std::min(6, static_cast<int>(suspects.size()))
		+ 2 * list->frameWidth() + 4);
	blockLayout->addWidget(list);
	auto* allow = new QPushButton(tr("Load again"), block);
	allow->setToolTip(tr("The selected plugins load again, from the next time a project opens"));
	allow->setEnabled(false);
	connect(list, &QListWidget::itemSelectionChanged, allow,
		[list, allow] { allow->setEnabled(!list->selectedItems().isEmpty()); });
	connect(allow, &QPushButton::clicked, this, [list] {
		for (auto* item : list->selectedItems())
		{
			PluginGuard::clearSuspect(item->data(Qt::UserRole).toString());
			delete item;
		}
	});
	blockLayout->addWidget(allow, 0, Qt::AlignLeft);
	addBlock(s, tr("Not loaded since a crash"), Applies::Now, block,
		tr("The program closed unexpectedly while they ran; projects keep their settings"));
}




// Pastas
void SetupDialog::buildFolders()
{
	const auto s = addSection(tr("Folders"), tr("Where the program keeps and looks for files."));
	auto* config = ConfigManager::inst();

	addRow(s, tr("Working folder"), Applies::Restart, pathSetting(config->workingDir(),
		tr("Choose the LinuxMusic working directory"), false, {}, [this](const QString& dir) {
			ConfigManager::inst()->setWorkingDir(dir);
			restartFor(tr("The working folder"));
		}), tr("Projects, samples, presets and recordings"));
	addRow(s, tr("VST plugins"), Applies::Now, pathSetting(config->vstDir(),
		tr("Choose your VST plugins directory"), false, {},
		[](const QString& dir) { ConfigManager::inst()->setVSTDir(dir); }));
	// (several folders, separated by commas: the chooser adds one)
	addRow(s, tr("LADSPA plugins"), Applies::Restart, pathSetting(config->ladspaDir(),
		tr("Choose your LADSPA plugins directory"), false, {}, [this](const QString& dir) {
			ConfigManager::inst()->setLADSPADir(dir);
			restartFor(tr("The LADSPA folders"));
		}), tr("Folders separated by commas; looked through when the program starts"));
	addRow(s, tr("SoundFonts"), Applies::Now, pathSetting(config->sf2Dir(), tr("Choose your SF2 directory"), false, {},
		[](const QString& dir) { ConfigManager::inst()->setSF2Dir(dir); }));
#ifdef LMMS_HAVE_FLUIDSYNTH
	addRow(s, tr("Default SoundFont"), Applies::Now, pathSetting(config->sf2File(), tr("Choose your default SF2"),
		true, tr("SoundFont 2 files (*.sf2)"), [](const QString& file) { ConfigManager::inst()->setSF2File(file); }),
		tr("For new Sf2 Player instruments"));
#endif
	addRow(s, tr("GIG files"), Applies::Now, pathSetting(config->gigDir(), tr("Choose your GIG directory"), false, {},
		[](const QString& dir) { ConfigManager::inst()->setGIGDir(dir); }));
}




// Aparência
void SetupDialog::buildAppearance()
{
	const auto s = addSection(tr("Appearance"), tr("How the windows look and behave."));

	addRow(s, tr("Tooltips"), Applies::Now, checkSetting("tooltips", "disabled", true, true));
	addRow(s, tr("Compact track headers"), Applies::Now, checkSetting("ui", "compacttrackbuttons", false),
		tr("Narrower; each header can still be widened by dragging its edge"));
	addRow(s, tr("Explorer on the right"), Applies::Now, checkSetting("ui", "sidebaronright", false));
	addRow(s, tr("One instrument window at a time"), Applies::Now,
		checkSetting("ui", "oneinstrumenttrackwindow", false), tr("Opening one closes the other"));
	addRow(s, tr("Note names in the Piano Roll"), Applies::Now, checkSetting("ui", "printnotelabels", false));
	addRow(s, tr("Fader ticks"), Applies::Now, checkSetting("ui", "showfaderticks", false));
	addRow(s, tr("Smooth scrolling in the Song Editor"), Applies::Now, checkSetting("ui", "smoothscroll", false));
	addRow(s, tr("Playback position in the AudioFileProcessor"), Applies::Now, checkSetting("ui", "animateafp", true));

	const auto choice = [this](const QString& cls, const QString& attribute, const QString& fallback,
		const std::vector<std::pair<QString, QString>>& items) {
		auto* box = new QComboBox;
		for (const auto& [text, value] : items) { box->addItem(text, value); }
		box->setCurrentIndex(std::max(0, box->findData(ConfigManager::inst()->value(cls, attribute, fallback))));
		connect(box, &QComboBox::currentIndexChanged, this,
			[this, box, cls, attribute] { set(cls, attribute, box->currentData().toString()); });
		return box;
	};
	addRow(s, tr("Detached windows"), Applies::Now, choice("ui", "detachbehavior", "show", {
		{tr("Attach and show when closed"), "show"},
		{tr("Attach and hide when closed"), "hide"},
		{tr("Always detached"), "detached"}}));
	addRow(s, tr("Loop edit mode"), Applies::Now, choice("app", "loopmarkermode", "dual", {
		{tr("Dual-button"), "dual"},
		{tr("Grab closest"), "closest"},
		{tr("Handles"), "handles"}}));
	addRow(s, tr("Following the playhead"), Applies::Now, choice("ui", "autoscroll",
		TimeLineWidget::AutoScrollSteppedString, {
		{tr("Off"), TimeLineWidget::AutoScrollDisabledString},
		{tr("Stepped (once it leaves the view)"), TimeLineWidget::AutoScrollSteppedString},
		{tr("Continuous (kept in the middle)"), TimeLineWidget::AutoScrollContinuousString}}),
		tr("The editors follow the playhead this way (each one's button changes it there)"));

	addRow(s, tr("Theme folder"), Applies::Restart, pathSetting(ConfigManager::inst()->themeDir(),
		tr("Choose your theme directory"), false, {}, [this](const QString& dir) {
			ConfigManager::inst()->setThemeDir(dir);
			restartFor(tr("The theme"));
		}));
}




// Teclado e atalhos
void SetupDialog::buildShortcuts()
{
	const auto s = addSection(tr("Keyboard and shortcuts"), tr("Each action's key, applied as it is set. The other "
		"keys of the editors and the menus' keys are fixed."));
	auto* table = new QTableWidget(static_cast<int>(Shortcuts::actions().size()), 3);
	table->setObjectName("settingsShortcuts");
	table->setHorizontalHeaderLabels({tr("Action"), tr("Key"), QString{}});
	table->verticalHeader()->hide();
	table->horizontalHeader()->setSectionResizeMode(0, QHeaderView::Stretch);
	table->horizontalHeader()->setSectionResizeMode(1, QHeaderView::Fixed);
	table->setColumnWidth(1, 220);
	table->setSelectionMode(QAbstractItemView::NoSelection);
	auto row = 0;
	for (const auto& action : Shortcuts::actions())
	{
		auto* label = new QTableWidgetItem(Shortcuts::text(action));
		label->setFlags(Qt::ItemIsEnabled);
		table->setItem(row, 0, label);
		auto* edit = new QKeySequenceEdit(Shortcuts::key(action.id), table);
		edit->setMaximumSequenceLength(1);
		table->setCellWidget(row, 1, edit);
		connect(edit, &QKeySequenceEdit::keySequenceChanged, this, [this, id = action.id](const QKeySequence& key) {
			if (m_building) { return; }
			Shortcuts::setKey(id, key);
			m_saveConfig.start();
		});
		auto* reset = new QPushButton(tr("Default"), table);
		connect(reset, &QPushButton::clicked, edit, [edit, fallback = action.fallback] { edit->setKeySequence(fallback); });
		table->setCellWidget(row, 2, reset);
		++row;
	}
	table->setMinimumHeight(std::min(row, 14) * table->verticalHeader()->defaultSectionSize()
		+ table->horizontalHeader()->height() + 4);
	addBlock(s, tr("Shortcuts"), Applies::Now, table);
	// (the actions' names, for the search)
	auto names = QStringList{};
	for (const auto& action : Shortcuts::actions()) { names << Shortcuts::text(action); }
	m_rows.back().searchText += ' ' + folded(names.join(' '));
}




// Avançado
void SetupDialog::buildAdvanced()
{
	const auto s = addSection(tr("Advanced"), tr("Safety and performance."));

	auto* sanitize = checkSetting("audioengine", "sanitizemix", true);
	connect(static_cast<QCheckBox*>(sanitize), &QCheckBox::toggled, this,
		[](bool on) { Engine::audioEngine()->setSanitizationEnabled(on); });
	addRow(s, tr("Mix sanitization"), Applies::Now, sanitize, tr("Silences a channel when a plugin or track makes "
		"something that is not a number (NaN or infinity), at a small cost"));
	addRow(s, tr("Effects keep running without input"), Applies::Now, checkSetting("ui", "disableautoquit", true),
		tr("Off: an effect whose input went silent stops after its tail, sparing the processor"));
}




void SetupDialog::resizeEvent(QResizeEvent* event)
{
	QDialog::resizeEvent(event);
	fitRows();
}




void SetupDialog::fitRows()
{
	// Wide: the line after a control beside it, as in the design; narrow: under it (nothing cut)
	const auto direction = m_pages->width() >= 900 ? QBoxLayout::LeftToRight : QBoxLayout::TopToBottom;
	for (const auto& row : m_rows)
	{
		if (row.body && row.body->direction() != direction) { row.body->setDirection(direction); }
	}
}




auto SetupDialog::sectionTitles(bool shownOnly) const -> QStringList
{
	auto titles = QStringList{};
	for (const auto& section : m_sections)
	{
		if (!shownOnly || !section.item->isHidden()) { titles << section.title; }
	}
	return titles;
}




auto SetupDialog::currentSection() const -> QString
{
	const auto row = m_sectionList->currentRow();
	return row >= 0 && row < static_cast<int>(m_sections.size()) ? m_sections[row].title : QString{};
}




void SetupDialog::search(const QString& text)
{
	if (m_search->text() != text)
	{
		m_search->setText(text); // (comes back here)
		return;
	}
	const auto wanted = folded(text.trimmed());
	auto found = std::vector<int>(m_sections.size(), 0);
	for (const auto& row : m_rows)
	{
		// A section's title finds all of its settings
		const bool shown = wanted.isEmpty() || row.searchText.contains(wanted)
			|| folded(m_sections[row.section].title).contains(wanted);
		const bool empty = (row.widget == m_audioDeviceRow || row.widget == m_midiDeviceRow) && !rowHasContent(row.widget);
		row.widget->setVisible(shown && !empty);
		if (shown && !empty) { ++found[row.section]; }
	}
	for (std::size_t i = 0; i < m_sections.size(); ++i)
	{
		m_sections[i].item->setHidden(found[i] == 0);
	}
	// The section shown is one that has what was searched
	if (const auto current = m_sectionList->currentRow(); current < 0 || found[current] == 0)
	{
		const auto first = std::find_if(found.begin(), found.end(), [](int count) { return count > 0; });
		if (first != found.end()) { m_sectionList->setCurrentRow(static_cast<int>(first - found.begin())); }
	}
}




auto SetupDialog::rowHasContent(const QWidget* row) const -> bool
{
	if (row == m_audioDeviceRow)
	{
		const auto* widget = m_audioSetupWidgets.value(m_audioInterfaces->currentData().toString());
		return widget && !widget->findChildren<QWidget*>().isEmpty();
	}
	if (row == m_midiDeviceRow)
	{
		const auto* widget = m_midiSetupWidgets.value(m_midiInterfaces->currentData().toString());
		return widget && !widget->configSection().isEmpty();
	}
	return true;
}




void SetupDialog::restartNow()
{
	auto* window = getGUI() ? getGUI()->mainWindow() : nullptr;
	if (!window) { return; }
	accept();
	// The project saved (or not, as the user says), and opened again by the program started again
	GuiApplication::restartAfterQuit(window);
}


} // namespace lmms::gui
