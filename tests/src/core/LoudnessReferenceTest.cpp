/*
 * LoudnessReferenceTest.cpp
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

#include <atomic>
#include <cmath>
#include <numbers>
#include <utility>
#include <vector>

#include <QDir>
#include <QDirIterator>
#include <QElapsedTimer>
#include <QTemporaryDir>
#include <QtTest>
#include <sndfile.h>

#include "ExportProcessor.h"

using namespace lmms;
using Meter = ExportProcessor::Meter;

//! A stretch of a stereo (or more channels) sine, @p hz, at @p dbfs (the amplitude of each channel), @p seconds long
struct Tone
{
	double dbfs;
	double seconds;
	double hz = 1000.0;
};
Q_DECLARE_METATYPE(std::vector<Tone>)

/**
 * Phase 10.1 of docs/plano-refatoracao.md: libebur128 against our own BS.1770 code, on the EBU's own test signals
 * (Tech 3341 for loudness and true peak, Tech 3342 for the loudness range), generated here as the EBU describes
 * them. Each measure is printed, for the plan to record.
 */
class LoudnessReferenceTest : public QObject
{
	Q_OBJECT

	QTemporaryDir m_dir;

	auto write(const QString& name, const std::vector<Tone>& tones, int rate = 48000, int channels = 2,
		double phase = 0.0) -> QString
	{
		const auto path = m_dir.filePath(name + ".wav");
		auto info = SF_INFO{};
		info.samplerate = rate;
		info.channels = channels;
		info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
		auto* file = sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
		if (!file) { return {}; }
		auto frame = std::int64_t{0};
		auto samples = std::vector<float>(static_cast<std::size_t>(channels));
		for (const auto& tone : tones)
		{
			const auto amplitude = tone.dbfs <= -200.0 ? 0.0 : std::pow(10.0, tone.dbfs / 20.0);
			const auto frames = static_cast<std::int64_t>(std::llround(tone.seconds * rate));
			for (std::int64_t f = 0; f < frames; ++f, ++frame)
			{
				const auto x = amplitude * std::sin(2.0 * std::numbers::pi * tone.hz * frame / rate + phase);
				std::fill(samples.begin(), samples.end(), static_cast<float>(x));
				sf_writef_float(file, samples.data(), 1);
			}
		}
		sf_close(file);
		return path;
	}

	static auto meterName(Meter meter) -> const char* { return meter == Meter::Own ? "ours" : "libebur128"; }

	static auto meters() -> std::vector<Meter>
	{
		auto all = std::vector<Meter>{Meter::Own};
		if (ExportProcessor::hasEbur128()) { all.push_back(Meter::Ebur128); }
		return all;
	}

private slots:
	void initTestCase() { QVERIFY(m_dir.isValid()); }

	//! EBU Tech 3341, cases 1 to 5: the integrated loudness within +-0.1 LU, by both
	void integratedLoudness_data()
	{
		QTest::addColumn<QString>("name");
		QTest::addColumn<std::vector<Tone>>("tones");
		QTest::addColumn<double>("expected");
		QTest::newRow("3341 case 1") << "c1" << std::vector<Tone>{{-23.0, 20.0}} << -23.0;
		QTest::newRow("3341 case 2") << "c2" << std::vector<Tone>{{-33.0, 20.0}} << -33.0;
		QTest::newRow("3341 case 3") << "c3" << std::vector<Tone>{{-36.0, 10.0}, {-23.0, 60.0}, {-36.0, 10.0}} << -23.0;
		QTest::newRow("3341 case 4") << "c4"
			<< std::vector<Tone>{{-72.0, 10.0}, {-36.0, 10.0}, {-23.0, 60.0}, {-36.0, 10.0}, {-72.0, 10.0}} << -23.0;
		QTest::newRow("3341 case 5") << "c5" << std::vector<Tone>{{-26.0, 20.0}, {-20.0, 20.1}, {-26.0, 20.0}} << -23.0;
	}

