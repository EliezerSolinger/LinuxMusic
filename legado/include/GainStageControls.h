/*
 * GainStageControls.h - the controls of a gain stage: its model, its GAIN pot and its SAT and CLIP LEDs
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

#ifndef LMMS_GUI_GAIN_STAGE_CONTROLS_H
#define LMMS_GUI_GAIN_STAGE_CONTROLS_H

#include <functional>
#include <vector>

#include <QCoreApplication>
#include <QPushButton>
#include <QWidget>

#include "AutomatableModel.h"
#include "DeskKnob.h"
#include "GainStage.h"

namespace lmms::gui
{

//! The gain stage's model (GainStage::Model): a click goes to the next one, the right button lists them; lit blue for
//! the op-amps and amber for the tubes
class DriveButton : public QPushButton
{
	Q_DECLARE_TR_FUNCTIONS(DriveButton)
public:
	using Model = GainStage::Model;

	DriveButton(IntModel* model, QWidget* parent);

	static auto name(Model model) -> QString;
	static auto description(Model model) -> QString;

protected:
	void contextMenuEvent(QContextMenuEvent*) override;

private:
	void refresh();

	IntModel* m_model;
};

/**
 * The SAT and CLIP LEDs of a gain stage, as on a desk: SAT glows as the stage saturates, brighter the harder it is
 * driven, and fades out; CLIP lights red as it comes within 3 dB of its limit, and holds a moment.
 * With a tube, a small lamp takes SAT's place: its heater glows as the tube saturates, yellow in the light mode and
 * red in the realistic one, and a click on it switches between them.
 */
class SaturationLeds : public QWidget
{
	Q_DECLARE_TR_FUNCTIONS(SaturationLeds)
public:
	SaturationLeds(GainStage* stage, IntModel* drive, BoolModel* realistic, QWidget* parent);

	static constexpr int Width = 50;
	static constexpr int Height = 12;

protected:
	void paintEvent(QPaintEvent*) override;
	void mousePressEvent(QMouseEvent* event) override;

private:
	static constexpr float LedSize = 7.f;
	static constexpr int ClipHold = 10; //!< GUI updates

	auto isTube() const -> bool;
	void refresh();
	void advance();
	void drawLamp(QPainter& painter, const QRectF& area);

	GainStage* m_stage;
	IntModel* m_drive;
	BoolModel* m_realistic;
	float m_saturation = 0.f;	//!< shown, 0 to 1
	int m_clipHold = 0;			//!< GUI updates the CLIP LED stays lit
};

//! The GAIN pot of an analog strip (AnalogChannelStrip::m_gainModel), showing its gain in dB
class GainKnob : public DeskKnob
{
public:
	GainKnob(const QColor& cap, const QString& label, const QString& name, QWidget* parent);

protected:
	auto currentValueToText() -> QString override;
};

/**
 * A button choosing among a few options of an IntModel, as a desk's switch: a click goes to the next one, the right
 * button lists them. Each option may have a "kind", which the style sheet colors the button after.
 */
class ChoiceButton : public QPushButton
{
	Q_DECLARE_TR_FUNCTIONS(ChoiceButton)
public:
	struct Choice
	{
		int value;
		QString name;
		QString description;
		QString kind;
	};

	//! @param what what the button chooses, for its tooltip
	ChoiceButton(IntModel* model, std::vector<Choice> choices, const QString& what, const QString& objectName,
		QWidget* parent);

protected:
	void contextMenuEvent(QContextMenuEvent*) override;

private:
	void refresh();

	IntModel* m_model;
	std::vector<Choice> m_choices;
	QString m_what;
};

//! A desk knob whose value reads as a function says, e.g. in the units of the compressor's model
class TextDeskKnob : public DeskKnob
{
public:
	TextDeskKnob(const QColor& cap, const QString& label, const QString& name, std::function<QString(float)> text,
		QWidget* parent);

protected:
	auto currentValueToText() -> QString override;

private:
	std::function<QString(float)> m_text;
};

//! A plain knob whose value reads as a function says
class TextKnob : public Knob
{
public:
	TextKnob(const QString& label, const QString& name, std::function<QString(float)> text, QWidget* parent);

protected:
	auto currentValueToText() -> QString override;

private:
	std::function<QString(float)> m_text;
};

} // namespace lmms::gui

#endif // LMMS_GUI_GAIN_STAGE_CONTROLS_H
