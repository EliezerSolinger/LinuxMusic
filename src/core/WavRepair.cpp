/*
 * WavRepair.cpp
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

#include "WavRepair.h"

#include <algorithm>

#include <QFile>
#include <QtEndian>

namespace lmms::WavRepair
{

namespace
{

auto readLittle32(QFile& file, qint64 offset, quint32& value) -> bool
{
	uchar bytes[4];
	if (!file.seek(offset) || file.read(reinterpret_cast<char*>(bytes), 4) != 4) { return false; }
	value = qFromLittleEndian<quint32>(bytes);
	return true;
}

auto writeLittle32(QFile& file, qint64 offset, quint32 value) -> bool
{
	uchar bytes[4];
	qToLittleEndian(value, bytes);
	return file.seek(offset) && file.write(reinterpret_cast<const char*>(bytes), 4) == 4;
}

} // namespace


auto completeLength(const QString& path) -> std::int64_t
{
	auto file = QFile{path};
	if (!file.open(QIODevice::ReadWrite)) { return -1; }
	const auto size = file.size();

	char riff[12];
	if (file.read(riff, 12) != 12 || QByteArray(riff, 4) != "RIFF" || QByteArray(riff + 8, 4) != "WAVE") { return -1; }

	// The chunks up to the data, which runs to the end of the file (it is written last)
	auto blockAlign = quint32{0};
	auto offset = qint64{12};
	while (offset + 8 <= size)
	{
		char id[4];
		auto chunkSize = quint32{0};
		if (!file.seek(offset) || file.read(id, 4) != 4 || !readLittle32(file, offset + 4, chunkSize)) { return -1; }
		const auto name = QByteArray(id, 4);
		if (name == "fmt ")
		{
			uchar align[2];
			if (!file.seek(offset + 8 + 12) || file.read(reinterpret_cast<char*>(align), 2) != 2) { return -1; }
			blockAlign = qFromLittleEndian<quint16>(align);
		}
		else if (name == "data")
		{
			if (blockAlign == 0) { return -1; }
			const auto dataStart = offset + 8;
			auto dataBytes = size - dataStart;
			dataBytes -= dataBytes % blockAlign;
			// A WAV file says at most 4 GB
			dataBytes = std::min<qint64>(dataBytes, (qint64{0xFFFFFFFF} - dataStart) / blockAlign * blockAlign);
			if (!writeLittle32(file, offset + 4, static_cast<quint32>(dataBytes))
				|| !writeLittle32(file, 4, static_cast<quint32>(dataStart + dataBytes - 8)))
			{
				return -1;
			}
			file.close();
			return dataBytes / blockAlign;
		}
		offset += 8 + chunkSize + (chunkSize & 1);
	}
	return -1;
}

} // namespace lmms::WavRepair
