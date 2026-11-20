/*
 * legacy-harness.cpp - runs lm-plugins-guitarix's A/B cases (cases.json) on the legacy Guitarix plugin (the .so
 * itself, libguitarix.so, as the legacy mixer runs an effect) and writes the references
 *
 * Built by build-references.sh against the legacy build. The engine's period is LM_PERIOD (the cases of that
 * period only are run): the legacy plugin processes in blocks of its period, which some units' sound depends on.
 * The input signals are made with + and * only (compiled with -ffp-contract=off), so that tests/ab.rs makes the
 * same input bit for bit.
 *
 * This file is part of LinuxMusic - GPL-2.0-or-later.
 */

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <memory>
#include <optional>
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
#include "ConfigManager.h"
#include "Effect.h"
#include "EffectChain.h"
#include "EffectControls.h"
#include "Engine.h"
#include "Mixer.h"
#include "PluginFactory.h"
#include "PluginInstance.h"
#include "SampleFrame.h"

using namespace lmms;

namespace
{

constexpr std::int64_t Rate = 48000;

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

	explicit Signal(const QJsonObject& o)
		: kind{o["kind"].toString()}
		, hz{static_cast<std::int64_t>(o["hz"].toDouble())}
		, amp{o["amp"].toDouble()}
		, state{static_cast<std::uint64_t>(o["seed"].toDouble())}
	{
	}

	auto at(std::int64_t n) -> float
	{
		double value = 0.0;
		if (kind == "sine")
		{
			// A parabolic sine, refined: + and * only
			const double phase = static_cast<double>((n * hz) % Rate) / static_cast<double>(Rate);
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
		// A note every half second: its attack, then a decay (guitar-like, through the units' dynamics)
		const auto t = n % (Rate / 2);
		const double envelope = 1.0 / (1.0 + static_cast<double>(t) * (8.0 / static_cast<double>(Rate)));
		return static_cast<float>(value * amp * envelope);
	}
};

//! The unit's parameters set from @p settings ({Guitarix id: value}), as the legacy plugin loads them from a project
void apply(Effect* effect, const QJsonObject& settings)
{
	auto doc = QDomDocument{};
	auto element = doc.createElement("controls");
	for (auto it = settings.begin(); it != settings.end(); ++it)
	{
		element.setAttribute(it.key(), QString::number(it.value().toDouble(), 'g', 9));
	}
	effect->controls()->restoreState(element);
}

auto run(const QJsonObject& c) -> std::vector<SampleFrame>
{
	const auto frames = static_cast<std::int64_t>(c["frames"].toDouble());
	const auto block = static_cast<std::int64_t>(c["block"].toDouble());
	const auto signal = c["signal"].toObject();
	auto left = Signal{signal};
	auto right = signal.contains("right") ? std::optional<Signal>{Signal{signal["right"].toObject()}} : std::nullopt;
	auto out = std::vector<SampleFrame>(static_cast<std::size_t>(frames));
	for (std::int64_t n = 0; n < frames; ++n)
	{
		const float l = left.at(n);
		out[n] = SampleFrame{l, right ? right->at(n) : l};
	}

	const auto info = getPluginFactory()->pluginInfo("guitarix");
	auto attributes = Plugin::Descriptor::SubPluginFeatures::Key::AttributeMap{};
	attributes["unit"] = c["unit"].toString();
	const auto key = Plugin::Descriptor::SubPluginFeatures::Key{info.descriptor, c["unit"].toString(), attributes};
	auto chain = EffectChain{nullptr};
	chain.appendEffect(std::make_unique<PluginInstance>(Plugin::Type::Effect, "guitarix", &chain, &key));
	auto* effect = dynamic_cast<Effect*>(chain.effects().back()->plugin());
	if (!effect) { std::fprintf(stderr, "no unit %s\n", qPrintable(c["unit"].toString())); std::exit(1); }
	apply(effect, c["settings"].toObject());
	// The unit made anew and started with them, as before a rest (the settings it reads only when it starts: a
	// cabinet's response, the presence, the Detune's latency setting)
	emit Engine::mixer()->processingStateDiscarding();

	auto buffer = AudioBuffer{static_cast<f_cnt_t>(block), 2, 2, true};
	buffer.setGroups(1, [](group_cnt_t, AudioBuffer::ChannelGroup&) { return ch_cnt_t{2}; });
	const auto then = c["then"].toObject();
	const auto at = then.isEmpty() ? frames : static_cast<std::int64_t>(then["at"].toDouble());
	for (std::int64_t start = 0; start < frames; start += block)
	{
		AutomatableModel::incrementPeriodCounter();
		if (start == at) { apply(effect, then["settings"].toObject()); }
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
	if (argc != 3)
	{
		std::fprintf(stderr, "usage: LM_PERIOD=<frames> legacy-harness <cases.json> <reference directory>\n");
		return 2;
	}
	const auto period = std::getenv("LM_PERIOD") ? std::atoi(std::getenv("LM_PERIOD")) : 256;
	auto app = QCoreApplication{argc, argv};
	ConfigManager::inst()->setValue("audioengine", "framesperaudiobuffer", QString::number(period));
	ConfigManager::inst()->setValue("audioengine", "samplerate", QString::number(Rate));
	Engine::init(true);
	Engine::audioEngine()->stopProcessing();
	if (static_cast<int>(Engine::audioEngine()->framesPerPeriod()) != period
		|| static_cast<std::int64_t>(Engine::audioEngine()->outputSampleRate()) != Rate)
	{
		std::fprintf(stderr, "the engine runs %d frames at %d Hz\n", static_cast<int>(Engine::audioEngine()->framesPerPeriod()),
			static_cast<int>(Engine::audioEngine()->outputSampleRate()));
		return 1;
	}

	auto file = QFile{argv[1]};
	if (!file.open(QIODevice::ReadOnly)) { std::fprintf(stderr, "cannot read %s\n", argv[1]); return 1; }
	const auto cases = QJsonDocument::fromJson(file.readAll()).object()["cases"].toArray();
	for (const auto& value : cases)
	{
		const auto c = value.toObject();
		if (static_cast<int>(c["block"].toDouble()) != period) { continue; }
		const auto name = c["name"].toString();
		const auto out = run(c);
		auto reference = QFile{QString{"%1/%2.f32"}.arg(argv[2], name)};
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
