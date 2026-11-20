/*
 * legacy-harness.cpp - runs the Amp & IR A/B cases (cases.json) through the legacy program's own plugin
 * (build/plugins/libampandir.so, as it is) and writes the references
 *
 * Built by build-references.sh against the legacy build (as its MixerTest is built and linked). The plugin is loaded
 * and run as the mixer runs an effect (wet only), its IR and model loaded through its own settings (the paths in the
 * "ir" and "nam" attributes, the gains in "input" and "output"), exactly as a project opens it.
 *
 * This file is part of LinuxMusic - GPL-2.0-or-later.
 */

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <vector>

#include <QCoreApplication>
#include <QDomDocument>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>

#include "AudioBuffer.h"
#include "AudioEngine.h"
#include "AutomatableModel.h"
#include "Effect.h"
#include "EffectChain.h"
#include "EffectControls.h"
#include "Engine.h"
#include "PluginInstance.h"
#include "SampleFrame.h"

using namespace lmms;

namespace
{

// The signals (the same code is in tests/ab.rs)

auto splitMix(std::uint64_t z) -> std::uint64_t
{
	z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
	z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
	return z ^ (z >> 31);
}

struct Signal
{
	QString kind;
	std::int64_t hz = 0;
	double amp = 0.0;
	std::uint64_t state = 0;
	std::int64_t rate = 44100;
	std::vector<std::pair<std::int64_t, double>> steps;

	Signal(const QJsonObject& o, std::int64_t rate)
		: kind{o["kind"].toString()}
		, hz{static_cast<std::int64_t>(o["hz"].toDouble())}
		, amp{o["amp"].toDouble()}
		, state{static_cast<std::uint64_t>(o["seed"].toDouble())}
		, rate{rate}
	{
		for (const auto& step : o["steps"].toArray())
		{
			const auto pair = step.toArray();
			steps.emplace_back(static_cast<std::int64_t>(pair[0].toDouble()), pair[1].toDouble());
		}
	}

