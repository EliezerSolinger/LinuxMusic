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
#include "DataFile.h"
#include "Mixer.h"
#include "ProjectJournal.h"
#include "SampleTrack.h"
#include "Song.h"
#include "SampleFrame.h"
#include "lmms_math.h"

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

	//! Changed parameters ramp to their new value over the next period (to avoid clicks).
	//! Run one period without signal; the following period then uses the final values.
	static void settle()
	{
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

	void parameterChangesWhileSilentDoNotRampLater()
	{
		// Changing the fader of a silent channel must not make the next sound start with a ramp
		const auto channel = mixer()->createChannel();
		mixer()->mixerChannel(channel)->m_volumeModel.setValue(0.25f);
		mix();
		feed(channel, 0.8f, 0.8f);
		compareOutput(mix(), 0.2f, 0.2f);
	}

	void parameterChangesWhileMutedDoNotRampLater()
	{
		const auto channel = mixer()->createChannel();
		mixer()->mixerChannel(channel)->m_muteModel.setValue(true);
		mixer()->mixerChannel(channel)->m_volumeModel.setValue(0.25f);
		mix();
		mixer()->mixerChannel(channel)->m_muteModel.setValue(false);
		feed(channel, 0.8f, 0.8f);
		compareOutput(mix(), 0.2f, 0.2f);
	}

	void panMovesTheSignalToOneSide()
	{
		const auto channel = mixer()->createChannel();
		mixer()->mixerChannel(channel)->m_panModel.setValue(-100.f);
		settle();
		feed(channel, 0.5f, 0.5f);
		compareOutput(mix(), 0.5f, 0.f);
	}

	void zeroWidthMakesMono()
	{
		const auto channel = mixer()->createChannel();
		mixer()->mixerChannel(channel)->m_widthModel.setValue(0.f);
		settle();
		feed(channel, 0.6f, 0.2f);
		compareOutput(mix(), 0.4f, 0.4f);
	}

	void phaseInvertAndSwap()
	{
		const auto channel = mixer()->createChannel();
		mixer()->mixerChannel(channel)->m_phaseInvertModel.setValue(true);
		mixer()->mixerChannel(channel)->m_swapChannelsModel.setValue(true);
		settle();
		feed(channel, 0.6f, 0.2f);
		compareOutput(mix(), -0.2f, -0.6f);
	}

	void trimIsAppliedBeforeTheFader()
	{
		const auto channel = mixer()->createChannel();
		mixer()->mixerChannel(channel)->m_trimModel.setValue(ampToDbfs(2.f));
		mixer()->mixerChannel(channel)->m_volumeModel.setValue(0.5f);
		settle();
		feed(channel, 0.3f, 0.1f);
		compareOutput(mix(), 0.3f, 0.1f);
	}

	void preFaderSendIgnoresTheSendersFader()
	{
		const auto source = mixer()->createChannel();
		const auto bus = mixer()->createChannel();
		auto send = mixer()->createChannelSend(source, bus);
		send->preFader()->setValue(true);
		// The source's own output is muted by its fader; only the pre-fader send reaches the master
		mixer()->mixerChannel(source)->m_volumeModel.setValue(0.f);
		mixer()->mixerChannel(source)->m_panModel.setValue(100.f);
		settle();
		feed(source, 0.4f, 0.2f);
		compareOutput(mix(), 0.4f, 0.2f);
	}

	void postFaderSendFollowsTheSendersFader()
	{
		const auto source = mixer()->createChannel();
		const auto bus = mixer()->createChannel();
		mixer()->deleteChannelSend(source, 0);
		mixer()->createChannelSend(source, bus);
		mixer()->mixerChannel(source)->m_volumeModel.setValue(0.5f);
		settle();
		feed(source, 0.4f, 0.2f);
		compareOutput(mix(), 0.2f, 0.1f);
	}

	void channelStripIsSavedAndLoaded()
	{
		const auto a = mixer()->createChannel();
		const auto b = mixer()->createChannel();
		auto ch = mixer()->mixerChannel(a);
		ch->m_trimModel.setValue(3.f);
		ch->m_widthModel.setValue(150.f);
		ch->m_panModel.setValue(-40.f);
		ch->m_phaseInvertModel.setValue(true);
		ch->m_swapChannelsModel.setValue(true);
		mixer()->createChannelSend(a, b)->preFader()->setValue(true);

		auto doc = DataFile{DataFile::Type::SongProject};
		auto element = doc.createElement("mixer");
		mixer()->saveSettings(doc, element);
		mixer()->clear();
		mixer()->loadSettings(element);

		ch = mixer()->mixerChannel(a);
		QCOMPARE(ch->m_trimModel.value(), 3.f);
		QCOMPARE(ch->m_widthModel.value(), 150.f);
		QCOMPARE(ch->m_panModel.value(), -40.f);
		QCOMPARE(ch->m_phaseInvertModel.value(), true);
		QCOMPARE(ch->m_swapChannelsModel.value(), true);
		QCOMPARE(mixer()->channelSendModel(a, b) != nullptr, true);
		const auto& sends = ch->m_sends;
		const auto it = std::find_if(sends.begin(), sends.end(), [b](auto r) { return r->receiverIndex() == b; });
		QVERIFY(it != sends.end());
		QCOMPARE((*it)->isPreFader(), true);
	}

	void masterSettingsSurviveSaveStateAndRestoreState()
	{
		// The path projects use: saveState() adds a journalling node next to the channels
		mixer()->mixerChannel(0)->m_volumeModel.setValue(0.5f);
		mixer()->mixerChannel(0)->m_panModel.setValue(30.f);

		auto doc = DataFile{DataFile::Type::SongProject};
		auto parent = doc.content();
		const auto element = mixer()->saveState(doc, parent);
		mixer()->clear();
		mixer()->restoreState(element);

		QCOMPARE(mixer()->mixerChannel(0)->m_volumeModel.value(), 0.5f);
		QCOMPARE(mixer()->mixerChannel(0)->m_panModel.value(), 30.f);
	}

	void channelInputDefaultsToNone()
	{
		const auto channel = mixer()->createChannel();
		const auto ch = mixer()->mixerChannel(channel);
		QVERIFY(ch->m_input == ChannelInput{});
		QCOMPARE(ch->m_recordArmModel.value(), false);
		QCOMPARE(ch->m_monitorModel.value(), static_cast<int>(MixerChannel::MonitorMode::Auto));
	}

	void channelInputIsSavedAndLoaded()
	{
		const auto channel = mixer()->createChannel();
		auto changed = QSignalSpy{mixer(), &Mixer::channelInputChanged};
		const auto input = ChannelInput{ChannelInput::Source::Hardware, 2, 1};
		mixer()->setChannelInput(channel, input);
		QCOMPARE(changed.count(), 1);
		mixer()->mixerChannel(channel)->m_recordArmModel.setValue(true);
		mixer()->mixerChannel(channel)->m_monitorModel.setValue(static_cast<int>(MixerChannel::MonitorMode::On));

		auto doc = DataFile{DataFile::Type::SongProject};
		auto element = doc.createElement("mixer");
		mixer()->saveSettings(doc, element);
		mixer()->clear();
		QVERIFY(mixer()->numChannels() == 1);
		mixer()->loadSettings(element);

		const auto ch = mixer()->mixerChannel(channel);
		QVERIFY(ch->m_input == input);
		QCOMPARE(ch->m_recordArmModel.value(), true);
		QCOMPARE(ch->m_monitorModel.value(), static_cast<int>(MixerChannel::MonitorMode::On));
	}

	void undoCreateChannel()
	{
		const auto journal = Engine::projectJournal();
		journal->setJournalling(true);
		journal->clearJournal();

		mixer()->createChannel();
		QCOMPARE(mixer()->numChannels(), 2);
		journal->undo();
		QCOMPARE(mixer()->numChannels(), 1);
		journal->redo();
		QCOMPARE(mixer()->numChannels(), 2);

		journal->setJournalling(false);
	}

	void undoDeleteChannelRestoresTracksAndSends()
	{
		const auto journal = Engine::projectJournal();
		const auto a = mixer()->createChannel();
		const auto b = mixer()->createChannel();
		mixer()->createChannelSend(a, b);
		auto track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, Engine::getSong()));
		QVERIFY(track);
		track->mixerChannelModel()->setValue(b);

		journal->setJournalling(true);
		journal->clearJournal();

		// Deleting channel a removes its sends and renumbers channel b, so the track moves to channel a
		mixer()->deleteChannel(a);
		QCOMPARE(mixer()->numChannels(), 2);
		QCOMPARE(track->mixerChannelModel()->value(), a);

		// One action is one undo step, and it brings back the channel, its send and the track's channel
		journal->undo();
		QCOMPARE(journal->canUndo(), false);
		QCOMPARE(mixer()->numChannels(), 3);
		QVERIFY(mixer()->channelSendModel(a, b) != nullptr);
		QCOMPARE(track->mixerChannelModel()->value(), b);

		journal->setJournalling(false);
		delete track;
	}

	void peakMeterShowsPostFaderLevel()
	{
		const auto channel = mixer()->createChannel();
		auto& meter = mixer()->mixerChannel(channel)->m_meter;
		mixer()->mixerChannel(channel)->m_volumeModel.setValue(0.5f);
		settle();
		meter.takePeaks();

		feed(channel, 0.8f, -0.4f);
		mix();
		const auto peaks = meter.takePeaks();
		QVERIFY(peaks.has_value());
		QVERIFY(qFuzzyCompare((*peaks)[0], 0.4f));
		QVERIFY(qFuzzyCompare((*peaks)[1], 0.2f));

		// Reading the peaks resets them until the next period
		QVERIFY(!meter.takePeaks().has_value());
	}

	void peakMeterKeepsTheHighestPeakUntilRead()
	{
		const auto channel = mixer()->createChannel();
		auto& meter = mixer()->mixerChannel(channel)->m_meter;
		settle();
		meter.takePeaks();

		feed(channel, 0.6f, 0.6f);
		mix();
		feed(channel, 0.2f, 0.2f);
		mix();
		const auto peaks = meter.takePeaks();
		QVERIFY(peaks.has_value());
		QVERIFY(qFuzzyCompare((*peaks)[0], 0.6f));
	}

	void rmsMeterFollowsALevelOverTime()
	{
		const auto channel = mixer()->createChannel();
		const auto& meter = mixer()->mixerChannel(channel)->m_meter;
		settle();

		// A constant signal's RMS is its level; feed long enough for the 300 ms average to settle
		const auto periods = static_cast<int>(2 * ChannelMeter::RmsWindow
			* Engine::audioEngine()->outputSampleRate() / frames()) + 1;
		for (int i = 0; i < periods; ++i)
		{
			feed(channel, 0.5f, 0.25f);
			mix();
		}
		QVERIFY(std::abs(meter.rms(0) - 0.5f) < 0.5f * 0.15f);
		QVERIFY(std::abs(meter.rms(1) - 0.25f) < 0.25f * 0.15f);

		// Muting drops the RMS level to zero
		mixer()->mixerChannel(channel)->m_muteModel.setValue(true);
		mix();
		QCOMPARE(meter.rms(0), 0.f);
	}
};

QTEST_GUILESS_MAIN(MixerTest)
#include "MixerTest.moc"
