/*
 * ExportWizard.h - exports the song step by step: range, formats, processing, destination
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

#ifndef LMMS_GUI_EXPORT_WIZARD_H
#define LMMS_GUI_EXPORT_WIZARD_H

#include <optional>
#include <utility>

#include <QWizard>

#include "TimePos.h"

#include "ExportProcessor.h"

class QCheckBox;
class QComboBox;
class QDoubleSpinBox;
class QLineEdit;
class QRadioButton;
class QSpinBox;

namespace lmms
{
class Track;
}

namespace lmms::gui
{

/**
 * The song's export as a wizard (docs/plano-anotacoes-13-11.md, 12.1):
 *   1. the range: the whole song or the cycle's range, how many times, as a loop (without the tail);
 *   2. the formats, several at once (WAV, FLAC, OGG, MP3), and the sample rate;
 *   3. the processing: normalization (to a peak or a loudness), fades, dither;
 *   4. the folder and the name.
 * The song is rendered once (to a float WAV in a temporary folder), then ExportProcessor writes each format from
 * it, with a progress dialog that can cancel.
 */
class ExportWizard : public QWizard
{
	Q_OBJECT
public:
	explicit ExportWizard(QWidget* parent);

	void accept() override;

	//! Where the files go (also chosen on the last page)
	void setFolderAndName(const QString& folder, const QString& name);

	/**
	 * Exports only @p track (the others are muted while it renders), between @p begin and @p end if given (a
	 * clip's span); the range page is skipped then
	 */
	void setOnly(Track* track, std::optional<std::pair<TimePos, TimePos>> span = std::nullopt);

	//! Exports the whole mix between @p begin and @p end (a region's span), named @p name
	void setSpan(TimePos begin, TimePos end, const QString& name);

private:
	auto rangePage() -> QWizardPage*;
	auto formatsPage() -> QWizardPage*;
	auto processingPage() -> QWizardPage*;
	auto destinationPage() -> QWizardPage*;
	auto settings() const -> ExportProcessor::Settings;
	//! Renders the song to @p path; @returns false if cancelled
	auto render(const QString& path, int sampleRate) -> bool;

	QRadioButton* m_wholeSong;
	QRadioButton* m_cycleRange;
	QCheckBox* m_asLoop;
	QSpinBox* m_repeats;

	QCheckBox* m_wav;
	QComboBox* m_wavBits;
	QCheckBox* m_flac;
	QComboBox* m_flacBits;
	QCheckBox* m_ogg;
	QSpinBox* m_oggQuality;
	QCheckBox* m_mp3;
	QComboBox* m_mp3Rate;
	QComboBox* m_sampleRate;

	QComboBox* m_normalization;
	QDoubleSpinBox* m_target;
	QDoubleSpinBox* m_fadeIn;
	QDoubleSpinBox* m_fadeOut;
	QCheckBox* m_dither;

	QLineEdit* m_folder;
	QLineEdit* m_name;

	Track* m_only = nullptr;
	std::optional<std::pair<TimePos, TimePos>> m_span;
};

} // namespace lmms::gui

#endif // LMMS_GUI_EXPORT_WIZARD_H
