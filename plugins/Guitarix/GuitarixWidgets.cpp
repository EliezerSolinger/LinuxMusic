/*
 * GuitarixWidgets.cpp - Guitarix's rack widgets, drawn as Guitarix draws them
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

#include "GuitarixWidgets.h"

#include <algorithm>
#include <cmath>
#include <numbers>

#include <QFontDatabase>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QVBoxLayout>

#include "GuiApplication.h"
#include "MainWindow.h"
#include "embed.h"

namespace lmms::gui::guitarix
{

namespace
{

// Guitarix's style's colors (gx_head_Guitarix.scss, minimal.scss)
const auto ControlColor = QColor{0xff, 0x90, 0x00}; //!< $cl-control: a knob's ring, a selector's text
const auto DisabledColor = QColor{0x21, 0x10, 0x06}; //!< a knob's ring where the value is not
const auto LabelColor = QColor{0x22, 0x22, 0x22}; //!< .rack_label, .effect_title
const auto SelectorFrame = QColor{0x27, 0x27, 0x27};
const auto MeterBackground = QColor{0x22, 0x22, 0x22};
const auto EntryBackground = QColor{0x1a, 0x1a, 0x1a}; //!< (* { background-color: rgb(25.5, 25.5, 25.5) })

constexpr auto Pi = std::numbers::pi;

//! Cairo's angles (clockwise on the screen) to Qt's (counterclockwise), in Qt's sixteenths of a degree
auto qtAngle(double radians) -> int { return static_cast<int>(std::lround(-radians * 180. / Pi * 16.)); }

//! gx_bevel: a gradient (light on top, dark at the bottom) applied in soft light, then over
void bevel(QPainter& p, const QRectF& rect, double amount)
{
	if (amount == 0) { return; }
	auto gradient = QLinearGradient{rect.topLeft(), rect.bottomLeft()};
	gradient.setColorAt(0, QColor::fromRgbF(1, 1, 1, static_cast<float>(amount / 2)));
	gradient.setColorAt(1, QColor::fromRgbF(0, 0, 0, static_cast<float>(amount)));
	p.save();
	p.setCompositionMode(QPainter::CompositionMode_SoftLight);
	p.fillRect(rect, gradient);
	p.setCompositionMode(QPainter::CompositionMode_SourceOver);
	p.fillRect(rect, gradient);
	p.restore();
}

//! gx_draw_inset
void inset(QPainter& p, const QRectF& rect, double radius, double depth)
{
	auto gradient = QLinearGradient{rect.topLeft(), rect.bottomLeft()};
	gradient.setColorAt(0, QColor::fromRgbF(0, 0, 0, 0.33f));
	gradient.setColorAt(1, QColor::fromRgbF(1, 1, 1, 0.1f));
	auto path = QPainterPath{};
	path.addRoundedRect(rect.adjusted(-depth, -depth, depth, depth), radius, radius);
	p.fillPath(path, gradient);
}

//! gx_draw_glass: a shadow at the top, 3 pixels deep
void glass(QPainter& p, const QRectF& rect, double radius)
{
	auto gradient = QLinearGradient{rect.topLeft(), rect.topLeft() + QPointF{0, 3}};
	gradient.setColorAt(0, QColor::fromRgbF(0, 0, 0, 0.5f));
	gradient.setColorAt(1, QColor::fromRgbF(0, 0, 0, 0));
	auto path = QPainterPath{};
	path.addRoundedRect(rect, radius, radius);
	p.fillPath(path, gradient);
}

//! A soft-light pass over a stroke: dark at its edges, light along its middle (GxKnob's ring and indicator)
void softLightStroke(QPainter& p, const QPainterPath& path, QPointF center, double inner, double outer, double width)
{
	auto gradient = QRadialGradient{center, outer, center, inner};
	gradient.setColorAt(0, QColor::fromRgbF(0, 0, 0, 0.6f));
	gradient.setColorAt(0.5, QColor::fromRgbF(1, 1, 1, 0.4f));
	gradient.setColorAt(1, QColor::fromRgbF(0, 0, 0, 0.6f));
	auto pen = p.pen();
	pen.setBrush(gradient);
	pen.setWidthF(width);
	p.save();
	p.setPen(pen);
	p.setCompositionMode(QPainter::CompositionMode_SoftLight);
	p.drawPath(path);
	p.restore();
}

//! log_meter (libgxw): a level in dB to the meter's deflection (0...1)
auto logMeter(double db) -> double
{
	auto def = 0.;
	if (db < -70.) { def = 0.; }
	else if (db < -60.) { def = (db + 70.) * 0.25; }
	else if (db < -50.) { def = (db + 60.) * 0.5 + 2.5; }
	else if (db < -40.) { def = (db + 50.) * 0.75 + 7.5; }
	else if (db < -30.) { def = (db + 40.) * 1.5 + 15.; }
	else if (db < -20.) { def = (db + 30.) * 2. + 30.; }
	else if (db < 6.) { def = (db + 20.) * 2.5 + 50.; }
	else { def = 115.; }
	return def / 115.;
}

} // namespace


auto image(const char* name) -> QPixmap
{
	return ::lmms::PLUGIN_NAME::getIconPixmap(name);
}


auto font(double points) -> QFont
{
	static const auto s_family = [] {
		const auto id = QFontDatabase::addApplicationFont(":/artwork/guitarix/RobotoCondensed-Regular.ttf");
		const auto families = id < 0 ? QStringList{} : QFontDatabase::applicationFontFamilies(id);
		return families.isEmpty() ? QString{} : families.front();
	}();
	auto f = s_family.isEmpty() ? QFont{} : QFont{s_family};
	if (s_family.isEmpty()) { f.setStretch(QFont::Condensed); }
	f.setPixelSize(static_cast<int>(std::lround(points * 96. / 72.)));
	return f;
}


void styleRackLabel(QLabel* label)
{
	label->setFont(font(8));
	auto palette = label->palette();
	palette.setColor(QPalette::WindowText, LabelColor);
	label->setPalette(palette);
}




RackUnit::RackUnit(const QString& title, QWidget* controls, QWidget* parent) :
	QWidget(parent)
{
	// The handles (32 pixels, make_bar) at the sides, the shadow's 4 under; the title bar on top (the on/off switch
	// it has in Guitarix, 23 pixels and its margins, is the effect's own here), its controls under it
	auto layout = new QVBoxLayout(this);
	layout->setContentsMargins(32, 0, 32, 4);
	layout->setSpacing(4);

	auto label = new QLabel(title, this);
	label->setFont(guitarix::font(10));
	auto palette = label->palette();
	palette.setColor(QPalette::WindowText, LabelColor);
	label->setPalette(palette);
	label->setFixedHeight(31);
	label->setContentsMargins(4, 0, 0, 0);
	layout->addWidget(label);
	layout->addWidget(controls, 1, Qt::AlignCenter);
	layout->setSizeConstraint(QLayout::SetFixedSize);
}


void RackUnit::paintEvent(QPaintEvent*)
{
	auto p = QPainter{this};
	const auto w = width();
	const auto h = height();
	const auto panelHeight = h - 4;

	// draw_skin: the panel; its lower 40 % in the other image when it is higher than 64 pixels
	p.drawTiledPixmap(QRect{0, 0, w, panelHeight}, image("background1"));
	if (h > 64)
	{
		const auto top = static_cast<int>(panelHeight * 0.6);
		p.drawTiledPixmap(QRect{0, top, w + 1, static_cast<int>(panelHeight * 0.4) + 1}, image("background2"),
			QPoint{0, top});
	}
	bevel(p, QRectF(0, 0, w, panelHeight), 0.11);
	auto shadow = QLinearGradient{QPointF(0, panelHeight), QPointF(0, h)};
	shadow.setColorAt(0, QColor::fromRgbF(0, 0, 0, 0.8f));
	shadow.setColorAt(1, QColor::fromRgbF(0, 0, 0, 0));
	p.fillRect(QRect{0, panelHeight, w, h - panelHeight}, shadow);

	// draw_handles
	const auto left = image("handle_left");
	p.drawTiledPixmap(QRect{0, 0, left.width(), panelHeight}, left);
	p.fillRect(QRect{left.width() - 1, 0, 1, panelHeight}, QColor::fromRgbF(0, 0, 0, 0.8f));
	p.fillRect(QRect{left.width(), 0, 1, panelHeight}, QColor::fromRgbF(1, 1, 1, 0.15f));
	const auto right = image("handle_right");
	p.drawTiledPixmap(QRect{w - right.width(), 0, right.width(), panelHeight}, right);
	p.fillRect(QRect{w - right.width() - 1, 0, 1, panelHeight}, QColor::fromRgbF(0, 0, 0, 0.5f));
	p.fillRect(QRect{w - right.width(), 0, 1, panelHeight}, QColor::fromRgbF(1, 1, 1, 0.05f));

	// draw_screws: at the corners, or halfway down the sides of a low unit
	const auto screw = image("screw");
	const auto screwHeight = h - 3;
	if (screwHeight > 2 * screw.height())
	{
		p.drawPixmap(0, 0, screw);
		p.drawPixmap(0, screwHeight - screw.height(), screw);
		p.drawPixmap(w - screw.width(), screwHeight - screw.height(), screw);
		p.drawPixmap(w - screw.width(), 0, screw);
	}
	else if (screwHeight > screw.height())
	{
		const auto y = screwHeight / 2 - screw.height() / 2;
		p.drawPixmap(0, y, screw);
		p.drawPixmap(w - screw.width(), y, screw);
	}
}




namespace
{

//! A knob's ring and indicator (minimal.scss: gx-small-knob, gx-small-knob-r, gx-mid-knob, gx-big-knob)
struct KnobStyle
{
	const char* image;
	double ringRadius;
	double ringWidth;
	double ledSize;
	double ledDistance;
	double indicatorRadius;
	double indicatorWidth;
	double indicatorLength;
};

auto knobStyle(KnobKind kind) -> const KnobStyle&
{
	static const KnobStyle s_small{"knob_small", 15, 2, 3, 2, 11, 2, 5};
	static const KnobStyle s_smallR{"knob_small_r", 26, 3, 3, 2, 12, 3, 3};
	static const KnobStyle s_mid{"knob_mid", 35, 6, 2, 2, 27, 2, 6};
	// (its "50.5" read as GTK reads it into an integer)
	static const KnobStyle s_big{"knob_big", 50, 3, 0, 0, 46, 3, 6};
	switch (kind)
	{
		case KnobKind::Small: return s_small;
		case KnobKind::SmallR: return s_smallR;
		case KnobKind::Mid: return s_mid;
		case KnobKind::Big: return s_big;
	}
	return s_small;
}

//! (GxKnob's dead zone at the bottom: 40 degrees each side)
constexpr auto ScaleZero = 40. * Pi / 180.;

// (a regler's padding: 4px 8px)
constexpr auto KnobPaddingX = 8;
constexpr auto KnobPaddingY = 4;

} // namespace


Knob::Knob(KnobKind kind, QWidget* parent) :
	FloatModelEditorBase(DirectionOfManipulation::Vertical, parent),
	m_kind(kind),
	m_image(image(knobStyle(kind).image))
{
	setFixedSize(m_image.width() + 2 * KnobPaddingX, m_image.height() + 2 * KnobPaddingY);
}


void Knob::paintEvent(QPaintEvent*)
{
	const auto& style = knobStyle(m_kind);
	const auto state = model() ? std::clamp(static_cast<double>(model()->normValue(model()->value())), 0., 1.) : 0.;

	// _gx_knob_expose: the image and its "shtuff" drawn on a surface of its size, then that on the widget
	auto surface = QImage{m_image.size(), QImage::Format_ARGB32_Premultiplied};
	surface.fill(Qt::transparent);
	{
		auto p = QPainter{&surface};
		p.setRenderHint(QPainter::Antialiasing);
		p.drawPixmap(0, 0, m_image);

		const auto center = QPointF(m_image.width() / 2., m_image.height() / 2.);
		const auto angle = ScaleZero + state * 2 * (Pi - ScaleZero);
		const auto addAngle = Pi / 2;
		const auto foreground = isEnabled() ? ControlColor : DisabledColor;

		const auto ring = QRectF{center.x() - style.ringRadius, center.y() - style.ringRadius,
			2 * style.ringRadius, 2 * style.ringRadius};
		auto pen = QPen{DisabledColor, style.ringWidth, Qt::SolidLine, Qt::FlatCap};
		if (style.ledSize > 0)
		{
			pen.setDashPattern({style.ledSize / style.ringWidth, style.ledDistance / style.ringWidth});
		}
		// the ring
		p.setPen(pen);
		p.drawArc(ring, qtAngle(addAngle + ScaleZero), qtAngle(320. * Pi / 180. - ScaleZero));
		// the value's arc on it
		if (ScaleZero < angle)
		{
			pen.setColor(foreground);
			p.setPen(pen);
			auto arc = QPainterPath{};
			arc.arcMoveTo(ring, qtAngle(addAngle + ScaleZero) / 16.);
			arc.arcTo(ring, qtAngle(addAngle + ScaleZero) / 16., qtAngle(angle - ScaleZero) / 16.);
			p.drawPath(arc);
			if (style.ringWidth >= 3)
			{
				// (the soft-light pass has the dashes too)
				auto lit = QPainterPathStroker{};
				lit.setWidth(style.ringWidth);
				lit.setCapStyle(Qt::FlatCap);
				if (style.ledSize > 0) { lit.setDashPattern(pen.dashPattern()); }
				auto gradient = QRadialGradient{center, style.ringRadius + style.ringWidth / 2, center,
					style.ringRadius - style.ringWidth / 2};
				gradient.setColorAt(0, QColor::fromRgbF(0, 0, 0, 0.6f));
				gradient.setColorAt(0.5, QColor::fromRgbF(1, 1, 1, 0.4f));
				gradient.setColorAt(1, QColor::fromRgbF(0, 0, 0, 0.6f));
				p.save();
				p.setCompositionMode(QPainter::CompositionMode_SoftLight);
				p.fillPath(lit.createStroke(arc), gradient);
				p.restore();
			}
		}

		// the indicator
		const auto from = center + QPointF{std::sin(angle) * -style.indicatorRadius,
			std::cos(angle) * style.indicatorRadius};
		const auto to = center + QPointF{std::sin(angle) * -(style.indicatorRadius - style.indicatorLength),
			std::cos(angle) * (style.indicatorRadius - style.indicatorLength)};
		p.setPen(QPen{foreground, style.indicatorWidth, Qt::SolidLine, Qt::FlatCap});
		auto indicator = QPainterPath{from};
		indicator.lineTo(to);
		p.drawPath(indicator);
		if (style.ringWidth >= 3)
		{
			softLightStroke(p, indicator, center, style.indicatorRadius - style.indicatorLength,
				style.indicatorRadius, style.indicatorWidth);
		}
	}
	auto p = QPainter{this};
	p.drawImage((width() - surface.width()) / 2, (height() - surface.height()) / 2, surface);
}




namespace
{

// minimal.scss's gx-selector: its border (4px 24px 4px 8px), its radius, its padding on top, its font
constexpr auto SelectorBorderTop = 4;
constexpr auto SelectorBorderRight = 24;
constexpr auto SelectorBorderBottom = 4;
constexpr auto SelectorBorderLeft = 8;
constexpr auto SelectorRadius = 3.;
constexpr auto SelectorPaddingTop = 3;
constexpr auto SelectorValueBorder = 4; //!< (its value border's sides)

auto selectorFont() -> QFont
{
	auto f = font(7.5); // (10 pixels)
	f.setPixelSize(10);
	return f;
}

} // namespace


Selector::Selector(const QStringList& labels, QWidget* parent) :
	FloatModelEditorBase(DirectionOfManipulation::Vertical, parent),
	m_labels(labels),
	m_icon(image("selector_icon"))
{
	setFont(selectorFont());
	setFixedSize(sizeHint());
	setCursor(Qt::PointingHandCursor);
}


auto Selector::sizeHint() const -> QSize
{
	const auto metrics = QFontMetrics{selectorFont()};
	auto widest = 0;
	for (const auto& label : m_labels) { widest = std::max(widest, metrics.horizontalAdvance(label)); }
	return {widest + 2 * SelectorValueBorder + SelectorBorderLeft + SelectorBorderRight,
		metrics.height() + 1 + SelectorPaddingTop + SelectorBorderTop + SelectorBorderBottom};
}


auto Selector::index() const -> int
{
	if (!model() || m_labels.isEmpty()) { return 0; }
	const auto position = static_cast<int>(std::lround(model()->value() - model()->minValue()));
	return std::clamp(position, 0, static_cast<int>(m_labels.size()) - 1);
}


auto Selector::currentValueToText() -> QString
{
	return m_labels.isEmpty() ? FloatModelEditorBase::currentValueToText() : m_labels[index()];
}


void Selector::paintEvent(QPaintEvent*)
{
	auto p = QPainter{this};
	p.setRenderHint(QPainter::Antialiasing);
	const auto frame = QRectF(rect());

	// its background (black) framed by its border
	auto outer = QPainterPath{};
	outer.addRoundedRect(frame, SelectorRadius, SelectorRadius);
	p.fillPath(outer, SelectorFrame);
	const auto text = QRectF(SelectorBorderLeft, SelectorBorderTop,
		width() - SelectorBorderLeft - SelectorBorderRight, height() - SelectorBorderTop - SelectorBorderBottom);
	p.fillRect(text, Qt::black);
	inset(p, text, 0, 1);
	p.fillRect(text, Qt::black);
	glass(p, text, 0);

	// the icon, in the middle of the right border
	p.drawPixmap(QPointF(width() - (SelectorBorderRight + m_icon.width()) / 2.,
		(height() - m_icon.height()) / 2.), m_icon);

	if (!m_labels.isEmpty())
	{
		p.setPen(isEnabled() ? ControlColor : DisabledColor);
		p.setFont(font());
		p.drawText(text.adjusted(0, SelectorPaddingTop, 0, 0), Qt::AlignHCenter | Qt::AlignTop, m_labels[index()]);
	}
}


void Selector::mousePressEvent(QMouseEvent* event)
{
	if (event->button() != Qt::LeftButton || event->modifiers() != Qt::NoModifier || !model())
	{
		FloatModelEditorBase::mousePressEvent(event);
		return;
	}
	// (its values, as GxSelector's menu lists them)
	auto menu = QMenu{this};
	for (int i = 0; i < m_labels.size(); ++i)
	{
		auto* action = menu.addAction(m_labels[i]);
		action->setCheckable(true);
		action->setChecked(i == index());
		connect(action, &QAction::triggered, this, [this, i] {
			model()->setValue(model()->minValue() + static_cast<float>(i));
		});
	}
	menu.exec(mapToGlobal(QPoint{0, height()}));
}


void Selector::mouseMoveEvent(QMouseEvent* event)
{
	// (no dragging through values: a click lists them)
	if (event->buttons() & Qt::LeftButton && event->modifiers() == Qt::NoModifier) { return; }
	FloatModelEditorBase::mouseMoveEvent(event);
}


void Selector::mouseReleaseEvent(QMouseEvent* event)
{
	if (event->button() == Qt::LeftButton && event->modifiers() == Qt::NoModifier) { return; }
	FloatModelEditorBase::mouseReleaseEvent(event);
}


void Selector::mouseDoubleClickEvent(QMouseEvent* event)
{
	// (as a second click)
	mousePressEvent(event);
}




namespace
{

// minimal.scss's gx-fast-meter
constexpr auto LedWidth = 2;
constexpr auto LedHeight = 2;
constexpr auto LedBorder = 1;
constexpr auto DefaultDimension = 2;
constexpr auto MinimumLeds = 1;
const auto MeterBottom = QColor{0xb9, 0x69, 0x01};
const auto MeterMiddle = QColor{0xd4, 0xcd, 0x00};
const auto MeterTop = QColor{0xff, 0x05, 0x00};
const auto MeterOver = QColor{0xff, 0x11, 0x00};
constexpr auto MeterMiddlePosition = 0.717;

//! (falloff: the scale in 500 ms)
constexpr auto FalloffPerMs = 1. / 500.;

} // namespace


Meter::Meter(const std::atomic<float>* level, Qt::Orientation orientation, int dimension, bool falloff,
	QWidget* parent) :
	QWidget(parent),
	m_level(level),
	m_orientation(orientation),
	m_dimension(dimension > 0 ? dimension : DefaultDimension),
	m_falloff(falloff)
{
	// (it takes what it is given, drawing its bar at the start of it: GxFastMeter)
	setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Preferred);
	// (by name: MainWindow is not exported to the plugins)
	connect(getGUI()->mainWindow(), SIGNAL(periodicUpdate()), this, SLOT(poll()));
	m_sincePoll.start();
}


auto Meter::sizeHint() const -> QSize
{
	// (GxFastMeter's size request: one LED along it)
	const auto across = LedBorder + m_dimension * (LedWidth + LedBorder);
	const auto along = LedBorder + MinimumLeds * (LedHeight + LedBorder);
	return m_orientation == Qt::Vertical ? QSize{across, along} : QSize{along, across};
}


void Meter::poll()
{
	const auto elapsedMs = static_cast<double>(m_sincePoll.restart());
	if (!m_level || !isVisible()) { return; }
	static_assert(std::atomic<float>::is_always_lock_free);
	const auto target = static_cast<float>(logMeter(m_level->load(std::memory_order_relaxed)));
	auto shown = target;
	if (m_falloff && target < m_shown)
	{
		shown = std::max(target, m_shown - static_cast<float>(elapsedMs * FalloffPerMs));
	}
	if (shown != m_shown)
	{
		m_shown = shown;
		update();
	}
}


void Meter::paintEvent(QPaintEvent*)
{
	const auto horizontal = m_orientation == Qt::Horizontal;
	const auto across = LedBorder + m_dimension * (LedWidth + LedBorder);
	auto width = horizontal ? this->width() : across;
	auto height = horizontal ? across : this->height();
	if (width <= 0 || height <= 0) { return; }
	auto p = QPainter{this};

	// request_meter: the background, the bar lit, the LEDs' borders, the inset
	p.fillRect(QRect{0, 0, width, height}, MeterBackground);
	// (no LED cut in half)
	if (horizontal) { width -= (width - LedBorder) % (LedHeight + LedBorder); }
	else { height -= (height - LedBorder) % (LedHeight + LedBorder); }
	const auto bars = static_cast<double>(horizontal ? width - 2 * LedBorder : height - 2 * LedBorder);
	if (bars <= 0) { return; }
	const auto top = (bars - LedHeight) / bars;
	auto gradient = QLinearGradient{QPointF(LedBorder, height - LedBorder),
		horizontal ? QPointF(width - LedBorder, height - LedBorder) : QPointF(LedBorder, LedBorder)};
	gradient.setColorAt(0, MeterBottom);
	gradient.setColorAt(MeterMiddlePosition, MeterMiddle);
	gradient.setColorAt(top, MeterTop);
	gradient.setColorAt(std::min(1., top + 0.0001), MeterOver);
	const auto bar = QRect{LedBorder, LedBorder, width - 2 * LedBorder, height - 2 * LedBorder};

	auto lit = QImage{width, height, QImage::Format_ARGB32_Premultiplied};
	lit.fill(Qt::transparent);
	{
		auto q = QPainter{&lit};
		q.fillRect(bar, gradient);
		const auto length = horizontal ? width : height;
		for (int j = 0; j < length; j += LedHeight + LedBorder)
		{
			q.fillRect(horizontal ? QRect{j, 0, LedBorder, height} : QRect{0, j, width, LedBorder}, EntryBackground);
		}
		for (int j = 1; j < m_dimension; ++j)
		{
			const auto at = j * (LedBorder + LedWidth);
			q.fillRect(horizontal ? QRect{0, at, width, LedBorder} : QRect{at, 0, LedBorder, height}, EntryBackground);
		}
		if (horizontal) { inset(q, QRectF(0, 0, width, height), 0, 1); }
		else { inset(q, QRectF(1, 1, width, height), 0, 0.5); }
	}
	// the bar dimmed, then lit up to the level
	p.drawImage(0, 0, lit);
	p.fillRect(QRect{0, 0, width, height}, QColor::fromRgbF(0, 0, 0, 0.8f));
	const auto leds = ((horizontal ? bar.width() : bar.height()) + LedBorder) / (LedHeight + LedBorder);
	const auto position = static_cast<int>(std::lround(leds * m_shown));
	if (position == 0) { return; }
	const auto extent = position * (LedHeight + LedBorder) - LedBorder;
	const auto shown = horizontal ? QRect{bar.x(), bar.y(), extent, bar.height()}
		: QRect{bar.x(), bar.bottom() + 1 - extent, bar.width(), extent};
	p.drawImage(shown, lit, shown);
}




namespace
{

constexpr auto SliderWidth = 36; //!< (gx-hslider's slider-width)

} // namespace


HSlider::HSlider(QWidget* parent) :
	FloatModelEditorBase(DirectionOfManipulation::Horizontal, parent),
	m_image(image("hslider"))
{
	setFixedSize(m_image.width() - 2 * SliderWidth, m_image.height());
}


void HSlider::paintEvent(QPaintEvent*)
{
	const auto track = width();
	const auto state = model() ? std::clamp(static_cast<double>(model()->normValue(model()->value())), 0., 1.) : 0.;
	const auto position = static_cast<int>(std::lround(state * (track - SliderWidth)));
	auto p = QPainter{this};
	p.drawPixmap(0, 0, m_image, 0, 0, track, height());
	p.drawPixmap(position, 0, m_image, track + (m_hovered ? SliderWidth : 0), 0, SliderWidth, height());
}


void HSlider::enterEvent(QEnterEvent* event)
{
	m_hovered = true;
	update();
	FloatModelEditorBase::enterEvent(event);
}


void HSlider::leaveEvent(QEvent* event)
{
	m_hovered = false;
	update();
	FloatModelEditorBase::leaveEvent(event);
}




ValueDisplay::ValueDisplay(int digits, QWidget* parent) :
	QWidget(parent),
	FloatModelView(new FloatModel(0, 0, 0, 1, nullptr, QString{}, true), this),
	m_digits(std::max(0, digits))
{
	setFont(guitarix::font(7));
	setFixedSize(sizeHint());
}


auto ValueDisplay::sizeHint() const -> QSize
{
	// (gx-value-display's value border: 4px 8px)
	const auto metrics = QFontMetrics{font()};
	return {metrics.horizontalAdvance(QStringLiteral("-000.").append(QString(m_digits, '0'))) + 16,
		metrics.height() + 8};
}


void ValueDisplay::paintEvent(QPaintEvent*)
{
	auto p = QPainter{this};
	p.setRenderHint(QPainter::Antialiasing);
	auto path = QPainterPath{};
	path.addRoundedRect(QRectF(rect()), 6, 6);
	p.fillPath(path, Qt::black);
	glass(p, QRectF(rect()), 6);
	p.setPen(ControlColor);
	p.drawText(rect(), Qt::AlignCenter, QString::number(model() ? model()->value() : 0.f, 'f', m_digits));
}

} // namespace lmms::gui::guitarix
