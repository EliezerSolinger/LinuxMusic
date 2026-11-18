/*
 * GuitarixLayout.cpp - a Guitarix unit's controls, laid out as Guitarix lays them out
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

#include "GuitarixLayout.h"

#include <cmath>

#include <QBoxLayout>
#include <QDomDocument>
#include <QGridLayout>
#include <QHash>
#include <QLabel>
#include <QPainter>
#include <QTabWidget>

#include "GuitarixEffect.h"
#include "GuitarixWidgets.h"

#include "gx_plugin.h"

namespace lmms::gui::guitarix
{

namespace
{

//! An object's (or a packing's) properties, their names as GtkBuilder takes them ('-' and '_' alike)
class Properties
{
public:
	explicit Properties(const QDomElement& element)
	{
		for (auto property = element.firstChildElement("property"); !property.isNull();
			property = property.nextSiblingElement("property"))
		{
			m_values.insert(property.attribute("name").replace('-', '_'), property.text());
		}
	}

	auto has(const QString& name) const -> bool { return m_values.contains(name); }
	auto text(const QString& name, const QString& otherwise = {}) const -> QString
	{
		return m_values.value(name, otherwise);
	}
	auto flag(const QString& name, bool otherwise) const -> bool
	{
		if (!has(name)) { return otherwise; }
		const auto value = text(name).toLower();
		return value == "true" || value == "yes" || value == "1";
	}
	auto number(const QString& name, double otherwise) const -> double
	{
		auto ok = false;
		const auto value = text(name).toDouble(&ok);
		return ok ? value : otherwise;
	}
	auto integer(const QString& name, int otherwise) const -> int
	{
		return static_cast<int>(std::lround(number(name, otherwise)));
	}

private:
	QHash<QString, QString> m_values;
};


//! An object's style classes: those its id names after a ':' (Guitarix's builder's way) and its <style>'s
auto classesOf(const QDomElement& object) -> QStringList
{
	auto classes = object.attribute("id").split(':').mid(1);
	for (auto klass = object.firstChildElement("style").firstChildElement("class"); !klass.isNull();
		klass = klass.nextSiblingElement("class"))
	{
		classes << klass.attribute("name");
	}
	return classes;
}


// Guitarix's style's colors for what is not on the panel (gx_head_Guitarix.scss)
const auto InverseLabelColor = QColor{0xaa, 0xaa, 0xaa}; //!< %rack-label-inverse
const auto TabBackground = QColor{0x1a, 0x1a, 0x1a}; //!< .tab_rack
const auto FrameInversedBorder = QColor{50, 50, 50, 178}; //!< #frame_inversed border


//! #rack_label_inverse: condensed 7pt, #aaaaaa (on the dark: a tab's page, an inversed frame)
void styleInverseLabel(QLabel* label)
{
	label->setFont(font(7));
	auto palette = label->palette();
	palette.setColor(QPalette::WindowText, InverseLabelColor);
	label->setPalette(palette);
}


/**
 * A GtkFrame: its label on top (at its label_xalign), its child under it, its padding (4 pixels); an inversed
 * one (#frame_inversed) has its border, the label breaking its top line.
 */
class Frame : public QWidget
{
public:
	Frame(QLabel* label, double labelAlignment, bool inversed, QWidget* parent) :
		QWidget(parent),
		m_label(label),
		m_inversed(inversed)
	{
		m_layout = new QVBoxLayout(this);
		m_layout->setContentsMargins(4, 0, 4, 4);
		m_layout->setSpacing(0);
		if (label)
		{
			label->setParent(this);
			const auto alignment = labelAlignment < 0.25 ? Qt::AlignLeft
				: labelAlignment > 0.75 ? Qt::AlignRight : Qt::AlignHCenter;
			m_layout->addWidget(label, 0, alignment);
		}
	}

