/*
 * ExportProcessor.cpp - finishes a rendered song: normalization, fades, dither, several formats at once
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

#include "ExportProcessor.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <memory>
#include <numbers>
#include <random>

#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <sndfile.h>

#include "lmmsconfig.h"

namespace lmms
{

namespace
{

constexpr auto BlockFrames = sf_count_t{8192};

auto tr(const char* text) -> QString
{
	return QCoreApplication::translate("ExportProcessor", text);
}

struct SndFileCloser
{
	void operator()(SNDFILE* file) const { sf_close(file); }
};
using SndFile = std::unique_ptr<SNDFILE, SndFileCloser>;

//! A biquad, transposed direct form II, one per channel
class Biquad
{
public:
	Biquad(double b0, double b1, double b2, double a0, double a1, double a2) :
		m_b0{b0 / a0}, m_b1{b1 / a0}, m_b2{b2 / a0}, m_a1{a1 / a0}, m_a2{a2 / a0} {}

	auto process(double x) -> double
	{
		const double y = m_b0 * x + m_z1;
		m_z1 = m_b1 * x - m_a1 * y + m_z2;
		m_z2 = m_b2 * x - m_a2 * y;
		return y;
	}

private:
	double m_b0, m_b1, m_b2, m_a1, m_a2;
	double m_z1 = 0.0, m_z2 = 0.0;
};

//! ITU-R BS.1770's K-weighting at @p rate: a high shelf (the head) and a high pass (RLB), designed from their
//! analog prototypes as libebur128 does, which gives the standard's coefficients at 48 kHz exactly
auto kWeighting(double rate) -> std::array<Biquad, 2>
{
	const auto shelf = [rate] {
		const double f0 = 1681.974450955533, gain = 3.999843853973347, q = 0.7071752369554196;
		const double k = std::tan(std::numbers::pi * f0 / rate);
		const double vh = std::pow(10.0, gain / 20.0);
		const double vb = std::pow(vh, 0.4996667741545416);
		return Biquad{vh + vb * k / q + k * k, 2.0 * (k * k - vh), vh - vb * k / q + k * k,
			1.0 + k / q + k * k, 2.0 * (k * k - 1.0), 1.0 - k / q + k * k};
	}();
	const auto highPass = [rate] {
		const double f0 = 38.13547087602444, q = 0.5003270373238773;
		const double k = std::tan(std::numbers::pi * f0 / rate);
		const double a0 = 1.0 + k / q + k * k;
		// (unity gain in the pass band: the standard's b are 1, -2, 1 over that a0)
		return Biquad{a0, -2.0 * a0, a0, a0, 2.0 * (k * k - 1.0), 1.0 - k / q + k * k};
	}();
	return {shelf, highPass};
}

auto open(const QString& path, SF_INFO& info) -> SndFile
{
	info = SF_INFO{};
	return SndFile{sf_open(QFile::encodeName(path).constData(), SFM_READ, &info)};
}

//! The gain of the fades at frame @p frame of @p total: a raised cosine at each end
auto fadeGain(sf_count_t frame, sf_count_t total, sf_count_t fadeIn, sf_count_t fadeOut) -> float
{
	auto gain = 1.0;
	if (fadeIn > 0 && frame < fadeIn) { gain *= 0.5 - 0.5 * std::cos(std::numbers::pi * frame / fadeIn); }
	const auto fromEnd = total - 1 - frame;
	if (fadeOut > 0 && fromEnd < fadeOut) { gain *= 0.5 - 0.5 * std::cos(std::numbers::pi * fromEnd / fadeOut); }
	return static_cast<float>(gain);
}

} // namespace


auto ExportProcessor::measure(const QString& source, bool* ok) -> Measure
{
	auto info = SF_INFO{};
	auto file = open(source, info);
	if (ok) { *ok = file != nullptr && info.channels > 0; }
	auto result = Measure{};
	if (!file || info.channels <= 0) { return result; }

	const auto channels = info.channels;
	auto filters = std::vector<std::array<Biquad, 2>>{};
	for (int c = 0; c < channels; ++c) { filters.push_back(kWeighting(info.samplerate)); }

	// The loudness of 400 ms blocks every 100 ms (BS.1770): the K-weighted mean square of each 100 ms hop, summed
	// over the channels (each with its weight: 1 for the front ones), four hops a block
	const auto hop = std::max<sf_count_t>(1, info.samplerate / 10);
	auto hops = std::vector<double>{};
	auto hopSum = 0.0;
	auto hopFrames = sf_count_t{0};
	auto buffer = std::vector<float>(static_cast<std::size_t>(BlockFrames * channels));
	for (sf_count_t got; (got = sf_readf_float(file.get(), buffer.data(), BlockFrames)) > 0;)
	{
		for (sf_count_t f = 0; f < got; ++f)
		{
			for (int c = 0; c < channels; ++c)
			{
				const auto x = static_cast<double>(buffer[f * channels + c]);
				result.peak = std::max(result.peak, std::abs(x));
				const auto k = filters[c][1].process(filters[c][0].process(x));
				hopSum += k * k;
			}
			if (++hopFrames == hop)
			{
				hops.push_back(hopSum / static_cast<double>(hop));
				hopSum = 0.0;
				hopFrames = 0;
			}
		}
	}

	auto blocks = std::vector<double>{}; // each block's mean square, summed over the channels
	for (std::size_t i = 3; i < hops.size(); ++i) { blocks.push_back((hops[i - 3] + hops[i - 2] + hops[i - 1] + hops[i]) / 4.0); }
	const auto loudnessOf = [](double z) { return -0.691 + 10.0 * std::log10(z); };
	const auto gatedMean = [&](double gate) {
		auto sum = 0.0;
		auto count = 0;
		for (const auto z : blocks)
		{
			if (z > 0.0 && loudnessOf(z) > gate) { sum += z; ++count; }
		}
		return count > 0 ? sum / count : 0.0;
	};
	// Absolute gate at -70 LUFS, then a relative one 10 LU below what passed it
	const auto absolute = gatedMean(-70.0);
	if (absolute <= 0.0) { return result; }
	const auto relative = gatedMean(loudnessOf(absolute) - 10.0);
	result.loudness = relative > 0.0 ? loudnessOf(relative) : loudnessOf(absolute);
	return result;
}


auto ExportProcessor::extension(Format format) -> QString
{
	switch (format)
	{
	case Format::Wav: return "wav";
	case Format::Flac: return "flac";
	case Format::Ogg: return "ogg";
	case Format::Mp3: return "mp3";
	}
	return "wav";
}


auto ExportProcessor::canWrite(Format format) -> bool
{
#ifdef LMMS_HAVE_SNDFILE_MP3
	return true;
#else
	return format != Format::Mp3;
#endif
}


auto ExportProcessor::process(const QString& source, const Settings& settings, const std::atomic<bool>& cancel,
	const std::function<void(double)>& progress) -> Result
{
	auto result = Result{};
	auto ok = false;
	const auto measured = measure(source, &ok);
	if (!ok)
	{
		result.error = tr("The rendering could not be read.");
		return result;
	}

	// The gain normalization asks for
	auto gain = 1.0;
	if (settings.normalization == Normalization::Peak && measured.peak > 0.0)
	{
		gain = std::pow(10.0, settings.target / 20.0) / measured.peak;
	}
	else if (settings.normalization == Normalization::Loudness && measured.loudness > -1e8)
	{
		gain = std::pow(10.0, (settings.target - measured.loudness) / 20.0);
	}

	if (!QDir{}.mkpath(settings.folder))
	{
		result.error = tr("The folder %1 could not be created.").arg(settings.folder);
		return result;
	}

	auto random = std::minstd_rand{0x5eed};
	auto uniform = std::uniform_real_distribution<float>{-0.5f, 0.5f};
	const auto outputs = static_cast<double>(settings.outputs.size());
	for (std::size_t o = 0; o < settings.outputs.size(); ++o)
	{
		const auto& output = settings.outputs[o];
		if (!canWrite(output.format))
		{
			result.error = tr("This build cannot write %1 files.").arg(extension(output.format).toUpper());
			return result;
		}
		auto info = SF_INFO{};
		auto input = open(source, info);
		if (!input)
		{
			result.error = tr("The rendering could not be read.");
			return result;
		}

		auto outInfo = SF_INFO{};
		outInfo.samplerate = info.samplerate;
		outInfo.channels = info.channels;
		auto ditherBits = 0;
		switch (output.format)
		{
		case Format::Wav:
			outInfo.format = SF_FORMAT_WAV | (output.bits == 16 ? SF_FORMAT_PCM_16 : output.bits == 24 ? SF_FORMAT_PCM_24
				: SF_FORMAT_FLOAT);
			ditherBits = output.bits == 16 || output.bits == 24 ? output.bits : 0;
			break;
		case Format::Flac:
			outInfo.format = SF_FORMAT_FLAC | (output.bits == 16 ? SF_FORMAT_PCM_16 : SF_FORMAT_PCM_24);
			ditherBits = output.bits == 16 ? 16 : 24;
			break;
		case Format::Ogg:
			outInfo.format = SF_FORMAT_OGG | SF_FORMAT_VORBIS;
			break;
		case Format::Mp3:
#ifdef LMMS_HAVE_SNDFILE_MP3
			outInfo.format = SF_FORMAT_MPEG | SF_FORMAT_MPEG_LAYER_III;
#endif
			break;
		}
		if (!settings.dither) { ditherBits = 0; }

		const auto path = settings.folder + '/' + settings.name + '.' + extension(output.format);
		const auto partial = path + ".part";
		auto file = SndFile{sf_open(QFile::encodeName(partial).constData(), SFM_WRITE, &outInfo)};
		if (!file)
		{
			result.error = tr("%1 could not be created: %2").arg(path, QString::fromUtf8(sf_strerror(nullptr)));
			return result;
		}
		// The encoders' quality: OGG's from its setting; MP3 at a constant bit rate (libsndfile maps 0 to 320 kbps
		// and 1 to 32 kbps)
		if (output.format == Format::Ogg)
		{
			auto level = std::clamp(1.0 - output.quality, 0.0, 1.0);
			sf_command(file.get(), SFC_SET_COMPRESSION_LEVEL, &level, sizeof(level));
		}
#ifdef LMMS_HAVE_SNDFILE_MP3
		if (output.format == Format::Mp3)
		{
			auto mode = static_cast<int>(SF_BITRATE_MODE_CONSTANT);
			sf_command(file.get(), SFC_SET_BITRATE_MODE, &mode, sizeof(mode));
			auto level = std::clamp((320.0 - output.kbps) / (320.0 - 32.0), 0.0, 1.0);
			sf_command(file.get(), SFC_SET_COMPRESSION_LEVEL, &level, sizeof(level));
		}
#endif
		// Floats are written as they are (a peak over 0 dBFS stays, in a float WAV); integers clip
		auto clip = SF_TRUE;
		sf_command(file.get(), SFC_SET_CLIPPING, &clip, sizeof(clip));

		const auto channels = info.channels;
		const auto total = info.frames;
		const auto fadeIn = static_cast<sf_count_t>(settings.fadeIn * info.samplerate);
		const auto fadeOut = static_cast<sf_count_t>(settings.fadeOut * info.samplerate);
		const auto lsb = ditherBits > 0 ? std::ldexp(1.0f, -(ditherBits - 1)) : 0.f;
		auto buffer = std::vector<float>(static_cast<std::size_t>(BlockFrames * channels));
		auto frame = sf_count_t{0};
		auto failed = false;
		for (sf_count_t got; (got = sf_readf_float(input.get(), buffer.data(), BlockFrames)) > 0;)
		{
			if (cancel.load(std::memory_order_relaxed))
			{
				file.reset();
				QFile::remove(partial);
				result.cancelled = true;
				return result;
			}
			for (sf_count_t f = 0; f < got; ++f, ++frame)
			{
				const auto g = static_cast<float>(gain) * fadeGain(frame, total, fadeIn, fadeOut);
				for (int c = 0; c < channels; ++c)
				{
					auto& x = buffer[f * channels + c];
					x *= g;
					// TPDF: the sum of two uniform noises of one step each
					if (lsb > 0.f) { x += (uniform(random) + uniform(random)) * lsb; }
				}
			}
			if (sf_writef_float(file.get(), buffer.data(), got) != got) { failed = true; break; }
			if (total > 0) { progress((o + static_cast<double>(frame) / total) / outputs); }
		}
		if (failed || sf_close(file.release()) != 0)
		{
			QFile::remove(partial);
			result.error = tr("%1 could not be written.").arg(path);
			return result;
		}
		QFile::remove(path);
		if (!QFile::rename(partial, path))
		{
			QFile::remove(partial);
			result.error = tr("%1 could not be written.").arg(path);
			return result;
		}
		result.written << path;
	}
	progress(1.0);
	return result;
}

} // namespace lmms
