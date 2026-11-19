/*
 * CompressedProjectTest.cpp
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

#include <QtTest>

#include "DataFile.h"

using namespace lmms;

//! Compressed projects (.mmpz) come from anywhere: their inflating is limited (see DataFile::uncompress())
class CompressedProjectTest : public QObject
{
	Q_OBJECT

	static auto withHeader(const QByteArray& compressed, quint32 size) -> QByteArray
	{
		auto data = compressed;
		data[0] = static_cast<char>(size >> 24);
		data[1] = static_cast<char>(size >> 16);
		data[2] = static_cast<char>(size >> 8);
		data[3] = static_cast<char>(size);
		return data;
	}

private slots:
	void whatQtCompressedComesBack()
	{
		const auto text = QByteArray{"<?xml version=\"1.0\"?><lmms-project/>"}.repeated(1000);
		const auto data = DataFile::uncompress(qCompress(text));
		QVERIFY(data.has_value());
		QCOMPARE(*data, text);
	}

	void notCompressedDataIsNothing()
	{
		QVERIFY(!DataFile::uncompress(QByteArray{}).has_value());
		QVERIFY(!DataFile::uncompress(QByteArray{"<?xml version=\"1.0\"?><lmms-project/>"}).has_value());
	}

	//! Saying it is bigger than the limit: refused before inflating anything
	void aSizeAboveTheLimitIsRefused()
	{
		const auto compressed = qCompress(QByteArray{"small"});
		QVERIFY(!DataFile::uncompress(withHeader(compressed, 0xffffffffu)).has_value());
		QVERIFY(!DataFile::uncompress(compressed, 4).has_value());
		QVERIFY(DataFile::uncompress(compressed, 5).has_value());
	}

	//! A bomb: a few kilobytes that inflate to far more than the size they say
	void inflatingPastTheSizeItSaysIsRefused()
	{
		const auto big = QByteArray(64 << 20, 'a');
		const auto compressed = qCompress(big, 9);
		QVERIFY(compressed.size() < (1 << 20));
		QVERIFY(!DataFile::uncompress(withHeader(compressed, 1000)).has_value());
		// (and inflating less than it says is refused as well)
		QVERIFY(!DataFile::uncompress(withHeader(compressed, (64 << 20) + 1)).has_value());
	}

	void aCutStreamIsRefused()
	{
		const auto compressed = qCompress(QByteArray(100000, 'b'));
		QVERIFY(!DataFile::uncompress(compressed.left(compressed.size() / 2)).has_value());
		auto damaged = compressed;
		damaged[10] = static_cast<char>(damaged[10] ^ 0x5a);
		const auto result = DataFile::uncompress(damaged);
		QVERIFY(!result.has_value() || *result == QByteArray(100000, 'b'));
	}
};

QTEST_GUILESS_MAIN(CompressedProjectTest)
#include "CompressedProjectTest.moc"
