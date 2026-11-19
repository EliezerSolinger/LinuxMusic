/*
 * LfoControllerDialog.h
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

#ifndef LMMS_GUI_LFO_CONTROLLER_DIALOG_H
#define LMMS_GUI_LFO_CONTROLLER_DIALOG_H

#include "ControllerDialog.h"
#include "LfoController.h"

namespace lmms
{

namespace gui
{

class AutomatableButtonGroup;
class Knob;
class PixmapButton;
class TempoSyncKnob;

class LfoControllerDialog : public ControllerDialog
{
	Q_OBJECT
public:
	LfoControllerDialog( Controller * _controller, QWidget * _parent );
	~LfoControllerDialog() override;


protected:
	void contextMenuEvent( QContextMenuEvent * _me ) override;
	void modelChanged() override;

	LfoController * m_lfo;

	Knob * m_baseKnob;
	TempoSyncKnob * m_speedKnob;
	Knob * m_amountKnob;
	Knob * m_phaseKnob;
	PixmapButton * m_userLfoBtn;
	AutomatableButtonGroup * m_waveBtnGrp;
	AutomatableButtonGroup * m_multiplierBtnGrp;


private:
	PixmapButton * m_userWaveBtn;

private slots:
	void askUserDefWave();

} ;

} // namespace gui

} // namespace lmms

#endif // LMMS_GUI_LFO_CONTROLLER_DIALOG_H
