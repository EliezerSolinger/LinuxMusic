/*
 * MasterSectionView.cpp - the master section of the mixer: meters, bus compressor and monitoring
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

#include "MasterSectionView.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <numbers>

#include <QCoreApplication>
#include <QGridLayout>
#include <QHBoxLayout>
#include <QLabel>
#include <QPainter>
#include <QPainterPath>
#include <QVBoxLayout>

#include "AutomatableButton.h"
#include "DeskKnob.h"
#include "GainStageControls.h"
#include "FontHelper.h"
#include "GuiApplication.h"
#include "MainWindow.h"
#include "Mixer.h"
#include "lmms_math.h"

namespace lmms::gui
{

namespace
{

//! The needle's travel for @p db (VU): proportional to the voltage, as on a real VU meter, with +3 VU at the end
auto vuPosition(float db) -> float
{
	return std::pow(10.f, db / 20.f) / std::pow(10.f, 3.f / 20.f);
}

//! Calls @p advance on every update of the GUI's meters, and repaints @p widget
template<class Widget>
void followGuiUpdates(Widget* widget)
{
	QObject::connect(getGUI()->mainWindow(), &MainWindow::periodicUpdate, widget, [widget] {
		widget->advance();
		widget->update();
	});
}

constexpr float NeedleSwing = 48.f; // degrees to each side

} // namespace




VuMeter::VuMeter(const QString& label, std::function<float()> level, QWidget* parent)
	: QWidget{parent}, m_label{label}, m_level{std::move(level)}
{
	setFixedSize(58, 44);
	followGuiUpdates(this);
}




void VuMeter::advance()
{
	const float rms = m_level();
	const float target = rms > 0.f ? std::min(vuPosition(ampToDbfs(rms) - ReferenceLevel), 1.08f) : 0.f;
	// The RMS level is averaged over 300 ms already; this only smooths the steps between updates
	m_position += (target - m_position) * 0.4f;
}




void VuMeter::paintEvent(QPaintEvent*)
{
	auto painter = QPainter{this};
	painter.setRenderHint(QPainter::Antialiasing);
	const auto face = QRectF{rect()}.adjusted(0.5, 0.5, -0.5, -0.5);

	// The backlit face
	auto light = QLinearGradient{face.topLeft(), face.bottomLeft()};
	light.setColorAt(0.0, QColor{250, 236, 190});
	light.setColorAt(1.0, QColor{226, 200, 140});
	painter.setPen(QPen{QColor{20, 22, 25}, 1.0});
	painter.setBrush(light);
	painter.drawRoundedRect(face, 3, 3);
	painter.setClipRect(face.adjusted(1, 1, -1, -1));

	const auto pivot = QPointF{width() / 2.0, height() + 10.0};
	const double radius = height() + 2.0;
	auto pointAt = [&](float position, double r) {
		const double angle = (-NeedleSwing + 2.0 * NeedleSwing * position) * std::numbers::pi / 180.0;
		return pivot + QPointF{std::sin(angle) * r, -std::cos(angle) * r};
	};

	// The scale: black up to 0 VU, red above
	const auto scaleRadius = radius - 12.0;
	auto arc = [&](float from, float to, const QColor& color, double width) {
		auto path = QPainterPath{};
		path.moveTo(pointAt(from, scaleRadius));
		for (float p = from; p <= to; p += 0.01f) { path.lineTo(pointAt(p, scaleRadius)); }
		painter.setPen(QPen{color, width});
		painter.setBrush(Qt::NoBrush);
		painter.drawPath(path);
	};
	arc(vuPosition(-20.f), vuPosition(0.f), QColor{30, 30, 30}, 1.0);
	arc(vuPosition(0.f), 1.f, QColor{200, 30, 30}, 2.5);

	painter.setFont(adjustedToPixelSize(font(), 6));
	for (const float db : {-20.f, -10.f, -7.f, -5.f, -3.f, -2.f, -1.f, 0.f, 1.f, 2.f, 3.f})
	{
		const float p = vuPosition(db);
		const auto color = db > 0.f ? QColor{200, 30, 30} : QColor{30, 30, 30};
		painter.setPen(QPen{color, 1.0});
		painter.drawLine(pointAt(p, scaleRadius), pointAt(p, scaleRadius + 3.0));
		if (db == -20.f || db == -10.f || db == -5.f || db == 0.f || db == 3.f)
		{
			const auto at = pointAt(p, scaleRadius + 8.0);
			painter.drawText(QRectF{at.x() - 8.0, at.y() - 4.0, 16.0, 8.0}, Qt::AlignCenter,
				QString::number(std::abs(static_cast<int>(db))));
		}
	}
	painter.setPen(QColor{40, 40, 40});
	painter.setFont(adjustedToPixelSize(font(), 8));
	painter.drawText(QRectF{0, height() - 16.0, static_cast<double>(width()), 10.0}, Qt::AlignCenter, "VU");
	painter.setFont(adjustedToPixelSize(font(), 7));
	painter.drawText(QRectF{3, height() - 11.0, 12.0, 9.0}, Qt::AlignLeft | Qt::AlignVCenter, m_label);

	// The needle
	painter.setPen(QPen{QColor{15, 15, 15}, 1.2, Qt::SolidLine, Qt::RoundCap});
	painter.drawLine(pointAt(m_position, 14.0), pointAt(m_position, radius - 4.0));
}




CorrelationMeter::CorrelationMeter(std::function<float()> correlation, QWidget* parent)
	: QWidget{parent}, m_correlation{std::move(correlation)}
{
	setFixedHeight(8);
	setToolTip(QCoreApplication::translate("CorrelationMeter",
		"Correlation of left and right: +1 mono, 0 wide, below 0 they cancel each other"));
	followGuiUpdates(this);
}




void CorrelationMeter::advance()
{
	m_shown += (m_correlation() - m_shown) * 0.3f;
}




void CorrelationMeter::paintEvent(QPaintEvent*)
{
	auto painter = QPainter{this};
	const auto r = rect();
	painter.fillRect(r, QColor{12, 13, 15});
	const int center = r.width() / 2;
	const int at = center + static_cast<int>(m_shown * (r.width() / 2 - 1));
	const auto color = m_shown < 0.f ? QColor{220, 60, 60} : QColor{74, 253, 133};
	painter.fillRect(QRect{std::min(center, at), 1, std::abs(at - center) + 1, r.height() - 2}, color);
	painter.fillRect(QRect{center, 0, 1, r.height()}, QColor{140, 150, 163});
}




Goniometer::Goniometer(std::function<void(std::vector<std::array<float, 2>>&)> points, QWidget* parent)
	: QWidget{parent}, m_points{std::move(points)}
{
	setFixedHeight(56);
	setToolTip(QCoreApplication::translate("Goniometer",
		"Goniometer: mid upwards, side sideways. Mono is a vertical line; sideways spread means cancelling phase"));
	followGuiUpdates(this);
}




void Goniometer::advance()
{
	m_points(m_current);
}




void Goniometer::paintEvent(QPaintEvent*)
{
	auto painter = QPainter{this};
	const auto r = QRectF{rect()};
	painter.fillRect(r, QColor{12, 13, 15});

	// The axes: mid (vertical), side (horizontal), and L and R on the diagonals
	const auto center = r.center();
	const double radius = r.height() / 2.0 - 2.0;
	painter.setPen(QPen{QColor{40, 45, 50}, 1.0});
	painter.drawLine(QPointF{center.x(), r.top()}, QPointF{center.x(), r.bottom()});
	painter.drawLine(QPointF{center.x() - radius, center.y()}, QPointF{center.x() + radius, center.y()});
	painter.drawLine(center + QPointF{-radius, -radius}, center + QPointF{radius, radius});
	painter.drawLine(center + QPointF{radius, -radius}, center + QPointF{-radius, radius});
	painter.setPen(QColor{90, 98, 108});
	painter.setFont(adjustedToPixelSize(font(), 6));
	painter.drawText(QRectF{center.x() - radius - 2.0, r.top(), 10.0, 8.0}, Qt::AlignLeft, "L");
	painter.drawText(QRectF{center.x() + radius - 8.0, r.top(), 10.0, 8.0}, Qt::AlignRight, "R");

	// The points, the latest brightest
	painter.setRenderHint(QPainter::Antialiasing);
	const auto count = static_cast<int>(m_current.size());
	for (int i = 0; i < count; ++i)
	{
		const auto& [side, mid] = m_current[i];
		const double x = std::clamp(-side * radius, -radius, radius); // left of the mid when L is louder
		const double y = std::clamp(mid * radius, -radius, radius);
		auto color = QColor{74, 253, 133};
		color.setAlphaF(0.15 + 0.6 * i / std::max(count, 1));
		painter.setPen(QPen{color, 1.2});
		painter.drawPoint(QPointF{center.x() + x, center.y() - y});
	}
}




GainReductionMeter::GainReductionMeter(std::function<float()> takeReduction, QWidget* parent)
	: QWidget{parent}, m_takeReduction{std::move(takeReduction)}
{
	setFixedHeight(7);
	setToolTip(QCoreApplication::translate("GainReductionMeter", "Gain reduction of the bus compressor (0 to 20 dB)"));
	followGuiUpdates(this);
}




void GainReductionMeter::advance()
{
	// Falls at once, recovers gently
	const float reduction = m_takeReduction();
	m_shown = reduction < m_shown ? reduction : m_shown + (reduction - m_shown) * 0.2f;
}




void GainReductionMeter::paintEvent(QPaintEvent*)
{
	auto painter = QPainter{this};
	const auto r = rect();
	painter.fillRect(r, QColor{12, 13, 15});
	const float amount = std::clamp(-m_shown / 20.f, 0.f, 1.f);
	painter.fillRect(QRect{1, 1, static_cast<int>(amount * (r.width() - 2)), r.height() - 2}, QColor{240, 170, 40});
}




MasterSectionView::MasterSectionView(Mixer* mixer, QWidget* parent)
	: QWidget{parent}
{
	setObjectName("masterSection");
	setAttribute(Qt::WA_StyledBackground, true);
	setFixedWidth(Width);
	auto& master = mixer->masterSection();

	auto layout = new QVBoxLayout{this};
	layout->setContentsMargins(4, 4, 4, 4);
	layout->setSpacing(3);
	auto addTitle = [this, layout](const QString& text) {
		auto title = new QLabel{text, this};
		title->setObjectName("masterSectionTitle");
		title->setAlignment(Qt::AlignCenter);
		title->setFont(adjustedToPixelSize(font(), 7));
		layout->addWidget(title);
	};
	auto makeButton = [this](BoolModel* model, const QString& text, const QString& tip) {
		auto button = new AutomatableButton{this, tip};
		button->setObjectName("masterButton");
		button->setText(text);
		button->setToolTip(tip);
		button->setCheckable(true);
		button->setModel(model);
		return button;
	};
	auto makeKnob = [this](FloatModel* model, const QString& label, const QString& name, const QString& unit) {
		auto knob = new DeskKnob{QColor{96, 102, 110}, label, name, this};
		knob->setModel(model);
		knob->setHintText(name + ":", unit);
		return knob;
	};

	// Meters: the master's level on VU meters, and its correlation
	addTitle(tr("METERS"));
	const auto masterChannel = mixer->mixerChannel(0);
	auto meters = new QHBoxLayout{};
	meters->setSpacing(4);
	meters->addWidget(new VuMeter{"L", [masterChannel] { return masterChannel->m_meter.rms(0); }, this});
	meters->addWidget(new VuMeter{"R", [masterChannel] { return masterChannel->m_meter.rms(1); }, this});
	layout->addLayout(meters);
	layout->addWidget(new Goniometer{[&master](auto& points) { master.scopePoints(points); }, this});
	layout->addWidget(new CorrelationMeter{[&master] { return master.correlation(); }, this});

	// Analog mode: the master's gain stage, which the PRE faders drive
	auto drive = new QWidget{this};
	auto driveLayout = new QVBoxLayout{drive};
	driveLayout->setContentsMargins(0, 0, 0, 0);
	driveLayout->setSpacing(2);
	auto driveTitle = new QLabel{tr("DRIVE"), drive};
	driveTitle->setObjectName("masterSectionTitle");
	driveTitle->setAlignment(Qt::AlignCenter);
	driveTitle->setFont(adjustedToPixelSize(font(), 7));
	driveLayout->addWidget(driveTitle);
	auto driveRow = new QHBoxLayout{};
	driveRow->setSpacing(4);
	driveRow->addWidget(new DriveButton{&master.m_driveModel, drive});
	driveRow->addWidget(new SaturationLeds{&master.gainStage(), &master.m_driveModel, &master.m_realisticModel, drive});
	driveLayout->addLayout(driveRow);
	auto transformer = makeButton(&master.m_transformerModel, tr("XFMR"),
		tr("Output transformer, Neve style, after the master's fader: thickens the lows when they are loud, rolls "
		"the highs off a little"));
	transformer->setParent(drive);
	driveLayout->addWidget(transformer, 0, Qt::AlignHCenter);
	layout->addWidget(drive);
	auto showDrive = [drive, mixer] { drive->setVisible(mixer->isAnalog()); };
	connect(mixer->analogModel(), &BoolModel::dataChanged, drive, showDrive);
	showDrive();

	// The stereo section, in mid/side, before the bus compressor
	addTitle(tr("STEREO"));
	auto stereo = new QHBoxLayout{};
	stereo->setSpacing(0);
	stereo->addWidget(makeKnob(&masterChannel->m_widthModel, tr("WIDTH"), tr("Stereo width"), "%"));
	stereo->addWidget(makeKnob(&master.m_monoBassModel, tr("BASS"),
		tr("Mono bass: how mono below the frequency (0 = off, 100 = all mono)"), "%"));
	stereo->addWidget(makeKnob(&master.m_bassFrequencyModel, tr("FREQ"), tr("Mono bass frequency"), " Hz"));
	stereo->addWidget(makeKnob(&master.m_airModel, tr("AIR"), tr("Air: high shelf of the side at 12 kHz"), " dB"));
	layout->addLayout(stereo);

	// The bus compressor
	addTitle(tr("BUS COMP"));
	auto compressorTop = new QHBoxLayout{};
	compressorTop->setSpacing(4);
	compressorTop->addWidget(makeButton(&master.m_compressorModel, tr("IN"), tr("Bus compressor in")));
	compressorTop->addWidget(new GainReductionMeter{[&master] { return master.takeGainReduction(); }, this}, 1);
	layout->addLayout(compressorTop);
	auto compressorKnobs = new QHBoxLayout{};
	compressorKnobs->setSpacing(0);
	compressorKnobs->addWidget(makeKnob(&master.m_thresholdModel, tr("THR"), tr("Threshold"), " dB"));
	compressorKnobs->addWidget(makeKnob(&master.m_attackModel, tr("ATK"), tr("Attack"), " ms"));
	compressorKnobs->addWidget(makeKnob(&master.m_releaseModel, tr("REL"), tr("Release"), " s"));
	compressorKnobs->addWidget(makeKnob(&master.m_makeupModel, tr("MAKE"), tr("Make-up gain"), " dB"));
	layout->addLayout(compressorKnobs);
	auto compressorButtons = new QHBoxLayout{};
	compressorButtons->setSpacing(2);
	auto ratios = new AutomatableButtonGroup{this, tr("Ratio")};
	for (const auto& text : {QString{"2"}, QString{"4"}, QString{"10"}})
	{
		auto button = new AutomatableButton{this, tr("Ratio %1:1").arg(text)};
		button->setObjectName("masterButton");
		button->setText(text);
		button->setToolTip(tr("Ratio %1:1").arg(text));
		button->setCheckable(true);
		ratios->addButton(button);
		compressorButtons->addWidget(button);
	}
	ratios->setModel(&master.m_ratioModel);
	compressorButtons->addStretch(1);
	compressorButtons->addWidget(makeButton(&master.m_autoReleaseModel, tr("AUTO"), tr("Auto release")));
	layout->addLayout(compressorButtons);

	// The monitoring: what is heard, not what is mixed or exported
	addTitle(tr("MONITOR"));
	auto monitor = new QHBoxLayout{};
	monitor->setSpacing(2);
	monitor->addWidget(makeKnob(&master.m_monitorLevelModel, tr("LEVEL"), tr("Monitor level"), " dB"));
	auto monitorButtons = new QGridLayout{};
	monitorButtons->setSpacing(2);
	monitorButtons->addWidget(makeButton(&master.m_dimModel, tr("DIM"), tr("Dim the monitors by 20 dB")), 0, 0);
	monitorButtons->addWidget(makeButton(&master.m_cutModel, tr("CUT"), tr("Cut the monitors")), 0, 1);
	monitorButtons->addWidget(makeButton(&master.m_monoModel, tr("MONO"), tr("Listen to the mid, in mono")), 1, 0);
	monitorButtons->addWidget(makeButton(&master.m_sideModel, tr("SIDE"), tr("Listen to the side")), 1, 1);
	monitor->addLayout(monitorButtons);
	layout->addLayout(monitor);
	layout->addStretch(1);
}

} // namespace lmms::gui
