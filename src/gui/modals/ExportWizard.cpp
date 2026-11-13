/*
 * ExportWizard.cpp - exports the song step by step: range, formats, processing, destination
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

#include "ExportWizard.h"

#include <atomic>
#include <thread>

#include <QCheckBox>
#include <QComboBox>
#include <QDesktopServices>
#include <QDoubleSpinBox>
#include <QEventLoop>
#include <QFileDialog>
#include <QFileInfo>
#include <QFormLayout>
#include <QHBoxLayout>
#include <QLabel>
#include <QLineEdit>
#include <QMessageBox>
#include <QProgressDialog>
#include <QPushButton>
#include <QRadioButton>
#include <QSpinBox>
#include <QTemporaryDir>
#include <QTimer>
#include <QUrl>
#include <QVBoxLayout>

#include "ConfigManager.h"
#include "Engine.h"
#include "OutputSettings.h"
#include "RenderManager.h"
#include "Song.h"

namespace lmms::gui
{

ExportWizard::ExportWizard(QWidget* parent) :
	QWizard(parent)
{
	setWindowTitle(tr("Export the song"));
	setWizardStyle(QWizard::ModernStyle);
	setOption(QWizard::NoBackButtonOnStartPage);
	addPage(rangePage());
	addPage(formatsPage());
	addPage(processingPage());
	addPage(destinationPage());
	setButtonText(QWizard::FinishButton, tr("Export"));
}




auto ExportWizard::rangePage() -> QWizardPage*
{
	auto page = new QWizardPage;
	page->setTitle(tr("What to export"));
	auto layout = new QVBoxLayout(page);
	m_wholeSong = new QRadioButton(tr("The whole song"), page);
	m_cycleRange = new QRadioButton(tr("The cycle's range (selected on the ruler)"), page);
	m_wholeSong->setChecked(true);
	m_asLoop = new QCheckBox(tr("As a loop: without the tail at the end (reverbs, releases)"), page);
	auto repeats = new QHBoxLayout;
	m_repeats = new QSpinBox(page);
	m_repeats->setRange(1, 64);
	repeats->addWidget(new QLabel(tr("Times:"), page));
	repeats->addWidget(m_repeats);
	repeats->addStretch(1);
	for (auto widget : std::initializer_list<QWidget*>{m_wholeSong, m_cycleRange, m_asLoop}) { layout->addWidget(widget); }
	layout->addLayout(repeats);
	layout->addStretch(1);
	return page;
}




auto ExportWizard::formatsPage() -> QWizardPage*
{
	auto page = new QWizardPage;
	page->setTitle(tr("Formats"));
	page->setSubTitle(tr("Several at once, each a file in the folder chosen at the end."));
	auto layout = new QFormLayout(page);
	const auto combo = [page](std::initializer_list<std::pair<QString, int>> items, int current) {
		auto box = new QComboBox(page);
		for (const auto& [text, value] : items) { box->addItem(text, value); }
		box->setCurrentIndex(box->findData(current));
		return box;
	};

	m_wav = new QCheckBox("WAV", page);
	m_wav->setChecked(true);
	m_wavBits = combo({{tr("16 bits"), 16}, {tr("24 bits"), 24}, {tr("32 bits float"), 32}}, 24);
	layout->addRow(m_wav, m_wavBits);
	m_flac = new QCheckBox("FLAC", page);
	m_flacBits = combo({{tr("16 bits"), 16}, {tr("24 bits"), 24}}, 24);
	layout->addRow(m_flac, m_flacBits);
	m_ogg = new QCheckBox("OGG Vorbis", page);
	m_oggQuality = new QSpinBox(page);
	m_oggQuality->setRange(0, 10);
	m_oggQuality->setValue(6);
	m_oggQuality->setPrefix(tr("quality "));
	layout->addRow(m_ogg, m_oggQuality);
	m_mp3 = new QCheckBox("MP3", page);
	m_mp3Rate = combo({{"128 kbps", 128}, {"192 kbps", 192}, {"256 kbps", 256}, {"320 kbps", 320}}, 320);
	m_mp3->setEnabled(ExportProcessor::canWrite(ExportProcessor::Format::Mp3));
	m_mp3Rate->setEnabled(m_mp3->isEnabled());
	layout->addRow(m_mp3, m_mp3Rate);
	m_sampleRate = combo({{"44100 Hz", 44100}, {"48000 Hz", 48000}, {"88200 Hz", 88200}, {"96000 Hz", 96000}}, 48000);
	layout->addRow(tr("Sample rate"), m_sampleRate);
	return page;
}




auto ExportWizard::processingPage() -> QWizardPage*
{
	auto page = new QWizardPage;
	page->setTitle(tr("Processing"));
	page->setSubTitle(tr("Applied to every format."));
	auto layout = new QFormLayout(page);
	m_normalization = new QComboBox(page);
	m_normalization->addItem(tr("None"), static_cast<int>(ExportProcessor::Normalization::None));
	m_normalization->addItem(tr("Peak (dBFS)"), static_cast<int>(ExportProcessor::Normalization::Peak));
	m_normalization->addItem(tr("Loudness (LUFS, ITU-R BS.1770)"), static_cast<int>(ExportProcessor::Normalization::Loudness));
	m_target = new QDoubleSpinBox(page);
	m_target->setRange(-60.0, 0.0);
	m_target->setDecimals(1);
	m_target->setValue(-1.0);
	m_target->setEnabled(false);
	connect(m_normalization, &QComboBox::currentIndexChanged, this, [this] {
		const auto kind = static_cast<ExportProcessor::Normalization>(m_normalization->currentData().toInt());
		m_target->setEnabled(kind != ExportProcessor::Normalization::None);
		// The usual targets: -1 dBFS for a peak, -14 LUFS for streaming
		if (kind == ExportProcessor::Normalization::Peak) { m_target->setValue(-1.0); }
		if (kind == ExportProcessor::Normalization::Loudness) { m_target->setValue(-14.0); }
	});
	layout->addRow(tr("Normalize"), m_normalization);
	layout->addRow(tr("Target"), m_target);
	m_fadeIn = new QDoubleSpinBox(page);
	m_fadeOut = new QDoubleSpinBox(page);
	for (auto box : {m_fadeIn, m_fadeOut})
	{
		box->setRange(0.0, 30.0);
		box->setSuffix(" s");
		box->setDecimals(2);
	}
	layout->addRow(tr("Fade in"), m_fadeIn);
	layout->addRow(tr("Fade out"), m_fadeOut);
	m_dither = new QCheckBox(tr("Dither (TPDF) to 16 and 24 bits"), page);
	m_dither->setChecked(true);
	layout->addRow(QString{}, m_dither);
	return page;
}




auto ExportWizard::destinationPage() -> QWizardPage*
{
	auto page = new QWizardPage;
	page->setTitle(tr("Where"));
	auto layout = new QFormLayout(page);
	const auto project = Engine::getSong()->projectFileName();
	m_folder = new QLineEdit(project.isEmpty() ? ConfigManager::inst()->userProjectsDir()
		: QFileInfo{project}.absolutePath() + "/exports", page);
	auto browse = new QPushButton(tr("Choose..."), page);
	connect(browse, &QPushButton::clicked, this, [this] {
		const auto folder = QFileDialog::getExistingDirectory(this, tr("Folder for the exported files"), m_folder->text());
		if (!folder.isEmpty()) { m_folder->setText(folder); }
	});
	auto folderRow = new QHBoxLayout;
	folderRow->addWidget(m_folder, 1);
	folderRow->addWidget(browse);
	layout->addRow(tr("Folder"), folderRow);
	m_name = new QLineEdit(project.isEmpty() ? tr("untitled") : QFileInfo{project}.completeBaseName(), page);
	layout->addRow(tr("Name"), m_name);
	return page;
}




auto ExportWizard::settings() const -> ExportProcessor::Settings
{
	auto settings = ExportProcessor::Settings{};
	settings.normalization = static_cast<ExportProcessor::Normalization>(m_normalization->currentData().toInt());
	settings.target = m_target->value();
	settings.fadeIn = m_fadeIn->value();
	settings.fadeOut = m_fadeOut->value();
	settings.dither = m_dither->isChecked();
	settings.folder = m_folder->text();
	settings.name = m_name->text();
	using Format = ExportProcessor::Format;
	if (m_wav->isChecked()) { settings.outputs.push_back({Format::Wav, m_wavBits->currentData().toInt()}); }
	if (m_flac->isChecked()) { settings.outputs.push_back({Format::Flac, m_flacBits->currentData().toInt()}); }
	if (m_ogg->isChecked()) { settings.outputs.push_back({Format::Ogg, 16, 320, m_oggQuality->value() / 10.0}); }
	if (m_mp3->isChecked()) { settings.outputs.push_back({Format::Mp3, 16, m_mp3Rate->currentData().toInt()}); }
	return settings;
}




auto ExportWizard::render(const QString& path, int sampleRate) -> bool
{
	auto* song = Engine::getSong();
	song->setExportLoop(m_asLoop->isChecked());
	song->setRenderBetweenMarkers(m_cycleRange->isChecked());
	song->setLoopRenderCount(m_repeats->value());

	const auto output = OutputSettings{static_cast<sample_rate_t>(sampleRate), 320, OutputSettings::BitDepth::Depth32Bit,
		OutputSettings::StereoMode::Stereo};
	auto renderer = RenderManager{output, ProjectRenderer::ExportFileFormat::Wave, path};
	auto progress = QProgressDialog{tr("Rendering the song..."), tr("Cancel"), 0, 100, this};
	progress.setWindowModality(Qt::ApplicationModal);
	progress.setMinimumDuration(0);
	auto loop = QEventLoop{};
	auto cancelled = false;
	connect(&renderer, &RenderManager::progressChanged, &progress, &QProgressDialog::setValue);
	connect(&renderer, &RenderManager::finished, &loop, &QEventLoop::quit);
	connect(&progress, &QProgressDialog::canceled, &loop, [&] {
		cancelled = true;
		renderer.abortProcessing();
		loop.quit();
	});
	renderer.renderProject();
	loop.exec();
	return !cancelled;
}




void ExportWizard::setFolderAndName(const QString& folder, const QString& name)
{
	m_folder->setText(folder);
	m_name->setText(name);
}




void ExportWizard::accept()
{
	const auto settings = this->settings();
	if (settings.outputs.empty())
	{
		QMessageBox::information(this, windowTitle(), tr("Choose at least one format."));
		return;
	}
	if (settings.name.trimmed().isEmpty() || settings.folder.trimmed().isEmpty())
	{
		QMessageBox::information(this, windowTitle(), tr("Choose the folder and the name."));
		return;
	}
	hide();

	// 1. The song, once, as a float WAV
	auto temporary = QTemporaryDir{};
	const auto rendering = temporary.filePath("render.wav");
	if (!temporary.isValid() || !render(rendering, m_sampleRate->currentData().toInt()))
	{
		QWizard::reject();
		return;
	}

	// 2. Each format from it, in a thread, with the progress
	auto cancel = std::atomic<bool>{false};
	auto done = std::atomic<double>{0.0};
	auto finished = std::atomic<bool>{false};
	auto result = ExportProcessor::Result{};
	auto worker = std::thread{[&] {
		result = ExportProcessor::process(rendering, settings, cancel, [&](double p) { done = p; });
		finished = true;
	}};
	auto progress = QProgressDialog{tr("Writing the files..."), tr("Cancel"), 0, 1000, parentWidget()};
	progress.setWindowModality(Qt::ApplicationModal);
	progress.setMinimumDuration(0);
	auto loop = QEventLoop{};
	auto poll = QTimer{};
	connect(&poll, &QTimer::timeout, &loop, [&] {
		progress.setValue(static_cast<int>(1000 * done.load()));
		if (finished) { loop.quit(); }
	});
	connect(&progress, &QProgressDialog::canceled, &loop, [&] { cancel = true; });
	poll.start(50);
	loop.exec();
	worker.join();
	progress.hide();

	if (!result.error.isEmpty())
	{
		QMessageBox::warning(parentWidget(), windowTitle(), result.error);
	}
	else if (!result.cancelled)
	{
		const auto box = QMessageBox::information(parentWidget(), windowTitle(),
			tr("Exported:\n%1").arg(result.written.join('\n')), QMessageBox::Open | QMessageBox::Ok, QMessageBox::Ok);
		if (box == QMessageBox::Open) { QDesktopServices::openUrl(QUrl::fromLocalFile(settings.folder)); }
	}
	QWizard::accept();
}

} // namespace lmms::gui
