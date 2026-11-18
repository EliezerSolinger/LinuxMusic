/*
 * ExportProcessor.h - finishes a rendered song: normalization, fades, dither, several formats at once
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

#ifndef LMMS_EXPORT_PROCESSOR_H
#define LMMS_EXPORT_PROCESSOR_H

#include <atomic>
#include <functional>
#include <limits>
#include <vector>

#include <QString>
#include <QStringList>

#include "lmms_export.h"
#include "lmmsconfig.h"

namespace lmms
{

/**
 * The export wizard's last steps (docs/plano-anotacoes-13-11.md, 12.1): the song is rendered once, to a 32-bit
 * float WAV; this measures it, then writes each format asked for from it, with the same post-processing:
 * normalization (to a peak, or to an integrated loudness per ITU-R BS.1770, K-weighted and gated), fades at the
 * ends, and TPDF dither when the samples become integers of fewer bits.
 *
 * Any thread.
 */
class LMMS_EXPORT ExportProcessor
{
public:
	enum class Normalization
	{
		None,
		Peak,		//!< the highest sample at the target, in dBFS
		Loudness	//!< the integrated loudness at the target, in LUFS
	};

	enum class Format
	{
		Wav,
		Flac,
		Ogg,
		Mp3
	};

	struct Output
	{
		Format format = Format::Wav;
		int bits = 24;			//!< WAV: 16, 24 or 32 (float); FLAC: 16 or 24
		int kbps = 320;			//!< MP3
		double quality = 0.6;	//!< OGG: 0 (smallest) to 1 (best)
	};

	struct Settings
	{
		Normalization normalization = Normalization::None;
		double target = -1.0;	//!< dBFS (Peak) or LUFS (Loudness)
		double fadeIn = 0.0;	//!< seconds
		double fadeOut = 0.0;	//!< seconds
		bool dither = true;		//!< on WAV or FLAC of 16 or 24 bits
		std::vector<Output> outputs;
		QString folder;
		QString name;			//!< the files' name, without extension
	};

	struct Measure
	{
		double peak = 0.0;			//!< linear, the highest sample
		double loudness = -1e9;		//!< LUFS, integrated (-1e9: silence)
		//! What only libebur128 measures (NaN without it): the true peak (linear, between the samples too, 4x
		//! oversampled, EBU Tech 3341) and the loudness range (LU, EBU Tech 3342)
		double truePeak = std::numeric_limits<double>::quiet_NaN();
		double loudnessRange = std::numeric_limits<double>::quiet_NaN();
	};

	//! Which code measures (docs/plano-refatoracao.md, phase 10.1: both are kept to be compared)
	enum class Meter
	{
		Own,		//!< ours: BS.1770 integrated loudness and the sample peak (every channel weighted 1)
		Ebur128		//!< libebur128: the same (BS.1770's surround weights too), the true peak and the loudness range
	};
	//! Whether this build has libebur128
	static auto hasEbur128() -> bool;

	//! Measures the rendering @p source (libebur128 when this build has it); @p ok tells whether it could be read
	static auto measure(const QString& source, bool* ok = nullptr) -> Measure;
	//! measure() with @p meter (nothing measured, @p ok false, for libebur128 in a build without it)
	static auto measureWith(const QString& source, Meter meter, bool* ok = nullptr) -> Measure;

	//! @returns the extension of @p format's files
	static auto extension(Format format) -> QString;
	//! Whether this build can write @p format
	static auto canWrite(Format format) -> bool;

	struct Result
	{
		QStringList written;	//!< the files made
		QString error;			//!< what failed, if anything (what was written before stays)
		bool cancelled = false;
	};

	/**
	 * Writes the outputs of @p settings from the rendering @p source. @p progress (0 to 1) is called now and then;
	 * @p cancel stops it (a file being written is removed).
	 */
	static auto process(const QString& source, const Settings& settings, const std::atomic<bool>& cancel,
		const std::function<void(double)>& progress) -> Result;

private:
	static auto measureOwn(const QString& source, bool* ok) -> Measure;
#ifdef LMMS_HAVE_EBUR128
	static auto measureEbur128(const QString& source, bool* ok) -> Measure;
#endif
};

} // namespace lmms

#endif // LMMS_EXPORT_PROCESSOR_H
