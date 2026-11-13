/*
 * PeakCache.cpp
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


#include "PeakCache.h"

#include <algorithm>

#include <QCoreApplication>
#include <QCryptographicHash>
#include <QDataStream>
#include <QDir>
#include <QFileInfo>
#include <QSaveFile>
#include <QStandardPaths>
#include <sndfile.h>

namespace lmms
{

namespace
{

constexpr quint32 Magic = 0x4c4d504b; // "LMPK"
constexpr quint32 Version = 1;

//! What a peak file is checked against: the audio file's size and time of change
struct Stamp
{
	qint64 size = -1;
	qint64 modified = -1;
};

auto stampOf(const QString& file) -> Stamp
{
	const auto info = QFileInfo{file};
	if (!info.exists()) { return {}; }
	return {info.size(), info.lastModified().toMSecsSinceEpoch()};
}

auto cacheFolder() -> QString
{
	if (const auto custom = qEnvironmentVariable("LINUXMUSIC_CACHE_DIR"); !custom.isEmpty())
	{
		return custom + "/peaks";
	}
	return QStandardPaths::writableLocation(QStandardPaths::CacheLocation) + "/peaks";
}

auto absolute(const QString& file) -> QString
{
	return QFileInfo{file}.absoluteFilePath();
}

} // namespace




void PeakBuilder::add(const float* interleaved, std::size_t frames, int channels)
{
	if (channels <= 0) { return; }
	for (std::size_t frame = 0; frame < frames; ++frame)
	{
		const auto* at = interleaved + frame * channels;
		// Summed to mono as the in-memory thumbnails (SampleThumbnailCache): mono as is, the first two channels
		// averaged otherwise (what the decoder keeps)
		const auto value = channels == 1 ? at[0] : (at[0] + at[1]) / 2.f;
		m_partial.min = std::min(m_partial.min, value);
		m_partial.max = std::max(m_partial.max, value);
		if (++m_inPartial == static_cast<std::size_t>(PeakCache::FramesPerPeak))
		{
			m_peaks.push_back(m_partial);
			m_partial = Peak{};
			m_inPartial = 0;
		}
	}
	m_frames += static_cast<std::int64_t>(frames);
}




auto PeakBuilder::peaks(bool withPartial) const -> std::vector<Peak>
{
	auto result = m_peaks;
	if (withPartial && m_inPartial > 0) { result.push_back(m_partial); }
	return result;
}




PeakCache::PeakCache()
{
	m_pool.setMaxThreadCount(2);
	m_deliverTimer.setInterval(30);
	connect(&m_deliverTimer, &QTimer::timeout, this, &PeakCache::deliver);
}




PeakCache::~PeakCache()
{
	m_pool.clear();
	m_pool.waitForDone();
}




auto PeakCache::instance() -> PeakCache&
{
	// Owned by the application, so the builds end before it does
	static auto* s_instance = [] {
		auto* cache = new PeakCache{};
		if (auto* app = QCoreApplication::instance()) { cache->setParent(app); }
		return cache;
	}();
	return *s_instance;
}




auto PeakCache::peakFileOf(const QString& file) -> QString
{
	const auto hash = QCryptographicHash::hash(absolute(file).toUtf8(), QCryptographicHash::Sha1).toHex();
	return cacheFolder() + "/" + QString::fromLatin1(hash) + ".peaks";
}




auto PeakCache::readPeakFile(const QString& file) -> std::shared_ptr<const SampleThumbnailCache>
{
	auto peakFile = QFile{peakFileOf(file)};
	if (!peakFile.open(QIODevice::ReadOnly)) { return nullptr; }
	auto in = QDataStream{&peakFile};
	in.setByteOrder(QDataStream::LittleEndian);
	in.setFloatingPointPrecision(QDataStream::SinglePrecision);

	quint32 magic = 0, version = 0;
	qint64 size = 0, modified = 0, frames = 0, count = 0;
	qint32 framesPerPeak = 0;
	in >> magic >> version >> size >> modified >> frames >> framesPerPeak >> count;
	const auto stamp = stampOf(file);
	if (in.status() != QDataStream::Ok || magic != Magic || version != Version || size != stamp.size
		|| modified != stamp.modified || framesPerPeak != FramesPerPeak || frames <= 0 || count <= 0
		|| count != (frames + FramesPerPeak - 1) / FramesPerPeak
		|| peakFile.size() - peakFile.pos() != count * 2 * static_cast<qint64>(sizeof(float)))
	{
		return nullptr;
	}

	auto peaks = std::vector<PeakBuilder::Peak>(static_cast<std::size_t>(count));
	for (auto& peak : peaks) { in >> peak.min >> peak.max; }
	if (in.status() != QDataStream::Ok) { return nullptr; }
	return std::make_shared<const SampleThumbnailCache>(std::move(peaks), static_cast<f_cnt_t>(frames));
}




auto PeakCache::writePeakFile(const QString& file, const std::vector<PeakBuilder::Peak>& peaks, std::int64_t frames)
	-> bool
{
	const auto path = peakFileOf(file);
	if (!QDir{}.mkpath(QFileInfo{path}.absolutePath())) { return false; }
	const auto stamp = stampOf(file);
	// (written whole or not at all)
	auto out = QSaveFile{path};
	if (!out.open(QIODevice::WriteOnly)) { return false; }
	auto stream = QDataStream{&out};
	stream.setByteOrder(QDataStream::LittleEndian);
	stream.setFloatingPointPrecision(QDataStream::SinglePrecision);
	stream << Magic << Version << static_cast<qint64>(stamp.size) << static_cast<qint64>(stamp.modified)
		<< static_cast<qint64>(frames) << static_cast<qint32>(FramesPerPeak) << static_cast<qint64>(peaks.size());
	for (const auto& peak : peaks) { stream << peak.min << peak.max; }
	return stream.status() == QDataStream::Ok && out.commit();
}




auto PeakCache::build(const QString& file) -> std::shared_ptr<const SampleThumbnailCache>
{
	auto info = SF_INFO{};
	auto* sndFile = sf_open(QFile::encodeName(file).constData(), SFM_READ, &info);
	if (!sndFile) { return nullptr; }

	auto builder = PeakBuilder{};
	constexpr sf_count_t Chunk = 65536;
	auto buffer = std::vector<float>(static_cast<std::size_t>(Chunk * info.channels));
	sf_count_t read = 0;
	while ((read = sf_readf_float(sndFile, buffer.data(), Chunk)) > 0)
	{
		builder.add(buffer.data(), static_cast<std::size_t>(read), info.channels);
	}
	sf_close(sndFile);
	if (builder.frames() == 0) { return nullptr; }

	auto peaks = builder.peaks();
	// (a cache that cannot be written only costs building again next time)
	writePeakFile(file, peaks, builder.frames());
	return std::make_shared<const SampleThumbnailCache>(std::move(peaks), static_cast<f_cnt_t>(builder.frames()));
}




auto PeakCache::fetch(const QString& fileName) -> std::shared_ptr<const SampleThumbnailCache>
{
	const auto file = absolute(fileName);
	if (const auto found = m_known.find(file); found != m_known.end())
	{
		if (auto peaks = found->second.lock()) { return peaks; }
		m_known.erase(found);
	}
	if (auto peaks = readPeakFile(file))
	{
		m_known[file] = peaks;
		return peaks;
	}
	if (m_building.insert(file).second)
	{
		m_pool.start([this, file] {
			auto peaks = build(file);
			// Back to the main thread (deliver()): kept, and told
			const auto lock = std::lock_guard{m_doneMutex};
			m_done.emplace_back(file, std::move(peaks));
		});
		m_deliverTimer.start();
	}
	return nullptr;
}




void PeakCache::deliver()
{
	auto done = [this] {
		const auto lock = std::lock_guard{m_doneMutex};
		return std::exchange(m_done, {});
	}();
	for (auto& [file, peaks] : done)
	{
		m_building.erase(file);
		if (!peaks) { continue; }
		// (held while told, for the ones that fetch them again on ready())
		m_known[file] = peaks;
		m_holding = std::move(peaks);
		emit ready(file);
		m_holding.reset();
	}
	if (m_building.empty()) { m_deliverTimer.stop(); }
}




void PeakCache::store(const QString& fileName, std::vector<PeakBuilder::Peak> peaks, std::int64_t frames)
{
	if (peaks.empty() || frames <= 0) { return; }
	const auto file = absolute(fileName);
	writePeakFile(file, peaks, frames);
	auto cache = std::make_shared<const SampleThumbnailCache>(std::move(peaks), static_cast<f_cnt_t>(frames));
	m_known[file] = cache;
	m_holding = cache; // (until a clip takes it)
	emit ready(file);
	m_holding.reset();
}




void PeakCache::forget(const QString& fileName)
{
	const auto file = absolute(fileName);
	m_known.erase(file);
	QFile::remove(peakFileOf(file));
}




void PeakCache::waitForBuilds()
{
	m_pool.waitForDone();
	deliver();
}

} // namespace lmms