	void setChild(QWidget* child) { m_layout->addWidget(child, 1); }

protected:
	void paintEvent(QPaintEvent*) override
	{
		if (!m_inversed) { return; }
		auto p = QPainter{this};
		const auto top = m_label ? m_label->geometry().center().y() : 0;
		const auto border = QRect{0, top, width() - 1, height() - 1 - top};
		p.setPen(FrameInversedBorder);
		if (!m_label)
		{
			p.drawRect(border);
			return;
		}
		// (its top line broken by the label)
		const auto gap = m_label->geometry().adjusted(-3, 0, 3, 0);
		p.drawLine(border.topLeft(), QPoint{gap.left(), top});
		p.drawLine(QPoint{gap.right(), top}, border.topRight());
		p.drawLine(border.topRight(), border.bottomRight());
		p.drawLine(border.bottomRight(), border.bottomLeft());
		p.drawLine(border.bottomLeft(), border.topLeft());
	}

private:
	QVBoxLayout* m_layout;
	QLabel* m_label;
	bool m_inversed;
};


//! What reading an object gave: its widget, and how it sits where it is put
struct Built
{
	QWidget* widget = nullptr;
	//! A control or a label (drawn in the middle of what it is given), not a box of others
	bool leaf = false;
	Properties properties;
};


/**
 * Reads a unit's glade into widgets, as GtkBuilder builds it and GTK lays it out: boxes (their children's
 * expand, fill, pack type and padding), grids, labels, frames, alignments, notebooks and Guitarix's controls,
 * each on the parameter its var_id names.
 */
class GladeReader
{
public:
	explicit GladeReader(GuitarixControls* controls) : m_controls(controls) {}

	//! The unit expanded ("rackbox"), from @p glade (none if it is not there)
	auto read(const QString& glade, QWidget* parent) -> QWidget*
	{
		auto document = QDomDocument{};
		if (!document.setContent(glade)) { return nullptr; }
		const auto objects = document.elementsByTagName("object");
		for (int i = 0; i < objects.size(); ++i)
		{
			const auto object = objects.at(i).toElement();
			if (object.attribute("id").section(':', 0, 0) != "rackbox") { continue; }
			auto built = this->object(object, parent);
			// (a label a knob refers to shows its parameter's name: GxRegler's cp_configure)
			for (const auto& [label, name] : m_labelNames)
			{
				if (auto* widget = m_labels.value(label)) { widget->setText(name); }
			}
			return built.widget;
		}
		return nullptr;
	}

private:
	auto object(const QDomElement& object, QWidget* parent) -> Built
	{
		auto built = Built{nullptr, false, Properties{object}};
		if (!built.properties.flag("visible", true)) { return built; }
		const auto kind = object.attribute("class");
		const auto& properties = built.properties;

		if (kind == "GtkBox") { built.widget = box(object, parent); }
		else if (kind == "GtkGrid") { built.widget = grid(object, parent); }
		else if (kind == "GtkLabel") { built.widget = label(object, parent); built.leaf = true; }
		else if (kind == "GtkFrame") { built.widget = frame(object, parent); }
		else if (kind == "GtkAlignment") { built.widget = alignment(object, parent); }
		else if (kind == "GtkNotebook") { built.widget = notebook(object, parent); }
		else if (kind == "GxSmallKnob") { built.widget = knob(KnobKind::Small, properties, parent); built.leaf = true; }
		else if (kind == "GxSmallKnobR") { built.widget = knob(KnobKind::SmallR, properties, parent); built.leaf = true; }
		else if (kind == "GxMidKnob") { built.widget = knob(KnobKind::Mid, properties, parent); built.leaf = true; }
		else if (kind == "GxBigKnob") { built.widget = knob(KnobKind::Big, properties, parent); built.leaf = true; }
		else if (kind == "GxSelector") { built.widget = selector(properties, parent); built.leaf = true; }
		// (a meter fills what it is given, its bar at the start of it)
		else if (kind == "GxFastMeter") { built.widget = meter(properties, parent); }
		else if (kind == "GxValueDisplay") { built.widget = valueDisplay(properties, parent); built.leaf = true; }
		else if (kind == "GxHSlider") { built.widget = slider(properties, parent); built.leaf = true; }
		else
		{
			// (a window, or what holds one child: what it holds)
			for (const auto& child : children(object))
			{
				if (!child.object.isNull()) { return this->object(child.object, parent); }
			}
			return built;
		}
		if (built.widget) { applyMargins(built, parent); }
		return built;
	}

