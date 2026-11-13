/*
 * PeakCache.h
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


#ifndef LMMS_PEAK_CACHE_H
#define LMMS_PEAK_CACHE_H

#include <map>
#include <memory>
#include <mutex>
#include <set>
#include <vector>

#include <QObject>
#include <QString>
#include <QThreadPool>
#include <QTimer>

#include "SampleThumbnailCache.h"
#include "lmms_export.h"

namespace lmms
{

/**
 * Accumulates the peaks of audio as it comes (from a file being read, or from a recording as it is captured): each
 * peak is the lowest and highest value, summed to mono, of PeakCache::FramesPerPeak frames.
 */
class LMMS_EXPORT PeakBuilder
{
public:
	using Peak = SampleThumbnailCache::Thumbnail::Peak;

	//! Adds @p frames frames of @p channels interleaved channels
	void add(const float* interleaved, std::size_t frames, int channels);
	//! The peaks of whole blocks so far, plus the partial one at the end if @p withPartial
	auto peaks(bool withPartial = true) const -> std::vector<Peak>;
	auto frames() const -> std::int64_t { return m_frames; }

private:
	std::vector<Peak> m_peaks;
	Peak m_partial;
	std::size_t m_inPartial = 0;
	std::int64_t m_frames = 0;
};


/**
 * The peaks of audio files for drawing their waveform without loading them (docs/plano-gravacao.md, "Mídia do
 * projeto"): a file streamed from disk is never in memory as a whole.
 *
 * The peaks of a file are kept in a peak file in the user's cache folder (one per file, named after a hash of its
 * path; LINUXMUSIC_CACHE_DIR moves the folder), checked against the file's size and time of change, so they are
 * built once. Building runs in the background; fetch() returns what is known at once and ready() tells when a file's
 * peaks have been built. Main thread, except build(), which is reentrant.
 */
class LMMS_EXPORT PeakCache : public QObject
{
	Q_OBJECT
public:
	static constexpr std::int64_t FramesPerPeak = 256;

	static auto instance() -> PeakCache&;

	//! The peaks of @p file, if they are known (in memory or in a peak file still valid). Otherwise null, and they
	//! start being built in the background (ready() follows).
	auto fetch(const QString& file) -> std::shared_ptr<const SampleThumbnailCache>;

	//! Reads @p file and returns its peaks, writing its peak file (null if it cannot be read)
	static auto build(const QString& file) -> std::shared_ptr<const SampleThumbnailCache>;

	//! The peaks in the peak file of @p file, if it is there and still matches the file
	static auto readPeakFile(const QString& file) -> std::shared_ptr<const SampleThumbnailCache>;

	//! Where the peak file of @p file is
	static auto peakFileOf(const QString& file) -> QString;

	//! Forgets the peaks of @p file (it was rewritten) and removes its peak file
	void forget(const QString& file);

	//! Waits until no peaks are being built (for tests and when quitting)
	void waitForBuilds();

	~PeakCache() override;

signals:
	void ready(const QString& file);

private:
	PeakCache();
	static auto writePeakFile(const QString& file, const std::vector<PeakBuilder::Peak>& peaks, std::int64_t frames)
		-> bool;

	std::map<QString, std::weak_ptr<const SampleThumbnailCache>> m_known;
	std::set<QString> m_building;
	std::shared_ptr<const SampleThumbnailCache> m_holding; //!< the peaks just built, while ready() is emitted

	//! Builds done, handed from the builders to the main thread (which looks for them while builds run)
	void deliver();
	QTimer m_deliverTimer;
	std::mutex m_doneMutex;
	std::vector<std::pair<QString, std::shared_ptr<const SampleThumbnailCache>>> m_done;
	QThreadPool m_pool;
};

} // namespace lmms

#endif // LMMS_PEAK_CACHE_H
