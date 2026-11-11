/*
 * AudioPortsTest.cpp - tests for the pin connector: AudioPortsModel, AudioPortsRouter and PluginInstance
 *
 * Copyright (c) 2026 Dalton Messmer <messmer.dalton/at/gmail.com>
 *
 * This file is part of LMMS - https://lmms.io
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

#include <functional>
#include <memory>
#include <optional>
#include <vector>

#include <QDomDocument>
#include <QDomElement>

#include "AudioBuffer.h"
#include "AudioEngine.h"
#include "AudioPortsModel.h"
#include "AudioPortsRouter.h"
#include "AudioProcessor.h"
#include "Effect.h"
#include "EffectControls.h"
#include "Engine.h"
#include "Instrument.h"
#include "PluginChecks.h"
#include "PluginInstance.h"

using namespace lmms;

namespace
{

Plugin::Descriptor s_fakeDescriptor = {
	"fakeeffect", "Fake effect", "Effect for tests", "LinuxMusic", 0x0100, Plugin::Type::Effect,
	nullptr, nullptr, nullptr
};

class FakeControls : public EffectControls
{
public:
	using EffectControls::EffectControls;
	void saveSettings(QDomDocument&, QDomElement&) override {}
	void loadSettings(const QDomElement&) override {}
	auto nodeName() const -> QString override { return "fakecontrols"; }
	auto controlCount() -> int override { return 0; }
	auto createView() -> gui::EffectControlDialog* override { return nullptr; }
};

//! An effect whose audio ports and processing are set by each test
class FakeEffect : public Effect
{
public:
	struct Ports
	{
		ch_cnt_t in = 2;
		ch_cnt_t out = 2;
		std::function<std::optional<ch_cnt_t>(ch_cnt_t)> inplacePair = [](ch_cnt_t ch) { return ch; };
		bool interleaved = false;
	};

	FakeEffect(Ports ports, std::function<ProcessStatus(ProcessContext&)> process)
		: Effect{&s_fakeDescriptor, nullptr, nullptr}
		, m_controls{this}
		, m_ports{std::move(ports)}
		, m_process{std::move(process)}
	{
	}

	auto controls() -> EffectControls* override { return &m_controls; }
	auto process(ProcessContext& context) -> ProcessStatus override { return m_process(context); }
	auto channelCount(bool isInput) const -> ch_cnt_t override { return isInput ? m_ports.in : m_ports.out; }
	auto channelGroupCount(bool isInput) const -> group_cnt_t override { return channelCount(isInput) > 0 ? 1 : 0; }
	auto getChannelGroup(group_cnt_t, bool isInput, AudioBuffer::ChannelGroup&) const -> ch_cnt_t override
	{
		return channelCount(isInput);
	}
	auto getInplacePair(ch_cnt_t input) const -> std::optional<ch_cnt_t> override { return m_ports.inplacePair(input); }
	auto usesInterleavedBuffers() const -> bool override { return m_ports.interleaved; }

	void setEnabled(bool enabled) { enabledModel()->setValue(enabled); }

private:
	FakeControls m_controls;
	Ports m_ports;
	std::function<ProcessStatus(ProcessContext&)> m_process;
};

Plugin::Descriptor s_fakeInstrumentDescriptor = {
	"fakeinstrument", "Fake instrument", "Instrument for tests", "LinuxMusic", 0x0100, Plugin::Type::Instrument,
	nullptr, nullptr, nullptr
};

//! A legacy instrument rendering a constant stereo signal, per period or per note
class FakeLegacyInstrument : public LegacyInstrument
{
public:
	FakeLegacyInstrument(float left, float right)
		: LegacyInstrument{&s_fakeInstrumentDescriptor, nullptr, nullptr, Flag::IsSingleStreamed}
		, m_left{left}
		, m_right{right}
	{
	}

	void saveSettings(QDomDocument&, QDomElement&) override {}
	void loadSettings(const QDomElement&) override {}
	auto nodeName() const -> QString override { return "fakeinstrument"; }
	auto instantiateView(QWidget*) -> gui::PluginView* override { return nullptr; }

protected:
	void playImpl(std::span<SampleFrame> out) override
	{
		// Adds to the buffer, so the result is only right if the host passed a silent buffer
		for (auto& frame : out) { frame += SampleFrame{m_left, m_right}; }
	}

	void playNoteImpl(NotePlayHandle*, std::span<SampleFrame> out) override
	{
		for (auto& frame : out) { frame = SampleFrame{m_left, m_right}; }
	}

private:
	float m_left;
	float m_right;
};

//! A planar instrument with any number of outputs; output channel `ch` is filled with 0.1 * (ch + 1)
class FakePlanarInstrument : public Instrument
{
public:
	explicit FakePlanarInstrument(ch_cnt_t outputs, std::vector<ch_cnt_t> groups = {})
		: Instrument{&s_fakeInstrumentDescriptor, nullptr, nullptr, Flag::IsSingleStreamed}
		, m_outputs{outputs}
		, m_groups{groups.empty() ? std::vector<ch_cnt_t>{outputs} : std::move(groups)}
	{
	}

	void saveSettings(QDomDocument&, QDomElement&) override {}
	void loadSettings(const QDomElement&) override {}
	auto nodeName() const -> QString override { return "fakeinstrument"; }
	auto instantiateView(QWidget*) -> gui::PluginView* override { return nullptr; }

	auto process(ProcessContext& context) -> ProcessStatus override
	{
		auto& out = context.out();
		for (ch_cnt_t ch = 0; ch < out.channels(); ++ch)
		{
			std::ranges::fill(out.buffer(ch), 0.1f * (ch + 1));
		}
		return ProcessStatus::Continue;
	}
	auto channelCount(bool isInput) const -> ch_cnt_t override { return isInput ? 0 : m_outputs; }
	auto channelGroupCount(bool isInput) const -> group_cnt_t override
	{
		return isInput ? 0 : static_cast<group_cnt_t>(m_groups.size());
	}
	auto getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup&) const -> ch_cnt_t override
	{
		return isInput ? 0 : m_groups[index];
	}
	auto getInplacePair(ch_cnt_t) const -> std::optional<ch_cnt_t> override { return std::nullopt; }

private:
	ch_cnt_t m_outputs;
	std::vector<ch_cnt_t> m_groups;
};

auto frames() -> f_cnt_t { return Engine::audioEngine()->framesPerPeriod(); }

//! A stereo track buffer (with the interleaved buffer mixer channels have) filled with a constant signal
auto trackBuffer(float left, float right, ch_cnt_t channels = 2) -> AudioBuffer
{
	auto buffer = AudioBuffer{frames(), channels, channels, true};
	for (ch_cnt_t ch = 0; ch < channels; ++ch)
	{
		std::ranges::fill(buffer.buffer(ch), ch == 0 ? left : ch == 1 ? right : 0.f);
		buffer.assumeNonSilent(ch);
	}
	toInterleaved(buffer.groupBuffers(0), buffer.interleavedBuffer());
	return buffer;
}

void compareChannel(const AudioBuffer& buffer, ch_cnt_t channel, float value)
{
	for (const auto sample : buffer.buffer(channel)) { QVERIFY(qFuzzyCompare(1.f + sample, 1.f + value)); }
}

auto makeInstance(FakeEffect::Ports ports, std::function<ProcessStatus(ProcessContext&)> process)
	-> std::unique_ptr<PluginInstance>
{
	return std::make_unique<PluginInstance>(
		std::make_unique<FakeEffect>(std::move(ports), std::move(process)), Plugin::Type::Effect);
}

//! Processes like the audio engine does: on an audio thread
auto processOnAudioThread(PluginInstance& instance, AudioBuffer& track, NotePlayHandle* nph = nullptr) -> bool
{
	const auto audioThread = ThreadCheck::AudioThreadScope{};
	return instance.process(track, nph);
}

auto scale(AudioBuffer& buffer, float factor)
{
	for (ch_cnt_t ch = 0; ch < buffer.channels(); ++ch)
	{
		for (auto& sample : buffer.buffer(ch)) { sample *= factor; }
	}
}

} // namespace


class AudioPortsTest : public QObject
{
	Q_OBJECT

	std::vector<std::string> m_violations;
	bool m_expectViolations = false;

private slots:
	void initTestCase()
	{
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();

		// Every test follows the plugin API's rules
		PluginChecks::setEnabled(true);
		PluginChecks::setViolationHandler([this](std::string_view message) {
			m_violations.emplace_back(message);
		});
	}

	void init()
	{
		m_violations.clear();
	}

	void cleanup()
	{
		for (const auto& message : m_violations) { qWarning("Plugin API violation: %s", message.c_str()); }
		QVERIFY(m_violations.empty() || m_expectViolations);
		m_expectViolations = false;
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	//////////////////////
	// AudioPortsModel  //
	//////////////////////

	void Model_ChannelCounts()
	{
		auto model = AudioPortsModel{false};
		model.setAllChannelCounts(2, 3, 4);
		QCOMPARE(model.trackChannelCount(), 2);
		QCOMPARE(model.in().channelCount(), 3);
		QCOMPARE(model.out().channelCount(), 4);
		QCOMPARE(model.in().trackChannelCount(), 2);
		QCOMPARE(model.out().trackChannelCount(), 2);
	}

	void Model_DefaultConnections_Stereo()
	{
		auto model = AudioPortsModel{false};
		model.setAllChannelCounts(2, 2, 2);
		for (const auto* matrix : {&model.in(), &model.out()})
		{
			QVERIFY(matrix->enabled(0, 0));
			QVERIFY(!matrix->enabled(0, 1));
			QVERIFY(!matrix->enabled(1, 0));
			QVERIFY(matrix->enabled(1, 1));
		}
	}

	void Model_DefaultConnections_Mono()
	{
		// Only the left channel goes in (summing both would make mono effects louder), the output goes to both
		auto model = AudioPortsModel{false};
		model.setAllChannelCounts(2, 1, 1);
		QVERIFY(model.in().enabled(0, 0));
		QVERIFY(!model.in().enabled(1, 0));
		QVERIFY(model.out().enabled(0, 0));
		QVERIFY(model.out().enabled(1, 0));
	}

	void Model_DefaultConnections_ExtraInputsUnconnected()
	{
		// e.g. a compressor with a sidechain input pair
		auto model = AudioPortsModel{false};
		model.setAllChannelCounts(2, 4, 2);
		for (ch_cnt_t trackChannel = 0; trackChannel < 2; ++trackChannel)
		{
			QVERIFY(!model.in().enabled(trackChannel, 2));
			QVERIFY(!model.in().enabled(trackChannel, 3));
		}
		QVERIFY(model.in().usedChannels()[0] && model.in().usedChannels()[1]);
		QVERIFY(!model.in().usedChannels()[2] && !model.in().usedChannels()[3]);
	}

	void Model_DefaultConnections_InstrumentHasNoInputPins()
	{
		auto model = AudioPortsModel{true};
		model.setAllChannelCounts(2, 2, 2);
		QVERIFY(!model.in().enabled(0, 0));
		QVERIFY(!model.in().enabled(1, 1));
		QVERIFY(model.out().enabled(0, 0));
		QVERIFY(model.out().enabled(1, 1));
	}

	void Model_UsedTrackChannels()
	{
		auto model = AudioPortsModel{false};
		model.setAllChannelCounts(2, 1, 1);
		QVERIFY(model.in().usedTrackChannels()[0]);
		QVERIFY(!model.in().usedTrackChannels()[1]);
		QVERIFY(model.out().usedTrackChannels()[0]);
		QVERIFY(model.out().usedTrackChannels()[1]);

		model.in().setPin(1, 0, true);
		QVERIFY(model.in().usedTrackChannels()[1]);
	}

	void Model_DirectRouting()
	{
		auto model = AudioPortsModel{false};
		model.setAllChannelCounts(2, 2, 2);
		QCOMPARE(model.directRouting(), std::optional<ch_cnt_t>{0});

		// Swapping the outputs is not a direct connection anymore
		model.out().setPin(0, 0, false);
		model.out().setPin(0, 1, true);
		QCOMPARE(model.directRouting(), std::nullopt);

		// A mono processor never qualifies
		model.setAllChannelCounts(2, 1, 1);
		QCOMPARE(model.directRouting(), std::nullopt);
	}

	void Model_SaveLoad()
	{
		auto model = AudioPortsModel{false};
		model.setAllChannelCounts(2, 4, 2);
		model.in().setPin(0, 2, true);
		model.in().setPin(1, 3, true);
		model.out().setPin(1, 1, false);

		auto doc = QDomDocument{};
		auto parent = doc.createElement("effect");
		doc.appendChild(parent);
		model.saveState(doc, parent);

		auto loaded = AudioPortsModel{false};
		loaded.setAllChannelCounts(2, 4, 2);
		loaded.restoreState(parent.firstChildElement(loaded.nodeName()));

		for (ch_cnt_t tc = 0; tc < 2; ++tc)
		{
			for (ch_cnt_t pc = 0; pc < 4; ++pc) { QCOMPARE(loaded.in().enabled(tc, pc), model.in().enabled(tc, pc)); }
			for (ch_cnt_t pc = 0; pc < 2; ++pc) { QCOMPARE(loaded.out().enabled(tc, pc), model.out().enabled(tc, pc)); }
		}
	}

	void Model_TrackChannelsUpperBoundFollowsPins()
	{
		auto model = AudioPortsModel{false};
		model.setAllChannelCounts(4, 2, 2);
		QCOMPARE(model.trackChannelsUpperBound(), 2);

		model.in().setPin(3, 0, true);
		QCOMPARE(model.trackChannelsUpperBound(), 4);
		model.in().setPin(3, 0, false);
		QCOMPARE(model.trackChannelsUpperBound(), 2);

		model.out().setPin(2, 1, true);
		QCOMPARE(model.trackChannelsUpperBound(), 3);
	}

	void Model_LoadDropsPinsToMissingChannels()
	{
		auto model = AudioPortsModel{false};
		model.setAllChannelCounts(4, 2, 2);
		model.in().setPin(3, 1, true);

		auto doc = QDomDocument{};
		auto parent = doc.createElement("effect");
		doc.appendChild(parent);
		model.saveSettings(doc, parent);

		// Fewer track channels than when saved: the pin to channel 4 has nowhere to go
		auto loaded = AudioPortsModel{false};
		loaded.setAllChannelCounts(2, 2, 2);
		loaded.loadSettings(parent);
		QVERIFY(loaded.in().enabled(0, 0));
		QVERIFY(loaded.in().enabled(1, 1));
		QCOMPARE(loaded.trackChannelsUpperBound(), 2);
	}

	//////////////////////
	// AudioPortsRouter //
	//////////////////////

	void Router_SendFromAuxiliaryTrackChannels()
	{
		auto model = AudioPortsModel{false};
		model.setAllChannelCounts(4, 2, 2);
		model.in().setPin(0, 0, false);
		model.in().setPin(1, 1, false);
		model.in().setPin(2, 0, true);
		model.in().setPin(3, 1, true);

		auto track = trackBuffer(0.25f, 0.5f, 4);
		std::ranges::fill(track.buffer(2), 0.125f);
		std::ranges::fill(track.buffer(3), 0.0625f);
		track.assumeNonSilent(2);
		track.assumeNonSilent(3);

		auto in = AudioBuffer{frames(), 2};
		AudioPortsRouter::send(model, track, in);
		compareChannel(in, 0, 0.125f);
		compareChannel(in, 1, 0.0625f);
	}

	void Router_SendSumsConnectedTrackChannels()
	{
		auto model = AudioPortsModel{false};
		model.setAllChannelCounts(2, 1, 1);
		model.in().setPin(1, 0, true); // both L and R into the mono input

		auto track = trackBuffer(0.25f, 0.5f);
		auto in = AudioBuffer{frames(), 1};
		AudioPortsRouter::send(model, track, in);
		compareChannel(in, 0, 0.75f);
		QVERIFY(in.hasAnySignal());
	}

	void Router_SendSilencesUnconnectedInputs()
	{
		auto model = AudioPortsModel{false};
		model.setAllChannelCounts(2, 4, 2);

		auto track = trackBuffer(0.25f, 0.5f);
		auto in = AudioBuffer{frames(), 4};
		std::ranges::fill(in.buffer(2), 9.f); // stale data must be cleared
		in.assumeNonSilent(2);
		AudioPortsRouter::send(model, track, in);
		compareChannel(in, 0, 0.25f);
		compareChannel(in, 1, 0.5f);
		compareChannel(in, 2, 0.f);
		compareChannel(in, 3, 0.f);
		QVERIFY(!in.hasSignal(AudioBuffer::ChannelFlags{}.set(2)));
	}

	void Router_ReceiveReplacesConnectedAndKeepsUnconnectedTrackChannels()
	{
		auto model = AudioPortsModel{false};
		model.setAllChannelCounts(2, 1, 1);
		model.out().setPin(1, 0, false); // the output only goes to L

		auto track = trackBuffer(0.25f, 0.5f);
		auto out = AudioBuffer{frames(), 1};
		std::ranges::fill(out.buffer(0), 0.75f);
		AudioPortsRouter::receive(model, out, track);
		compareChannel(track, 0, 0.75f);
		compareChannel(track, 1, 0.5f);
	}

	////////////////////
	// PluginInstance //
	////////////////////

	void Instance_StereoInterleavedEffectWorksOnTheTrackDirectly()
	{
		const AudioBuffer* processed = nullptr;
		auto instance = makeInstance({.interleaved = true}, [&](ProcessContext& context) {
			processed = &context.inOut();
			for (auto& frame : context.inOut().interleavedBuffer().asSampleFrames()) { frame *= 0.5f; }
			return ProcessStatus::Continue;
		});
		QVERIFY(instance->audioPortsModel()->directRouting().has_value());

		auto track = trackBuffer(0.8f, 0.4f);
		QVERIFY(processOnAudioThread(*instance, track));
		QCOMPARE(processed, &track);
		compareChannel(track, 0, 0.4f);
		compareChannel(track, 1, 0.2f);
		QCOMPARE(track.interleavedBuffer().frame(0)[0], 0.4f);
	}

	void Instance_InterleavedEffectWithoutDirectRouting()
	{
		auto instance = makeInstance({.interleaved = true}, [&](ProcessContext& context) {
			for (auto& frame : context.inOut().interleavedBuffer().asSampleFrames()) { frame *= 0.5f; }
			return ProcessStatus::Continue;
		});
		// Swap the inputs: the plugin's buffers and the router are used
		auto& in = instance->audioPortsModel()->in();
		in.setPin(0, 0, false);
		in.setPin(1, 0, true);
		in.setPin(1, 1, false);
		in.setPin(0, 1, true);
		QVERIFY(!instance->audioPortsModel()->directRouting());

		auto track = trackBuffer(0.8f, 0.4f);
		QVERIFY(processOnAudioThread(*instance, track));
		compareChannel(track, 0, 0.2f);
		compareChannel(track, 1, 0.4f);
		QCOMPARE(track.interleavedBuffer().frame(0)[0], 0.2f); // kept in sync
	}

	void Instance_PlanarEffectWithSeparateInputsAndOutputs()
	{
		auto instance = makeInstance({.inplacePair = [](ch_cnt_t) { return std::nullopt; }},
			[&](ProcessContext& context) {
				const auto& in = context.in();
				auto& out = context.out();
				for (ch_cnt_t ch = 0; ch < 2; ++ch)
				{
					std::ranges::transform(in.buffer(ch), out.buffer(ch).begin(), [](float s) { return s * 2.f; });
				}
				return ProcessStatus::Continue;
			});

		auto track = trackBuffer(0.1f, 0.2f);
		QVERIFY(processOnAudioThread(*instance, track));
		compareChannel(track, 0, 0.2f);
		compareChannel(track, 1, 0.4f);
	}

	void Instance_InplacePairsShareBuffers()
	{
		// Only input 0 and output 0 are an in-place pair
		bool shared = false;
		bool separate = false;
		auto instance = makeInstance({.inplacePair = [](ch_cnt_t ch) { return ch == 0 ? std::optional<ch_cnt_t>{0} : std::nullopt; }},
			[&](ProcessContext& context) {
				shared = context.in().buffer(0).data() == context.out().buffer(0).data();
				separate = context.in().buffer(1).data() != context.out().buffer(1).data();
				std::ranges::copy(context.in().buffer(1), context.out().buffer(1).begin());
				return ProcessStatus::Continue;
			});

		auto track = trackBuffer(0.1f, 0.2f);
		QVERIFY(processOnAudioThread(*instance, track));
		QVERIFY(shared);
		QVERIFY(separate);
		compareChannel(track, 0, 0.1f);
		compareChannel(track, 1, 0.2f);
	}

	void Instance_MonoEffect()
	{
		auto instance = makeInstance({.in = 1, .out = 1}, [&](ProcessContext& context) {
			scale(context.inOut(), 0.5f);
			return ProcessStatus::Continue;
		});

		// Only L goes in, the output goes to both L and R
		auto track = trackBuffer(0.8f, 0.4f);
		QVERIFY(processOnAudioThread(*instance, track));
		compareChannel(track, 0, 0.4f);
		compareChannel(track, 1, 0.4f);
	}

	void Instance_SidechainInput()
	{
		// Inputs 3/4 are a sidechain; the output is the main input ducked by the sidechain level
		auto instance = makeInstance({.in = 4, .out = 2, .inplacePair = [](ch_cnt_t ch) {
				return ch < 2 ? std::optional<ch_cnt_t>{ch} : std::nullopt;
			}},
			[&](ProcessContext& context) {
				const auto& in = context.in();
				auto& out = context.out();
				for (ch_cnt_t ch = 0; ch < 2; ++ch)
				{
					std::ranges::transform(in.buffer(ch), in.buffer(ch + 2), out.buffer(ch).begin(),
						[](float main, float sidechain) { return main * (1.f - sidechain); });
				}
				return ProcessStatus::Continue;
			});

		// Unconnected sidechain: silence, so the main signal passes unchanged
		auto track = trackBuffer(0.8f, 0.4f);
		QVERIFY(processOnAudioThread(*instance, track));
		compareChannel(track, 0, 0.8f);
		compareChannel(track, 1, 0.4f);

		// Feed the track's right channel to both sidechain inputs
		instance->audioPortsModel()->in().setPin(1, 2, true);
		instance->audioPortsModel()->in().setPin(1, 3, true);
		auto track2 = trackBuffer(0.8f, 0.5f);
		QVERIFY(processOnAudioThread(*instance, track2));
		compareChannel(track2, 0, 0.4f);
		compareChannel(track2, 1, 0.25f);
	}

	void Instance_DisabledEffectPassesThrough()
	{
		bool called = false;
		auto instance = makeInstance({.interleaved = true}, [&](ProcessContext&) {
			called = true;
			return ProcessStatus::Continue;
		});
		static_cast<FakeEffect*>(instance->effect())->setEnabled(false);

		auto track = trackBuffer(0.8f, 0.4f);
		QVERIFY(!processOnAudioThread(*instance, track));
		QVERIFY(!called);
		compareChannel(track, 0, 0.8f);
		compareChannel(track, 1, 0.4f);
	}

	void Instance_SleepingEffectWakesUpOnInput()
	{
		int calls = 0;
		auto instance = makeInstance({.interleaved = true}, [&](ProcessContext&) {
			++calls;
			return ProcessStatus::Sleep;
		});

		// The effect asks to sleep after processing
		auto track = trackBuffer(0.8f, 0.4f);
		QVERIFY(!processOnAudioThread(*instance, track));
		QCOMPARE(calls, 1);
		QVERIFY(!instance->effect()->isAwake());

		// Asleep without input: not processed, and its output track channels are silenced
		auto silent = AudioBuffer{frames(), 2, 2, true};
		QVERIFY(!processOnAudioThread(*instance, silent));
		QCOMPARE(calls, 1);
		QVERIFY(!silent.hasAnySignal());

		// Input wakes it up
		auto track2 = trackBuffer(0.8f, 0.4f);
		processOnAudioThread(*instance, track2);
		QCOMPARE(calls, 2);
	}

	void Instance_SanitizesItsOutput()
	{
		auto instance = makeInstance({.interleaved = true}, [&](ProcessContext& context) {
			context.inOut().interleavedBuffer().frame(3)[0] = std::numeric_limits<float>::quiet_NaN();
			return ProcessStatus::Continue;
		});

		auto track = trackBuffer(0.8f, 0.4f);
		processOnAudioThread(*instance, track);
		if (Engine::audioEngine()->sanitizationEnabled())
		{
			compareChannel(track, 0, 0.f);
			QVERIFY(instance->effect()->isCorrupted());
		}
	}

	void Instance_SidechainGroupConnectsToChannels3And4()
	{
		// Two input groups: the main pair and a sidechain pair
		auto instance = makeInstance({.in = 4, .out = 2, .inplacePair = [](ch_cnt_t) { return std::nullopt; }},
			[](ProcessContext&) { return ProcessStatus::Continue; });
		const auto& pins = instance->audioPortsModel()->in();
		QVERIFY(!pins.enabled(0, 2) && !pins.enabled(1, 3));

		// The fake effect has one input group, so nothing happens
		instance->setTrackChannelCount(4);
		QCOMPARE(instance->audioPortsModel()->trackChannelCount(), 4);
		QVERIFY(!pins.enabled(2, 2));
	}

	void Instance_ProcessingStartsOnTheAudioThread()
	{
		auto instance = makeInstance({}, [](ProcessContext&) { return ProcessStatus::Continue; });
		QVERIFY(instance->hasState(PluginInstance::State::Active));
		QVERIFY(!instance->hasState(PluginInstance::State::Processing));

		auto track = trackBuffer(0.5f, 0.5f);
		processOnAudioThread(*instance, track);
		QVERIFY(instance->hasState(PluginInstance::State::Processing));

		// Preparing again stops and deactivates it first, then it starts again in the next period
		instance->reactivate();
		QVERIFY(instance->hasState(PluginInstance::State::Active));
		QVERIFY(!instance->hasState(PluginInstance::State::Processing));
		QVERIFY(m_violations.empty());
	}

	void Instance_ProcessOnTheMainThreadIsAViolation()
	{
		auto instance = makeInstance({}, [](ProcessContext&) { return ProcessStatus::Continue; });
		auto track = trackBuffer(0.5f, 0.5f);
		instance->process(track);
		QCOMPARE(m_violations.size(), std::size_t{1});
		QVERIFY(m_violations.front().find("audio thread") != std::string::npos);
		m_expectViolations = true;
	}

	void Instance_PinsAreSavedAndLoaded()
	{
		auto instance = makeInstance({.in = 4, .out = 2}, [](ProcessContext&) { return ProcessStatus::Continue; });
		instance->audioPortsModel()->in().setPin(1, 3, true);

		auto doc = QDomDocument{};
		auto element = doc.createElement("effect");
		doc.appendChild(element);
		instance->savePins(doc, element);

		auto other = makeInstance({.in = 4, .out = 2}, [](ProcessContext&) { return ProcessStatus::Continue; });
		QVERIFY(!other->audioPortsModel()->in().enabled(1, 3));
		other->loadPins(element);
		QVERIFY(other->audioPortsModel()->in().enabled(1, 3));
	}

	/////////////////
	// Instruments //
	/////////////////

	void Instrument_LegacyRendersIntoSilentBuffer()
	{
		auto instance = PluginInstance{std::make_unique<FakeLegacyInstrument>(0.5f, 0.25f), Plugin::Type::Instrument};
		QVERIFY(instance.isInstrument());
		QCOMPARE(instance.audioPortsModel()->in().channelCount(), 0);
		QCOMPARE(instance.audioPortsModel()->out().channelCount(), 2);

		// Twice: the instrument adds to its buffer, so it must be silent each period
		for (int period = 0; period < 2; ++period)
		{
			auto track = trackBuffer(0.f, 0.f);
			QVERIFY(processOnAudioThread(instance, track));
			compareChannel(track, 0, 0.5f);
			compareChannel(track, 1, 0.25f);
		}
	}

	void Instrument_PlanarWithMoreOutputsThanTrackChannels()
	{
		auto instance = PluginInstance{std::make_unique<FakePlanarInstrument>(4), Plugin::Type::Instrument};
		QCOMPARE(instance.audioPortsModel()->out().channelCount(), 4);

		// By default, the first two outputs go to the track's main channels
		auto track = trackBuffer(0.f, 0.f);
		processOnAudioThread(instance, track);
		compareChannel(track, 0, 0.1f);
		compareChannel(track, 1, 0.2f);

		// Outputs connected to the same track channel are summed
		instance.audioPortsModel()->out().setPin(0, 2, true);
		instance.audioPortsModel()->out().setPin(1, 1, false);
		instance.audioPortsModel()->out().setPin(1, 3, true);
		track = trackBuffer(0.f, 0.f);
		processOnAudioThread(instance, track);
		compareChannel(track, 0, 0.1f + 0.3f);
		compareChannel(track, 1, 0.4f);
	}

	void Instrument_OutputGroupsGoToTheMainPair()
	{
		// e.g. a drum kit: a mono kick, a mono snare and stereo cymbals
		auto instance = PluginInstance{std::make_unique<FakePlanarInstrument>(4, std::vector<ch_cnt_t>{1, 1, 2}),
			Plugin::Type::Instrument};
		const auto& pins = instance.audioPortsModel()->out();
		QVERIFY(pins.enabled(0, 0) && pins.enabled(1, 0)); // mono groups to both sides
		QVERIFY(pins.enabled(0, 1) && pins.enabled(1, 1));
		QVERIFY(pins.enabled(0, 2) && !pins.enabled(1, 2)); // stereo group to L/R
		QVERIFY(!pins.enabled(0, 3) && pins.enabled(1, 3));

		auto track = trackBuffer(0.f, 0.f);
		processOnAudioThread(instance, track);
		compareChannel(track, 0, 0.1f + 0.2f + 0.3f);
		compareChannel(track, 1, 0.1f + 0.2f + 0.4f);
	}

	void Instrument_NoteOutputFollowsPins()
	{
		auto instance = PluginInstance{std::make_unique<FakeLegacyInstrument>(0.5f, 0.25f), Plugin::Type::Instrument};
		auto note = std::vector<SampleFrame>(frames());

		// Default routing: unchanged
		instance.playNote(nullptr, note);
		for (const auto& frame : note)
		{
			QCOMPARE(frame.left(), 0.5f);
			QCOMPARE(frame.right(), 0.25f);
		}

		// Swapped channels
		auto& pins = instance.audioPortsModel()->out();
		pins.setPin(0, 0, false);
		pins.setPin(0, 1, true);
		pins.setPin(1, 1, false);
		pins.setPin(1, 0, true);
		instance.playNote(nullptr, note);
		for (const auto& frame : note)
		{
			QCOMPARE(frame.left(), 0.25f);
			QCOMPARE(frame.right(), 0.5f);
		}

		// Both outputs into the left channel, nothing into the right one
		pins.setPin(0, 0, true);
		pins.setPin(1, 0, false);
		instance.playNote(nullptr, note);
		for (const auto& frame : note)
		{
			QCOMPARE(frame.left(), 0.75f);
			QCOMPARE(frame.right(), 0.f);
		}
	}
};

QTEST_GUILESS_MAIN(AudioPortsTest)
#include "AudioPortsTest.moc"