	struct Child
	{
		QDomElement object;
		Properties packing;
		QString type;
	};

	static auto children(const QDomElement& object) -> std::vector<Child>
	{
		auto result = std::vector<Child>{};
		for (auto child = object.firstChildElement("child"); !child.isNull(); child = child.nextSiblingElement("child"))
		{
			const auto element = child.firstChildElement("object");
			if (element.isNull()) { continue; } // (a placeholder)
			result.push_back({element, Properties{child.firstChildElement("packing")}, child.attribute("type")});
		}
		return result;
	}

	//! (a margin is outside the widget: a box of others takes it as its own, a control is put in one with it)
	static void applyMargins(Built& built, QWidget* parent)
	{
		const auto& p = built.properties;
		const auto left = p.integer("margin_left", p.integer("margin_start", 0));
		const auto right = p.integer("margin_right", p.integer("margin_end", 0));
		const auto top = p.integer("margin_top", 0);
		const auto bottom = p.integer("margin_bottom", 0);
		if (left == 0 && right == 0 && top == 0 && bottom == 0) { return; }
		if (!built.leaf)
		{
			const auto margins = built.widget->contentsMargins();
			built.widget->setContentsMargins(margins.left() + left, margins.top() + top,
				margins.right() + right, margins.bottom() + bottom);
			return;
		}
		auto holder = new QWidget(parent);
		auto layout = new QVBoxLayout(holder);
		layout->setContentsMargins(left, top, right, bottom);
		built.widget->setParent(holder);
		layout->addWidget(built.widget, 0, Qt::AlignCenter);
		built.widget = holder;
		built.leaf = false;
	}

	//! Where a child sits across its box (its halign, else filling it; a control in the middle)
	static auto crossAlignment(const Built& built, bool horizontalBox) -> Qt::Alignment
	{
		const auto halign = built.properties.text("halign");
		const auto valign = built.properties.text("valign");
		const auto& align = horizontalBox ? valign : halign;
		if (align == "start") { return horizontalBox ? Qt::AlignTop : Qt::AlignLeft; }
		if (align == "end") { return horizontalBox ? Qt::AlignBottom : Qt::AlignRight; }
		if (align == "center" || built.leaf) { return horizontalBox ? Qt::AlignVCenter : Qt::AlignHCenter; }
		return {};
	}

	auto box(const QDomElement& object, QWidget* parent) -> QWidget*
	{
		const auto properties = Properties{object};
		const auto horizontal = properties.text("orientation", "horizontal") != "vertical";
		auto widget = new QWidget(parent);
		auto layout = new QBoxLayout(horizontal ? QBoxLayout::LeftToRight : QBoxLayout::TopToBottom, widget);
		layout->setContentsMargins(0, 0, 0, 0);
		layout->setSpacing(properties.integer("spacing", 0));

		struct Packed
		{
			Built built;
			bool expand;
			bool fill;
			int padding;
		};
		auto start = std::vector<Packed>{};
		auto end = std::vector<Packed>{};
		auto anyExpands = false;
		for (const auto& child : children(object))
		{
			auto built = this->object(child.object, widget);
			if (!built.widget) { continue; }
			// (GtkBox's packing defaults: not expanding, filling; a child's own hexpand/vexpand expands it)
			const auto expand = child.packing.flag("expand", false)
				|| built.properties.flag(horizontal ? "hexpand" : "vexpand", false);
			auto packed = Packed{built, expand, child.packing.flag("fill", true), child.packing.integer("padding", 0)};
			anyExpands = anyExpands || expand;
			(child.packing.text("pack_type") == "end" ? end : start).push_back(packed);
		}
		const auto add = [&](const Packed& packed) {
			auto alignment = crossAlignment(packed.built, horizontal);
			// (in the middle of what it is given: a control, or a box that does not fill it)
			if (packed.built.leaf || (packed.expand && !packed.fill))
			{
				alignment |= horizontal ? Qt::AlignHCenter : Qt::AlignVCenter;
			}
			if (packed.padding > 0) { layout->addSpacing(packed.padding); }
			layout->addWidget(packed.built.widget, packed.expand ? 1 : 0, alignment);
			if (packed.padding > 0) { layout->addSpacing(packed.padding); }
		};
		for (const auto& packed : start) { add(packed); }
		if (!end.empty() && !anyExpands) { layout->addStretch(1); }
		for (auto packed = end.rbegin(); packed != end.rend(); ++packed) { add(*packed); }
		return widget;
	}

