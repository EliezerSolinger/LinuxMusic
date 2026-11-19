/*
 * AmplifikationLiteControls.h - Amplifikation Lite's knobs, switches and panel
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

#ifndef LMMS_AMPLIFIKATION_LITE_CONTROLS_H
#define LMMS_AMPLIFIKATION_LITE_CONTROLS_H

#include <QPixmap>

#include "AmpLiteDsp.h"
#include "EffectControlDialog.h"
#include "EffectControls.h"
#include "FloatModelEditorBase.h"

namespace lmms
{

class AmplifikationLiteEffect;

class AmplifikationLiteControls : public EffectControls
{
	Q_OBJECT
public:
	enum class Cabinet { Internal, Off, External };

	explicit AmplifikationLiteControls(AmplifikationLiteEffect* effect);

	void saveSettings(QDomDocument& doc, QDomElement& parent) override;
	void loadSettings(const QDomElement& parent) override;
	//! (as DataFile::ELEMENTS_WITH_RESOURCES names it: its "ir" is a file the project keeps with it)
	auto nodeName() const -> QString override { return "amplifikationlite"; }
	auto createView() -> gui::EffectControlDialog* override;
	auto controlCount() -> int override { return 9; }

	//! [audio thread] The knobs, for the model
	auto settings() const -> amplite::AmpLiteDsp::Settings;
	auto cabinet() const -> Cabinet;

	//! Loads @p path as the external cabinet's impulse response (empty: none); @returns an error, or nothing
	auto setImpulseResponse(const QString& path) -> QString;
	auto impulseResponse() const -> const QString& { return m_impulseResponse; }

	// The original's parameters, with its ranges (0..1) and defaults
	FloatModel m_input;
	FloatModel m_channel;	//!< 0 clean, 1 rhythm, 2 lead
	FloatModel m_gain;
	FloatModel m_bass;
	FloatModel m_middle;
	FloatModel m_treble;
	FloatModel m_presence;
	FloatModel m_volume;
	FloatModel m_cabinet;	//!< 0 INT, 1 OFF, 2 EXT

signals:
	void impulseResponseChanged();

private:
	AmplifikationLiteEffect* m_effect;
	QString m_impulseResponse;
};


namespace gui
{

//! A knob drawn from the original's film strip (frames stacked vertically)
class FilmstripKnob : public FloatModelEditorBase
{
	Q_OBJECT
public:
	FilmstripKnob(const QPixmap& strip, int frameSize, QWidget* parent);
protected:
	void paintEvent(QPaintEvent* event) override;
private:
	QPixmap m_strip;
	int m_frameSize;
	int m_frames;
};


//! The original's three-position toggle switch: a click on its upper, middle or lower third
class ToggleSwitch : public FloatModelEditorBase
{
	Q_OBJECT
public:
	ToggleSwitch(const QPixmap& strip, QWidget* parent);
protected:
	void paintEvent(QPaintEvent* event) override;
	void mousePressEvent(QMouseEvent* event) override;
	void mouseMoveEvent(QMouseEvent* event) override;
	void mouseReleaseEvent(QMouseEvent* event) override;
	void wheelEvent(QWheelEvent* event) override;
private:
	QPixmap m_strip;
};


class AmplifikationLiteControlDialog : public EffectControlDialog
{
	Q_OBJECT
public:
	explicit AmplifikationLiteControlDialog(AmplifikationLiteControls* controls);

private:
	void chooseImpulseResponse();
	void showAbout();
	AmplifikationLiteControls* m_controls;
};

} // namespace gui

} // namespace lmms

#endif // LMMS_AMPLIFIKATION_LITE_CONTROLS_H
