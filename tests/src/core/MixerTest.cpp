/*
 * MixerTest.cpp - tests for mixing and routing between mixer channels
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

#include <vector>

#include "AudioBuffer.h"
#include "AudioEngine.h"
#include "Engine.h"
#include "Mixer.h"
#include "SampleFrame.h"

using namespace lmms;

class MixerTest : public QObject
{
	Q_OBJECT

	static auto mixer() -> Mixer* { return Engine::mixer(); }
	static auto frames() -> f_cnt_t { return Engine::audioEngine()->framesPerPeriod(); }

	//! Sends a constant stereo signal into a mixer channel
	static void feed(mix_ch_t channel, float left, float right)
	{
		auto buffer = AudioBuffer{frames()};
		auto l = buffer.buffer(0);
		auto r = buffer.buffer(1);
		std::fill(l.begin(), l.end(), left);
		std::fill(r.begin(), r.end(), right);
		buffer.assumeNonSilent(0);
		buffer.assumeNonSilent(1);
		mixer()->mixToChannel(buffer, channel);
	}

	//! Runs the mixer for one period and returns the master output
	static auto mix() -> std::vector<SampleFrame>
	{
		// A new period, as the audio engine does before rendering
		AutomatableModel::incrementPeriodCounter();
		auto out = std::vector<SampleFrame>(frames());
		mixer()->masterMix(out.data());
		return out;
	}

	//! Changed parameters ramp to their new value over the next period that processes them (to avoid
	//! clicks). Channels without signal skip their parameters, so feed every channel a zero signal and
	//! run one period; the following period then uses the final values.
	static void settle()
	{
		for (auto channel = 0; channel < mixer()->numChannels(); ++channel) { feed(channel, 0.f, 0.f); }
		mix();
	}

	static void compareOutput(const std::vector<SampleFrame>& out, float left, float right)
	{
		for (const auto& frame : out)
		{
			QVERIFY(qFuzzyCompare(1.f + frame.left(), 1.f + left));
			QVERIFY(qFuzzyCompare(1.f + frame.right(), 1.f + right));
		}
	}

private slots:
	void initTestCase()
	{
		Engine::init(true);
		// The tests render the periods themselves
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	//! Start each test from a mixer with only the master channel at its defaults
	void init()
	{
		mixer()->clear();
		mixer()->clearChannel(0);
		settle();
	}

	void signalIntoMasterReachesOutput()
	{
		feed(0, 0.5f, -0.25f);
		compareOutput(mix(), 0.5f, -0.25f);
	}

	void masterVolumeScalesOutput()
	{
		mixer()->mixerChannel(0)->m_volumeModel.setValue(0.5f);
		settle();
		feed(0, 0.8f, 0.4f);
		compareOutput(mix(), 0.4f, 0.2f);
	}

	void newChannelSendsToMaster()
	{
		const auto channel = mixer()->createChannel();
		QVERIFY(mixer()->channelSendModel(channel, 0) != nullptr);
		QCOMPARE(mixer()->channelSendModel(channel, 0)->value(), 1.f);

		feed(channel, 0.3f, 0.6f);
		compareOutput(mix(), 0.3f, 0.6f);
	}

	void channelVolumeAndSendAmountMultiply()
	{
		const auto channel = mixer()->createChannel();
		mixer()->mixerChannel(channel)->m_volumeModel.setValue(0.5f);
		mixer()->channelSendModel(channel, 0)->setValue(0.5f);
		settle();

		feed(channel, 0.8f, 0.8f);
		compareOutput(mix(), 0.2f, 0.2f);
	}

	void channelsAreSummed()
	{
		const auto a = mixer()->createChannel();
		const auto b = mixer()->createChannel();
		feed(a, 0.25f, 0.f);
		feed(b, 0.25f, 0.5f);
		compareOutput(mix(), 0.5f, 0.5f);
	}

	void chainedChannelsApplyEachVolume()
	{
		const auto first = mixer()->createChannel();
		const auto second = mixer()->createChannel();

		// first -> second -> master
		mixer()->deleteChannelSend(first, 0);
		mixer()->createChannelSend(first, second);
		mixer()->mixerChannel(first)->m_volumeModel.setValue(0.5f);
		mixer()->mixerChannel(second)->m_volumeModel.setValue(0.5f);
		settle();

		feed(first, 0.8f, 0.4f);
		compareOutput(mix(), 0.2f, 0.1f);
	}

	void mutedChannelIsSilent()
	{
		const auto channel = mixer()->createChannel();
		mixer()->mixerChannel(channel)->m_muteModel.setValue(true);
		settle();
		feed(channel, 0.5f, 0.5f);
		compareOutput(mix(), 0.f, 0.f);
	}

	void channelBuffersAreClearedBetweenPeriods()
	{
		const auto channel = mixer()->createChannel();
		feed(channel, 0.5f, 0.5f);
		mix();
		compareOutput(mix(), 0.f, 0.f);
	}

	void soloMutesTheOtherChannels()
	{
		const auto soloed = mixer()->createChannel();
		const auto other = mixer()->createChannel();
		mixer()->mixerChannel(soloed)->m_soloModel.setValue(true);
		mixer()->toggledSolo();
		settle();

		feed(soloed, 0.25f, 0.25f);
		feed(other, 0.5f, 0.5f);
		compareOutput(mix(), 0.25f, 0.25f);

		mixer()->mixerChannel(soloed)->m_soloModel.setValue(false);
		mixer()->toggledSolo();
		QCOMPARE(mixer()->mixerChannel(other)->m_muteModel.value(), false);
	}

	void routingLoopsAreDetected()
	{
		const auto a = mixer()->createChannel();
		const auto b = mixer()->createChannel();
		mixer()->createChannelSend(a, b);
		QVERIFY(mixer()->isInfiniteLoop(b, a));
		QVERIFY(!mixer()->isInfiniteLoop(a, b));
	}

	void deletingChannelRemovesItsSends()
	{
		const auto a = mixer()->createChannel();
		const auto b = mixer()->createChannel();
		mixer()->createChannelSend(a, b);
		mixer()->deleteChannel(b);

		QCOMPARE(mixer()->numChannels(), 2);
		QVERIFY(mixer()->channelSendModel(a, 0) != nullptr);
		feed(a, 0.5f, 0.5f);
		compareOutput(mix(), 0.5f, 0.5f);
	}

	void peakMeterShowsPostFaderLevel()
	{
		const auto channel = mixer()->createChannel();
		auto ch = mixer()->mixerChannel(channel);
		ch->m_peakLeft = ch->m_peakRight = 0.f;
		ch->m_volumeModel.setValue(0.5f);
		settle();
		feed(channel, 0.8f, -0.4f);
		mix();
		QVERIFY(qFuzzyCompare(ch->m_peakLeft, 0.4f));
		QVERIFY(qFuzzyCompare(ch->m_peakRight, 0.2f));
	}
};

QTEST_GUILESS_MAIN(MixerTest)
#include "MixerTest.moc"