	auto grid(const QDomElement& object, QWidget* parent) -> QWidget*
	{
		const auto properties = Properties{object};
		auto widget = new QWidget(parent);
		auto layout = new QGridLayout(widget);
		layout->setContentsMargins(0, 0, 0, 0);
		layout->setHorizontalSpacing(properties.integer("column_spacing", 0));
		layout->setVerticalSpacing(properties.integer("row_spacing", 0));
		auto widest = 0;
		auto highest = 0;
		for (const auto& child : children(object))
		{
			auto built = this->object(child.object, widget);
			if (!built.widget) { continue; }
			auto alignment = Qt::Alignment{};
			if (built.leaf) { alignment = Qt::AlignCenter; }
			const auto halign = built.properties.text("halign");
			if (halign == "start") { alignment = (alignment & ~Qt::AlignHorizontal_Mask) | Qt::AlignLeft; }
			else if (halign == "end") { alignment = (alignment & ~Qt::AlignHorizontal_Mask) | Qt::AlignRight; }
			else if (halign == "center") { alignment = (alignment & ~Qt::AlignHorizontal_Mask) | Qt::AlignHCenter; }
			layout->addWidget(built.widget, child.packing.integer("top_attach", 0),
				child.packing.integer("left_attach", 0), child.packing.integer("height", 1),
				child.packing.integer("width", 1), alignment);
			widest = std::max(widest, built.widget->sizeHint().width());
			highest = std::max(highest, built.widget->sizeHint().height());
		}
		// (homogeneous: every column, every row, as large as the largest)
		if (properties.flag("column_homogeneous", false))
		{
			for (int c = 0; c < layout->columnCount(); ++c)
			{
				layout->setColumnMinimumWidth(c, widest);
				layout->setColumnStretch(c, 1);
			}
		}
		if (properties.flag("row_homogeneous", false))
		{
			for (int r = 0; r < layout->rowCount(); ++r)
			{
				layout->setRowMinimumHeight(r, highest);
				layout->setRowStretch(r, 1);
			}
		}
		return widget;
	}

	auto label(const QDomElement& object, QWidget* parent) -> QLabel*
	{
		const auto properties = Properties{object};
		auto widget = new QLabel(properties.text("label"), parent);
		widget->setTextFormat(properties.flag("use_markup", false) ? Qt::RichText : Qt::PlainText);
		const auto classes = classesOf(object);
		if (classes.contains("rack_label_inverse") || m_onDark > 0) { styleInverseLabel(widget); }
		else { styleRackLabel(widget); }
		const auto xalign = properties.number("xalign", 0.5);
		widget->setAlignment((xalign < 0.25 ? Qt::AlignLeft : xalign > 0.75 ? Qt::AlignRight : Qt::AlignHCenter)
			| Qt::AlignVCenter);
		m_labels.insert(object.attribute("id"), widget);
		return widget;
	}

