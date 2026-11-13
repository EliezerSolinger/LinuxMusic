/*
 * AmpModelTest.cpp
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

#include <QObject>
#include <QtTest>

#include "AmpModel.h"

using namespace lmms;

namespace
{

const auto Models = QString{AMP_MODEL_EXAMPLES};

//! Runs a 220 Hz sine at @p rate through @p model in periods of @p period; @returns the output
auto play(AmpModel& model, double rate, int period, int frames) -> std::vector<float>
{
	auto out = std::vector<float>(frames);
	for (int f = 0; f < frames; ++f) { out[f] = 0.3f * static_cast<float>(std::sin(2 * std::numbers::pi * 220.0 * f / rate)); }
	for (int at = 0; at < frames; at += period) { model.process(out.data() + at, std::min(period, frames - at)); }
	return out;
}

} // namespace

class AmpModelTest : public QObject
{
	Q_OBJECT

private slots:
	void runsAtItsRateWithoutLatency()
	{
		auto error = QString{};
		auto model = AmpModel::load(Models + "/wavenet.nam", 48000.0, error);
		QVERIFY2(model, qPrintable(error));
		QCOMPARE(model->modelRate(), 48000.0);
		QCOMPARE(model->latency(), 0);
		const auto out = play(*model, 48000.0, 256, 48000);
		auto energy = 0.0;
		for (const auto x : out) { QVERIFY(std::isfinite(x)); energy += x * x; }
		QVERIFY(energy > 0.0);
	}

	void resamplesAtAnotherRate_data()
	{
		QTest::addColumn<QString>("file");
		QTest::addColumn<double>("rate");
		QTest::newRow("WaveNet at 44.1 kHz") << "wavenet.nam" << 44100.0;
		QTest::newRow("LSTM at 96 kHz") << "lstm.nam" << 96000.0;
	}

	void resamplesAtAnotherRate()
	{
		QFETCH(QString, file);
		QFETCH(double, rate);
		auto error = QString{};
		auto model = AmpModel::load(Models + "/" + file, rate, error);
		QVERIFY2(model, qPrintable(error));
		QCOMPARE(model->modelRate(), 48000.0);
		QVERIFY(model->latency() > 0);
		// Odd periods: the converters' jitter never starves the output past its latency
		const auto frames = static_cast<int>(rate);
		const auto out = play(*model, rate, 97, frames);
		for (const auto x : out) { QVERIFY(std::isfinite(x)); }
		auto silentRun = 0;
		auto longest = 0;
		for (int f = model->latency() + 64; f < frames; ++f)
		{
			silentRun = out[f] == 0.f ? silentRun + 1 : 0;
			longest = std::max(longest, silentRun);
		}
		QVERIFY2(longest < 8, qPrintable(QString::number(longest)));
	}

	void aBadFileIsAnError()
	{
		auto error = QString{};
		QVERIFY(!AmpModel::load(Models + "/nothing.nam", 48000.0, error));
		QVERIFY(!error.isEmpty());
	}
};

QTEST_GUILESS_MAIN(AmpModelTest)
#include "AmpModelTest.moc"