	void integratedLoudness()
	{
		QFETCH(QString, name);
		QFETCH(std::vector<Tone>, tones);
		QFETCH(double, expected);
		const auto path = write(name, tones);
		for (const auto meter : meters())
		{
			auto ok = false;
			const auto measured = ExportProcessor::measureWith(path, meter, &ok);
			QVERIFY(ok);
			qInfo("%s: %-10s %.3f LUFS (expected %.1f)", qPrintable(name), meterName(meter), measured.loudness, expected);
			QVERIFY2(std::abs(measured.loudness - expected) <= 0.1, meterName(meter));
		}
	}

	//! Other rates and channel counts we export, and silence: the same loudness by both
	void ratesChannelsAndSilence()
	{
		for (const auto& [rate, channels] : std::vector<std::pair<int, int>>{{44100, 2}, {96000, 2}, {48000, 1}, {48000, 6}})
		{
			const auto path = write(QString{"r%1c%2"}.arg(rate).arg(channels), {{-23.0, 20.0}}, rate, channels);
			auto values = std::vector<double>{};
			for (const auto meter : meters())
			{
				const auto measured = ExportProcessor::measureWith(path, meter);
				qInfo("%d Hz, %d channels: %-10s %.3f LUFS", rate, channels, meterName(meter), measured.loudness);
				values.push_back(measured.loudness);
			}
			if (values.size() < 2) { continue; }
			// Up to stereo the same; in 5.1 libebur128 weights the channels as BS.1770 does (L, R and C 1, the LFE
			// 0, the surrounds 1.41) and ours weights each 1: a difference of exactly that
			const auto expected = channels == 6 ? 10.0 * std::log10((3.0 + 2.0 * 1.41) / 6.0) : 0.0;
			QVERIFY2(std::abs(values[1] - values[0] - expected) <= 0.01, "the two differ");
		}
		const auto silence = write("silence", {{-300.0, 5.0}});
		for (const auto meter : meters()) { QVERIFY(ExportProcessor::measureWith(silence, meter).loudness < -1e8); }
	}

	//! EBU Tech 3342, cases 1 to 4: the loudness range within +-1 LU (libebur128 only: ours has none)
	void loudnessRange_data()
	{
		QTest::addColumn<QString>("name");
		QTest::addColumn<std::vector<Tone>>("tones");
		QTest::addColumn<double>("expected");
		QTest::newRow("3342 case 1") << "l1" << std::vector<Tone>{{-20.0, 20.0}, {-30.0, 20.0}} << 10.0;
		QTest::newRow("3342 case 2") << "l2" << std::vector<Tone>{{-20.0, 20.0}, {-15.0, 20.0}} << 5.0;
		QTest::newRow("3342 case 3") << "l3" << std::vector<Tone>{{-40.0, 20.0}, {-20.0, 20.0}} << 20.0;
		QTest::newRow("3342 case 4") << "l4"
			<< std::vector<Tone>{{-50.0, 20.0}, {-35.0, 20.0}, {-20.0, 20.0}, {-35.0, 20.0}, {-50.0, 20.0}} << 15.0;
	}

	void loudnessRange()
	{
		QFETCH(QString, name);
		QFETCH(std::vector<Tone>, tones);
		QFETCH(double, expected);
		if (!ExportProcessor::hasEbur128()) { QSKIP("built without libebur128"); }
		const auto measured = ExportProcessor::measureWith(write(name, tones), Meter::Ebur128);
		qInfo("%s: LRA %.2f LU (expected %.0f)", qPrintable(name), measured.loudnessRange, expected);
		QVERIFY(std::abs(measured.loudnessRange - expected) <= 1.0);
		QVERIFY(std::isnan(ExportProcessor::measureWith(write(name + "o", tones), Meter::Own).loudnessRange));
	}

