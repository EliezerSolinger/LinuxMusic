/*
 * GainStageControls.cpp - the controls of a gain stage: its model, its GAIN pot and its SAT and CLIP LEDs
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

#include "GainStageControls.h"

#include <algorithm>

#include <QContextMenuEvent>
#include <QCursor>
#include <QMenu>
#include <QPainter>
#include <QRadialGradient>
#include <QStyle>

#include "AnalogChannelStrip.h"
#include "FontHelper.h"
#include "GuiApplication.h"
#include "MainWindow.h"
#include "lmms_math.h"

namespace lmms::gui
{

DriveButton::DriveButton(IntModel* model, QWidget* parent) : QPushButton{parent}, m_model{model}
{
	setObjectName("btn-drive");
	setFocusPolicy(Qt::NoFocus);
	connect(this, &QPushButton::clicked, this, [this] {
		const auto current = static_cast<Model>(m_model->value());
		const auto& order = GainStage::DisplayOrder;
		const auto it = std::find(order.begin(), order.end(), current);
		const auto next = it == order.end() || it + 1 == order.end() ? order.front() : *(it + 1);
		m_model->setValue(static_cast<int>(next));
	});
	connect(m_model, &IntModel::dataChanged, this, &DriveButton::refresh);
	refresh();
}

auto DriveButton::name(Model model) -> QString
{
	switch (model)
	{
		case Model::Flat: return tr("FLAT");
		case Model::OpAmp4558: return "4558";
		case Model::OpAmpTL072: return "TL072";
		case Model::OpAmp5532: return "5532";
		case Model::OpAmp5534: return "5534";
		case Model::Tube12AX7: return "12AX7";
		case Model::Tube12AX7Vintage: return "12AX7 V";
		case Model::Tube12AX7Hot: return "12AX7 H";
		case Model::Tube12AT7: return "12AT7";
		case Model::Tube12AU7: return "12AU7";
		default: return {};
	}
}

auto DriveButton::description(Model model) -> QString
{
	switch (model)
	{
		case Model::Flat: return tr("Flat: a clean gain stage");
		case Model::OpAmp4558: return tr("4558 op-amp, of many Soundcraft and Mackie desks: soft, slow, even harmonics");
		case Model::OpAmpTL072: return tr("TL072 op-amp, JFET input, of much studio gear: a little earlier, slightly asymmetric");
		case Model::OpAmp5532: return tr("NE5532 op-amp, of SSL, Neve and Amek desks: clean and fast");
		case Model::OpAmp5534: return tr("NE5534 op-amp, the single NE5532: clean and faster");
		case Model::Tube12AX7: return tr("12AX7 tube, from a SPICE simulation");
		case Model::Tube12AX7Vintage: return tr("12AX7 tube, vintage: warm, rich in even harmonics");
		case Model::Tube12AX7Hot: return tr("12AX7 tube, hot: crunchier, stronger in odd harmonics");
		case Model::Tube12AT7: return tr("12AT7 tube: less gain than the 12AX7, bends more gently");
		case Model::Tube12AU7: return tr("12AU7 tube: low gain, the cleanest and most open tube");
		default: return {};
	}
}

void DriveButton::contextMenuEvent(QContextMenuEvent*)
{
	auto menu = QMenu{this};
	for (const auto model : GainStage::DisplayOrder)
	{
		const int value = static_cast<int>(model);
		auto action = menu.addAction(QString{"%1 - %2"}.arg(name(model), description(model)), this,
			[this, value] { m_model->setValue(value); });
		action->setCheckable(true);
		action->setChecked(m_model->value() == value);
	}
	menu.exec(QCursor::pos());
}

void DriveButton::refresh()
{
	const auto model = static_cast<Model>(m_model->value());
	setText(name(model));
	setToolTip(tr("Gain stage: %1 (click for the next one, right button for the list)").arg(description(model)));
	// The style sheet colors the button after its kind
	setProperty("kind", GainStage::isTube(model) ? "tube" : GainStage::isOpAmp(model) ? "opamp" : "flat");
	style()->unpolish(this);
	style()->polish(this);
}




SaturationLeds::SaturationLeds(GainStage* stage, QWidget* parent) : QWidget{parent}, m_stage{stage}
{
	setFixedSize(Width, Height);
	setToolTip(tr("SAT: lights up as the gain stage saturates\nCLIP: lights up as it nears its limit, 3 dB before it"));
	connect(getGUI()->mainWindow(), &MainWindow::periodicUpdate, this, &SaturationLeds::advance);
}

void SaturationLeds::advance()
{
	const auto saturation = m_stage->takeSaturation();
	const bool clip = m_stage->takeClip();
	// Like an LED on a desk: on at once, fading out
	const auto shown = std::max(saturation, m_saturation * 0.6f);
	const auto hold = clip ? ClipHold : std::max(m_clipHold - 1, 0);
	if (shown == m_saturation && hold == m_clipHold) { return; }
	m_saturation = shown < 0.02f ? 0.f : shown;
	m_clipHold = hold;
	update();
}

void SaturationLeds::paintEvent(QPaintEvent*)
{
	auto painter = QPainter{this};
	painter.setRenderHint(QPainter::Antialiasing);
	painter.setFont(adjustedToPixelSize(font(), 6));
	// Unlit, an LED still shows its color, dimly, as its lens does on a desk
	const auto drawLed = [&painter](int x, const QColor& lit, float level, const QString& label) {
		const auto off = QColor::fromRgbF(lit.redF() * 0.28f, lit.greenF() * 0.28f, lit.blueF() * 0.28f);
		const auto color = QColor::fromRgbF(
			off.redF() + (lit.redF() - off.redF()) * level,
			off.greenF() + (lit.greenF() - off.greenF()) * level,
			off.blueF() + (lit.blueF() - off.blueF()) * level);
		const auto led = QRectF{x + 0.5, 1.5, LedSize, LedSize};
		if (level > 0.f)
		{
			auto glow = QRadialGradient{led.center(), LedSize};
			auto halo = lit;
			halo.setAlphaF(0.45 * level);
			glow.setColorAt(0, halo);
			glow.setColorAt(1, Qt::transparent);
			painter.setPen(Qt::NoPen);
			painter.setBrush(glow);
			painter.drawEllipse(led.center(), LedSize, LedSize);
		}
		painter.setPen(QColor{12, 13, 15});
		painter.setBrush(color);
		painter.drawEllipse(led);
		painter.setPen(QColor{150, 158, 168});
		painter.drawText(QRectF{x + LedSize + 2, 0, 20, Height}, Qt::AlignLeft | Qt::AlignVCenter, label);
	};
	drawLed(1, QColor{255, 196, 40}, m_saturation, tr("SAT"));
	drawLed(Width / 2 + 1, QColor{255, 40, 30}, m_clipHold > 0 ? 1.f : 0.f, tr("CLIP"));
}




GainKnob::GainKnob(const QColor& cap, const QString& label, const QString& name, QWidget* parent)
	: DeskKnob{cap, label, name, parent}
{
}

auto GainKnob::currentValueToText() -> QString
{
	const float gain = AnalogChannelStrip::potGain(model()->value());
	if (gain <= 0.f) { return "-inf"; }
	const float db = ampToDbfs(gain);
	return (db > 0.05f ? "+" : "") + QString::number(db, 'f', 1);
}

} // namespace lmms::gui
