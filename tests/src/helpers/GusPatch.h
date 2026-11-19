/*
 * GusPatch.h - Gravis Ultrasound patches (.pat) made for the tests (the program comes with none)
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

#ifndef LMMS_TESTS_GUS_PATCH_H
#define LMMS_TESTS_GUS_PATCH_H

#include <cmath>
#include <numbers>

#include <QByteArray>
#include <QDataStream>
#include <QFile>
#include <QString>

namespace lmms::tests
{

/**
 * Writes a patch at @p path: one sample, a second of a looped 440 Hz tone at @p rate in 16 bits, its root pitch
 * @p rootMilliHertz (thousandths of a hertz; 0 for a damaged one, with no pitch). @returns false if it could not
 */
inline auto writeGusPatch(const QString& path, int rate, quint32 rootMilliHertz = 440000) -> bool
{
	auto file = QFile{path};
	if (!file.open(QIODevice::WriteOnly)) { return false; }
	auto header = QByteArray(239, '\0');
	header.replace(0, 22, QByteArray("GF1PATCH110\0ID#000002", 22));
	header[82] = 1; // instruments
	header[151] = 1; // layers
	header[198] = 1; // samples
	file.write(header);

	// The sample's header, 96 bytes: little-endian fields, zeros for what the program skips (a stream on a byte
	// array writes from its start and cannot skip: each byte written)
	const auto frames = static_cast<quint32>(rate);
	auto sample = QByteArray{};
	auto stream = QDataStream{&sample, QIODevice::WriteOnly};
	stream.setByteOrder(QDataStream::LittleEndian);
	const auto zeros = [&stream](int count) { for (int i = 0; i < count; ++i) { stream << quint8{0}; } };
	zeros(8); // (its name, fractions)
	stream << quint32{frames * 2} << quint32{0} << quint32{frames * 2} << quint16(rate);
	zeros(8); // (its low and high frequency)
	stream << rootMilliHertz;
	zeros(21);
	stream << quint8{1 | 4}; // (16 bits, looping)
	zeros(40);
	if (sample.size() != 96) { return false; }
	file.write(sample);

	auto data = QByteArray{};
	auto samples = QDataStream{&data, QIODevice::WriteOnly};
	samples.setByteOrder(QDataStream::LittleEndian);
	for (quint32 frame = 0; frame < frames; ++frame)
	{
		samples << static_cast<qint16>(12000 * std::sin(2 * std::numbers::pi * 440 * frame / rate));
	}
	return file.write(data) == data.size();
}

} // namespace lmms::tests

#endif // LMMS_TESTS_GUS_PATCH_H
