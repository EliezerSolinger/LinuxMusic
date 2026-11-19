/*
 * AudioInputTest.cpp - the audio device's input reaches the engine in step with the output
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

#include <QObject>
#include <QtTest>

#include <vector>

#include "AudioCoreAudio.h"
#include "AudioDevice.h"
#include "AudioEngine.h"
#include "AudioDummy.h"
#include "ConfigManager.h"
#include "Engine.h"

namespace
{

using namespace lmms;

//! A device the test drives by hand: it captures a known signal on a few input channels
class FakeDuplexDevice : public AudioDevice
{
public:
	FakeDuplexDevice(ch_cnt_t inputs, AudioEngine* engine)
		: AudioDevice{2, engine}
	{
		setInputChannels(inputs);
	}

	auto inputLatency() const -> f_cnt_t override { return 32; }
	auto outputLatency() const -> f_cnt_t override { return 64; }

private:
	void startProcessingImpl() override {}
	void stopProcessingImpl() override {}
};

//! A device that opens its input on demand, counting how often it is asked to
class OnDemandDevice : public AudioDevice
{
public:
	explicit OnDemandDevice(AudioEngine* engine)
		: AudioDevice{2, engine}
	{
		setInputChannels(2);
	}

	auto capturesOnDemand() const -> bool override { return true; }
	void setCaptureEnabled(bool enabled) override
	{
		capturing = enabled;
		++changes;
	}

	bool capturing = false;
	int changes = 0;

private:
	void startProcessingImpl() override {}
	void stopProcessingImpl() override {}
};

//! The input sample of channel @p channel at absolute frame @p frame: unique, and exact in a float
auto inputSample(ch_cnt_t channel, std::int64_t frame) -> float
{
	return static_cast<float>(frame % 100000) + 0.25f * static_cast<float>(channel);
}

} // namespace

class AudioInputTest : public QObject
{
	Q_OBJECT

private:
	/**
	 * Drives the engine like a duplex device would, with callbacks of @p callbackFrames frames, and checks after each
	 * callback that capture() holds the input of the frames handed over with the output of the period before the
	 * last one rendered.
	 */
	void driveAndCheck(f_cnt_t callbackFrames, ch_cnt_t deviceInputs, ch_cnt_t engineInputs)
	{
		auto* engine = Engine::audioEngine();
		engine->setAudioDevice(std::unique_ptr<AudioDevice>{new FakeDuplexDevice{engineInputs, engine}}, false);
		QCOMPARE(engine->audioDev()->inputChannels(), engineInputs);
		QCOMPARE(engine->audioDev()->inputLatency(), f_cnt_t{32});

		const auto period = engine->framesPerPeriod();
		const auto startPeriods = engine->periodsRendered();
		std::int64_t framesHanded = 0;

		auto inputs = std::vector<std::vector<float>>(deviceInputs, std::vector<float>(callbackFrames));
		auto inputPointers = std::vector<const float*>{};
		for (const auto& channel : inputs) { inputPointers.push_back(channel.data()); }
		auto output = std::vector<float>(2 * callbackFrames);

		// A new device starts on a new period, so the engine's periods start where the device's frames start
		for (int callback = 0; callback < 40; ++callback)
		{
			for (ch_cnt_t channel = 0; channel < deviceInputs; ++channel)
			{
				for (f_cnt_t frame = 0; frame < callbackFrames; ++frame)
				{
					inputs[channel][frame] = inputSample(channel, framesHanded + frame);
				}
			}
			engine->renderNextBuffer(PlanarBufferView<const float>{inputPointers.data(), deviceInputs, callbackFrames},
				InterleavedBufferView<float>{output.data(), 2, callbackFrames});
			framesHanded += callbackFrames;

			// The last period rendered started at device frame (n - 1) * period; its capture is the period before
			const auto rendered = engine->periodsRendered() - startPeriods;
			const auto capture = engine->capture();
			QCOMPARE(capture.channels(), engineInputs);
			QCOMPARE(capture.frames(), period);
			const auto captureStart = (rendered - 2) * static_cast<std::int64_t>(period);
			for (ch_cnt_t channel = 0; channel < engineInputs; ++channel)
			{
				for (f_cnt_t frame = 0; frame < period; ++frame)
				{
					const auto expected = captureStart < 0 || channel >= deviceInputs
						? 0.f
						: inputSample(channel, captureStart + frame);
					if (capture.sample(channel, frame) != expected)
					{
						QFAIL(qPrintable(QString{"callback %1, channel %2, frame %3: %4 instead of %5"}
							.arg(callback).arg(channel).arg(frame).arg(capture.sample(channel, frame))
							.arg(expected)));
					}
				}
			}
		}
	}

