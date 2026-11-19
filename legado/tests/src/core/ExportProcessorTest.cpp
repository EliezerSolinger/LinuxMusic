/*
 * ExportProcessorTest.cpp
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

#include <cmath>
#include <numbers>
#include <vector>

#include <QFileInfo>
#include <QObject>
#include <QTemporaryDir>
#include <QtTest>
#include <sndfile.h>

#include "ExportProcessor.h"

using namespace lmms;

namespace
{

//! A float WAV: a sine of @p hz at @p amplitude on @p channels channels, @p seconds long
void writeSine(const QString& path, int channels, double hz, double amplitude, double seconds, int rate = 48000)
{
	auto info = SF_INFO{};
	info.samplerate = rate;
	info.channels = channels;
	info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
	auto* file = sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
	QVERIFY(file);
	const auto frames = static_cast<int>(seconds * rate);
	auto samples = std::vector<float>(static_cast<std::size_t>(frames * channels));
	for (int f = 0; f < frames; ++f)
	{
		const auto x = static_cast<float>(amplitude * std::sin(2 * std::numbers::pi * hz * f / rate));
		for (int c = 0; c < channels; ++c) { samples[f * channels + c] = x; }
	}
	sf_writef_float(file, samples.data(), frames);
	sf_close(file);
}

auto readAll(const QString& path, SF_INFO& info) -> std::vector<float>
{
	info = SF_INFO{};
	auto* file = sf_open(QFile::encodeName(path).constData(), SFM_READ, &info);
	if (!file) { return {}; }
	auto samples = std::vector<float>(static_cast<std::size_t>(info.frames * info.channels));
	sf_readf_float(file, samples.data(), info.frames);
	sf_close(file);
	return samples;
}

const auto NoProgress = [](double) {};

} // namespace

class ExportProcessorTest : public QObject
{
	Q_OBJECT

private:
	QTemporaryDir m_directory;

private slots:
	void loudnessFollowsBs1770()
	{
		// The standard's reference: a 997 Hz sine at full scale in one channel is -3.01 LUFS
		const auto mono = m_directory.filePath("reference.wav");
		writeSine(mono, 1, 997.0, 1.0, 10.0);
		auto ok = false;
		const auto reference = ExportProcessor::measure(mono, &ok);
		QVERIFY(ok);
		QVERIFY2(std::abs(reference.loudness + 3.01) < 0.05, qPrintable(QString::number(reference.loudness)));
		QVERIFY(std::abs(reference.peak - 1.0) < 1e-3);

		// Both channels at -20 dBFS: about -20 LUFS; at another rate, the same
		const auto stereo = m_directory.filePath("stereo.wav");
		writeSine(stereo, 2, 997.0, 0.1, 10.0, 44100);
		const auto measured = ExportProcessor::measure(stereo);
		QVERIFY2(std::abs(measured.loudness + 20.0) < 0.1, qPrintable(QString::number(measured.loudness)));

		// Silence has none
		const auto silence = m_directory.filePath("silence.wav");
		writeSine(silence, 2, 997.0, 0.0, 2.0);
		QVERIFY(ExportProcessor::measure(silence).loudness < -1e8);
	}

	void normalizesToALoudnessOrAPeak()
	{
		const auto source = m_directory.filePath("song.wav");
		writeSine(source, 2, 440.0, 0.05, 6.0);
		auto cancel = std::atomic<bool>{false};

		auto settings = ExportProcessor::Settings{};
		settings.folder = m_directory.filePath("out");
		settings.name = "loud";
		settings.normalization = ExportProcessor::Normalization::Loudness;
		settings.target = -14.0;
		settings.outputs = {{ExportProcessor::Format::Wav, 32}};
		auto result = ExportProcessor::process(source, settings, cancel, NoProgress);
		QVERIFY2(result.error.isEmpty(), qPrintable(result.error));
		QCOMPARE(result.written.size(), qsizetype{1});
		const auto loudness = ExportProcessor::measure(result.written.front()).loudness;
		QVERIFY2(std::abs(loudness + 14.0) < 0.05, qPrintable(QString::number(loudness)));

		settings.name = "peak";
		settings.normalization = ExportProcessor::Normalization::Peak;
		settings.target = -1.0;
		result = ExportProcessor::process(source, settings, cancel, NoProgress);
		const auto peak = ExportProcessor::measure(result.written.front()).peak;
		QVERIFY(std::abs(20.0 * std::log10(peak) + 1.0) < 0.01);
	}

	void writesEveryFormatAtOnceWithFades()
	{
		const auto source = m_directory.filePath("formats.wav");
		writeSine(source, 2, 440.0, 0.5, 3.0);
		auto cancel = std::atomic<bool>{false};
		auto settings = ExportProcessor::Settings{};
		settings.folder = m_directory.filePath("formats");
		settings.name = "mix";
		settings.fadeIn = 0.5;
		settings.fadeOut = 0.5;
		settings.outputs = {{ExportProcessor::Format::Wav, 16}, {ExportProcessor::Format::Flac, 24},
			{ExportProcessor::Format::Ogg, 24, 320, 0.5}};
		if (ExportProcessor::canWrite(ExportProcessor::Format::Mp3))
		{
			settings.outputs.push_back({ExportProcessor::Format::Mp3, 16, 192});
		}
		auto last = 0.0;
		const auto result = ExportProcessor::process(source, settings, cancel, [&](double p) { QVERIFY(p >= last); last = p; });
		QVERIFY2(result.error.isEmpty(), qPrintable(result.error));
		QCOMPARE(result.written.size(), static_cast<qsizetype>(settings.outputs.size()));
		QCOMPARE(last, 1.0);
		for (const auto& path : result.written)
		{
			auto info = SF_INFO{};
			const auto samples = readAll(path, info);
			QVERIFY2(!samples.empty(), qPrintable(path));
			QCOMPARE(info.samplerate, 48000);
			QCOMPARE(info.channels, 2);
			QVERIFY2(std::abs(info.frames - 3 * 48000) < 4096, qPrintable(path)); // (the lossy ones pad a little)
			QVERIFY(!QFileInfo::exists(path + ".part"));
		}
		// The WAV's fades: silent at both ends, full in the middle
		auto info = SF_INFO{};
		const auto wav = readAll(settings.folder + "/mix.wav", info);
		QVERIFY(std::abs(wav[0]) < 1e-3);
		QVERIFY(std::abs(wav[wav.size() - 1]) < 1e-3);
		auto middle = 0.f;
		for (auto f = info.frames / 2 - 200; f < info.frames / 2 + 200; ++f) { middle = std::max(middle, std::abs(wav[f * 2])); }
		QVERIFY(std::abs(middle - 0.5f) < 0.01f);
	}

	void aCancelledExportLeavesNoHalfFile()
	{
		const auto source = m_directory.filePath("cancel.wav");
		writeSine(source, 2, 440.0, 0.5, 2.0);
		auto cancel = std::atomic<bool>{true};
		auto settings = ExportProcessor::Settings{};
		settings.folder = m_directory.filePath("cancelled");
		settings.name = "mix";
		settings.outputs = {{ExportProcessor::Format::Wav, 24}};
		const auto result = ExportProcessor::process(source, settings, cancel, NoProgress);
		QVERIFY(result.cancelled);
		QVERIFY(!QFileInfo::exists(settings.folder + "/mix.wav"));
		QVERIFY(!QFileInfo::exists(settings.folder + "/mix.wav.part"));
	}
};

QTEST_GUILESS_MAIN(ExportProcessorTest)
#include "ExportProcessorTest.moc"