	auto frame(const QDomElement& object, QWidget* parent) -> QWidget*
	{
		const auto properties = Properties{object};
		const auto inversed = classesOf(object).contains("frame_inversed");
		auto labelObject = QDomElement{};
		auto childObject = QDomElement{};
		for (const auto& child : children(object))
		{
			(child.type == "label" ? labelObject : childObject) = child.object;
		}
		auto* labelWidget = static_cast<QLabel*>(nullptr);
		if (!labelObject.isNull() && labelObject.attribute("class") == "GtkLabel")
		{
			labelWidget = label(labelObject, parent);
		}
		else if (properties.has("label"))
		{
			labelWidget = new QLabel(properties.text("label"), parent);
			styleRackLabel(labelWidget);
		}
		auto widget = new Frame(labelWidget, properties.number("label_xalign", 0), inversed, parent);
		if (!childObject.isNull())
		{
			if (auto built = this->object(childObject, widget); built.widget) { widget->setChild(built.widget); }
		}
		return widget;
	}

	auto alignment(const QDomElement& object, QWidget* parent) -> QWidget*
	{
		const auto properties = Properties{object};
		auto widget = new QWidget(parent);
		auto layout = new QVBoxLayout(widget);
		layout->setContentsMargins(properties.integer("left_padding", 0), properties.integer("top_padding", 0),
			properties.integer("right_padding", 0), properties.integer("bottom_padding", 0));
		for (const auto& child : children(object))
		{
			if (auto built = this->object(child.object, widget); built.widget)
			{
				layout->addWidget(built.widget, 1, built.leaf ? Qt::AlignCenter : Qt::Alignment{});
				break;
			}
		}
		return widget;
	}

	//! A notebook; a .tab_rack one dark, its tabs' labels in the control color (the current one brighter)
	auto notebook(const QDomElement& object, QWidget* parent) -> QWidget*
	{
		auto widget = new QTabWidget(parent);
		widget->setDocumentMode(true);
		widget->setFont(font(8));
		widget->setStyleSheet(QString{
			"QTabWidget::pane { background: %1; border: none; margin: 0px 4px 4px 4px; }"
			"QTabWidget > QWidget > QWidget { background: %1; }"
			"QTabBar::tab { background: transparent; color: #995600; padding: 2px 8px 4px 8px; border: none; }"
			"QTabBar::tab:selected { color: #ff9000; }"
			"QTabBar::tab:hover { color: #555555; }"}.arg(TabBackground.name()));
		++m_onDark;
		auto page = static_cast<QWidget*>(nullptr);
		for (const auto& child : children(object))
		{
			if (child.type == "tab")
			{
				if (page)
				{
					widget->addTab(page, Properties{child.object}.text("label"));
					page = nullptr;
				}
				continue;
			}
			if (page) { widget->addTab(page, QString{}); }
			auto built = this->object(child.object, widget);
			page = built.widget;
			if (page) { page->setContentsMargins(5, 5, 5, 5); }
		}
		if (page) { widget->addTab(page, QString{}); }
		--m_onDark;
		return widget;
	}

	auto parameterOf(const Properties& properties) -> GuitarixParameter*
	{
		return m_controls->parameter(properties.text("var_id"));
	}

	auto knob(KnobKind kind, const Properties& properties, QWidget* parent) -> QWidget*
	{
		auto widget = new Knob(kind, parent);
		auto* parameter = parameterOf(properties);
		if (!parameter || !parameter->model)
		{
			widget->setEnabled(false);
			return widget;
		}
		widget->setModel(parameter->model.get());
		widget->setHintText(parameter->name + ":", "");
		if (properties.has("label_ref")) { m_labelNames.emplace_back(properties.text("label_ref"), parameter->name); }
		return widget;
	}

