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

#include <cmath>
#include <numbers>
#include <functional>
#include <vector>

#include "AnalogChannelStrip.h"
#include "AudioBuffer.h"
#include "AudioEngine.h"
#include "AutomationClip.h"
#include "AutomationTrack.h"
#include "Engine.h"
#include "DataFile.h"
#include "Effect.h"
#include "EffectControls.h"
#include "FolderTrack.h"
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "MasterSection.h"
#include "Mixer.h"
#include "PluginChecks.h"
#include "PluginInstance.h"
#include "ProjectJournal.h"
#include "SampleTrack.h"
#include "Song.h"
#include "SampleFrame.h"
#include "lmms_math.h"

using namespace lmms;

namespace
{

Plugin::Descriptor s_duckerDescriptor = {
	"fakeducker", "Fake ducker", "Effect for tests", "LinuxMusic", 0x0100, Plugin::Type::Effect,
	nullptr, nullptr, nullptr
};

class DuckerControls : public EffectControls
{
public:
	using EffectControls::EffectControls;
	void saveSettings(QDomDocument&, QDomElement&) override {}
	void loadSettings(const QDomElement&) override {}
	auto nodeName() const -> QString override { return "duckercontrols"; }
	auto controlCount() -> int override { return 0; }
	auto createView() -> gui::EffectControlDialog* override { return nullptr; }
};

//! Like a sidechain compressor: stereo main inputs and a stereo sidechain input (a second channel group).
//! The output is the main input at a quarter of its level while the sidechain has any signal.
class FakeDucker : public Effect
{
public:
	FakeDucker() : Effect{&s_duckerDescriptor, nullptr, nullptr}, m_controls{this} {}

	auto controls() -> EffectControls* override { return &m_controls; }
	auto process(ProcessContext& context) -> ProcessStatus override
	{
		const auto& in = context.in();
		auto& out = context.out();
		bool sidechain = false;
		for (ch_cnt_t ch = 2; ch < 4; ++ch)
		{
			for (const auto sample : in.buffer(ch)) { sidechain = sidechain || sample != 0.f; }
		}
		const float gain = sidechain ? 0.25f : 1.f;
		for (ch_cnt_t ch = 0; ch < 2; ++ch)
		{
			const auto src = in.buffer(ch);
			auto dst = out.buffer(ch);
			for (std::size_t f = 0; f < dst.size(); ++f) { dst[f] = src[f] * gain; }
		}
		return ProcessStatus::Continue;
	}
	auto channelCount(bool isInput) const -> ch_cnt_t override { return isInput ? 4 : 2; }
	auto channelGroupCount(bool isInput) const -> group_cnt_t override { return isInput ? 2 : 1; }
	auto getChannelGroup(group_cnt_t, bool, AudioBuffer::ChannelGroup&) const -> ch_cnt_t override { return 2; }
	auto getInplacePair(ch_cnt_t) const -> std::optional<ch_cnt_t> override { return std::nullopt; }

private:
	DuckerControls m_controls;
};

//! Delays its stereo input by a fixed number of frames and reports it as its latency, like a lookahead limiter
class FakeDelay : public Effect
{
public:
	explicit FakeDelay(f_cnt_t delay) :
		Effect{&s_duckerDescriptor, nullptr, nullptr},
		m_controls{this},
		m_lines(2, std::vector<float>(delay, 0.f))
	{}

	auto controls() -> EffectControls* override { return &m_controls; }
	auto process(ProcessContext& context) -> ProcessStatus override
	{
		const auto& in = context.in();
		auto& out = context.out();
		const auto delay = m_lines[0].size();
		auto position = m_position;
		for (ch_cnt_t ch = 0; ch < 2; ++ch)
		{
			position = m_position;
			const auto src = in.buffer(ch);
			auto dst = out.buffer(ch);
			for (std::size_t f = 0; f < dst.size(); ++f)
			{
				const float delayed = m_lines[ch][position];
				m_lines[ch][position] = src[f];
				dst[f] = delayed;
				position = (position + 1) % delay;
			}
		}
		m_position = position;
		mixDrySignal(in, out, 2);
		return ProcessStatus::Continue;
	}
	auto latency() const -> f_cnt_t override { return m_lines[0].size(); }
	auto channelCount(bool) const -> ch_cnt_t override { return 2; }
	auto channelGroupCount(bool) const -> group_cnt_t override { return 1; }
	auto getChannelGroup(group_cnt_t, bool, AudioBuffer::ChannelGroup&) const -> ch_cnt_t override { return 2; }
	auto getInplacePair(ch_cnt_t) const -> std::optional<ch_cnt_t> override { return std::nullopt; }

private:
	DuckerControls m_controls;
	std::vector<std::vector<float>> m_lines;
	std::size_t m_position = 0;
};

Plugin::Descriptor s_multiOutputDescriptor = {
	"fakemultioutput", "Fake multi-output instrument", "Instrument for tests", "LinuxMusic", 0x0100,
	Plugin::Type::Instrument, nullptr, nullptr, nullptr
};

//! An instrument with named output groups, like a drum kit
class FakeMultiOutput : public Instrument
{
public:
	FakeMultiOutput() : Instrument{&s_multiOutputDescriptor, nullptr, nullptr, Flag::IsSingleStreamed} {}

	void saveSettings(QDomDocument&, QDomElement&) override {}
	void loadSettings(const QDomElement&) override {}
	auto nodeName() const -> QString override { return "fakemultioutput"; }
	auto instantiateView(QWidget*) -> gui::PluginView* override { return nullptr; }
	auto process(ProcessContext&) -> ProcessStatus override { return ProcessStatus::Continue; }
	auto channelCount(bool isInput) const -> ch_cnt_t override { return isInput ? 0 : 4; }
	auto channelGroupCount(bool isInput) const -> group_cnt_t override { return isInput ? 0 : 3; }
	auto getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup&) const -> ch_cnt_t override
	{
		return isInput ? 0 : (index < 2 ? 1 : 2);
	}
	auto getInplacePair(ch_cnt_t) const -> std::optional<ch_cnt_t> override { return std::nullopt; }
	auto channelGroupName(group_cnt_t index, bool isInput) const -> std::string override
	{
		return isInput ? std::string{} : std::array{"Kick", "Snare", "Cymbals"}[index];
	}
};

} // namespace

