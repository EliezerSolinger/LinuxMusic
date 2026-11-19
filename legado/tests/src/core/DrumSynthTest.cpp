/*
 * DrumSynthTest.cpp
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

#include <QDirIterator>
#include <QObject>
#include <QTemporaryDir>
#include <QtTest>

#include <memory>
#include <vector>

#include "DrumSynth.h"

namespace {

using lmms::DrumSynth;

//! A kick like the TR-909 one: a tone with droop and bit reduction; %1 is the tone's envelope, %2 the rate
const char* const KickTemplate = R"([General]
Version=DrumSynth v2.0
Tuning=0.00
Stretch=100.0
Level=0
Filter=0
HighPass=0
FilterEnv=0,100 442000,100 443000,0

[Tone]
On=1
Level=181
F1=250
F2=80
Droop=28
Phase=0
Envelope=%1

[Noise]
On=0
Level=0
Slope=0
Envelope=0,100 1000,0
FixedSeq=0

[Overtones]
On=0
Level=0
F1=200
Wave1=0
Track1=0
F2=120
Wave2=0
Track2=0
Filter=0
Method=0
Param=50
Envelope1=0,100 1000,0
Envelope2=0,100 1000,0

[NoiseBand]
On=0
Level=0
F=1000
dF=50
Envelope=0,100 1000,0

[NoiseBand2]
On=0
Level=0
F=1000
dF=50
Envelope=0,100 1000,0

[Distortion]
On=1
Clipping=0
Bits=0
Rate=%2
)";

auto render(const QString& file) -> std::vector<int16_t>
{
	int16_t* data = nullptr;
	const auto frames = DrumSynth{}.GetDSFileSamples(file, data, 1, 44100);
	const auto owner = std::unique_ptr<int16_t[]>{data};
	if (frames <= 0 || !data) { return {}; }
	return {data, data + frames};
}

} // namespace

class DrumSynthTest : public QObject
{
	Q_OBJECT

	auto write(const QString& envelope, int rate) -> QString
	{
		const auto path = m_dir.filePath(QString{"kick-%1.ds"}.arg(m_count++));
		auto file = QFile{path};
		if (!file.open(QIODevice::WriteOnly)) { return {}; }
		file.write(QString{KickTemplate}.arg(envelope).arg(rate).toLatin1());
		return path;
	}

	QTemporaryDir m_dir;
	int m_count = 0;

private slots:
	//! A space at the end of an envelope adds no point: the TR-909 kick's tone was not a number, then silence
	void trailingSpaceAddsNoPoint()
	{
		// An earlier file leaves a point after its envelope's end, as the TR-909 kick found it
		render(write("0,100 5250,30 10500,0 20000,50", 0));

		const auto plain = render(write("0,100 5250,30 10500,0", 0));
		const auto spaced = render(write("0,100 5250,30 10500,0 ", 0));
		QVERIFY(!plain.empty());
		QCOMPARE(spaced, plain);

		auto loudest = 0;
		for (const auto sample : spaced) { loudest = std::max(loudest, std::abs(int{sample})); }
		QVERIFY2(loudest > 1000, "the kick's tone sounds");
	}

	//! Two spaces in a row, or a long number, are read as they are (the number went into 8 bytes)
	void unusualSpacingAndLongNumbers()
	{
		const auto plain = render(write("0,100 5250,30 10500,0", 0));
		QCOMPARE(render(write("0,100  5250,30   10500,0", 0)), plain);
		QCOMPARE(render(write("0.000000000,100.0000000 5250.0000000000,30 10500.000000000000,0", 0)), plain);
	}

	//! More points than there is room for keep the first ones
	void tooManyPoints()
	{
		auto envelope = QString{};
		for (int i = 0; i < 100; ++i) { envelope += QString{"%1,%2 "}.arg(i * 100).arg(i % 2 ? 0 : 100); }
		QVERIFY(!render(write(envelope, 0)).empty());
	}

	//! A downsampling rate out of the 0 to 6 the editor offers stays in the block (under ASan: no overflow)
	void ratesOutOfRange()
	{
		for (const int rate : {-5, -1, 7, 8, 11, 2000})
		{
			QVERIFY(!render(write("0,100 5250,30 10500,0", rate)).empty());
		}
	}

	//! A time too large for the length makes a drum of bounded length
	void hugeTimes()
	{
		const auto drum = render(write("0,100 1e30,0", 0));
		QVERIFY(!drum.empty());
		QVERIFY(drum.size() <= (1u << 24) + 2400);
	}

	//! Every kit the program comes with renders
	void bundledKits()
	{
		auto it = QDirIterator{QFINDTESTDATA("../../../data/samples/drumsynth"), {"*.ds"}, QDir::Files,
			QDirIterator::Subdirectories};
		int count = 0;
		while (it.hasNext())
		{
			const auto file = it.next();
			QVERIFY2(!render(file).empty(), qPrintable(file));
			++count;
		}
		QVERIFY(count > 100);
	}
};

QTEST_GUILESS_MAIN(DrumSynthTest)
#include "DrumSynthTest.moc"
