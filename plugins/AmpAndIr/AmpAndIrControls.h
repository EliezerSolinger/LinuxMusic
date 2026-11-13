/*
 * AmpAndIrControls.h - the Amp & IR plugin's controls and their window
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

#ifndef LMMS_AMP_AND_IR_CONTROLS_H
#define LMMS_AMP_AND_IR_CONTROLS_H

#include "EffectControlDialog.h"
#include "EffectControls.h"

class QLabel;

namespace lmms
{

class AmpAndIrEffect;

class AmpAndIrControls : public EffectControls
{
	Q_OBJECT
public:
	explicit AmpAndIrControls(AmpAndIrEffect* effect);

	void saveSettings(QDomDocument& doc, QDomElement& parent) override;
	void loadSettings(const QDomElement& parent) override;
	//! (as DataFile::ELEMENTS_WITH_RESOURCES names it: its "ir" is a file the project keeps with it)
	auto nodeName() const -> QString override { return "ampir"; }
	auto createView() -> gui::EffectControlDialog* override;
	auto controlCount() -> int override { return 2; }

	//! Loads @p path as the impulse response (empty: none); @returns an error, or nothing
	auto setImpulseResponse(const QString& path) -> QString;
	auto impulseResponse() const -> const QString& { return m_impulseResponse; }
	//! Loads @p path as the amp model (.nam; empty: none); @returns an error, or nothing
	auto setAmpModel(const QString& path) -> QString;
	auto ampModel() const -> const QString& { return m_ampModel; }

	FloatModel m_inputModel;	//!< dB
	FloatModel m_outputModel;	//!< dB

signals:
	void impulseResponseChanged();
	void ampModelChanged();

private:
	AmpAndIrEffect* m_effect;
	QString m_impulseResponse;
	QString m_ampModel;
};


namespace gui
{

class AmpAndIrControlDialog : public EffectControlDialog
{
	Q_OBJECT
public:
	explicit AmpAndIrControlDialog(AmpAndIrControls* controls);

private:
	void chooseImpulseResponse();
	void chooseAmpModel();
	void warn(const QString& error);
	AmpAndIrControls* m_controls;
	QLabel* m_name;
	QLabel* m_ampName;
};

} // namespace gui

} // namespace lmms

#endif // LMMS_AMP_AND_IR_CONTROLS_H