//! Levels of the master's output (see MixerTest::playStereo())
struct StereoLevels
{
	float mid;
	float side;
	float biggestStep;
};

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

	/**
	 * Plays a sine of @p frequency and @p amplitude into channel @p channel for @p periods periods and returns the
	 * RMS level of the master output over the last quarter of them, relative to the sine's
	 */
	static auto sineGain(mix_ch_t channel, float frequency, float amplitude = 0.25f, int periods = 64) -> float
	{
		const auto rate = static_cast<float>(Engine::audioEngine()->outputSampleRate());
		double sum = 0.;
		f_cnt_t measured = 0;
		f_cnt_t position = 0;
		for (int period = 0; period < periods; ++period)
		{
			auto buffer = AudioBuffer{frames()};
			for (f_cnt_t f = 0; f < frames(); ++f, ++position)
			{
				const float x = amplitude * std::sin(2.f * std::numbers::pi_v<float> * frequency * position / rate);
				buffer.buffer(0)[f] = x;
				buffer.buffer(1)[f] = x;
			}
			buffer.assumeNonSilent(0);
			buffer.assumeNonSilent(1);
			mixer()->mixToChannel(buffer, channel);
			const auto out = mix();
			if (period < periods * 3 / 4) { continue; }
			for (const auto& frame : out) { sum += frame.left() * frame.left(); }
			measured += out.size();
		}
		return static_cast<float>(std::sqrt(sum / measured) / (amplitude / std::sqrt(2.)));
	}

	//! Runs the mixer for one period and returns the master output
	static auto mix() -> std::vector<SampleFrame>
	{
		// A new period, as the audio engine does before rendering
		AutomatableModel::incrementPeriodCounter();
		auto out = std::vector<SampleFrame>(frames());
		const auto audioThread = ThreadCheck::AudioThreadScope{}; // like the audio engine's rendering thread
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

		// The effects in the tests follow the plugin API's rules
		PluginChecks::setEnabled(true);
		PluginChecks::setViolationHandler([](std::string_view message) {
			QFAIL(qPrintable(QString{"Plugin API violation: "} + QString::fromUtf8(message.data(), message.size())));
		});
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	//! Start each test from a mixer with only the master channel at its defaults
	void init()
	{
		// As in projects, with the effect returns
		mixer()->clear();
		mixer()->clearChannel(0);
		mixer()->ensureFxReturns();
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

	void soloSilencesTheOtherChannels()
	{
		const auto soloed = mixer()->createChannel();
		const auto other = mixer()->createChannel();
		mixer()->mixerChannel(soloed)->m_soloModel.setValue(true);
		settle();

		feed(soloed, 0.25f, 0.25f);
		feed(other, 0.5f, 0.5f);
		compareOutput(mix(), 0.25f, 0.25f);
		// Solo leaves the mute buttons alone
		QCOMPARE(mixer()->mixerChannel(other)->m_muteModel.value(), false);

		mixer()->mixerChannel(soloed)->m_soloModel.setValue(false);
		feed(soloed, 0.25f, 0.25f);
		feed(other, 0.5f, 0.5f);
		compareOutput(mix(), 0.75f, 0.75f);
	}

	void soloKeepsThePathAndTheSources()
	{
		// source --> bus --> master, other --> bus, solo on the bus: both sources are heard;
		// solo on the source: the bus passes it, the other source is silent
		const auto source = mixer()->createChannel();
		const auto other = mixer()->createChannel();
		const auto bus = mixer()->createChannel();
		for (const auto channel : {source, other})
		{
			mixer()->deleteChannelSend(channel, 0);
			mixer()->createChannelSend(channel, bus);
		}
		settle();

		mixer()->mixerChannel(bus)->m_soloModel.setValue(true);
		feed(source, 0.25f, 0.25f);
		feed(other, 0.5f, 0.5f);
		compareOutput(mix(), 0.75f, 0.75f);
		mixer()->mixerChannel(bus)->m_soloModel.setValue(false);

		mixer()->mixerChannel(source)->m_soloModel.setValue(true);
		feed(source, 0.25f, 0.25f);
		feed(other, 0.5f, 0.5f);
		compareOutput(mix(), 0.25f, 0.25f);

		// Two solos at once
		mixer()->mixerChannel(other)->m_soloModel.setValue(true);
		feed(source, 0.25f, 0.25f);
		feed(other, 0.5f, 0.5f);
		compareOutput(mix(), 0.75f, 0.75f);
		mixer()->mixerChannel(source)->m_soloModel.setValue(false);
		mixer()->mixerChannel(other)->m_soloModel.setValue(false);
	}

	void soloedChannelKeepsItsSidechainSource()
	{
		// The kick feeds the bass's sidechain input: soloing the bass keeps the kick in its sidechain only
		const auto kick = mixer()->createChannel();
		const auto bass = mixer()->createChannel();
		QVERIFY(mixer()->toggleSidechainSend(kick, bass));
		mixer()->mixerChannel(bass)->m_soloModel.setValue(true);
		settle();

		feed(kick, 0.5f, 0.5f);
		feed(bass, 0.25f, 0.25f);
		compareOutput(mix(), 0.25f, 0.25f);
		QVERIFY(!mixer()->mixerChannel(kick)->m_silencedBySolo);
		QVERIFY(mixer()->findChannelSend(kick, bass, 0, 1)->isLive());
		QVERIFY(!mixer()->findChannelSend(kick, 0)->isLive());

		mixer()->mixerChannel(bass)->m_soloModel.setValue(false);
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

		QCOMPARE(mixer()->numChannels(), 2 + Mixer::FxReturnCount);
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

		const auto before = mixer()->numChannels();
		mixer()->createChannel();
		QCOMPARE(mixer()->numChannels(), before + 1);
		journal->undo();
		QCOMPARE(mixer()->numChannels(), before);
		journal->redo();
		QCOMPARE(mixer()->numChannels(), before + 1);

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

		// (the track got a channel of its own when created, which it does not use anymore)
		const auto channels = mixer()->numChannels();

		// Deleting channel a removes its sends and renumbers channel b, so the track moves to channel a
		mixer()->deleteChannel(a);
		QCOMPARE(mixer()->numChannels(), channels - 1);
		QCOMPARE(track->mixerChannelModel()->value(), a);

		// One action is one undo step, and it brings back the channel, its send and the track's channel
		journal->undo();
		QCOMPARE(journal->canUndo(), false);
		QCOMPARE(mixer()->numChannels(), channels);
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

	///////////////////////////////////////////
	// Track channels beyond the main pair   //
	///////////////////////////////////////////

	void channelCountIsEvenAndLimited()
	{
		const auto channel = mixer()->createChannel();
		const auto ch = mixer()->mixerChannel(channel);
		QCOMPARE(ch->channelCount(), 2);

		mixer()->setChannelCount(channel, 4);
		QCOMPARE(ch->channelCount(), 4);
		QCOMPARE(ch->pairCount(), 2);
		QCOMPARE(ch->m_buffer.groupCount(), 2);
		QCOMPARE(ch->m_buffer.group(1).channels(), 2);

		mixer()->setChannelCount(channel, 5);
		QCOMPARE(ch->channelCount(), 6);
		mixer()->setChannelCount(channel, 1000);
		QCOMPARE(ch->channelCount(), MaxMixerChannelTrackChannels);
		mixer()->setChannelCount(channel, 0);
		QCOMPARE(ch->channelCount(), 2);
	}

	void sendNeedsTheDestinationPair()
	{
		const auto kick = mixer()->createChannel();
		const auto bass = mixer()->createChannel();
		QVERIFY(mixer()->createChannelSend(kick, bass, 1.f, 0, 1) == nullptr);
		mixer()->setChannelCount(bass, 4);
		const auto send = mixer()->createChannelSend(kick, bass, 1.f, 0, 1);
		QVERIFY(send != nullptr);
		QVERIFY(send->isSidechain());

		// A sidechain send and a normal send between the same channels are different sends
		QVERIFY(mixer()->createChannelSend(kick, bass) != send);
		QCOMPARE(mixer()->findChannelSend(kick, bass, 0, 1), send);
	}

	void auxiliaryPairsDoNotReachTheOutput()
	{
		const auto kick = mixer()->createChannel();
		const auto bass = mixer()->createChannel();
		mixer()->setChannelCount(bass, 4);
		mixer()->createChannelSend(kick, bass, 1.f, 0, 1);

		feed(kick, 0.4f, 0.4f);
		feed(bass, 0.8f, 0.8f);

		// The kick reaches the master directly; at the bass channel, it only arrives at the channels 3/4
		compareOutput(mix(), 1.2f, 1.2f);
	}

	void sidechainSendReachesTheSidechainInput()
	{
		const auto kick = mixer()->createChannel();
		const auto bass = mixer()->createChannel();
		mixer()->setChannelCount(bass, 4);
		mixer()->createChannelSend(kick, bass, 1.f, 0, 1);

		auto& chain = mixer()->mixerChannel(bass)->m_fxChain;
		chain.appendEffect(std::make_unique<PluginInstance>(std::make_unique<FakeDucker>(), Plugin::Type::Effect));

		// The sidechain inputs were connected to the channels 3/4 by default
		const auto& pins = chain.effects().front()->audioPortsModel()->in();
		QVERIFY(pins.enabled(2, 2));
		QVERIFY(pins.enabled(3, 3));
		QVERIFY(!pins.enabled(0, 2));

		// Without the kick, the bass passes unchanged
		feed(bass, 0.8f, 0.8f);
		compareOutput(mix(), 0.8f, 0.8f);

		// With it, the bass is ducked
		feed(kick, 0.4f, 0.4f);
		feed(bass, 0.8f, 0.8f);
		compareOutput(mix(), 0.4f + 0.2f, 0.4f + 0.2f);

		chain.clear();
	}

	void sidechainInputGetsConnectedWhenTheChannelGrows()
	{
		const auto bass = mixer()->createChannel();
		auto& chain = mixer()->mixerChannel(bass)->m_fxChain;
		chain.appendEffect(std::make_unique<PluginInstance>(std::make_unique<FakeDucker>(), Plugin::Type::Effect));
		const auto model = chain.effects().front()->audioPortsModel();
		QCOMPARE(model->trackChannelCount(), 2);

		mixer()->setChannelCount(bass, 4);
		QCOMPARE(model->trackChannelCount(), 4);
		QVERIFY(model->in().enabled(2, 2));
		QVERIFY(model->in().enabled(3, 3));

		chain.clear();
	}

	void auxiliaryPairCanBeSentOn()
	{
		const auto kick = mixer()->createChannel();
		const auto bus = mixer()->createChannel();
		const auto out = mixer()->createChannel();
		mixer()->setChannelCount(bus, 4);
		mixer()->deleteChannelSend(kick, 0);
		mixer()->deleteChannelSend(bus, 0);
		mixer()->createChannelSend(kick, bus, 1.f, 0, 1);
		mixer()->createChannelSend(bus, out, 0.5f, 1, 0);

		feed(kick, 0.4f, 0.2f);
		compareOutput(mix(), 0.2f, 0.1f);
	}

	void shrinkingAChannelDeletesSendsToMissingPairs()
	{
		const auto kick = mixer()->createChannel();
		const auto bass = mixer()->createChannel();
		mixer()->setChannelCount(bass, 4);
		mixer()->createChannelSend(kick, bass, 1.f, 0, 1);
		mixer()->createChannelSend(bass, kick, 1.f, 1, 0);

		mixer()->setChannelCount(bass, 2);
		QVERIFY(mixer()->findChannelSend(kick, bass, 0, 1) == nullptr);
		QVERIFY(mixer()->findChannelSend(bass, kick, 1, 0) == nullptr);
		QVERIFY(mixer()->findChannelSend(bass, 0) != nullptr);
	}

	void channelCountAndPairSendsAreSavedAndLoaded()
	{
		// The receiver has a higher index than the sender, so its channels 3/4 do not exist yet when the
		// sender's sends are read
		const auto kick = mixer()->createChannel();
		const auto bass = mixer()->createChannel();
		mixer()->setChannelCount(bass, 4);
		mixer()->createChannelSend(kick, bass, 0.7f, 0, 1);

		auto doc = DataFile{DataFile::Type::SongProject};
		auto element = doc.createElement("mixer");
		mixer()->saveSettings(doc, element);
		mixer()->clear();
		QCOMPARE(mixer()->numChannels(), 1);
		mixer()->loadSettings(element);

		QCOMPARE(mixer()->mixerChannel(bass)->channelCount(), 4);
		QCOMPARE(mixer()->mixerChannel(kick)->channelCount(), 2);
		const auto send = mixer()->findChannelSend(kick, bass, 0, 1);
		QVERIFY(send != nullptr);
		QVERIFY(qFuzzyCompare(send->amount()->value(), 0.7f));
	}

	void instrumentAuxiliaryOutputsReachTheChannelsAuxiliaryPairs()
	{
		// A multi-output instrument's track buffer: main pair plus channels 3/4
		const auto drums = mixer()->createChannel();
		const auto toms = mixer()->createChannel();
		mixer()->setChannelCount(drums, 4);
		mixer()->createChannelSend(drums, toms, 1.f, 1, 0);

		auto buffer = AudioBuffer{frames(), 4};
		for (ch_cnt_t ch = 0; ch < 4; ++ch)
		{
			std::ranges::fill(buffer.buffer(ch), ch < 2 ? 0.f : 0.25f);
			if (ch >= 2) { buffer.assumeNonSilent(ch); }
		}
		mixer()->mixAuxiliaryToChannel(buffer, drums);

		// Only through the send from the channels 3/4 to the toms channel, which goes to the master
		compareOutput(mix(), 0.25f, 0.25f);

		// A channel without the auxiliary pair ignores it
		mixer()->mixAuxiliaryToChannel(buffer, toms);
		compareOutput(mix(), 0.f, 0.f);
	}

	void clearingAChannelResetsItsChannelCount()
	{
		const auto channel = mixer()->createChannel();
		mixer()->setChannelCount(channel, 8);
		mixer()->clearChannel(channel);
		QCOMPARE(mixer()->mixerChannel(channel)->channelCount(), 2);
	}

	void channelCountChangeIsUndoable()
	{
		const auto channel = mixer()->createChannel();
		const auto journal = Engine::projectJournal();
		journal->setJournalling(true);
		journal->clearJournal();

		mixer()->setChannelCount(channel, 4);
		QCOMPARE(mixer()->mixerChannel(channel)->channelCount(), 4);
		journal->undo();
		QCOMPARE(mixer()->mixerChannel(channel)->channelCount(), 2);
		journal->redo();
		QCOMPARE(mixer()->mixerChannel(channel)->channelCount(), 4);

		journal->setJournalling(false);
	}

	/////////////////////////////////////
	// A mixer channel for each track  //
	/////////////////////////////////////

	void newTrackGetsItsOwnChannel()
	{
		auto track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
		QVERIFY(track);
		const auto index = track->mixerChannelModel()->value();
		QVERIFY(index > 0);
		const auto channel = mixer()->mixerChannel(index);
		QVERIFY(channel->m_trackChannel);
		QVERIFY(mixer()->channelSendModel(index, 0) != nullptr);

		// The channel follows the track's name and color
		track->setName("Bass");
		QCOMPARE(channel->m_name, QString{"Bass"});
		track->setColor(QColor{Qt::red});
		QCOMPARE(channel->color(), std::optional<QColor>{QColor{Qt::red}});

		// And goes away with the track
		mixer()->releaseTrackChannel(track);
		QCOMPARE(mixer()->numChannels(), 1 + Mixer::FxReturnCount);
		QCOMPARE(track->mixerChannelModel()->value(), 0);
		delete track;
	}

	void trackStripMovesIntoItsChannel()
	{
		// Like a track of a project from before: on the master, with its own volume, panning and an effect
		auto track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
		mixer()->releaseTrackChannel(track);
		QCOMPARE(track->mixerChannelModel()->value(), 0);
		track->volumeModel()->setValue(50.f);
		track->panningModel()->setValue(-40.f);
		track->audioBusHandle()->effects()->appendEffect(
			std::make_unique<PluginInstance>(std::make_unique<FakeDucker>(), Plugin::Type::Effect));

		const auto index = mixer()->setUpTrackChannel(track);
		QVERIFY(index > 0);
		QCOMPARE(track->mixerChannelModel()->value(), index);

		const auto channel = mixer()->mixerChannel(index);
		// The volume came before the effects, so it becomes the input trim, which does too
		QVERIFY(std::abs(channel->m_trimModel.value() - ampToDbfs(0.5f)) < 0.001f);
		QCOMPARE(channel->m_volumeModel.value(), 1.f);
		QVERIFY(qFuzzyCompare(channel->m_panModel.value(), -40.f));
		QCOMPARE(track->volumeModel()->value(), static_cast<float>(DefaultVolume));
		QCOMPARE(track->panningModel()->value(), static_cast<float>(DefaultPanning));

		// The effect is the channel's now (a DummyEffect, since the fake effect cannot be loaded by name)
		QVERIFY(track->audioBusHandle()->effects()->effects().empty());
		QCOMPARE(channel->m_fxChain.effects().size(), std::size_t{1});

		// Setting up again changes nothing
		QCOMPARE(mixer()->setUpTrackChannel(track), index);
		QCOMPARE(channel->m_fxChain.effects().size(), std::size_t{1});

		channel->m_fxChain.clear();
		delete track;
	}

	void trackOnASharedChannelGetsItsOwnSendingThere()
	{
		const auto bus = mixer()->createChannel();
		auto a = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, Engine::getSong()));
		auto b = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, Engine::getSong()));
		mixer()->releaseTrackChannel(a);
		mixer()->releaseTrackChannel(b);
		a->mixerChannelModel()->setValue(bus);
		b->mixerChannelModel()->setValue(bus);

		mixer()->setUpTrackChannels();
		const auto ownA = a->mixerChannelModel()->value();
		const auto ownB = b->mixerChannelModel()->value();
		QVERIFY(ownA != bus && ownB != bus && ownA != ownB);

		// Both still reach the bus, and through it the master
		QVERIFY(mixer()->channelSendModel(ownA, bus) != nullptr);
		QVERIFY(mixer()->channelSendModel(ownB, bus) != nullptr);
		QVERIFY(mixer()->channelSendModel(ownA, 0) == nullptr);

		delete a;
		delete b;
	}

	void releasingKeepsAChannelOthersUse()
	{
		auto track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, Engine::getSong()));
		const auto index = track->mixerChannelModel()->value();
		const auto other = mixer()->createChannel();
		mixer()->createChannelSend(other, index);

		mixer()->releaseTrackChannel(track);
		QCOMPARE(track->mixerChannelModel()->value(), index);
		QVERIFY(index < mixer()->numChannels());
		delete track;
	}

	void automatedTrackVolumeStaysOnTheTrack()
	{
		auto track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
		mixer()->releaseTrackChannel(track);
		track->volumeModel()->setValue(50.f);
		track->panningModel()->setValue(30.f);

		auto automationTrack = AutomationTrack{Engine::getSong()};
		auto clip = AutomationClip{&automationTrack};
		clip.addObject(track->volumeModel());

		const auto index = mixer()->setUpTrackChannel(track);
		const auto channel = mixer()->mixerChannel(index);
		QCOMPARE(mixer()->trackChannel(track), channel);

		// The automation acts on the track's volume, so it stays there; the panning moves
		QCOMPARE(track->volumeModel()->value(), 50.f);
		QCOMPARE(channel->m_trimModel.value(), 0.f);
		QVERIFY(qFuzzyCompare(channel->m_panModel.value(), 30.f));
		QCOMPARE(track->panningModel()->value(), static_cast<float>(DefaultPanning));

		// A track sending to a channel made for another track has no channel of its own
		auto other = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
		mixer()->releaseTrackChannel(other);
		other->mixerChannelModel()->setValue(index);
		QCOMPARE(mixer()->trackChannel(other), static_cast<MixerChannel*>(nullptr));

		other->mixerChannelModel()->setValue(0);
		delete other;
		mixer()->releaseTrackChannel(track);
		QCOMPARE(mixer()->trackChannel(track), static_cast<MixerChannel*>(nullptr));
		delete track;
	}

	//! Adds an effect delaying by @p delay frames, with that latency, to channel @p channel
	static auto addLatentEffect(mix_ch_t channel, f_cnt_t delay) -> PluginInstance*
	{
		auto& chain = mixer()->mixerChannel(channel)->m_fxChain;
		chain.appendEffect(std::make_unique<PluginInstance>(std::make_unique<FakeDelay>(delay), Plugin::Type::Effect));
		return chain.effects().back().get();
	}

	//! Expects @p value in the left channel of frames [@p from, @p to) and 0 elsewhere
	static void compareStep(const std::vector<SampleFrame>& out, f_cnt_t from, f_cnt_t to, float value)
	{
		for (f_cnt_t f = 0; f < out.size(); ++f)
		{
			const float expected = f >= from && f < to ? value : 0.f;
			if (std::abs(out[f].left() - expected) > 1e-6f)
			{
				QFAIL(qPrintable(QString{"frame %1: %2 instead of %3"}.arg(f).arg(out[f].left()).arg(expected)));
			}
		}
	}

	void latentEffectDelaysTheOtherPaths()
	{
		const f_cnt_t delay = 100;
		const auto latent = mixer()->createChannel();
		const auto plain = mixer()->createChannel();
		addLatentEffect(latent, delay);

		// One period of signal into both channels: they come out together, after the latency
		feed(latent, 0.5f, 0.5f);
		feed(plain, 0.25f, 0.25f);
		auto out = mix();
		QCOMPARE(mixer()->latency(), delay);
		compareStep(out, delay, frames(), 0.75f);
		compareStep(mix(), 0, delay, 0.75f);
		compareStep(mix(), 0, 0, 0.f);

		mixer()->mixerChannel(latent)->m_fxChain.clear();
	}

	void latencyOfASenderDelaysTheReceiversTracks()
	{
		const f_cnt_t delay = 64;
		const auto latent = mixer()->createChannel();
		const auto bus = mixer()->createChannel();
		mixer()->deleteChannelSend(latent, 0);
		mixer()->createChannelSend(latent, bus);
		addLatentEffect(latent, delay);

		// The bus's own audio (from tracks) waits for what the latent channel sends
		feed(latent, 0.5f, 0.5f);
		feed(bus, 0.125f, 0.125f);
		compareStep(mix(), delay, frames(), 0.625f);
		compareStep(mix(), 0, delay, 0.625f);
		QCOMPARE(mixer()->latency(), delay);

		mixer()->mixerChannel(latent)->m_fxChain.clear();
	}

	void drySignalOfALatentEffectIsDelayed()
	{
		const f_cnt_t delay = 40;
		const auto latent = mixer()->createChannel();
		const auto effect = addLatentEffect(latent, delay);
		effect->effect()->wetDryModel()->setValue(0.5f);

		// Half dry, half wet, both after the latency
		feed(latent, 0.5f, 0.5f);
		compareStep(mix(), delay, frames(), 0.5f);
		compareStep(mix(), 0, delay, 0.5f);

		mixer()->mixerChannel(latent)->m_fxChain.clear();
	}

	void latentTrackDelaysTheOtherPaths()
	{
		const f_cnt_t delay = 48;
		const auto latent = mixer()->createChannel();
		const auto plain = mixer()->createChannel();
		auto channel = IntModel{latent, 0, 100};
		auto latency = std::atomic<f_cnt_t>{delay};
		mixer()->addLatencySource(&channel, &latency);

		// The latent track's audio comes late already: the plain channel's audio waits for it
		feed(plain, 0.25f, 0.25f);
		compareStep(mix(), delay, frames(), 0.25f);
		QCOMPARE(mixer()->mixerChannel(latent)->m_latency, delay);
		QCOMPARE(mixer()->latency(), delay);
		compareStep(mix(), 0, delay, 0.25f);

		// The latent channel's own tracks are not delayed again
		feed(latent, 0.5f, 0.5f);
		compareStep(mix(), 0, frames(), 0.5f);

		// The source follows its channel, and goes away
		channel.setValue(plain);
		mix();
		QCOMPARE(mixer()->mixerChannel(latent)->m_latency, f_cnt_t{0});
		QCOMPARE(mixer()->mixerChannel(plain)->m_latency, delay);
		mixer()->removeLatencySource(&latency);
		mix();
		QCOMPARE(mixer()->latency(), f_cnt_t{0});
	}

	void disabledEffectHasNoLatency()
	{
		const auto latent = mixer()->createChannel();
		const auto effect = addLatentEffect(latent, 32);
		mix();
		QCOMPARE(mixer()->latency(), f_cnt_t{32});

		effect->effect()->enabledModel()->setValue(false);
		feed(latent, 0.5f, 0.5f);
		compareStep(mix(), 0, frames(), 0.5f);
		QCOMPARE(mixer()->latency(), f_cnt_t{0});

		mixer()->mixerChannel(latent)->m_fxChain.clear();
	}

	void latencyAddsUpAlongTheWay()
	{
		const auto first = mixer()->createChannel();
		const auto second = mixer()->createChannel();
		mixer()->deleteChannelSend(first, 0);
		mixer()->createChannelSend(first, second);
		addLatentEffect(first, 10);
		addLatentEffect(second, 20);
		addLatentEffect(0, 5);
		mix();
		QCOMPARE(mixer()->mixerChannel(first)->m_latency, f_cnt_t{10});
		QCOMPARE(mixer()->mixerChannel(second)->m_latency, f_cnt_t{30});
		QCOMPARE(mixer()->latency(), f_cnt_t{35});

		mixer()->mixerChannel(0)->m_fxChain.clear();
		mixer()->mixerChannel(first)->m_fxChain.clear();
		mixer()->mixerChannel(second)->m_fxChain.clear();
	}

	//! The channel @p track sends its main output to, or -1 if none or more than one
	static auto outputOf(const Track* track) -> int
	{
		const auto channel = mixer()->trackChannel(track);
		if (!channel) { return -1; }
		int output = -1;
		for (const MixerRoute* route : channel->m_sends)
		{
			if (route->sourcePair() != 0 || route->destinationPair() != 0 || route->receiver()->m_fxReturn) { continue; }
			if (output != -1) { return -1; }
			output = route->receiverIndex();
		}
		return output;
	}

	//! Where channel @p channel sends its main output (not counting the effect returns), or -1
	static auto mainOutputOf(const MixerChannel* channel) -> int
	{
		for (const MixerRoute* route : channel->m_sends)
		{
			if (route->sourcePair() == 0 && route->destinationPair() == 0 && !route->receiver()->m_fxReturn)
			{
				return route->receiverIndex();
			}
		}
		return -1;
	}

	static void deleteTrack(Track* track)
	{
		mixer()->releaseTrackChannel(track);
		delete track;
	}

	void folderRoutesItsTracksToItsGroupChannel()
	{
		const auto song = Engine::getSong();
		auto folder = dynamic_cast<FolderTrack*>(Track::create(Track::Type::Folder, song));
		QVERIFY(folder);
		auto track = Track::create(Track::Type::Instrument, song);
		const auto group = mixer()->trackChannel(folder);
		QVERIFY(group);
		QVERIFY(group->m_groupChannel);
		QCOMPARE(outputOf(track), 0);

		// Into the folder: the track's channel sends to the group channel, which sends to the master
		song->changeFolderDepth(track, 1);
		QCOMPARE(track->folderDepth(), 1);
		QCOMPARE(song->folderOf(track), folder);
		QCOMPARE(outputOf(track), group->index());
		QCOMPARE(outputOf(folder), 0);

		// The group channel's fader acts on the track's sound
		const auto trackChannel = mixer()->trackChannel(track)->index();
		group->m_volumeModel.setValue(0.5f);
		settle();
		feed(trackChannel, 0.8f, 0.4f);
		compareOutput(mix(), 0.4f, 0.2f);

		// Muting the folder (its group channel) mutes what it holds
		group->m_muteModel.setValue(true);
		QVERIFY(track->isMutedInMix() == false);
		feed(trackChannel, 0.8f, 0.4f);
		compareOutput(mix(), 0.f, 0.f);
		group->m_muteModel.setValue(false);

		// Out of the folder again
		song->changeFolderDepth(track, -1);
		QCOMPARE(track->folderDepth(), 0);
		QCOMPARE(outputOf(track), 0);

		deleteTrack(track);
		deleteTrack(folder);
	}

	void foldersNest()
	{
		const auto song = Engine::getSong();
		auto outer = Track::create(Track::Type::Folder, song);
		auto inner = Track::create(Track::Type::Folder, song);
		auto track = Track::create(Track::Type::Sample, song);
		song->changeFolderDepth(inner, 1);
		song->changeFolderDepth(track, 1);
		song->changeFolderDepth(track, 1);
		QCOMPARE(track->folderDepth(), 2);
		QCOMPARE(song->folderOf(track), inner);
		QCOMPARE(song->folderOf(inner), outer);
		QVERIFY(song->isInFolder(track, outer));
		QCOMPARE(outputOf(track), mixer()->trackChannel(inner)->index());
		QCOMPARE(outputOf(inner), mixer()->trackChannel(outer)->index());

		// Moving a folder out moves what it holds along
		song->changeFolderDepth(inner, -1);
		QCOMPARE(inner->folderDepth(), 0);
		QCOMPARE(track->folderDepth(), 1);
		QCOMPARE(song->folderOf(track), inner);
		QCOMPARE(outputOf(inner), 0);

		// Deleting a folder moves its tracks up a level, to the master here
		deleteTrack(inner);
		QCOMPARE(track->folderDepth(), 0);
		QCOMPARE(outputOf(track), 0);

		deleteTrack(track);
		deleteTrack(outer);
	}

	void soloInAFolder()
	{
		const auto song = Engine::getSong();
		auto folder = Track::create(Track::Type::Folder, song);
		auto soloed = Track::create(Track::Type::Instrument, song);
		auto sibling = Track::create(Track::Type::Instrument, song);
		auto outside = Track::create(Track::Type::Instrument, song);
		song->changeFolderDepth(soloed, 1);
		song->changeFolderDepth(sibling, 1);
		const auto soloedChannel = mixer()->trackChannel(soloed)->index();
		const auto siblingChannel = mixer()->trackChannel(sibling)->index();
		const auto outsideChannel = mixer()->trackChannel(outside)->index();
		settle();
		auto play = [&] {
			feed(soloedChannel, 0.125f, 0.125f);
			feed(siblingChannel, 0.25f, 0.25f);
			feed(outsideChannel, 0.5f, 0.5f);
			return mix();
		};

		// Soloing a track keeps its folder's group channel passing it
		mixer()->trackChannel(soloed)->m_soloModel.setValue(true);
		compareOutput(play(), 0.125f, 0.125f);
		mixer()->trackChannel(soloed)->m_soloModel.setValue(false);

		// Soloing a folder keeps what it holds playing
		mixer()->trackChannel(folder)->m_soloModel.setValue(true);
		compareOutput(play(), 0.375f, 0.375f);
		mixer()->trackChannel(folder)->m_soloModel.setValue(false);
		compareOutput(play(), 0.875f, 0.875f);

		deleteTrack(outside);
		deleteTrack(sibling);
		deleteTrack(soloed);
		deleteTrack(folder);
	}

	void trackMuteAndSoloMoveToTheChannel()
	{
		// As in a project from before: a muted track, and a soloed one (LMMS muted the others for it)
		const auto song = Engine::getSong();
		auto doc = DataFile{DataFile::Type::SongProject};
		auto element = doc.createElement("track");
		element.setAttribute("muted", 1);
		element.setAttribute("mutedBeforeSolo", 1);
		element.setAttribute("type", static_cast<int>(Track::Type::Sample));
		const auto wasMuted = Track::create(element, song);
		element.setAttribute("solo", 1);
		element.setAttribute("muted", 0);
		element.setAttribute("mutedBeforeSolo", 0);
		const auto soloed = Track::create(element, song);

		song->resolveLegacySolo();
		mixer()->setUpTrackChannels();
		QVERIFY(!wasMuted->isMuted());
		QVERIFY(mixer()->trackChannel(wasMuted)->m_muteModel.value());
		QVERIFY(wasMuted->isMutedInMix());
		QVERIFY(mixer()->trackChannel(soloed)->m_soloModel.value());
		QVERIFY(!soloed->legacySolo());

		mixer()->trackChannel(soloed)->m_soloModel.setValue(false);
		deleteTrack(soloed);
		deleteTrack(wasMuted);
	}

	void folderChangeIsOneUndoStep()
	{
		const auto song = Engine::getSong();
		auto folder = Track::create(Track::Type::Folder, song);
		auto inner = Track::create(Track::Type::Folder, song);
		auto track = Track::create(Track::Type::Instrument, song);
		song->changeFolderDepth(inner, 1);
		song->changeFolderDepth(track, 1);
		song->changeFolderDepth(track, 1);

		const auto journal = Engine::projectJournal();
		journal->setJournalling(true);
		song->changeFolderDepth(inner, -1);
		QCOMPARE(track->folderDepth(), 1);
		journal->undo();
		journal->setJournalling(false);

		// The folder and what it holds are back, and so is the routing
		QCOMPARE(inner->folderDepth(), 1);
		QCOMPARE(track->folderDepth(), 2);
		QCOMPARE(outputOf(inner), mixer()->trackChannel(folder)->index());

		deleteTrack(track);
		deleteTrack(inner);
		deleteTrack(folder);
	}

	void renameAndColorAreUndoable()
	{
		const auto bus = mixer()->createChannel();
		const auto journal = Engine::projectJournal();
		journal->setJournalling(true);
		mixer()->setChannelName(bus, "Reverb");
		mixer()->setChannelColor(bus, QColor{Qt::red});
		QCOMPARE(mixer()->mixerChannel(bus)->m_name, QString{"Reverb"});
		journal->undo();
		QVERIFY(!mixer()->mixerChannel(bus)->color());
		journal->undo();
		journal->setJournalling(false);
		QCOMPARE(mixer()->mixerChannel(bus)->m_name, QString{"Channel %1"}.arg(bus));

		// A track's channel renames its track, which the channel follows
		const auto track = Track::create(Track::Type::Sample, Engine::getSong());
		const auto own = mixer()->trackChannel(track);
		mixer()->setChannelName(own->index(), "Vocals");
		QCOMPARE(track->name(), QString{"Vocals"});
		QCOMPARE(own->m_name, QString{"Vocals"});
		deleteTrack(track);
	}

	void tracksMoveWithTheirFolders()
	{
		const auto song = Engine::getSong();
		const auto folder = dynamic_cast<FolderTrack*>(Track::create(Track::Type::Folder, song));
		const auto a = Track::create(Track::Type::Sample, song);
		const auto b = Track::create(Track::Type::Sample, song);
		const auto x = Track::create(Track::Type::Sample, song);
		song->changeFolderDepth(a, 1);
		song->changeFolderDepth(b, 1);
		auto order = [song] { return TrackContainer::TrackList(song->tracks().end() - 4, song->tracks().end()); };
		auto depths = [&] {
			auto result = std::vector<int>{};
			for (const auto track : song->tracks()) { result.push_back(track->folderDepth()); }
			return std::vector<int>(result.end() - 4, result.end());
		};

		// Up from below the folder: into it, at the level of the track above
		QVERIFY(song->moveTrackStep(x, false));
		QCOMPARE(order(), (TrackContainer::TrackList{folder, a, x, b}));
		QCOMPARE(depths(), (std::vector<int>{0, 1, 1, 1}));
		QCOMPARE(outputOf(x), mixer()->trackChannel(folder)->index());

		// Out of the folder: below what it holds, so that b stays in it
		song->changeFolderDepth(x, -1);
		QCOMPARE(order(), (TrackContainer::TrackList{folder, a, b, x}));
		QCOMPARE(depths(), (std::vector<int>{0, 1, 1, 0}));

		// A folder moves with what it holds, and a collapsed one is passed at once
		QVERIFY(song->moveTrackStep(x, false));
		QVERIFY(song->moveTrackStep(x, false));
		QVERIFY(song->moveTrackStep(x, false));
		QCOMPARE(order().front(), x);
		QCOMPARE(x->folderDepth(), 0);
		QVERIFY(song->moveTrackStep(folder, false));
		QCOMPARE(song->tracks().back(), x);
		QCOMPARE(song->tracks()[song->tracks().size() - 2], b);
		folder->setCollapsed(true);
		QVERIFY(song->moveTrackStep(x, false));
		QCOMPARE(order(), (TrackContainer::TrackList{x, folder, a, b}));
		QCOMPARE(depths(), (std::vector<int>{0, 0, 1, 1}));
		folder->setCollapsed(false);

		// The order is one undo step with the depths
		const auto journal = Engine::projectJournal();
		journal->setJournalling(true);
		song->addLayoutCheckPoint();
		QVERIFY(song->moveTrackStep(x, true));
		QCOMPARE(order(), (TrackContainer::TrackList{folder, x, a, b}));
		QCOMPARE(x->folderDepth(), 1);
		journal->undo();
		journal->setJournalling(false);
		QCOMPARE(order(), (TrackContainer::TrackList{x, folder, a, b}));
		QCOMPARE(x->folderDepth(), 0);

		deleteTrack(x);
		deleteTrack(b);
		deleteTrack(a);
		deleteTrack(folder);
	}

	void fxReturnsTakeTheChannelsSends()
	{
		const auto returns = mixer()->fxReturns();
		QCOMPARE(static_cast<int>(returns.size()), Mixer::FxReturnCount);
		QVERIFY(mixer()->isChannelInUse(returns[0]->index()));

		// Every channel sends to them, with nothing at first
		const auto channel = mixer()->createChannel();
		const auto send = mixer()->fxSend(channel, 0);
		QVERIFY(send);
		QCOMPARE(send->amount()->value(), 0.f);
		QVERIFY(mixer()->fxSend(channel, 1));
		settle();
		feed(channel, 0.5f, 0.5f);
		compareOutput(mix(), 0.5f, 0.5f);

		// Turned up, the effect return adds the channel's signal after its fader
		send->amount()->setValue(0.5f);
		mixer()->mixerChannel(channel)->m_volumeModel.setValue(0.5f);
		settle();
		feed(channel, 0.5f, 0.5f);
		compareOutput(mix(), 0.25f + 0.125f, 0.25f + 0.125f);

		// Saving and loading keeps them, and projects from before get them
		auto doc = DataFile{DataFile::Type::SongProject};
		auto element = doc.createElement("mixer");
		doc.content().appendChild(element);
		mixer()->saveSettings(doc, element);
		const auto channels = mixer()->numChannels();
		mixer()->loadSettings(element);
		QCOMPARE(mixer()->numChannels(), channels);
		QCOMPARE(mixer()->fxSend(channel, 0)->amount()->value(), 0.5f);
		mixer()->clear();
		mixer()->loadSettings(doc.createElement("mixer"));
		QCOMPARE(static_cast<int>(mixer()->fxReturns().size()), Mixer::FxReturnCount);
	}

	void busCompressorSqueezesTheMix()
	{
		auto& master = mixer()->masterSection();
		const auto channel = mixer()->createChannel();
		settle();

		// Out by default: untouched
		QVERIFY(std::abs(sineGain(channel, 1000.f, 0.5f) - 1.f) < 0.01f);
		QCOMPARE(master.takeGainReduction(), 0.f);

		master.m_compressorModel.setValue(true);
		master.m_thresholdModel.setValue(-20.f);
		master.m_ratioModel.setValue(static_cast<int>(MasterSection::Ratio::Ten));
		const float loud = sineGain(channel, 1000.f, 0.5f);
		QVERIFY(loud < 0.5f);
		QVERIFY(master.takeGainReduction() < -6.f);
		// Below the threshold, untouched (no make-up) once released
		master.m_autoReleaseModel.setValue(false);
		master.m_releaseModel.setValue(0.1f);
		QVERIFY(std::abs(sineGain(channel, 1000.f, 0.01f, 512) - 1.f) < 0.02f);
		master.setDefaults();
	}

	void correlationMeterFollowsThePhase()
	{
		const auto channel = mixer()->createChannel();
		settle();
		auto play = [&](float left, float right) {
			for (int i = 0; i < 512; ++i)
			{
				feed(channel, left * (i % 2 ? 1.f : -1.f), right * (i % 2 ? 1.f : -1.f));
				mix();
			}
			return mixer()->masterSection().correlation();
		};
		QVERIFY(play(0.5f, 0.5f) > 0.99f);
		QVERIFY(play(0.5f, -0.5f) < -0.99f);
	}

	void monitoringChangesOnlyWhatIsHeard()
	{
		auto& master = mixer()->masterSection();
		const auto channel = mixer()->createChannel();
		settle();

		// DIM, once ramped
		master.m_dimModel.setValue(true);
		feed(channel, 0.5f, 0.25f);
		mix();
		feed(channel, 0.5f, 0.25f);
		compareOutput(mix(), 0.05f, 0.025f);
		master.m_dimModel.setValue(false);

		// MONO
		master.m_monoModel.setValue(true);
		feed(channel, 0.5f, 0.25f);
		mix();
		feed(channel, 0.5f, 0.25f);
		compareOutput(mix(), 0.375f, 0.375f);
		master.m_monoModel.setValue(false);

		// CUT
		master.m_cutModel.setValue(true);
		feed(channel, 0.5f, 0.25f);
		mix();
		feed(channel, 0.5f, 0.25f);
		compareOutput(mix(), 0.f, 0.f);
		// The meters show the mix, not what is heard
		QVERIFY(mixer()->mixerChannel(0)->m_meter.rms(0) > 0.1f);
		master.m_cutModel.setValue(false);
		mix();
	}

	/**
	 * Plays sines of @p frequency into channel @p channel, @p left and @p right their amplitudes (a negative one is
	 * the sine inverted), and returns the RMS levels of the master's mid and side over the last quarter of the
	 * periods, and the biggest step between two frames of the left channel
	 */
	static auto playStereo(mix_ch_t channel, float frequency, float left, float right, int periods = 128,
		const std::function<void(int)>& atPeriod = {}) -> StereoLevels
	{
		const auto rate = static_cast<float>(Engine::audioEngine()->outputSampleRate());
		double mid = 0., side = 0.;
		float step = 0.f, previous = 0.f;
		f_cnt_t measured = 0;
		f_cnt_t position = 0;
		for (int period = 0; period < periods; ++period)
		{
			if (atPeriod) { atPeriod(period); }
			auto buffer = AudioBuffer{frames()};
			for (f_cnt_t f = 0; f < frames(); ++f, ++position)
			{
				const float x = std::sin(2.f * std::numbers::pi_v<float> * frequency * position / rate);
				buffer.buffer(0)[f] = left * x;
				buffer.buffer(1)[f] = right * x;
			}
			buffer.assumeNonSilent(0);
			buffer.assumeNonSilent(1);
			mixer()->mixToChannel(buffer, channel);
			const auto out = mix();
			for (const auto& frame : out)
			{
				if (period > 4) { step = std::max(step, std::abs(frame.left() - previous)); }
				previous = frame.left();
			}
			if (period < periods * 3 / 4) { continue; }
			for (const auto& frame : out)
			{
				const double m = (frame.left() + frame.right()) / 2., sd = (frame.left() - frame.right()) / 2.;
				mid += m * m;
				side += sd * sd;
			}
			measured += out.size();
		}
		return {static_cast<float>(std::sqrt(mid / measured)), static_cast<float>(std::sqrt(side / measured)), step};
	}

	void masterWidthIsAppliedBeforeTheBusCompressor()
	{
		const auto channel = mixer()->createChannel();
		mixer()->mixerChannel(0)->m_widthModel.setValue(0.f);
		settle();
		feed(channel, 0.5f, 0.1f);
		mix();
		feed(channel, 0.5f, 0.1f);
		compareOutput(mix(), 0.3f, 0.3f);
		mixer()->mixerChannel(0)->m_widthModel.setValue(100.f);
		mix();
	}

	void monoBassNarrowsOnlyTheLows()
	{
		auto& master = mixer()->masterSection();
		const auto channel = mixer()->createChannel();
		settle();
		// Left only: mid and side of 0.5 * 0.5 / sqrt(2) RMS each
		const float level = 0.25f / std::sqrt(2.f);
		master.m_bassFrequencyModel.setValue(120.f);
		master.m_bassWidthModel.setValue(0.f);

		const auto low = playStereo(channel, 40.f, 0.5f, 0.f, 256);
		QVERIFY2(low.side < level * 0.05f, qPrintable(QString::number(low.side / level)));
		QVERIFY(std::abs(low.mid / level - 1.f) < 0.02f); // the mid's allpass keeps its level

		const auto high = playStereo(channel, 2000.f, 0.5f, 0.f);
		QVERIFY(std::abs(high.side / level - 1.f) < 0.02f);
		QVERIFY(std::abs(high.mid / level - 1.f) < 0.02f);

		master.setDefaults();
		mix();
	}

	void monoBassSwitchesAndSweepsWithoutClicks()
	{
		auto& master = mixer()->masterSection();
		const auto channel = mixer()->createChannel();
		settle();
		// A 100 Hz sine moves at most 2 pi 100 / 44100 * 0.5 = 0.0071 per frame; a click would be a bigger step
		const auto levels = playStereo(channel, 100.f, 0.5f, -0.2f, 96, [&master](int period) {
			if (period == 20) { master.m_bassWidthModel.setValue(0.f); }
			if (period == 40) { master.m_bassFrequencyModel.setValue(250.f); }
			if (period == 60) { master.m_bassFrequencyModel.setValue(60.f); }
			if (period == 80) { master.m_bassWidthModel.setValue(100.f); }
		});
		QVERIFY2(levels.biggestStep < 0.0075f, qPrintable(QString::number(levels.biggestStep)));
		master.setDefaults();
		mix();
	}

	void airBoostsOnlyTheSidesHighs()
	{
		auto& master = mixer()->masterSection();
		const auto channel = mixer()->createChannel();
		settle();
		const float level = 0.25f / std::sqrt(2.f);
		master.m_airModel.setValue(6.f);
		const auto high = playStereo(channel, 16000.f, 0.5f, 0.f);
		QVERIFY(high.side / level > dbfsToAmp(4.f));
		QVERIFY(std::abs(high.mid / level - 1.f) < 0.02f);
		const auto low = playStereo(channel, 500.f, 0.5f, 0.f);
		QVERIFY(std::abs(low.side / level - 1.f) < 0.05f);
		master.setDefaults();
		mix();
	}

	void preFadersMoveTogetherWhileLinked()
	{
		auto& master = mixer()->masterSection();
		const auto channel = mixer()->createChannel();
		QVERIFY(master.m_preLinkModel.value());
		master.m_preLeftModel.setValue(0.5f);
		QCOMPARE(master.m_preRightModel.value(), 0.5f);

		master.m_preLinkModel.setValue(false);
		master.m_preRightModel.setValue(1.f);
		QCOMPARE(master.m_preLeftModel.value(), 0.5f);
		feed(channel, 0.5f, 0.5f);
		mix();
		feed(channel, 0.5f, 0.5f);
		compareOutput(mix(), 0.25f, 0.5f);

		// Linking again puts the right one where the left one is
		master.m_preLinkModel.setValue(true);
		QCOMPARE(master.m_preRightModel.value(), 0.5f);
		master.setDefaults();
		mix();
	}

	void monitorSideAndGoniometer()
	{
		auto& master = mixer()->masterSection();
		const auto channel = mixer()->createChannel();
		settle();
		master.m_sideModel.setValue(true);
		feed(channel, 0.5f, 0.25f);
		mix();
		feed(channel, 0.5f, 0.25f);
		compareOutput(mix(), 0.125f, 0.125f);
		// MONO and SIDE exclude each other
		master.m_monoModel.setValue(true);
		QVERIFY(!master.m_sideModel.value());
		master.m_monoModel.setValue(false);

		auto points = std::vector<std::array<float, 2>>{};
		master.scopePoints(points);
		QCOMPARE(static_cast<int>(points.size()), MasterSection::ScopePoints);
		QCOMPARE(points.back()[0], 0.125f); // side
		QCOMPARE(points.back()[1], 0.375f); // mid
	}

	void opAmpSaturatesOnlyAboveItsKnee()
	{
		using Strip = AnalogChannelStrip;
		QCOMPARE(Strip::opAmp(1.5f), 1.5f);
		QCOMPARE(Strip::opAmp(-7.f), -7.f);
		const float pushed = Strip::opAmp(12.f);
		QVERIFY(pushed > Strip::OpAmpKnee && pushed < 12.f);
		QVERIFY(Strip::opAmp(100.f) <= Strip::OpAmpRail);
		// The negative rail is a little lower: even harmonics
		QVERIFY(-Strip::opAmp(-100.f) < Strip::opAmp(100.f));
	}

	void tubeIsUnityForSmallSignalsAndColorsLoudOnes()
	{
		using Strip = AnalogChannelStrip;
		QVERIFY(std::abs(Strip::tube(0.001f) - 0.001f) < 2e-5f);
		QVERIFY(std::abs(Strip::tube(-0.001f) + 0.001f) < 2e-5f);
		// Asymmetric (even harmonics), and limited when driven hard
		QVERIFY(std::abs(Strip::tube(2.f) + Strip::tube(-2.f)) > 0.01f);
		QVERIFY(std::abs(Strip::tube(40.f)) < 20.f);
		QVERIFY(std::abs(Strip::tube(-40.f)) < 20.f);
	}

	//! The magnitude of @p frequency in the left channel of @p out (Goertzel)
	static auto magnitudeAt(const std::vector<SampleFrame>& out, float frequency) -> float
	{
		const auto rate = static_cast<float>(Engine::audioEngine()->outputSampleRate());
		const float coefficient = 2.f * std::cos(2.f * std::numbers::pi_v<float> * frequency / rate);
		float s1 = 0.f, s2 = 0.f;
		for (const auto& frame : out)
		{
			const float s0 = frame.left() + coefficient * s1 - s2;
			s2 = s1;
			s1 = s0;
		}
		return std::sqrt(s1 * s1 + s2 * s2 - coefficient * s1 * s2) / static_cast<float>(out.size());
	}

	void drivenOpAmpDoesNotAlias()
	{
		// A 15 kHz sine driven into the op-amp's rails: its 3rd harmonic (45 kHz) would fold back to 900 Hz at
		// 44.1 kHz without the oversampling
		const auto rate = static_cast<float>(Engine::audioEngine()->outputSampleRate());
		if (rate != 44100.f) { QSKIP("Needs a sample rate of 44.1 kHz"); }
		const auto channel = mixer()->createChannel();
		mixer()->mixerChannel(channel)->m_volumeModel.setValue(1.f / 16.f);
		settle();
		auto out = std::vector<SampleFrame>{};
		f_cnt_t position = 0;
		for (int period = 0; period < 64; ++period)
		{
			auto buffer = AudioBuffer{frames()};
			for (f_cnt_t f = 0; f < frames(); ++f, ++position)
			{
				const float x = 20.f * std::sin(2.f * std::numbers::pi_v<float> * 15000.f * position / rate);
				buffer.buffer(0)[f] = x;
				buffer.buffer(1)[f] = x;
			}
			buffer.assumeNonSilent(0);
			buffer.assumeNonSilent(1);
			mixer()->mixToChannel(buffer, channel);
			const auto period_out = mix();
			if (period >= 16) { out.insert(out.end(), period_out.begin(), period_out.end()); }
		}
		const float fundamental = magnitudeAt(out, 15000.f);
		const float alias = magnitudeAt(out, 900.f);
		QVERIFY(fundamental > 0.1f);
		QVERIFY2(alias < fundamental * dbfsToAmp(-50.f), qPrintable(QString{"alias %1 dB below the fundamental"}
			.arg(ampToDbfs(fundamental / alias))));
	}

	void tubeStageColorsTheChannel()
	{
		const auto channel = mixer()->createChannel();
		auto& strip = mixer()->mixerChannel(channel)->m_analogStrip;
		settle();
		strip.m_tubeModel.setValue(true);
		// Unity for quiet signals, compressed when loud
		QVERIFY(std::abs(sineGain(channel, 1000.f, 0.01f) - 1.f) < 0.02f);
		QVERIFY(sineGain(channel, 1000.f, 8.f) < 0.9f);
		strip.setNeutral();
	}

	void analogStripFiltersAndEq()
	{
		QVERIFY(mixer()->isAnalog());
		const auto channel = mixer()->createChannel();
		auto& strip = mixer()->mixerChannel(channel)->m_analogStrip;
		settle();

		// Neutral: untouched
		QVERIFY(std::abs(sineGain(channel, 1000.f) - 1.f) < 0.01f);

		// The low cut takes the sub-bass away and leaves the mids
		strip.m_lowCutModel.setValue(true);
		QVERIFY(sineGain(channel, 30.f) < 0.15f);
		QVERIFY(std::abs(sineGain(channel, 1000.f) - 1.f) < 0.02f);
		strip.m_lowCutModel.setValue(false);

		// High-pass and low-pass
		strip.m_highPassModel.setValue(500.f);
		QVERIFY(sineGain(channel, 60.f) < 0.05f);
		strip.m_highPassModel.setValue(strip.m_highPassModel.minValue());
		strip.m_lowPassModel.setValue(2000.f);
		QVERIFY(sineGain(channel, 10000.f) < 0.1f);
		strip.m_lowPassModel.setValue(strip.m_lowPassModel.maxValue());

		// EQ: +12 dB low shelf, -12 dB at the mid frequency
		strip.m_eqLowModel.setValue(12.f);
		QVERIFY(std::abs(sineGain(channel, 30.f) - dbfsToAmp(12.f)) < 0.3f);
		strip.m_eqLowModel.setValue(0.f);
		strip.m_eqHighMidModel.setValue(-12.f);
		QVERIFY(std::abs(sineGain(channel, 2500.f) - dbfsToAmp(-12.f)) < 0.03f);
		strip.m_eqHighMidModel.setValue(0.f);
		strip.m_eqLowMidModel.setValue(9.f);
		QVERIFY(std::abs(sineGain(channel, 400.f) - dbfsToAmp(9.f)) < 0.1f);

		// Digital mode: no built-in processing, and the settings stay
		mixer()->analogModel()->setValue(false);
		QVERIFY(std::abs(sineGain(channel, 1000.f) - 1.f) < 0.01f);
		QCOMPARE(strip.m_eqLowMidModel.value(), 9.f);
		mixer()->analogModel()->setValue(true);

		strip.setNeutral();
	}

	void analogStripDynamics()
	{
		const auto channel = mixer()->createChannel();
		auto& strip = mixer()->mixerChannel(channel)->m_analogStrip;
		settle();

		// The gate closes below its threshold and stays open above
		strip.m_gateModel.setValue(-20.f);
		QVERIFY(sineGain(channel, 1000.f, 0.01f, 256) < 0.01f);
		QVERIFY(std::abs(sineGain(channel, 1000.f, 0.5f) - 1.f) < 0.01f);
		strip.m_gateModel.setValue(strip.m_gateModel.minValue());

		// The compressor squeezes loud signals more than quiet ones (make-up gain included)
		strip.m_compressorModel.setValue(-20.f);
		const float loud = sineGain(channel, 1000.f, 0.9f);
		const float quiet = sineGain(channel, 1000.f, 0.02f);
		QVERIFY(loud < quiet);
		QVERIFY(quiet > 1.f); // below the threshold, only the make-up gain
		strip.setNeutral();
	}

	void analogStripAndModeAreSaved()
	{
		const auto channel = mixer()->createChannel();
		mixer()->mixerChannel(channel)->m_analogStrip.m_eqHighModel.setValue(4.5f);
		mixer()->mixerChannel(channel)->m_analogStrip.m_lowCutModel.setValue(true);
		mixer()->analogModel()->setValue(false);

		auto doc = DataFile{DataFile::Type::SongProject};
		auto element = doc.createElement("mixer");
		doc.content().appendChild(element);
		mixer()->saveSettings(doc, element);
		mixer()->analogModel()->setValue(true);
		mixer()->loadSettings(element);

		QVERIFY(!mixer()->isAnalog());
		QCOMPARE(mixer()->mixerChannel(channel)->m_analogStrip.m_eqHighModel.value(), 4.5f);
		QVERIFY(mixer()->mixerChannel(channel)->m_analogStrip.m_lowCutModel.value());

		// Projects from before the modes are analog, with neutral strips
		element.removeAttribute("analog");
		mixer()->loadSettings(element);
		QVERIFY(mixer()->isAnalog());
		mixer()->mixerChannel(channel)->m_analogStrip.setNeutral();
	}

	void folderDepthIsSaved()
	{
		const auto song = Engine::getSong();
		auto folder = Track::create(Track::Type::Folder, song);
		auto track = Track::create(Track::Type::Sample, song);
		song->changeFolderDepth(track, 1);

		auto doc = QDomDocument{};
		auto parent = doc.createElement("tracks");
		doc.appendChild(parent);
		track->saveState(doc, parent);
		folder->saveState(doc, parent);
		QCOMPARE(parent.firstChildElement().attribute("folderdepth"), QString{"1"});

		// The mixer remembers which channels are group channels
		auto mixerDoc = QDomDocument{};
		auto mixerElement = mixerDoc.createElement("mixer");
		mixerDoc.appendChild(mixerElement);
		mixer()->saveSettings(mixerDoc, mixerElement);
		const auto group = mixer()->trackChannel(folder)->index();
		bool found = false;
		for (auto ch = mixerElement.firstChildElement("mixerchannel"); !ch.isNull(); ch = ch.nextSiblingElement("mixerchannel"))
		{
			if (ch.attribute("num").toInt() == group) { found = ch.attribute("groupchannel") == "1"; }
		}
		QVERIFY(found);

		deleteTrack(track);
		deleteTrack(folder);
	}

	void effectsCanBeInsertedMovedAndReplaced()
	{
		auto& chain = mixer()->mixerChannel(0)->m_fxChain;
		auto a = addLatentEffect(0, 1);
		auto b = addLatentEffect(0, 2);
		chain.insertEffect(1, std::make_unique<PluginInstance>(std::make_unique<FakeDelay>(3), Plugin::Type::Effect));
		QCOMPARE(chain.effects().size(), std::size_t{3});
		QCOMPARE(chain.effects()[0].get(), a);
		QCOMPARE(chain.effects()[1]->latency(), f_cnt_t{3});
		QCOMPARE(chain.effects()[2].get(), b);

		chain.moveEffect(0, 2);
		QCOMPARE(chain.effects()[2].get(), a);
		QCOMPARE(chain.effects()[1].get(), b);

		chain.replaceEffect(1, std::make_unique<PluginInstance>(std::make_unique<FakeDelay>(7), Plugin::Type::Effect));
		QCOMPARE(chain.effects()[1]->latency(), f_cnt_t{7});
		mix();
		QCOMPARE(mixer()->latency(), f_cnt_t{3 + 7 + 1});

		chain.clear();
	}

	void sidechainSendCanBeToggledFromEitherSide()
	{
		const auto kick = mixer()->createChannel();
		const auto bass = mixer()->createChannel();
		QVERIFY(mixer()->toggleSidechainSend(kick, bass));
		QCOMPARE(mixer()->mixerChannel(bass)->channelCount(), 4);
		QVERIFY(mixer()->findChannelSend(kick, bass, 0, 1) != nullptr);
		QVERIFY(!mixer()->toggleSidechainSend(kick, bass));
		QVERIFY(mixer()->findChannelSend(kick, bass, 0, 1) == nullptr);
		QCOMPARE(mixer()->channelOf(&mixer()->mixerChannel(bass)->m_fxChain), int{bass});
	}

	void multiOutputInstrumentGetsAChannelPerOutput()
	{
		auto track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
		track->setInstrumentInstance(
			std::make_unique<PluginInstance>(std::make_unique<FakeMultiOutput>(), Plugin::Type::Instrument));
		const auto trackIndex = track->mixerChannelModel()->value();

		const auto group = mixer()->setUpChannelPerOutput(track);
		QVERIFY(group > 0);
		QVERIFY(mixer()->mixerChannel(group)->m_groupChannel);
		QCOMPARE(mixer()->mixerChannel(trackIndex)->channelCount(), 8);

		// The pins route group g to the pair g + 1, mono groups to both of its channels
		const auto& pins = track->instrumentInstance()->audioPortsModel()->out();
		QVERIFY(!pins.enabled(0, 0));
		QVERIFY(pins.enabled(2, 0) && pins.enabled(3, 0));
		QVERIFY(pins.enabled(4, 1) && pins.enabled(5, 1));
		QVERIFY(pins.enabled(6, 2) && pins.enabled(7, 3));

		// A channel per output, named after it, takes its pair and sends to the group channel
		const auto& receivers = mixer()->mixerChannel(trackIndex)->m_sends;
		auto names = QStringList{};
		for (const MixerRoute* route : receivers)
		{
			if (route->sourcePair() == 0) { continue; }
			names << route->receiver()->m_name;
			QCOMPARE(mainOutputOf(route->receiver()), group);
		}
		QCOMPARE(names, (QStringList{"Kick", "Snare", "Cymbals"}));
		QCOMPARE(mainOutputOf(mixer()->mixerChannel(group)), 0);

		// Sound in a pair reaches the master through its channel and the group channel
		auto buffer = AudioBuffer{frames(), 8};
		std::ranges::fill(buffer.buffer(4), 0.5f);
		std::ranges::fill(buffer.buffer(5), 0.25f);
		buffer.assumeNonSilent(4);
		buffer.assumeNonSilent(5);
		mixer()->mixAuxiliaryToChannel(buffer, trackIndex);
		compareOutput(mix(), 0.5f, 0.25f);

		deleteTrack(track);
	}

	void channelsPerOutputBelongToTheTrack()
	{
		const auto song = Engine::getSong();
		const auto before = mixer()->numChannels();
		auto folder = Track::create(Track::Type::Folder, song);
		auto track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		auto other = Track::create(Track::Type::Sample, song);
		track->setInstrumentInstance(
			std::make_unique<PluginInstance>(std::make_unique<FakeMultiOutput>(), Plugin::Type::Instrument));
		const auto trackChannel = mixer()->trackChannel(track);
		const auto otherChannel = mixer()->trackChannel(other);

		// Made once, next to the track's channel (before the other track's): the group channel, then the pieces
		const auto group = mixer()->setUpChannelPerOutput(track);
		QCOMPARE(mixer()->setUpChannelPerOutput(track), group);
		QCOMPARE(group, trackChannel->index() + 1);
		QCOMPARE(otherChannel->index(), group + 4);
		const auto owned = mixer()->ownedChannels(trackChannel->index());
		QCOMPARE(owned.size(), std::size_t{4});
		for (auto i = std::size_t{0}; i < owned.size(); ++i)
		{
			QCOMPARE(owned[i]->index(), trackChannel->index() + 1 + static_cast<int>(i));
		}

		// The group channel follows the track into a folder
		song->changeFolderDepth(track, 1);
		QCOMPARE(outputOf(track), mixer()->trackChannel(folder)->index());
		QCOMPARE(mainOutputOf(mixer()->mixerChannel(group)), mixer()->trackChannel(folder)->index());

		// Arranging puts the channels back like the tracks: folder, track and its channels, other track
		mixer()->moveChannelRight(trackChannel->index());
		mixer()->moveChannelLeft(otherChannel->index());
		QCOMPARE(track->mixerChannelModel()->value(), before + 2);
		mixer()->arrangeChannelsByTracks();
		const int first = 1; // the tracks' channels come first, the effect returns last
		QCOMPARE(mixer()->trackChannel(folder)->index(), first);
		QCOMPARE(trackChannel->index(), first + 1);
		QCOMPARE(track->mixerChannelModel()->value(), first + 1);
		QCOMPARE(mixer()->mixerChannel(first + 2)->m_owner, trackChannel);
		QCOMPARE(otherChannel->index(), first + 6);

		// The owners are saved
		auto doc = DataFile{DataFile::Type::SongProject};
		auto element = doc.createElement("mixer");
		doc.content().appendChild(element);
		mixer()->saveSettings(doc, element);
		mixer()->loadSettings(element);
		QCOMPARE(mixer()->ownedChannels(first + 1).size(), std::size_t{4});

		// (Projects load the tracks' channels with the tracks)
		dynamic_cast<FolderTrack*>(folder)->mixerChannelModel()->setValue(first);
		track->mixerChannelModel()->setValue(first + 1);
		dynamic_cast<SampleTrack*>(other)->mixerChannelModel()->setValue(first + 6);

		// And go away with the track
		deleteTrack(track);
		deleteTrack(other);
		deleteTrack(folder);
		QCOMPARE(mixer()->numChannels(), before);
	}
};

QTEST_GUILESS_MAIN(MixerTest)
#include "MixerTest.moc"
