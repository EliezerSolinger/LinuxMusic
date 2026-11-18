/*
 * TransportBarTest.cpp
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

#include <QtTest>

#include "AudioDummy.h"
#include "ConfigManager.h"
#include "Engine.h"
#include "Song.h"
#include "TransportBar.h"

using namespace lmms;
using namespace lmms::gui;

/**
 * The transport's clocks and tempo (design 03): the position in each format, a format chosen kept, and the tempo
 * edited in place (the wheel, a drag, taps).
 */
class TransportBarTest : public QObject
{
	Q_OBJECT

private slots:
	void initTestCase()
	{
		ConfigManager::inst()->setValue("audioengine", "audiodev", AudioDummy::name());
		Engine::init(true);
	}

	void cleanupTestCase() { Engine::destroy(); }

	void theFormats()
	{
		using Format = TransportClock::Format;
		// 1 h 2 min 3.456 s
		const auto ms = 3723456;
		QCOMPARE(TransportClock::formatted(Format::MinutesSeconds, ms, 0), QString{"1:02:03.456"});
		QCOMPARE(TransportClock::formatted(Format::MinutesSeconds, 5214, 0), QString{"0:05.214"});
		QCOMPARE(TransportClock::formatted(Format::Samples, 0, 230000), QString{"230000"});
		// Frames of video at 30 per second: 3.456 s is frame 103 (3 s and 13 frames)
		QCOMPARE(TransportClock::formatted(Format::Timecode, ms, 0), QString{"01:02:03:13"});
		// The song at its start: bar 1, beat 1, tick 0
		Engine::getSong()->setPlayPos(0, Song::PlayMode::Song);
		QCOMPARE(TransportClock::formatted(Format::BarsBeats, 0, 0), QString{"001.1.000"});
	}

	void aFormatChosenIsKept()
	{
		{
			auto clock = TransportClock{false, "testformat", TransportClock::Format::BarsBeats, nullptr};
			clock.setFormat(TransportClock::Format::Timecode);
		}
		auto again = TransportClock{false, "testformat", TransportClock::Format::BarsBeats, nullptr};
		QCOMPARE(again.format(), TransportClock::Format::Timecode);
	}

	void theTempoInPlace()
	{
		auto& tempo = Engine::getSong()->tempoModel();
		tempo.setValue(120);
		auto widget = TransportTempo{nullptr};
		widget.resize(widget.sizeHint());

		// The wheel: one a notch, ten with Shift
		auto wheel = QWheelEvent{QPointF{5, 5}, QPointF{5, 5}, QPoint{}, QPoint{0, 120}, Qt::NoButton, Qt::NoModifier,
			Qt::NoScrollPhase, false};
		QCoreApplication::sendEvent(&widget, &wheel);
		QCOMPARE(tempo.value(), 121);
		auto shiftWheel = QWheelEvent{QPointF{5, 5}, QPointF{5, 5}, QPoint{}, QPoint{0, -120}, Qt::NoButton,
			Qt::ShiftModifier, Qt::NoScrollPhase, false};
		QCoreApplication::sendEvent(&widget, &shiftWheel);
		QCOMPARE(tempo.value(), 111);

		// Dragged up 30 pixels: ten faster
		QTest::mousePress(&widget, Qt::LeftButton, Qt::NoModifier, QPoint{10, 40});
		QTest::mouseMove(&widget, QPoint{10, 10});
		auto release = QMouseEvent{QEvent::MouseButtonRelease, QPointF{10, 10}, QPointF{10, 10}, Qt::LeftButton,
			Qt::NoButton, Qt::NoModifier};
		QCoreApplication::sendEvent(&widget, &release);
		QCOMPARE(tempo.value(), 121);

		// Tapped every half second: 120
		widget.startTapping();
		QVERIFY(widget.tapping());
		for (int i = 0; i < 4; ++i)
		{
			widget.tap();
			QTest::qWait(500);
		}
		QVERIFY2(std::abs(tempo.value() - 120) <= 2, qPrintable(QString::number(tempo.value())));
		// It stops by itself two seconds after the last tap
		QTRY_VERIFY_WITH_TIMEOUT(!widget.tapping(), 4000);
	}
};

QTEST_MAIN(TransportBarTest)
#include "TransportBarTest.moc"