	auto at(std::int64_t n) -> float
	{
		double value = 0.0;
		if (kind == "sine")
		{
			// A parabolic sine, refined: + and * only
			const double phase = static_cast<double>((n * hz) % rate) / static_cast<double>(rate);
			const double x = phase * 2.0 - 1.0;
			const double a = 4.0 * x;
			const double b = 4.0 * x * std::abs(x);
			const double y = a - b;
			value = 0.225 * (y * std::abs(y) - y) + y;
		}
		else
		{
			state += 0x9e3779b97f4a7c15ULL;
			const auto z = splitMix(state);
			value = static_cast<double>(z >> 40) / 16777216.0 * 2.0 - 1.0;
		}
		double gain = 1.0;
		for (const auto& [frame, g] : steps)
		{
			if (frame <= n) { gain = g; }
		}
		return static_cast<float>(value * amp * gain);
	}
};

//! The plugin's settings, as a project saves them: the gains, and the files' paths (in @p data)
void apply(EffectControls* controls, const QJsonObject& s, const QString& data)
{
	auto document = QDomDocument{};
	auto element = document.createElement("ampir");
	for (const auto* key : {"input", "output"})
	{
		element.setAttribute(key, QString::number(s.contains(key) ? s[key].toDouble() : 0.0));
	}
	for (const auto* key : {"ir", "nam"})
	{
		element.setAttribute(key, s.contains(key) ? data + "/" + s[key].toString() : QString{});
	}
	controls->restoreState(element);
}

auto run(const QJsonObject& c, const QString& data) -> std::vector<SampleFrame>
{
	const auto frames = static_cast<std::int64_t>(c["frames"].toDouble());
	const auto block = static_cast<std::int64_t>(c["block"].toDouble());
	const auto rate = static_cast<std::int64_t>(c["rate"].toDouble());
	if (static_cast<std::int64_t>(Engine::audioEngine()->framesPerPeriod()) != block
		|| static_cast<std::int64_t>(Engine::audioEngine()->outputSampleRate()) != rate)
	{
		std::fprintf(stderr, "the engine runs at %d Hz in periods of %d\n",
			static_cast<int>(Engine::audioEngine()->outputSampleRate()),
			static_cast<int>(Engine::audioEngine()->framesPerPeriod()));
		std::exit(1);
	}
	const auto signal = c["signal"].toObject();
	auto left = Signal{signal, rate};
	auto right = Signal{signal.contains("right") ? signal["right"].toObject() : signal, rate};
	const auto twoSignals = signal.contains("right");
	auto out = std::vector<SampleFrame>(static_cast<std::size_t>(frames));
	for (std::int64_t n = 0; n < frames; ++n)
	{
		const auto l = left.at(n);
		out[n] = SampleFrame{l, twoSignals ? right.at(n) : l};
	}

	auto chain = EffectChain{nullptr};
	chain.appendEffect(std::make_unique<PluginInstance>(Plugin::Type::Effect, "ampandir", &chain, nullptr));
	auto* effect = dynamic_cast<Effect*>(chain.effects().back()->plugin());
	if (!effect) { std::fprintf(stderr, "the plugin did not load\n"); std::exit(1); }
	apply(effect->controls(), c["settings"].toObject(), data);

	auto buffer = AudioBuffer{static_cast<f_cnt_t>(block), 2, 2, true};
	buffer.setGroups(1, [](group_cnt_t, AudioBuffer::ChannelGroup&) { return ch_cnt_t{2}; });
	const auto then = c["then"].toObject();
	const auto at = then.isEmpty() ? frames : static_cast<std::int64_t>(then["at"].toDouble());
	for (std::int64_t start = 0; start < frames; start += block)
	{
		AutomatableModel::incrementPeriodCounter();
		if (start == at)
		{
			// (only the gains change: the files stay as they were)
			auto settings = c["settings"].toObject();
			const auto changed = then["settings"].toObject();
			for (auto it = changed.begin(); it != changed.end(); ++it) { settings[it.key()] = it.value(); }
			for (auto* model : effect->controls()->findChildren<AutomatableModel*>())
			{
				const auto name = model->displayName();
				const auto key = name == "Input gain" ? "input" : name == "Output gain" ? "output" : "";
				if (*key && settings.contains(key)) { model->setValue(static_cast<float>(settings[key].toDouble())); }
			}
		}
		for (auto* model : effect->controls()->findChildren<AutomatableModel*>()) { model->valueBuffer(); }
		auto interleaved = buffer.interleavedBuffer().asSampleFrames();
		std::copy_n(out.data() + start, block, interleaved.data());
		buffer.interleavedToMainPair();
		buffer.assumeNonSilent(0);
		buffer.assumeNonSilent(1);
		chain.effects().back()->process(buffer);
		std::copy_n(buffer.interleavedBuffer().asSampleFrames().data(), block, out.data() + start);
	}
	return out;
}

} // namespace

auto main(int argc, char** argv) -> int
{
	if (argc != 4)
	{
		std::fprintf(stderr, "usage: legacy-harness <cases.json> <data directory> <reference directory>\n");
		return 2;
	}
	auto app = QCoreApplication{argc, argv};
	Engine::init(true);
	Engine::audioEngine()->stopProcessing();

	auto file = QFile{argv[1]};
	if (!file.open(QIODevice::ReadOnly)) { std::fprintf(stderr, "cannot read %s\n", argv[1]); return 1; }
	const auto cases = QJsonDocument::fromJson(file.readAll()).object()["cases"].toArray();
	for (const auto& value : cases)
	{
		const auto c = value.toObject();
		const auto name = c["name"].toString();
		const auto out = run(c, QString::fromLocal8Bit(argv[2]));
		auto reference = QFile{QString{"%1/%2.f32"}.arg(argv[3], name)};
		if (!reference.open(QIODevice::WriteOnly)) { std::fprintf(stderr, "cannot write %s\n", qPrintable(name)); return 1; }
		for (const auto& frame : out)
		{
			const auto pair = std::array{frame[0], frame[1]};
			reference.write(reinterpret_cast<const char*>(pair.data()), sizeof(pair));
		}
		std::printf("%s\n", qPrintable(name));
	}
	Engine::destroy();
	return 0;
}
