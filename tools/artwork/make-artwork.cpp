/*
 * make-artwork.cpp - renders LinuxMusic's icons and splash screen from tools/artwork/icon.svg and project.svg
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

/*
 * Build and run from the source tree's root (Qt 6 with its SVG module):
 *
 *   c++ -std=c++20 tools/artwork/make-artwork.cpp -o /tmp/make-artwork \
 *       $(pkg-config --cflags --libs Qt6Svg Qt6Gui)
 *   /tmp/make-artwork .
 *
 * It writes the theme's icons and splash, the Linux icons (with the SVGs), the Windows installer's logos and
 * banner (and the .ico files, with ImageMagick's "magick") and the macOS .icns files (with "iconutil").
 */

#include <QDir>
#include <QFile>
#include <QFont>
#include <QGuiApplication>
#include <QImage>
#include <QLinearGradient>
#include <QPainter>
#include <QPainterPath>
#include <QProcess>
#include <QRadialGradient>
#include <QSvgRenderer>
#include <QTemporaryDir>

#include <array>
#include <cmath>
#include <cstdio>

namespace
{

QString s_root;

auto path(const QString& relative) -> QString
{
	return QDir{s_root}.filePath(relative);
}

void check(bool ok, const QString& what)
{
	if (!ok)
	{
		std::fprintf(stderr, "failed: %s\n", qPrintable(what));
		std::exit(1);
	}
}

//! @p svg rendered @p size pixels square, with alpha
auto render(QSvgRenderer& svg, int size) -> QImage
{
	auto image = QImage{size, size, QImage::Format_ARGB32_Premultiplied};
	image.fill(Qt::transparent);
	auto painter = QPainter{&image};
	painter.setRenderHint(QPainter::Antialiasing);
	painter.setRenderHint(QPainter::SmoothPixmapTransform);
	svg.render(&painter, QRectF{0, 0, static_cast<double>(size), static_cast<double>(size)});
	return image;
}

void save(const QImage& image, const QString& relative)
{
	QDir{}.mkpath(QFileInfo{path(relative)}.absolutePath());
	check(image.save(path(relative)), relative);
}

void run(const QString& program, const QStringList& arguments)
{
	auto process = QProcess{};
	process.start(program, arguments);
	check(process.waitForFinished(120000) && process.exitStatus() == QProcess::NormalExit && process.exitCode() == 0,
		program + ' ' + arguments.join(' ') + ": " + QString::fromLocal8Bit(process.readAllStandardError()));
}

//! The icon's ring and bars, white, centred at @p centre with the ring's radius @p radius
void drawGlyph(QPainter& painter, QPointF centre, double radius, const QColor& color)
{
	// As in icon.svg: ring r76 (stroke 15), bars 13 wide 22 apart, 40/76/112 high
	const auto scale = radius / 76.0;
	painter.save();
	painter.translate(centre);
	painter.scale(scale, scale);
	painter.setPen(QPen{color, 15});
	painter.setBrush(Qt::NoBrush);
	painter.drawEllipse(QPointF{0, 0}, 76, 76);
	painter.setPen(Qt::NoPen);
	painter.setBrush(color);
	constexpr auto heights = std::array{40.0, 76.0, 112.0, 76.0, 40.0};
	for (int bar = 0; bar < 5; ++bar)
	{
		const auto x = (bar - 2) * 22.0 - 6.5;
		painter.drawRoundedRect(QRectF{x, -heights[bar] / 2, 13, heights[bar]}, 6.5, 6.5);
	}
	painter.restore();
}

//! The splash screen: the theme's green, waves across it, the glyph, the name and where it comes from
auto splash() -> QImage
{
	const auto width = 681;
	const auto height = 573;
	auto image = QImage{width, height, QImage::Format_ARGB32_Premultiplied};
	auto painter = QPainter{&image};
	painter.setRenderHint(QPainter::Antialiasing);

	auto background = QLinearGradient{0, 0, 0, static_cast<double>(height)};
	background.setColorAt(0, QColor{0x2f, 0xb8, 0x69});
	background.setColorAt(1, QColor{0x1e, 0x93, 0x50});
	painter.fillRect(image.rect(), background);

	// Waveforms across, faint: each a sum of two sines, damped towards the edges
	for (int line = 0; line < 9; ++line)
	{
		auto wave = QPainterPath{};
		const auto baseline = 60.0 + line * 58.0;
		const auto amplitude = 10.0 + 6.0 * std::sin(line * 1.3);
		for (int x = -10; x <= width + 10; x += 3)
		{
			const auto phase = x / static_cast<double>(width) * 6.28318;
			const auto y = baseline + amplitude * (std::sin(phase * (2.0 + line % 3) + line)
				+ 0.5 * std::sin(phase * (5.0 + line % 4) + 2 * line));
			if (x == -10) { wave.moveTo(x, y); }
			else { wave.lineTo(x, y); }
		}
		painter.setPen(QPen{QColor{255, 255, 255, 18 + 4 * (line % 3)}, 2.0});
		painter.drawPath(wave);
	}

	// A soft light behind the glyph
	auto glow = QRadialGradient{QPointF{width / 2.0, 235.0}, 190.0};
	glow.setColorAt(0, QColor{255, 255, 255, 45});
	glow.setColorAt(1, QColor{255, 255, 255, 0});
	painter.fillRect(image.rect(), glow);

	drawGlyph(painter, QPointF{width / 2.0, 239.0}, 80, QColor{0, 0, 0, 40});
	drawGlyph(painter, QPointF{width / 2.0, 235.0}, 80, Qt::white);

	auto name = QFont{"Arial Rounded MT Bold"};
	name.setPixelSize(48);
	painter.setFont(name);
	painter.setPen(QColor{0, 0, 0, 40});
	painter.drawText(QRectF{0, 343, static_cast<double>(width), 60}, Qt::AlignCenter, "LinuxMusic");
	painter.setPen(Qt::white);
	painter.drawText(QRectF{0, 340, static_cast<double>(width), 60}, Qt::AlignCenter, "LinuxMusic");

	auto credit = QFont{"Arial"};
	credit.setPixelSize(15);
	painter.setFont(credit);
	painter.setPen(QColor{255, 255, 255, 200});
	painter.drawText(QRectF{0, 400, static_cast<double>(width), 30}, Qt::AlignCenter,
		"a fork of LMMS · free software under the GNU GPL");
	return image;
}

//! The Windows installer's banner: the icon on white
auto banner(QSvgRenderer& icon) -> QImage
{
	auto image = QImage{150, 57, QImage::Format_RGB32};
	image.fill(Qt::white);
	auto painter = QPainter{&image};
	painter.setRenderHint(QPainter::Antialiasing);
	painter.drawImage(QPointF{22, 2}, render(icon, 53));
	return image;
}

} // namespace