	auto selector(const Properties& properties, QWidget* parent) -> QWidget*
	{
		auto* parameter = parameterOf(properties);
		auto widget = new Selector(parameter ? parameter->labels : QStringList{}, parent);
		if (!parameter || !parameter->model)
		{
			widget->setEnabled(false);
			return widget;
		}
		widget->setModel(parameter->model.get());
		widget->setHintText(parameter->name + ":", "");
		return widget;
	}

	auto meter(const Properties& properties, QWidget* parent) -> QWidget*
	{
		auto* parameter = parameterOf(properties);
		auto widget = new Meter(parameter ? parameter->output.get() : nullptr,
			properties.text("orientation") == "vertical" ? Qt::Vertical : Qt::Horizontal,
			properties.integer("dimen", 0), properties.flag("falloff", false), parent);
		if (properties.has("tooltip_text")) { widget->setToolTip(properties.text("tooltip_text")); }
		return widget;
	}

	auto valueDisplay(const Properties& properties, QWidget* parent) -> QWidget*
	{
		auto widget = new ValueDisplay(properties.integer("round_digits", 0), parent);
		if (auto* parameter = parameterOf(properties); parameter && parameter->model)
		{
			widget->setModel(parameter->model.get());
		}
		return widget;
	}

	auto slider(const Properties& properties, QWidget* parent) -> QWidget*
	{
		auto widget = new HSlider(parent);
		auto* parameter = parameterOf(properties);
		if (!parameter || !parameter->model)
		{
			widget->setEnabled(false);
			return widget;
		}
		widget->setModel(parameter->model.get());
		widget->setHintText(parameter->name + ":", "");
		return widget;
	}

	GuitarixControls* m_controls;
	QHash<QString, QLabel*> m_labels; //!< by their ids
	std::vector<std::pair<QString, QString>> m_labelNames; //!< labels the knobs refer to, and their names
	int m_onDark = 0; //!< (within how many dark pages: a notebook's)
};


//! The glade a unit gives (its load_ui asked for it), or nothing
auto gladeOf(PluginDef* definition) -> QString
{
	static auto s_glade = QString{};
	if (!definition || !definition->load_ui) { return {}; }
	s_glade.clear();
	auto builder = UiBuilder{};
	builder.plugin = definition;
	builder.load_glade = [](const char* data) { s_glade = QString::fromUtf8(data); };
	// (only its glade is asked for: what else it can give is laid out as below)
	definition->load_ui(builder, UI_FORM_GLADE);
	return s_glade;
}


//! A unit that gives no glade: its parameters, each a knob (a selector for one with values) under its name
auto plainControls(GuitarixControls* controls, QWidget* parent) -> QWidget*
{
	auto widget = new QWidget(parent);
	auto layout = new QHBoxLayout(widget);
	layout->setContentsMargins(0, 0, 0, 0);
	layout->setSpacing(10);
	for (auto& parameter : controls->parameters())
	{
		if (!parameter.model) { continue; }
		auto column = new QVBoxLayout{};
		auto label = new QLabel(parameter.name, widget);
		styleRackLabel(label);
		column->addWidget(label, 0, Qt::AlignHCenter);
		if (!parameter.labels.isEmpty())
		{
			auto selector = new Selector(parameter.labels, widget);
			selector->setModel(parameter.model.get());
			column->addWidget(selector, 1, Qt::AlignCenter);
		}
		else
		{
			auto knob = new Knob(KnobKind::SmallR, widget);
			knob->setModel(parameter.model.get());
			knob->setHintText(parameter.name + ":", "");
			column->addWidget(knob, 1, Qt::AlignCenter);
		}
		layout->addLayout(column);
	}
	return widget;
}

} // namespace


auto buildControls(GuitarixControls* controls, QWidget* parent) -> QWidget*
{
	const auto glade = gladeOf(controls->effect()->definition());
	if (!glade.isEmpty())
	{
		if (auto* widget = GladeReader{controls}.read(glade, parent)) { return widget; }
	}
	return plainControls(controls, parent);
}

} // namespace lmms::gui::guitarix