	//! A sine at a quarter of the rate, 45 degrees off: its samples peak at -3.01 dBFS, the wave itself at 0 dBTP.
	//! The true peak within the EBU's +0.2/-0.4 dB; normalizing to a peak then keeps the export under the target
	//! between its samples too
	void truePeakAndNormalization()
	{
		if (!ExportProcessor::hasEbur128()) { QSKIP("built without libebur128"); }
		const auto path = write("tp", {{0.0, 5.0, 12000.0}}, 48000, 2, std::numbers::pi / 4);
		const auto measured = ExportProcessor::measureWith(path, Meter::Ebur128);
		const auto own = ExportProcessor::measureWith(path, Meter::Own);
		const auto truePeakDb = 20.0 * std::log10(measured.truePeak);
		qInfo("sample peak %.2f dBFS (ours %.2f), true peak %.2f dBTP", 20.0 * std::log10(measured.peak),
			20.0 * std::log10(own.peak), truePeakDb);
		QVERIFY(std::abs(20.0 * std::log10(measured.peak) + 3.01) < 0.05);
		QVERIFY(truePeakDb >= -0.4 && truePeakDb <= 0.2);

		// Normalized to -1 dB: by the sample peak (ours) the wave would reach about +2 dBTP
		auto settings = ExportProcessor::Settings{};
		settings.normalization = ExportProcessor::Normalization::Peak;
		settings.target = -1.0;
		settings.folder = m_dir.filePath("tp-out");
		settings.name = "tp";
		settings.outputs = {ExportProcessor::Output{}};
		const auto cancel = std::atomic<bool>{false};
		const auto result = ExportProcessor::process(path, settings, cancel, [](double) {});
		QVERIFY2(result.error.isEmpty(), qPrintable(result.error));
		const auto after = ExportProcessor::measureWith(result.written.front(), Meter::Ebur128);
		qInfo("normalized to -1 dB: true peak %.2f dBTP", 20.0 * std::log10(after.truePeak));
		QVERIFY(20.0 * std::log10(after.truePeak) <= -1.0 + 0.2);
	}

	//! The same result every time, and how long a long render takes to measure
	void sameEveryTimeAndHowLong()
	{
		const auto path = write("long", {{-23.0, 300.0, 997.0}, {-30.0, 300.0, 440.0}}); // ten minutes
		for (const auto meter : meters())
		{
			QElapsedTimer timer;
			timer.start();
			const auto first = ExportProcessor::measureWith(path, meter);
			const auto took = timer.elapsed();
			const auto again = ExportProcessor::measureWith(path, meter);
			qInfo("10 min stereo 48 kHz: %-10s %.3f LUFS in %lld ms", meterName(meter), first.loudness,
				static_cast<long long>(took));
			QCOMPARE(again.loudness, first.loudness);
			QCOMPARE(again.peak, first.peak);
		}
	}

	//! LINUXMUSIC_LOUDNESS_RENDERS: a folder of renders (the A/B battery's), each measured by both, the difference
	//! printed (for the plan; not judged)
	void renders()
	{
		const auto folder = qEnvironmentVariable("LINUXMUSIC_LOUDNESS_RENDERS");
		if (folder.isEmpty() || !ExportProcessor::hasEbur128()) { QSKIP("LINUXMUSIC_LOUDNESS_RENDERS not given"); }
		auto largest = 0.0;
		auto count = 0;
		for (auto it = QDirIterator{folder, {"*.wav"}, QDir::Files, QDirIterator::Subdirectories}; it.hasNext();)
		{
			const auto path = it.next();
			const auto own = ExportProcessor::measureWith(path, Meter::Own);
			const auto lib = ExportProcessor::measureWith(path, Meter::Ebur128);
			if (own.loudness < -1e8 || lib.loudness < -1e8) { continue; }
			largest = std::max(largest, std::abs(own.loudness - lib.loudness));
			++count;
		}
		qInfo("%d renders: the largest difference between the two is %.4f LU", count, largest);
	}
};

QTEST_GUILESS_MAIN(LoudnessReferenceTest)
#include "LoudnessReferenceTest.moc"
