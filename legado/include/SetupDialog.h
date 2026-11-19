/*
 * SetupDialog.h - the settings
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

#ifndef LMMS_GUI_SETUP_DIALOG_H
#define LMMS_GUI_SETUP_DIALOG_H

#include <QDialog>
#include <QMap>
#include <QStringList>
#include <QTimer>
#include <functional>
#include <vector>

class QBoxLayout;
class QButtonGroup;
class QComboBox;
class QLabel;
class QLineEdit;
class QListWidget;
class QListWidgetItem;
class QSlider;
class QSpinBox;
class QStackedWidget;
class QVBoxLayout;

namespace lmms::gui
{

class AudioDeviceSetupWidget;
class MidiSetupWidget;

/**
 * The settings (design 08 in docs/design-sugestoes): sections in a list at the side, with a search, and each
 * setting applied as it changes, as far as it can be: the audio device and the MIDI driver open again by
 * themselves (a moment after the last change, so that dragging the buffer opens it once). Each setting says before
 * it is changed whether it applies at once or asks for a restart; the ones that do (the language, the theme...)
 * show a strip offering to restart, which saves the project and opens it again.
 */
class SetupDialog : public QDialog
{
	Q_OBJECT

public:
	//! The section to open at (kept for the callers of the tabs it had)
	enum class ConfigTab
	{
		GeneralSettings,
		PerformanceSettings,
		AudioSettings,
		MidiSettings,
		PathsSettings,
		ShortcutsSettings
	};

	//! @param page shows it inside @p page (the settings page of the main window) rather than as a window
	SetupDialog(ConfigTab tabToOpen = ConfigTab::GeneralSettings, QWidget* page = nullptr);
	~SetupDialog() override;

	//! What changed and asks for a restart (shown in the strip at the bottom)
	auto restartNeeded() const -> const QStringList& { return m_restartNeeded; }

	//! The section titles (in their order) and which are shown by the search
	auto sectionTitles(bool shownOnly = false) const -> QStringList;
	auto currentSection() const -> QString;
	//! Searches the settings (as typing in the search box)
	void search(const QString& text);

public slots:
	//! Applies what is still waiting (the device opening again a moment after the last change), and closes
	void accept() override;
	void reject() override;

private:
	enum class Applies
	{
		Now,		//!< as it changes
		Restart		//!< after the program starts again
	};

	struct Section
	{
		QString title;
		QWidget* page = nullptr;
		QVBoxLayout* rows = nullptr;
		QListWidgetItem* item = nullptr;
	};

	struct Row
	{
		QWidget* widget = nullptr;
		int section = 0;
		QString searchText;	//!< what the search finds it by (label, hint, choices), folded
		QBoxLayout* body = nullptr;	//!< the control and the line after it (see fitRows())
	};

	auto addSection(const QString& title, const QString& subtitle) -> int;
	//! A setting: its name with what applying it takes, the control, and a line after it (empty for none)
	auto addRow(int section, const QString& label, Applies applies, QWidget* control, const QString& hint = {})
		-> QWidget*;
	//! A setting whose control is a whole block (the device's own settings, the shortcuts table)
	auto addBlock(int section, const QString& label, Applies applies, QWidget* block, const QString& hint = {})
		-> QWidget*;
	auto tagFor(Applies applies, QWidget* parent) -> QLabel*;

	//! A check box saved in @p cls/@p attribute (as "1"/"0", or inverted), applied as it changes
	auto checkSetting(const QString& cls, const QString& attribute, bool fallback, bool inverted = false) -> QWidget*;
	//! Sets a value; the parts of the program that use it hear ConfigManager::valueChanged
	void set(const QString& cls, const QString& attribute, const QString& value);
	void restartFor(const QString& what);

	void buildGeneral();
	void buildAudio();
	void buildMidi();
	void buildRecording();
	void buildPlugins();
	void buildFolders();
	void buildAppearance();
	void buildShortcuts();
	void buildAdvanced();

	//! A folder or file chooser saved with @p save (and a restart asked when @p applies says so)
	auto pathSetting(const QString& value, const QString& caption, bool file, const QString& filter,
		std::function<void(const QString&)> save) -> QWidget*;

	void showAudioInterface(const QString& name);
	void showMidiInterface(const QString& name);
	void audioSettingChanged();
	void reopenAudioDevice();
	void midiSettingChanged();
	void reopenMidiClient();
	void updateBufferLabel();
	void updateLatencyLabel();
	void measureLatency();
	void restartNow();
	void resizeEvent(QResizeEvent* event) override;
	void fitRows();
	auto rowHasContent(const QWidget* row) const -> bool;

	std::vector<Section> m_sections;
	std::vector<Row> m_rows;
	QListWidget* m_sectionList = nullptr;
	QStackedWidget* m_pages = nullptr;
	QLineEdit* m_search = nullptr;

	QWidget* m_restartStrip = nullptr;
	QLabel* m_restartText = nullptr;
	QStringList m_restartNeeded;

	QComboBox* m_audioInterfaces = nullptr;
	QWidget* m_audioDeviceRow = nullptr;
	QWidget* m_midiDeviceRow = nullptr;
	QMap<QString, AudioDeviceSetupWidget*> m_audioSetupWidgets;	//!< by driver name
	QButtonGroup* m_sampleRates = nullptr;
	QSlider* m_bufferSlider = nullptr;
	QLabel* m_bufferLabel = nullptr;
	QLabel* m_latencyLabel = nullptr;
	QLabel* m_bufferWarning = nullptr;
	QTimer m_audioReopen;	//!< the device opens again a moment after the last change
	QLabel* m_audioStatus = nullptr;	//!< what opening it again said (another device, another rate...)
	QSpinBox* m_latencyOffset = nullptr;

	QComboBox* m_midiInterfaces = nullptr;
	QMap<QString, MidiSetupWidget*> m_midiSetupWidgets;	//!< by driver name
	QTimer m_midiReopen;
	QLabel* m_midiStatus = nullptr;
	QComboBox* m_assignableMidiDevices = nullptr;

	QTimer m_saveConfig;	//!< the settings file written a moment after the last change
	bool m_building = true;	//!< (the controls being set up: nothing to apply)
};


} // namespace lmms::gui

#endif // LMMS_GUI_SETUP_DIALOG_H