int main(int argc, char* argv[])
{
	auto app = QGuiApplication{argc, argv};
	if (argc < 2)
	{
		std::fprintf(stderr, "usage: %s <LinuxMusic source tree>\n", argv[0]);
		return 1;
	}
	s_root = QString::fromLocal8Bit(argv[1]);
	auto icon = QSvgRenderer{path("tools/artwork/icon.svg")};
	auto iconSmall = QSvgRenderer{path("tools/artwork/icon-small.svg")};
	auto project = QSvgRenderer{path("tools/artwork/project.svg")};
	check(icon.isValid() && iconSmall.isValid() && project.isValid(), "reading the SVGs");
	// The icon at a size: the bolder one up to 24 pixels
	const auto appIcon = [&](int size) { return render(size <= 24 ? iconSmall : icon, size); };

	// The theme
	save(render(icon, 128), "data/themes/default/icon.png");
	save(render(icon, 32), "data/themes/default/icon_small.png");
	save(splash(), "data/themes/default/splash.png");

	// Linux: each size, and twice it for the @2 ones; the SVGs themselves
	for (const auto size : {16, 24, 32, 48, 64, 128, 256})
	{
		const auto folder = QString{"cmake/linux/icons/%1x%1"}.arg(size);
		save(appIcon(size), folder + "/apps/linuxmusic.png");
		save(render(project, size), folder + "/mimetypes/application-x-linuxmusic-project.png");
		if (size <= 128)
		{
			const auto doubled = folder + "@2";
			save(appIcon(2 * size), doubled + "/apps/linuxmusic.png");
			save(render(project, 2 * size), doubled + "/mimetypes/application-x-linuxmusic-project.png");
		}
	}
	for (const auto& [from, to] : {std::pair{"tools/artwork/icon.svg", "cmake/linux/icons/scalable/apps/linuxmusic.svg"},
		std::pair{"tools/artwork/project.svg", "cmake/linux/icons/scalable/mimetypes/application-x-linuxmusic-project.svg"}})
	{
		QFile::remove(path(to));
		check(QFile::copy(path(from), path(to)), to);
	}

	// Windows: the installer's logos and banner, the icons
	save(render(icon, 600), "cmake/nsis/assets/Logo.png");
	save(render(icon, 192), "cmake/nsis/assets/SmallLogo.png");
	{
		auto temporary = QTemporaryDir{};
		const auto bmp = temporary.filePath("banner.png");
		check(banner(icon).save(bmp), "the banner");
		run("magick", {bmp, "-type", "Palette", "-colors", "256", "BMP3:" + path("cmake/nsis/nsis_branding.bmp")});
	}
	for (const auto& [renderer, file] : {std::pair{&icon, "cmake/nsis/icon.ico"}, std::pair{&project, "cmake/nsis/project.ico"}})
	{
		auto temporary = QTemporaryDir{};
		auto sizes = QStringList{};
		for (const auto size : {16, 24, 32, 48, 64, 96, 128, 256})
		{
			const auto png = temporary.filePath(QString{"%1.png"}.arg(size));
			check((renderer == &icon ? appIcon(size) : render(*renderer, size)).save(png), png);
			sizes << png;
		}
		run("magick", sizes << path(file));
	}

	// macOS: the .icns from an iconset
	for (const auto& [renderer, file] : {std::pair{&icon, "cmake/apple/icon.icns"}, std::pair{&project, "cmake/apple/project.icns"}})
	{
		auto temporary = QTemporaryDir{};
		const auto set = temporary.filePath("icon.iconset");
		QDir{}.mkpath(set);
		for (const auto size : {16, 32, 128, 256, 512})
		{
			check((renderer == &icon ? appIcon(size) : render(*renderer, size))
				.save(QString{"%1/icon_%2x%2.png"}.arg(set).arg(size)), "iconset");
			check(render(*renderer, 2 * size).save(QString{"%1/icon_%2x%2@2x.png"}.arg(set).arg(size)), "iconset");
		}
		run("iconutil", {"-c", "icns", set, "-o", path(file)});
	}
	std::printf("done\n");
	return 0;
}
