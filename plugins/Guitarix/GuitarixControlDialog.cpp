/*
 * GuitarixControlDialog.cpp - the controls of a Guitarix unit
 *
 * This file is part of LinuxMusic, based on LMMS - https://lmms.io
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

#include "GuitarixControlDialog.h"

#include <QGridLayout>

#include "GuitarixEffect.h"
#include "Knob.h"
#include "LedCheckBox.h"

namespace lmms::gui
{

GuitarixControlDialog::GuitarixControlDialog(GuitarixControls* controls) :
	EffectControlDialog(controls)
{
	auto layout = new QGridLayout(this);
	auto column = 0;
	for (auto& parameter : controls->parameters())
	{
		if (parameter.kind.startsWith('B'))
		{
			auto led = new LedCheckBox(parameter.name, this);
			led->setModel(parameter.model.get());
			layout->addWidget(led, 0, column++, Qt::AlignCenter);
			continue;
		}
		auto knob = new Knob(KnobType::Bright26, parameter.name.toUpper(), this);
		knob->setModel(parameter.model.get());
		knob->setHintText(parameter.name + ":", "");
		layout->addWidget(knob, 0, column++, Qt::AlignHCenter);
	}
	layout->setSizeConstraint(QLayout::SetFixedSize);
}

} // namespace lmms::gui
