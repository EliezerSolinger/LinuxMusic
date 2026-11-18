/*
 * GuitarixWidgets.h - Guitarix's rack widgets, drawn as Guitarix draws them
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

#ifndef LMMS_GUI_GUITARIX_WIDGETS_H
#define LMMS_GUI_GUITARIX_WIDGETS_H

#include <atomic>

#include <QElapsedTimer>
#include <QLabel>
#include <QPixmap>
#include <QWidget>

#include "FloatModelEditorBase.h"

namespace lmms::gui::guitarix
{

/*
 * The look of Guitarix's default style ("Guitarix": rcstyles/gx_head_Guitarix.scss over minimal.scss), its images
 * (gx/rcstyles), and its drawing (libgxw: GxPaintBox, GxKnob, GxSelector, GxFastMeter), done with QPainter.
 */

//! An image of Guitarix's style (gx/rcstyles), by its name
auto image(const char* name) -> QPixmap;
//! Guitarix's font (Roboto Condensed, as its fonts/), @p points large (Guitarix's sizes are points at 96 dpi)
auto font(double points) -> QFont;

//! What its labels are (".rack_label": condensed 8pt, #222222 on the panel)
void styleRackLabel(QLabel* label);


/**
 * A unit in the rack, as GxPaintBox's gx_rack_unit_expose paints it: the panel (background1, the lower part in
 * background2 when it is high enough: "alternate-box: 60 0 0 0"), its bevel and the shadow beneath, the handles at
 * its sides and the screws at its corners. Its layout: the handles' room at the sides, the unit's title bar on top,
 * its controls under it, the shadow's room under them (Guitarix's RackBox::make_full_box).
 */
class RackUnit : public QWidget
{
	Q_OBJECT
public:
	RackUnit(const QString& title, QWidget* controls, QWidget* parent = nullptr);

protected:
	void paintEvent(QPaintEvent* event) override;
};


//! The kinds of knob Guitarix's units use, each its image and its ring (minimal.scss)
enum class KnobKind
{
	Small, //!< GxSmallKnob: knob_small.png
	SmallR, //!< GxSmallKnobR: knob_small_r.png
	Mid, //!< GxMidKnob: knob_mid.png
	Big //!< GxBigKnob: knob_big.png
};

/**
 * A knob as GxKnob draws it (_gx_knob_draw_shtuff): its image, a dashed ring with the value's arc on it in the
 * control color, and the indicator. It handles as LinuxMusic's knobs do (drag, wheel, automation, its value on
 * hover).
 */
class Knob : public FloatModelEditorBase
{
	Q_OBJECT
public:
	explicit Knob(KnobKind kind, QWidget* parent = nullptr);

protected:
	void paintEvent(QPaintEvent* event) override;

private:
	KnobKind m_kind;
	QPixmap m_image;
};


/**
 * A selector as GxSelector draws it: the value's label in the control color on black, framed (#272727), the
 * selector icon at its right. A click shows its values; the wheel steps through them.
 */
class Selector : public FloatModelEditorBase
{
	Q_OBJECT
public:
	Selector(const QStringList& labels, QWidget* parent = nullptr);

	auto sizeHint() const -> QSize override;

protected:
	void paintEvent(QPaintEvent* event) override;
	void mousePressEvent(QMouseEvent* event) override;
	void mouseMoveEvent(QMouseEvent* event) override;
	void mouseReleaseEvent(QMouseEvent* event) override;
	void mouseDoubleClickEvent(QMouseEvent* event) override;
	auto currentValueToText() -> QString override;

private:
	auto index() const -> int;

	QStringList m_labels;
	QPixmap m_icon;
};


/**
 * A level meter as GxFastMeter draws it: a bar of LEDs, those under the level lit in the gradient of Guitarix's
 * style, the rest dimmed; the level falls off linearly (500 ms for the scale). Its parameter's range is its scale
 * (cp_configure), or, for a parameter with a step (a "power" meter), the level is a power shown in dB by log_meter.
 */
class Meter : public QWidget
{
	Q_OBJECT
public:
	struct Scale
	{
		float lower = 0.f;
		float upper = 1.f;
		bool power = false;
	};

	Meter(const std::atomic<float>* level, Scale scale, Qt::Orientation orientation, int dimension, bool falloff,
		QWidget* parent = nullptr);

	auto sizeHint() const -> QSize override;
	auto minimumSizeHint() const -> QSize override { return sizeHint(); }

protected:
	void paintEvent(QPaintEvent* event) override;

private slots:
	//! The level now, the shown one falling off toward it [MainWindow's periodic update]
	void poll();

private:

	const std::atomic<float>* m_level;
	Scale m_scale;
	Qt::Orientation m_orientation;
	int m_dimension;
	bool m_falloff;
	float m_shown = 0.f; //!< the deflection shown (0...1)
	QElapsedTimer m_sincePoll;
};


/**
 * A horizontal slider as GxHSlider draws it: its image's track (hslider.png's first part), its knob on it (the
 * next 36 pixels; the 36 after those when hovered).
 */
class HSlider : public FloatModelEditorBase
{
	Q_OBJECT
public:
	explicit HSlider(QWidget* parent = nullptr);

protected:
	void paintEvent(QPaintEvent* event) override;
	void enterEvent(QEnterEvent* event) override;
	void leaveEvent(QEvent* event) override;

private:
	QPixmap m_image;
	bool m_hovered = false;
};


/**
 * A vertical slider as GxVSlider draws it: its image's track (vslider.png's top), its knob on it (the next 36
 * pixels; the 36 after those when hovered), up for more.
 */
class VSlider : public FloatModelEditorBase
{
	Q_OBJECT
public:
	explicit VSlider(QWidget* parent = nullptr);

protected:
	void paintEvent(QPaintEvent* event) override;
	void enterEvent(QEnterEvent* event) override;
	void leaveEvent(QEvent* event) override;

private:
	QPixmap m_image;
	bool m_hovered = false;
};


/**
 * A switch as GxSwitch draws it: its base name's images, <name>_on and <name>_off (switch, switch_mid,
 * frbutton...). A click turns it over.
 */
class Switch : public FloatModelEditorBase
{
	Q_OBJECT
public:
	explicit Switch(const QString& baseName, QWidget* parent = nullptr);

protected:
	void paintEvent(QPaintEvent* event) override;
	void mousePressEvent(QMouseEvent* event) override;
	void mouseMoveEvent(QMouseEvent* event) override;
	void mouseReleaseEvent(QMouseEvent* event) override;
	void mouseDoubleClickEvent(QMouseEvent* event) override;

private:
	auto isOn() const -> bool;

	QPixmap m_on;
	QPixmap m_off;
};


//! A box painted as a GxPaintBox's paint function paints it: one of Guitarix's backgrounds, tiled
class PaintBox : public QWidget
{
	Q_OBJECT
public:
	PaintBox(const QString& paintFunction, QWidget* parent = nullptr);

protected:
	void paintEvent(QPaintEvent* event) override;

private:
	QPixmap m_tile;
};


//! A value shown as GxValueDisplay shows it (its digits in the control color on black)
class ValueDisplay : public QWidget, public FloatModelView
{
	Q_OBJECT
public:
	ValueDisplay(int digits, QWidget* parent = nullptr);

	auto sizeHint() const -> QSize override;

protected:
	void paintEvent(QPaintEvent* event) override;

private:
	int m_digits;
};

} // namespace lmms::gui::guitarix

#endif // LMMS_GUI_GUITARIX_WIDGETS_H