private slots:
	void initTestCase()
	{
		Engine::init(true);
		// The tests drive the engine themselves
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	void renderOnlyDeviceCapturesNothing()
	{
		QCOMPARE(Engine::audioEngine()->audioDev()->inputChannels(), ch_cnt_t{0});
		QCOMPARE(Engine::audioEngine()->capture().channels(), ch_cnt_t{0});
	}

	void inputArrivesWithCallbacksOfAPeriod()
	{
		driveAndCheck(Engine::audioEngine()->framesPerPeriod(), 2, 2);
	}

	void inputArrivesWithCallbacksOfAnyLength()
	{
		driveAndCheck(37, 3, 3);
		driveAndCheck(Engine::audioEngine()->framesPerPeriod() * 3 + 5, 2, 2);
	}

	void missingInputChannelsAreSilentAndExtraOnesIgnored()
	{
		driveAndCheck(100, 1, 2);
		driveAndCheck(100, 4, 2);
	}

	void captureChangesWaitWhileTheModelChanges()
	{
		auto* engine = Engine::audioEngine();
		auto* device = new OnDemandDevice{engine};
		engine->setAudioDevice(std::unique_ptr<AudioDevice>{device}, false);
		engine->setCaptureEnabled(false);
		QCoreApplication::processEvents();
		const auto changes = device->changes;

		// Holding the change lock (as loading a project does), the device is not touched: stopping it would wait
		// for its callback, which waits for the lock
		{
			const auto guard = engine->requestChangesGuard();
			engine->setCaptureEnabled(true);
			QCOMPARE(device->changes, changes);
		}

		// Then it is
		QCoreApplication::processEvents();
		QVERIFY(device->capturing);
		QVERIFY(device->changes > changes);
		engine->setCaptureEnabled(false);
		QVERIFY(!device->capturing);
	}

	void outputOnlyCallbacksLeaveSilence()
	{
		auto* engine = Engine::audioEngine();
		engine->setAudioDevice(std::unique_ptr<AudioDevice>{new FakeDuplexDevice{2, engine}}, false);
		const auto period = engine->framesPerPeriod();
		auto output = std::vector<float>(2 * period);
		for (int callback = 0; callback < 3; ++callback)
		{
			engine->renderNextBuffer(InterleavedBufferView<float>{output.data(), 2, period});
		}
		const auto capture = engine->capture();
		for (ch_cnt_t channel = 0; channel < 2; ++channel)
		{
			for (f_cnt_t frame = 0; frame < period; ++frame) { QCOMPARE(capture.sample(channel, frame), 0.f); }
		}
	}

#ifdef LMMS_BUILD_APPLE
	//! Opens the real devices of this Mac; only with LINUXMUSIC_TEST_HARDWARE=1 (it may ask for the microphone)
	void coreAudioRunsDuplex_data()
	{
		QTest::addColumn<QString>("input");
		QTest::newRow("output only") << QString{"none"};
		QTest::newRow("default input") << QString{};
	}

	void coreAudioRunsDuplex()
	{
		if (qEnvironmentVariableIntValue("LINUXMUSIC_TEST_HARDWARE") != 1)
		{
			QSKIP("Set LINUXMUSIC_TEST_HARDWARE=1 to test the audio hardware");
		}
		QFETCH(QString, input);
		if (input != "none" && qEnvironmentVariableIntValue("LINUXMUSIC_TEST_MICROPHONE") != 1)
		{
			// Opening an input waits for the user to allow the microphone (macOS' privacy prompt)
			QSKIP("Set LINUXMUSIC_TEST_MICROPHONE=1 too, once this program may use the microphone");
		}
		ConfigManager::inst()->setValue("audiocoreaudio", "inputdevice", input);

		auto* engine = Engine::audioEngine();
		auto successful = false;
		auto* device = new AudioCoreAudio{successful, engine};
		QVERIFY(successful);
		qInfo("rate %u, inputs %d, input latency %d, output latency %d", device->sampleRate(),
			static_cast<int>(device->inputChannels()), static_cast<int>(device->inputLatency()),
			static_cast<int>(device->outputLatency()));
		for (ch_cnt_t channel = 0; channel < device->inputChannels(); ++channel)
		{
			qInfo("input %d: %s", channel + 1, qPrintable(device->inputChannelName(channel)));
		}
		QVERIFY(device->outputLatency() > 0);
		if (input == "none") { QCOMPARE(device->inputChannels(), ch_cnt_t{0}); }
		QVERIFY(device->capturesOnDemand());

		engine->setAudioDevice(std::unique_ptr<AudioDevice>{device}, true);
		engine->setCaptureEnabled(input != "none"); // the input opens only when asked for
		const auto before = engine->periodsRendered();
		QTest::qWait(1000);
		const auto rendered = engine->periodsRendered() - before;
		engine->stopProcessing();

		// The input brings sound (the room's noise at least), not silence
		if (input != "none")
		{
			const auto capture = engine->capture();
			auto peak = 0.f;
			for (ch_cnt_t channel = 0; channel < capture.channels(); ++channel)
			{
				for (f_cnt_t frame = 0; frame < capture.frames(); ++frame)
				{
					peak = std::max(peak, std::abs(capture.sample(channel, frame)));
				}
			}
			// Silence is all macOS hands a program not allowed to use the microphone (or a muted one)
			if (peak > 0.f) { qInfo("input peak %.6f", peak); }
			else { qWarning("the input is silent: is this program allowed to use the microphone, is it muted?"); }
		}
		engine->setCaptureEnabled(false);

		// About a second of periods (allowing for the device starting up)
		const auto expected = device->sampleRate() / engine->framesPerPeriod();
		qInfo("%lld periods in a second (%u expected), %u xruns", static_cast<long long>(rendered), expected,
			device->xruns());
		QVERIFY(rendered > expected / 2);
		QVERIFY(rendered < expected * 2);
	}
#endif

	void theDeviceOpensAgainWithNewSettingsWithoutRestarting()
	{
		auto* engine = Engine::audioEngine();
		const auto period = engine->framesPerPeriod();
		ConfigManager::inst()->setValue("audioengine", "audiodev", AudioDummy::name());
		ConfigManager::inst()->setValue("audioengine", "samplerate", "48000");
		ConfigManager::inst()->setValue("audioengine", "framesperaudiobuffer", "1024");
		const auto told = engine->reopenAudioDevice();
		QVERIFY2(told.isEmpty(), qPrintable(told));
		QCOMPARE(engine->audioDevName(), AudioDummy::name());
		QCOMPARE(engine->framesPerAudioBuffer(), f_cnt_t{1024});
		QCOMPARE(engine->outputSampleRate(), sample_rate_t{48000});
		QCOMPARE(engine->framesPerPeriod(), period); // (the engine's period stays)
		engine->stopProcessing(); // (the tests drive the engine themselves)
		QCOMPARE(engine->renderNextPeriod().size(), static_cast<std::size_t>(period));
	}
};

QTEST_GUILESS_MAIN(AudioInputTest)
#include "AudioInputTest.moc"
