/*
 * LoadCorruptedTest.cpp
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

#include <QDir>
#include <QFile>
#include <QRegularExpression>
#include <QTemporaryDir>
#include <QtTest>

#include <cmath>
#include <cstdint>

#include "AudioEngine.h"
#include "DataFile.h"
#include "Engine.h"
#include "Song.h"

using namespace lmms;

/**
 * Projects come from anywhere: damaged copies of the A/B projects (cut, bytes changed, numbers replaced by
 * absurd ones, elements removed, file paths huge or invalid) are loaded and played a little. Refusing them is
 * fine; crashing is not (with an AddressSanitizer build, neither is any memory error). The same damage, always:
 * the random numbers have a fixed seed. tools/check/corrupt-all.sh does it on a larger scale, a process each.
 */
class LoadCorruptedTest : public QObject
{
	Q_OBJECT

	QTemporaryDir m_directory;

	//! A fixed sequence of numbers (a linear congruential generator)
	struct Random
	{
		std::uint32_t state;
		auto next(std::uint32_t below) -> std::uint32_t
		{
			state = state * 1664525u + 1013904223u;
			return (state >> 8) % below;
		}
	};

	static auto variants(const QByteArray& project, std::uint32_t seed) -> QList<QPair<QString, QByteArray>>
	{
		auto result = QList<QPair<QString, QByteArray>>{};
		auto random = Random{seed};
		for (const auto cut : {0.05, 0.3, 0.6, 0.9, 0.99})
		{
			result.append({QString{"cut %1"}.arg(cut), project.left(static_cast<qsizetype>(project.size() * cut))});
		}
		for (int round = 0; round < 3; ++round)
		{
			auto data = project;
			for (int flip = 0; flip < 20; ++flip)
			{
				data[random.next(static_cast<std::uint32_t>(data.size()))] = static_cast<char>(random.next(256));
			}
			result.append({QString{"bytes %1"}.arg(round), data});
		}
		// Numbers become absurd: they turn into indices, sizes, positions, rates
		const auto text = QString::fromUtf8(project);
		static const auto number = QRegularExpression{"(\\w+)=\"(-?[0-9.]+)\""};
		auto matches = QList<QRegularExpressionMatch>{};
		for (auto it = number.globalMatch(text); it.hasNext();) { matches.append(it.next()); }
		for (const auto* bad : {"nan", "inf", "-1e30", "1e30", "-2147483648", "99999999999", "", "abc", "-1"})
		{
			auto damaged = text;
			// From the end, so that the matches' positions stay right
			for (auto i = matches.size() - 1; i >= 0; --i)
			{
				if (random.next(3) != 0) { continue; }
				const auto& match = matches[i];
				damaged.replace(match.capturedStart(2), match.capturedLength(2), QString::fromLatin1(bad));
			}
			result.append({QString{"numbers %1"}.arg(bad), damaged.toUtf8()});
		}
		// Elements removed
		static const auto element = QRegularExpression{"<(\\w+)[^<>]*/>"};
		auto elements = QList<QRegularExpressionMatch>{};
		for (auto it = element.globalMatch(text); it.hasNext();) { elements.append(it.next()); }
		for (int round = 0; round < 3 && !elements.isEmpty(); ++round)
		{
			const auto& match = elements[random.next(static_cast<std::uint32_t>(elements.size()))];
			auto damaged = text;
			damaged.remove(match.capturedStart(), match.capturedLength());
			result.append({QString{"without <%1>"}.arg(match.captured(1)), damaged.toUtf8()});
		}
		// Every file it names: huge, invalid, outside, a folder
		static const auto file = QRegularExpression{"(src|file|sf2|patch|wavefile|userwavefile)=\"([^\"]*)\""};
		for (const auto& path : {QString(100000, 'x'), QString{"\x01\x02\x7f"}, QString{"../../../../../../etc/passwd"},
			QString{"/"}, QString{"factorysample:"}})
		{
			auto damaged = text;
			damaged.replace(file, QString{"\\1=\"%1\""}.arg(path.toHtmlEscaped()));
			result.append({QString{"paths %1"}.arg(path.left(20)), damaged.toUtf8()});
		}
		return result;
	}

private slots:
	void initTestCase()
	{
		QVERIFY(m_directory.isValid());
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	void damagedProjectsLoadOrAreRefused_data()
	{
		QTest::addColumn<QString>("project");
		const auto projects = QDir{LMMS_TEST_AB_PROJECTS}.entryList({"*.mmp"}, QDir::Files, QDir::Name);
		QVERIFY(!projects.isEmpty());
		for (const auto& project : projects) { QTest::newRow(qPrintable(project)) << project; }
	}

	void damagedProjectsLoadOrAreRefused()
	{
		QFETCH(QString, project);
		auto source = QFile{QDir{LMMS_TEST_AB_PROJECTS}.filePath(project)};
		QVERIFY(source.open(QIODevice::ReadOnly));
		const auto data = source.readAll();
		auto seed = std::uint32_t{0};
		for (const auto c : project.toUtf8()) { seed = seed * 31 + static_cast<unsigned char>(c); }

		auto* song = Engine::getSong();
		for (const auto& [what, damaged] : variants(data, seed))
		{
			const auto path = m_directory.filePath("damaged.mmp");
			auto file = QFile{path};
			QVERIFY(file.open(QIODevice::WriteOnly | QIODevice::Truncate));
			file.write(damaged);
			file.close();

			song->loadProject(path);
			// A little of it played, whatever loaded
			song->playSong();
			for (int period = 0; period < 8; ++period)
			{
				for (const auto& frame : Engine::audioEngine()->renderNextPeriod())
				{
					QVERIFY2(std::isfinite(frame[0]) && std::isfinite(frame[1]), qPrintable(what));
				}
			}
			song->stop();
		}
		song->createNewProject();
	}
};

QTEST_GUILESS_MAIN(LoadCorruptedTest)
#include "LoadCorruptedTest.moc"
