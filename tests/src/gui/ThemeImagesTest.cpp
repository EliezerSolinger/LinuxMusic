/*
 * ThemeImagesTest.cpp
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

#include <QApplication>
#include <QDir>
#include <QDirIterator>
#include <QImageReader>
#include <QObject>
#include <QtTest>

#include "ConfigManager.h"
#include "LmmsStyle.h"
#include "embed.h"

using namespace lmms;

/**
 * The theme's images load without Qt's SVG image-format plugin, which Linux distributions ship apart: without it,
 * the SVG icons (the faders' handles among them) came out empty.
 */
class ThemeImagesTest : public QObject
{
	Q_OBJECT

private slots:
	void initTestCase()
	{
		// No plugins from here on: the image formats are loaded on first use, after this
		QCoreApplication::setLibraryPaths({});
		if (QImageReader::supportedImageFormats().contains("svg"))
		{
			QSKIP("The SVG image-format plugin is built into this Qt");
		}
		ConfigManager::inst();
		QDir::addSearchPath("artwork", ConfigManager::inst()->defaultThemeDir());
		QDir::setSearchPaths("resources", {ConfigManager::inst()->defaultThemeDir()});
	}

	void everySvgIconLoads()
	{
		auto count = 0;
		for (auto it = QDirIterator{ConfigManager::inst()->defaultThemeDir(), {"*.svg"}}; it.hasNext();)
		{
			const auto name = QFileInfo{it.next()}.completeBaseName().toStdString();
			const auto pixmap = embed::getIconPixmap(name);
			QVERIFY2(pixmap.width() > 1 && pixmap.height() > 1, name.c_str());
			++count;
		}
		QVERIFY(count > 0);

		// At a size asked for too
		QCOMPARE(embed::getIconPixmap("gear", 20, 20).deviceIndependentSize().toSize(), QSize(20, 20));
	}

	void theStyleSheetPointsAtImagesQtCanRead()
	{
		auto style = gui::LmmsStyle{};
		const auto sheet = qApp->styleSheet();
		QVERIFY(!sheet.isEmpty());
		QVERIFY2(!sheet.contains(".svg"), qPrintable(sheet.left(200)));
		QVERIFY(sheet.contains("gear.png"));
	}
};

QTEST_MAIN(ThemeImagesTest)
#include "ThemeImagesTest.moc"
