/*
 * CategoryIcon.cpp - the icon of a plugin category: its colour and two letters
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

#include "CategoryIcon.h"

#include <QGuiApplication>
#include <QPainter>
#include <QPixmapCache>
#include <QScreen>

namespace lmms::gui
{

namespace
{

auto ratio() -> qreal
{
	const auto* screen = QGuiApplication::primaryScreen();
	return screen ? screen->devicePixelRatio() : 1.0;
}

auto canvas(int size) -> QPixmap
{
	auto pixmap = QPixmap{QSize{size, size} * ratio()};
	pixmap.setDevicePixelRatio(ratio());
	pixmap.fill(Qt::transparent);
	return pixmap;
}

} // namespace


auto categoryIcon(const PluginCategories::Style& style, int size) -> QPixmap
{
	const auto key = QString{"category_%1_%2_%3"}.arg(style.color.name(), style.code).arg(size);
	if (auto cached = QPixmap{}; QPixmapCache::find(key, &cached)) { return cached; }

	auto pixmap = canvas(size);
	auto painter = QPainter{&pixmap};
	painter.setRenderHint(QPainter::Antialiasing);
	painter.setPen(Qt::NoPen);
	painter.setBrush(style.color);
	painter.drawRoundedRect(QRectF{0.5, 0.5, size - 1.0, size - 1.0}, size * 0.22, size * 0.22);
	auto font = painter.font();
	font.setPixelSize(std::max(6, static_cast<int>(size * 0.5)));
	font.setBold(true);
	painter.setFont(font);
	// Dark letters on light colours, light on dark ones
	painter.setPen(style.color.lightnessF() > 0.55 ? QColor{20, 22, 26} : QColor{245, 247, 250});
	painter.drawText(QRectF{0, 0, static_cast<qreal>(size), static_cast<qreal>(size)}, Qt::AlignCenter, style.code);
	painter.end();

	QPixmapCache::insert(key, pixmap);
	return pixmap;
}




auto pluginIcon(const QPixmap& logo, const PluginCategories::Style& style, int size) -> QPixmap
{
	if (logo.isNull() || logo.width() <= 1) { return categoryIcon(style, size); }

	auto pixmap = canvas(size);
	auto painter = QPainter{&pixmap};
	painter.setRenderHint(QPainter::Antialiasing);
	painter.setRenderHint(QPainter::SmoothPixmapTransform);
	painter.setPen(QPen{style.color, 1.5});
	painter.setBrush(Qt::NoBrush);
	painter.drawRoundedRect(QRectF{0.75, 0.75, size - 1.5, size - 1.5}, size * 0.22, size * 0.22);
	const auto inner = QRectF{2.5, 2.5, size - 5.0, size - 5.0};
	painter.drawPixmap(inner, logo, QRectF{logo.rect()});
	painter.end();
	return pixmap;
}

} // namespace lmms::gui
