/*
 * TimeStretcher.cpp - renders a recording to a new timing (a change of tempo), keeping its pitch
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

#include "TimeStretcher.h"

#include <algorithm>
#include <cmath>
#include <map>
#include <memory>
#include <vector>

#include <QCoreApplication>
#include <QFile>
#include <samplerate.h>
#include <sndfile.h>

#include "lmmsconfig.h"

#ifdef LMMS_HAVE_RUBBERBAND
#include <rubberband/RubberBandStretcher.h>
#endif

namespace lmms
{

namespace
{

constexpr auto BlockFrames = std::size_t{4096};

auto tr(const char* text) -> QString
{
	return QCoreApplication::translate("TimeStretcher", text);
}

struct SndFileCloser
{
	void operator()(SNDFILE* file) const { sf_close(file); }
};
using SndFile = std::unique_ptr<SNDFILE, SndFileCloser>;

//! Writes the rendering: exactly @c total frames, the excess dropped and what is missing padded with silence
class Writer
{
public:
	Writer(SndFile file, int channels, sf_count_t total) :
		m_file{std::move(file)}, m_channels{channels}, m_total{total}
	{}

	//! Writes @p frames interleaved frames; @returns false when writing failed
	auto write(const float* interleaved, sf_count_t frames) -> bool
	{
		frames = std::min(frames, m_total - m_written);
		if (frames <= 0) { return true; }
		if (sf_writef_float(m_file.get(), interleaved, frames) != frames) { return false; }
		m_written += frames;
		return true;
	}

	auto written() const -> sf_count_t { return m_written; }

	//! Pads the rest with silence and closes the file; @returns false when that failed
	auto finish() -> bool
	{
		const auto silence = std::vector<float>(BlockFrames * m_channels, 0.f);
		while (m_written < m_total)
		{
			if (!write(silence.data(), std::min<sf_count_t>(BlockFrames, m_total - m_written))) { return false; }
		}
		return sf_close(m_file.release()) == 0;
	}

private:
	SndFile m_file;
	int m_channels;
	sf_count_t m_total;
	sf_count_t m_written = 0;
};

//! The point frames rounded and made strictly increasing (a point that does not move on is dropped from both)
void keyFrames(const TimeStretcher::Timing& from, const TimeStretcher::Timing& to, sf_count_t sourceFrames,
	std::vector<sf_count_t>& in, std::vector<sf_count_t>& out)
{
	in = {0};
	out = {0};
	for (std::size_t i = 1; i < from.size(); ++i)
	{
		const auto a = std::min(static_cast<sf_count_t>(std::llround(from[i].frame)), sourceFrames);
		const auto b = static_cast<sf_count_t>(std::llround(to[i].frame));
		if (a <= in.back() || b <= out.back()) { continue; }
		in.push_back(a);
		out.push_back(b);
	}
}

#ifdef LMMS_HAVE_RUBBERBAND

auto stretch(SNDFILE* source, const SF_INFO& info, Writer& writer, const std::vector<sf_count_t>& in,
	const std::vector<sf_count_t>& out, sample_rate_t rate, const std::atomic<bool>& cancel,
	const std::function<void(double)>& progress) -> TimeStretcher::Result
{
	using RubberBand::RubberBandStretcher;
	const auto channels = static_cast<std::size_t>(info.channels);
	const auto inFrames = in.back();
	const auto outFrames = out.back();

	// The source plays at its own rate, the rendering at the song's: the pitch is scaled back by their ratio
	auto stretcher = RubberBandStretcher{static_cast<std::size_t>(info.samplerate), channels,
		RubberBandStretcher::OptionProcessOffline | RubberBandStretcher::OptionEngineFiner,
		static_cast<double>(outFrames) / static_cast<double>(inFrames),
		static_cast<double>(info.samplerate) / static_cast<double>(rate)};
	stretcher.setMaxProcessSize(BlockFrames);

	auto interleaved = std::vector<float>(BlockFrames * channels);
	auto planar = std::vector<std::vector<float>>(channels, std::vector<float>(BlockFrames));
	auto pointers = std::vector<float*>(channels);
	for (std::size_t c = 0; c < channels; ++c) { pointers[c] = planar[c].data(); }

	// Reads the next block of the source's first inFrames frames into planar; @returns the frames read
	auto position = sf_count_t{0};
	const auto read = [&]() -> std::size_t {
		const auto wanted = std::min<sf_count_t>(BlockFrames, inFrames - position);
		const auto got = wanted > 0 ? sf_readf_float(source, interleaved.data(), wanted) : 0;
		// A file shorter than it said: the rest is silence
		std::fill(interleaved.begin() + std::max<sf_count_t>(got, 0) * channels,
			interleaved.begin() + std::max<sf_count_t>(wanted, 0) * channels, 0.f);
		for (sf_count_t f = 0; f < wanted; ++f)
		{
			for (std::size_t c = 0; c < channels; ++c) { planar[c][f] = interleaved[f * channels + c]; }
		}
		position += std::max<sf_count_t>(wanted, 0);
		return static_cast<std::size_t>(std::max<sf_count_t>(wanted, 0));
	};

	// First pass: study
	while (position < inFrames)
	{
		if (cancel.load(std::memory_order_relaxed)) { return {TimeStretcher::Status::Cancelled, {}}; }
		const auto frames = read();
		stretcher.study(pointers.data(), frames, position >= inFrames);
		progress(0.25 * static_cast<double>(position) / static_cast<double>(inFrames));
	}

	// Every point is a key frame: the tick it was played at falls on the frame the rendering plays it at
	auto map = std::map<std::size_t, std::size_t>{};
	for (std::size_t i = 1; i + 1 < in.size(); ++i)
	{
		map[static_cast<std::size_t>(in[i])] = static_cast<std::size_t>(out[i]);
	}
	stretcher.setKeyFrameMap(map);

	// Second pass: process and write what comes out
	if (sf_seek(source, 0, SEEK_SET) != 0) { return {TimeStretcher::Status::Failed, tr("The recording could not be read again.")}; }
	position = 0;
	auto output = std::vector<std::vector<float>>(channels, std::vector<float>(BlockFrames));
	auto outputPointers = std::vector<float*>(channels);
	for (std::size_t c = 0; c < channels; ++c) { outputPointers[c] = output[c].data(); }
	auto outInterleaved = std::vector<float>(BlockFrames * channels);
	const auto drain = [&]() -> bool {
		for (auto available = stretcher.available(); available > 0; available = stretcher.available())
		{
			const auto frames = stretcher.retrieve(outputPointers.data(),
				std::min(static_cast<std::size_t>(available), BlockFrames));
			for (std::size_t f = 0; f < frames; ++f)
			{
				for (std::size_t c = 0; c < channels; ++c) { outInterleaved[f * channels + c] = output[c][f]; }
			}
			if (!writer.write(outInterleaved.data(), static_cast<sf_count_t>(frames))) { return false; }
		}
		return true;
	};
	const auto writeFailed = TimeStretcher::Result{TimeStretcher::Status::Failed, tr("The rendering could not be written.")};
	while (position < inFrames)
	{
		if (cancel.load(std::memory_order_relaxed)) { return {TimeStretcher::Status::Cancelled, {}}; }
		const auto frames = read();
		stretcher.process(pointers.data(), frames, position >= inFrames);
		if (!drain()) { return writeFailed; }
		progress(0.25 + 0.75 * static_cast<double>(position) / static_cast<double>(inFrames));
	}
	while (stretcher.available() >= 0)
	{
		if (cancel.load(std::memory_order_relaxed)) { return {TimeStretcher::Status::Cancelled, {}}; }
		if (!drain()) { return writeFailed; }
		if (stretcher.available() == 0)
		{
			// Offline, all input given: what is left comes out as it is processed. Nothing more coming: done
			stretcher.process(pointers.data(), 0, true);
			if (stretcher.available() == 0) { break; }
		}
	}
	return {TimeStretcher::Status::Done, {}};
}

#else

//! Without Rubber Band: each piece between two points is resampled to its new length (the pitch changes)
auto stretch(SNDFILE* source, const SF_INFO& info, Writer& writer, const std::vector<sf_count_t>& in,
	const std::vector<sf_count_t>& out, sample_rate_t, const std::atomic<bool>& cancel,
	const std::function<void(double)>& progress) -> TimeStretcher::Result
{
	const auto channels = info.channels;
	auto error = 0;
	auto state = std::unique_ptr<SRC_STATE, decltype(&src_delete)>{src_new(SRC_SINC_BEST_QUALITY, channels, &error), &src_delete};
	if (!state) { return {TimeStretcher::Status::Failed, QString::fromUtf8(src_strerror(error))}; }

	auto input = std::vector<float>(BlockFrames * channels);
	auto output = std::vector<float>();
	auto position = sf_count_t{0};
	for (std::size_t piece = 0; piece + 1 < in.size(); ++piece)
	{
		const auto ratio = static_cast<double>(out[piece + 1] - out[piece]) / static_cast<double>(in[piece + 1] - in[piece]);
		output.resize((static_cast<std::size_t>(std::ceil(BlockFrames * ratio)) + 64) * channels);
		const auto last = piece + 2 == in.size();
		while (position < in[piece + 1])
		{
			if (cancel.load(std::memory_order_relaxed)) { return {TimeStretcher::Status::Cancelled, {}}; }
			const auto wanted = std::min<sf_count_t>(BlockFrames, in[piece + 1] - position);
			const auto got = std::max<sf_count_t>(sf_readf_float(source, input.data(), wanted), 0);
			std::fill(input.begin() + got * channels, input.begin() + wanted * channels, 0.f);
			position += wanted;

			auto data = SRC_DATA{};
			data.data_in = input.data();
			data.input_frames = static_cast<long>(wanted);
			data.src_ratio = ratio;
			data.end_of_input = last && position >= in[piece + 1];
			do
			{
				data.data_out = output.data();
				data.output_frames = static_cast<long>(output.size() / channels);
				if (const auto failure = src_process(state.get(), &data))
				{
					return {TimeStretcher::Status::Failed, QString::fromUtf8(src_strerror(failure))};
				}
				if (!writer.write(output.data(), data.output_frames_gen))
				{
					return {TimeStretcher::Status::Failed, tr("The rendering could not be written.")};
				}
				data.data_in += data.input_frames_used * channels;
				data.input_frames -= data.input_frames_used;
			}
			while (data.input_frames > 0 || (data.end_of_input && data.output_frames_gen > 0));
			progress(static_cast<double>(position) / static_cast<double>(in.back()));
		}
	}
	return {TimeStretcher::Status::Done, {}};
}

#endif

} // namespace


auto TimeStretcher::canKeepPitch() -> bool
{
#ifdef LMMS_HAVE_RUBBERBAND
	return true;
#else
	return false;
#endif
}


auto TimeStretcher::render(const QString& source, const QString& target, const Timing& from, const Timing& to,
	sample_rate_t rate, const std::atomic<bool>& cancel, const std::function<void(double)>& progress) -> Result
{
	if (from.size() < 2 || from.size() != to.size() || rate == 0)
	{
		return {Status::Failed, tr("The recording's timing is not valid.")};
	}
	for (std::size_t i = 0; i < from.size(); ++i)
	{
		if (from[i].tick != to[i].tick) { return {Status::Failed, tr("The recording's timing is not valid.")}; }
	}

	auto info = SF_INFO{};
	auto input = SndFile{sf_open(QFile::encodeName(source).constData(), SFM_READ, &info)};
	if (!input || info.channels <= 0 || info.samplerate <= 0)
	{
		return {Status::Failed, tr("The recording %1 could not be opened.").arg(source)};
	}

	auto in = std::vector<sf_count_t>{};
	auto out = std::vector<sf_count_t>{};
	keyFrames(from, to, info.frames, in, out);
	if (in.size() < 2) { return {Status::Failed, tr("The recording's timing is not valid.")}; }

	const auto partial = target + ".part";
	auto outInfo = SF_INFO{};
	outInfo.samplerate = static_cast<int>(rate);
	outInfo.channels = info.channels;
	outInfo.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
	auto output = SndFile{sf_open(QFile::encodeName(partial).constData(), SFM_WRITE, &outInfo)};
	if (!output) { return {Status::Failed, tr("The rendering %1 could not be created.").arg(target)}; }

	auto writer = Writer{std::move(output), info.channels, out.back()};
	auto result = stretch(input.get(), info, writer, in, out, rate, cancel, progress);
	if (result.status == Status::Done && !writer.finish())
	{
		result = {Status::Failed, tr("The rendering could not be written.")};
	}
	if (result.status != Status::Done)
	{
		writer = Writer{nullptr, info.channels, 0}; // closes the file
		QFile::remove(partial);
		return result;
	}

	QFile::remove(target);
	if (!QFile::rename(partial, target))
	{
		QFile::remove(partial);
		return {Status::Failed, tr("The rendering %1 could not be created.").arg(target)};
	}
	progress(1.0);
	return result;
}

} // namespace lmms
