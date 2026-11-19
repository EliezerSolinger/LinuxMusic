/*
 * DeskKnob.cpp - a knob drawn like the knobs of an analog mixing desk
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

#include "DeskKnob.h"

#include <cmath>
#include <numbers>

#include <QPainter>

#include "FontHelper.h"

namespace lmms::gui
{

DeskKnob::DeskKnob(const QColor& cap, const QString& label, const QString& name, QWidget* parent)
	: Knob{KnobType::Styled, parent, name}, m_cap{cap}, m_label{label}
{
	setFixedSize(Width, Diameter + LabelHeight);
}




void DeskKnob::paintEvent(QPaintEvent*)
{
	auto painter = QPainter{this};
	painter.setRenderHint(QPainter::Antialiasing);
	const auto center = QPointF{width() / 2.0, Diameter / 2.0};

	// Skirt, then the cap with light from the top left
	painter.setPen(Qt::NoPen);
	painter.setBrush(QColor{18, 20, 23});
	painter.drawEllipse(center, Diameter / 2.0, Diameter / 2.0);
	auto gradient = QRadialGradient{center - QPointF{2.0, 2.5}, Diameter / 2.0};
	gradient.setColorAt(0.0, m_cap.lighter(150));
	gradient.setColorAt(1.0, m_cap.darker(130));
	painter.setBrush(gradient);
	const double cap = Diameter / 2.0 - 2.5;
	painter.drawEllipse(center, cap, cap);

	// The pointer: 0 at the top, the range from -135 to 135 degrees
	const auto knobModel = model();
	const float position = (knobModel->inverseScaledValue(knobModel->value()) - knobModel->minValue())
		/ (knobModel->maxValue() - knobModel->minValue());
	const double angle = (position - 0.5) * TotalAngle * std::numbers::pi / 180.0;
	const auto direction = QPointF{std::sin(angle), -std::cos(angle)};
	painter.setPen(QPen{QColor{245, 245, 245}, 1.6, Qt::SolidLine, Qt::RoundCap});
	painter.drawLine(center + direction * 1.5, center + direction * (cap + 1.0));

	painter.setPen(QColor{170, 178, 190});
	painter.setFont(adjustedToPixelSize(font(), 7));
	painter.drawText(QRect{0, Diameter, width(), LabelHeight}, Qt::AlignCenter, m_label);
}

} // namespace lmms::gui
