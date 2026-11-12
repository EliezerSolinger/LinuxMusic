/*
 * MasterSectionView.h - the master section of the mixer: meters, bus compressor and monitoring
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

#ifndef LMMS_GUI_MASTER_SECTION_VIEW_H
#define LMMS_GUI_MASTER_SECTION_VIEW_H

#include <functional>

#include <QWidget>

namespace lmms
{

class Mixer;

namespace gui
{

/**
 * An analog VU meter: a backlit face with the VU scale (-20 to +3) and a needle. The needle shows an RMS level, with
 * the meter's ballistics (300 ms). 0 VU is -18 dBFS RMS, the usual alignment of digital to analog levels.
 */
class VuMeter : public QWidget
{
public:
	//! @param level the RMS level to show, linear
	VuMeter(const QString& label, std::function<float()> level, QWidget* parent);

	static constexpr float ReferenceLevel = -18.f; // dBFS RMS at 0 VU

	//! Moves the needle towards the current level [GUI thread, at the GUI's update rate]
	void advance();

protected:
	void paintEvent(QPaintEvent*) override;

private:
	QString m_label;
	std::function<float()> m_level;
	float m_position = 0.f; //!< of the needle, 0 (left) to 1 (+3 VU)
};

//! The correlation of the master's left and right channels, from -1 (cancelling) through 0 to +1 (mono)
class CorrelationMeter : public QWidget
{
public:
	CorrelationMeter(std::function<float()> correlation, QWidget* parent);
	void advance();

protected:
	void paintEvent(QPaintEvent*) override;

private:
	std::function<float()> m_correlation;
	float m_shown = 0.f;
};

//! The bus compressor's gain reduction, 0 to -20 dB
class GainReductionMeter : public QWidget
{
public:
	GainReductionMeter(std::function<float()> takeReduction, QWidget* parent);
	void advance();

protected:
	void paintEvent(QPaintEvent*) override;

private:
	std::function<float()> m_takeReduction;
	float m_shown = 0.f; //!< dB, 0 or less
};

/**
 * The master section next to the master strip, as on a recording studio's desk: VU meters and a correlation meter,
 * the bus compressor and the monitoring (see MasterSection).
 */
class MasterSectionView : public QWidget
{
	Q_OBJECT
public:
	MasterSectionView(Mixer* mixer, QWidget* parent);

	static constexpr int Width = 128;
};

} // namespace gui

} // namespace lmms

#endif // LMMS_GUI_MASTER_SECTION_VIEW_H
