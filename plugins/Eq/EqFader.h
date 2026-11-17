/* eqfader.h - defination of EqFader class.
*
* Copyright (c) 2014 David French <dave/dot/french3/at/googlemail/dot/com>
*
* This file is part of LMMS - https://lmms.io
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

#ifndef EQFADER_H
#define EQFADER_H
#include <QList>
#include <QWidget>

#include "AtomicPeak.h"
#include "EffectControls.h"
#include "Fader.h"
#include "GuiApplication.h"
#include "MainWindow.h"
#include "TextFloat.h"

namespace lmms::gui
{


class EqFader : public Fader
{

public:
	Q_OBJECT
public:
	EqFader(FloatModel* model, const QString& name, QWidget* parent, AtomicPeak* lPeak, AtomicPeak* rPeak) :
		Fader(model, name, parent, false)
	{
		setMinimumSize( 23, 116 );
		setMaximumSize( 23, 116 );
		resize( 23, 116 );
		m_lPeak = lPeak;
		m_rPeak = rPeak;
		connect( getGUI()->mainWindow(), SIGNAL( periodicUpdate() ), this, SLOT( updateVuMeters() ) );
		m_model = model;
		setPeak_L( 0 );
		setPeak_R( 0 );
	}



	~EqFader() override = default;


private slots:

	void updateVuMeters()
	{
		const float opl = getPeak_L();
		const float opr = getPeak_R();
		const float fallOff = 1.07f;
		// A peak above what the meter shows is shown (and taken); else the meter falls
		setPeak_L(m_lPeak->takeAbove(opl).value_or(opl / fallOff));
		setPeak_R(m_rPeak->takeAbove(opr).value_or(opr / fallOff));
		update();
	}




private:
	AtomicPeak* m_lPeak;
	AtomicPeak* m_rPeak;
	FloatModel* m_model;

};


} // namespace lmms::gui

#endif // EQFADER_H
