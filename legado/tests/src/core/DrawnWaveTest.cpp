/*
 * DrawnWaveTest.cpp
 *
 * This file is part of LinuxMusic, based on LMMS - https://lmms.io
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
#include <vector>

#include <QtTest>

#include "DrawnWave.h"
#include "../helpers/Spectrum.h"

using namespace lmms;

class DrawnWaveTest : public QObject
{
	Q_OBJECT

	//! A saw drawn in @p points points, from -1 up to almost 1
	static auto saw(std::size_t points) -> std::vector<float>
	{
		auto wave = std::vector<float>(points);
		for (std::size_t i = 0; i < points; ++i) { wave[i] = -1.f + 2.f * static_cast<float>(i) / points; }
		return wave;
	}

	//! @p wave played at @p frequency for @p frames frames at @p sampleRate, as an instrument plays it (a phase that
	//! goes round, and the table of the note's frequency)
	static auto play(const DrawnWave& wave, double frequency, double sampleRate, std::size_t frames)
		-> std::vector<float>
	{
		auto out = std::vector<float>(frames);
		const auto* table = wave.tableFor(static_cast<float>(frequency), static_cast<float>(sampleRate));
		if (!table) { return out; }
		auto phase = 0.0;
		for (auto& sample : out)
		{
			sample = DrawnWave::at(*table, static_cast<float>(phase));
			phase += frequency / sampleRate;
			phase -= std::floor(phase);
		}
		return out;
	}

	//! The same wave played as the instruments did: the points straight (held, or joined by lines), not band limited
	static auto playPoints(const std::vector<float>& points, DrawnWave::Hold hold, double frequency, double sampleRate,
		std::size_t frames) -> std::vector<float>
	{
		auto out = std::vector<float>(frames);
		auto phase = 0.0;
		for (auto& sample : out)
		{
			const auto position = phase * points.size();
			const auto index = static_cast<std::size_t>(position) % points.size();
			sample = hold == DrawnWave::Hold::Step ? points[index]
				: std::lerp(points[index], points[(index + 1) % points.size()], static_cast<float>(position - std::floor(position)));
			phase += frequency / sampleRate;
			phase -= std::floor(phase);
		}
		return out;
	}

private slots:
	//! #6098: drawn waves (BitInvader's, Watsyn's) fold their high harmonics back down; band limited, nothing does
	void highNotesHaveNothingBetweenTheirHarmonics()
	{
		for (const auto hold : {DrawnWave::Hold::Step, DrawnWave::Hold::Linear})
		{
			const auto points = saw(200);
			const auto wave = DrawnWave{points, hold};
			for (const auto frequency : {1046.5, 2093.0, 4186.0, 7040.0})
			{
				for (const auto sampleRate : {44100.0, 48000.0, 96000.0})
				{
					const auto name = QString{"%1 Hz at %2, %3"}.arg(frequency).arg(sampleRate)
						.arg(hold == DrawnWave::Hold::Step ? "held" : "joined");
					const auto before = tests::aliasing(
						playPoints(points, hold, frequency, sampleRate, 1 << 16), 1000, frequency, sampleRate);
					const auto after = tests::aliasing(
						play(wave, frequency, sampleRate, 1 << 16), 1000, frequency, sampleRate);
					QVERIFY2(after < -80, qPrintable(QString{"%1: %2 dB folded (%3 dB before)"}.arg(name).arg(after).arg(before)));
					// (it was there to remove)
					if (sampleRate == 44100.0 && frequency >= 4186.0) { QVERIFY2(before > -40, qPrintable(name)); }
				}
			}
		}
	}

	//! Low notes keep the wave as drawn: every harmonic below the cutoff is there, as it was
	void lowNotesSoundAsDrawn()
	{
		for (const auto hold : {DrawnWave::Hold::Step, DrawnWave::Hold::Linear})
		{
			auto points = std::vector<float>(64);
			for (std::size_t i = 0; i < points.size(); ++i) { points[i] = std::sin(i * 0.37f) * 0.6f + (i % 7) * 0.05f; }
			const auto wave = DrawnWave{points, hold};
			const auto* table = wave.tableFor(55.f, 44100.f);
			QVERIFY(table);
			// (20 kHz / 55 Hz: 363 harmonics; the table under it, two semitones down at most)
			QVERIFY(table->harmonics <= 363 && table->harmonics >= 300);
			// At the steps or the corners the band limited wave rings a little; halfway along a step, or at a point
			// joined by lines, it is the drawing
			for (std::size_t i = 0; i < points.size(); ++i)
			{
				const auto phase = hold == DrawnWave::Hold::Step ? (i + 0.5f) / points.size() : static_cast<float>(i) / points.size();
				const auto drawn = points[i];
				QVERIFY2(std::abs(DrawnWave::at(*table, phase) - drawn) < 0.03f,
					qPrintable(QString{"point %1: %2 drawn, %3 played"}.arg(i).arg(drawn).arg(DrawnWave::at(*table, phase))));
			}
		}
	}

	//! A note picks the table with the most harmonics under the cutoff; above it, none (silence, not a folded tone)
	void notesTakeTheirTable()
	{
		const auto wave = DrawnWave{saw(256), DrawnWave::Hold::Step};
		for (const auto frequency : {20.f, 110.f, 440.f, 3000.f, 9000.f, 15000.f})
		{
			const auto* table = wave.tableFor(frequency, 44100.f);
			QVERIFY(table);
			const auto allowed = std::floor(DrawnWave::cutoff(44100.f) / frequency);
			QVERIFY(table->harmonics <= allowed);
			QVERIFY(table->harmonics >= std::min<double>(allowed * 0.89 - 1, DrawnWave::MostHarmonics * 0.89));
		}
		// (20 kHz at any rate; a little under half a low one)
		QVERIFY(!wave.tableFor(21000.f, 96000.f));
		QVERIFY(wave.tableFor(19000.f, 96000.f));
		QVERIFY(!wave.tableFor(11000.f, 22050.f));
		// Fewer harmonics, from the most down to one
		const auto& tables = wave.tables();
		QCOMPARE(tables.back().harmonics, std::size_t{1});
		for (std::size_t i = 1; i < tables.size(); ++i) { QVERIFY(tables[i].harmonics < tables[i - 1].harmonics); }
	}

	//! Nothing drawn, a flat line, a single point: no table to divide by zero or to read past
	void oddDrawings()
	{
		const auto none = DrawnWave{std::vector<float>{}, DrawnWave::Hold::Step};
		QVERIFY(!none.tableFor(440.f, 44100.f));
		const auto flat = DrawnWave{std::vector<float>(10, 0.25f), DrawnWave::Hold::Linear};
		const auto* table = flat.tableFor(440.f, 44100.f);
		QVERIFY(table);
		QVERIFY(std::abs(DrawnWave::at(*table, 0.3f) - 0.25f) < 1e-5f);
		const auto single = DrawnWave{std::vector<float>{0.5f}, DrawnWave::Hold::Step};
		QVERIFY(single.tableFor(440.f, 44100.f));
		QVERIFY(std::isfinite(DrawnWave::at(*single.tableFor(440.f, 44100.f), 0.999f)));
	}
};

QTEST_GUILESS_MAIN(DrawnWaveTest)
#include "DrawnWaveTest.moc"
