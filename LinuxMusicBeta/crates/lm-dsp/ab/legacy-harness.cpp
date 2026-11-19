/*
 * legacy-harness.cpp - runs lm-dsp's A/B cases (cases.json) on the legacy program's code and writes the references
 *
 * Built by build-references.sh against the legacy build's objects (build/, configured from legado/). The signals are
 * made with + and * only (compiled with -ffp-contract=off), so that lm-dsp's tests make the same input bit for bit.
 *
 * This file is part of LinuxMusic - GPL-2.0-or-later.
 */

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <optional>
#include <span>
#include <vector>

#include <QCoreApplication>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>

#include "AnalogChannelStrip.h"
#include "AutomatableModel.h"
#include "AudioEngine.h"
#include "Engine.h"
#include "DeskTolerance.h"
#include "GainStage.h"
#include "MasterSection.h"
#include "Oscillator.h"
#include "SampleFrame.h"

using namespace lmms;

namespace
{

constexpr sample_rate_t Rate = 48000;

// ------------------------------------------------------------------------------------------------------------------
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
	std::vector<std::pair<std::int64_t, double>> steps;

	explicit Signal(const QJsonObject& o)
		: kind{o["kind"].toString()}
		, hz{static_cast<std::int64_t>(o["hz"].toDouble())}
		, amp{o["amp"].toDouble()}
		, state{static_cast<std::uint64_t>(o["seed"].toDouble())}
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
		double gain = 1.0;
		for (const auto& [frame, g] : steps)
		{
			if (frame <= n) { gain = g; }
		}
		return static_cast<float>(value * amp * gain);
	}
};

// ------------------------------------------------------------------------------------------------------------------
// The settings

auto gainModel(const QString& key) -> int
{
	const auto keys = QStringList{"flat", "4558", "5532", "12ax7", "12ax7-vintage", "12ax7-hot", "5534", "tl072",
		"12at7", "12au7"};
	return static_cast<int>(keys.indexOf(key));
}

auto compressorModel(const QString& key) -> int
{
	const auto keys = QStringList{"ssl", "api2500", "dbx160", "urei1176", "la2a", "fairchild670", "gates-sta-level",
		"altec436", "rca-ba6a"};
	return static_cast<int>(keys.indexOf(key));
}

auto gateMode(const QString& key) -> int
{
	const auto keys = QStringList{"std", "kick", "snare", "toms", "vox", "guitar", "bass", "noise", "expander"};
	return static_cast<int>(keys.indexOf(key));
}

auto speed(const QString& key) -> float
{
	return key == "fast" ? 0.f : key == "slow" ? 2.f : 1.f;
}

//! Sets the strip as @p s says; what @p s does not say is the neutral setting
void apply(AnalogChannelStrip& strip, const QJsonObject& s)
{
	strip.setNeutral();
	if (s.contains("gain")) { strip.m_gainModel.setValue(s["gain"].toDouble()); }
	if (s.contains("drive")) { strip.m_driveModel.setValue(gainModel(s["drive"].toString())); }
	strip.m_realisticModel.setValue(s["realistic"].toBool());
	strip.m_lowCutModel.setValue(s["lowCut"].toBool());
	if (s.contains("highPass")) { strip.m_highPassModel.setValue(s["highPass"].toDouble()); }
	if (s.contains("lowPass")) { strip.m_lowPassModel.setValue(s["lowPass"].toDouble()); }
	if (s.contains("monoBass")) { strip.m_monoBassModel.setValue(s["monoBass"].toDouble()); }
	if (s.contains("monoBassFrequency")) { strip.m_monoBassFrequencyModel.setValue(s["monoBassFrequency"].toDouble()); }
	if (s.contains("compressor"))
	{
		const auto c = s["compressor"].toObject();
		strip.m_compressorTypeModel.setValue(compressorModel(c["model"].toString()));
		strip.m_compressorAmountModel.setValue(c["amount"].toDouble());
		strip.m_compressorRatioModel.setValue(c["ratio"].toDouble());
		strip.m_compressorSpeedModel.setValue(speed(c["speed"].toString()));
		strip.m_compressorReleaseModel.setValue(c["release"].toDouble());
	}
	if (s.contains("gate"))
	{
		const auto g = s["gate"].toObject();
		strip.m_gateModel.setValue(g["threshold"].toDouble());
		strip.m_gateRangeModel.setValue(g["range"].toDouble());
		strip.m_gateModeModel.setValue(gateMode(g["mode"].toString()));
	}
	if (s.contains("eq"))
	{
		const auto eq = s["eq"].toArray();
		strip.m_eqLowModel.setValue(eq[0].toDouble());
		strip.m_eqLowMidModel.setValue(eq[1].toDouble());
		strip.m_eqHighMidModel.setValue(eq[2].toDouble());
		strip.m_eqHighModel.setValue(eq[3].toDouble());
	}
}

struct Master
{
	MasterSection section{nullptr};
	float width = 100.f;
	bool drive = false;
	bool transformer = false;

	void apply(const QJsonObject& s)
	{
		section.m_preLinkModel.setValue(false);
		const auto pre = s.contains("pre") ? s["pre"].toArray() : QJsonArray{1.0, 1.0};
		section.m_preLeftModel.setValue(pre[0].toDouble());
		section.m_preRightModel.setValue(pre[1].toDouble());
		drive = s.contains("drive");
		if (drive) { section.m_driveModel.setValue(gainModel(s["drive"].toString())); }
		section.m_realisticModel.setValue(s["realistic"].toBool());
		const auto stereo = s["stereo"].toObject();
		width = static_cast<float>(stereo.contains("width") ? stereo["width"].toDouble() : 100.0);
		section.m_monoBassModel.setValue(stereo["monoBass"].toDouble());
		section.m_bassFrequencyModel.setValue(stereo.contains("bassFrequency") ? stereo["bassFrequency"].toDouble() : 120.0);
		section.m_airModel.setValue(stereo["air"].toDouble());
		const auto bus = s["bus"].toObject();
		section.m_compressorModel.setValue(bus["on"].toBool());
		section.m_thresholdModel.setValue(bus.contains("threshold") ? bus["threshold"].toDouble() : 5.0);
		section.m_ratioModel.setValue(bus.contains("ratio") ? bus["ratio"].toDouble() : 1.0);
		section.m_attackModel.setValue(bus.contains("attack") ? bus["attack"].toDouble() : 4.0);
		section.m_releaseModel.setValue(bus.contains("release") ? bus["release"].toDouble() : 4.0);
		section.m_makeupModel.setValue(bus["makeup"].toDouble());
		transformer = s["transformer"].toBool();
		const auto monitor = s["monitor"].toObject();
		section.m_monitorLevelModel.setValue(monitor["level"].toDouble());
		section.m_dimModel.setValue(monitor["dim"].toBool());
		section.m_cutModel.setValue(monitor["cut"].toBool());
		section.m_monoModel.setValue(monitor["mono"].toBool());
		section.m_sideModel.setValue(monitor["side"].toBool());
	}

	void process(std::span<SampleFrame> frames)
	{
		section.processPreFaders(frames, Rate);
		if (drive) { section.processDrive(frames, Rate); }
		section.processStereo(frames, Rate, width);
		section.processCompressor(frames, Rate);
		if (transformer) { section.processTransformer(frames, Rate); }
		section.processMonitoring(frames);
	}
};

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

	const auto unit = c["unit"].toString();
	if (unit == "osc")
	{
		// A chain of oscillators, as the TripleOscillator makes them, at the engine's rate
		const auto rate = static_cast<double>(Engine::audioEngine()->outputSampleRate());
		if (rate != c["rate"].toDouble()) { std::fprintf(stderr, "the engine runs at %g\n", rate); std::exit(1); }
		const auto oscs = c["settings"].toObject()["oscs"].toArray();
		const auto count = static_cast<std::size_t>(oscs.size());
		const auto shapes = QStringList{"sine", "triangle", "saw", "square", "moog-saw", "exponential", "white-noise", "user"};
		const auto algos = QStringList{"pm", "am", "mix", "sync", "fm"};
		std::vector<std::unique_ptr<IntModel>> shapeModels, algoModels;
		std::vector<float> freqs(count), phases(count), volumes(count);
		std::vector<double> detunings(count);
		std::vector<std::unique_ptr<Oscillator>> chain(count);
		for (auto i = count; i-- > 0;)
		{
			const auto o = oscs[static_cast<int>(i)].toObject();
			shapeModels.push_back(std::make_unique<IntModel>(static_cast<int>(shapes.indexOf(o["shape"].toString())), 0, 7));
			algoModels.push_back(std::make_unique<IntModel>(static_cast<int>(algos.indexOf(o["algo"].toString())), 0, 4));
			freqs[i] = static_cast<float>(o["freq"].toDouble());
			detunings[i] = o["ratio"].toDouble() / rate;
			phases[i] = static_cast<float>(o["phase"].toDouble());
			volumes[i] = static_cast<float>(o["volume"].toDouble());
			chain[i] = std::make_unique<Oscillator>(shapeModels.back().get(), algoModels.back().get(), freqs[i],
				detunings[i], phases[i], volumes[i], i + 1 < count ? chain[i + 1].get() : nullptr);
			chain[i]->setUseWaveTable(o["tables"].toBool());
			if (o.contains("user"))
			{
				// A user's wave: one period of the harness's noise
				const auto user = o["user"].toObject();
				auto noise = Signal{QJsonObject{{"kind", "noise"}, {"seed", user["seed"]}, {"amp", 1.0}}};
				auto data = std::vector<SampleFrame>(static_cast<std::size_t>(user["frames"].toDouble()));
				for (std::size_t f = 0; f < data.size(); ++f)
				{
					const float x = noise.at(static_cast<std::int64_t>(f));
					data[f] = SampleFrame{x, x};
				}
				auto buffer = std::make_shared<const SampleBuffer>(std::move(data), static_cast<int>(rate));
				chain[i]->setUserWave(buffer);
				chain[i]->setUserAntiAliasWaveTable(Oscillator::generateAntiAliasUserWaveTable(buffer.get()));
			}
		}
		for (std::int64_t start = 0; start < frames; start += block)
		{
			const auto n = std::min(block, frames - start);
			chain[0]->update(out.data() + start, n, 0);
			for (std::int64_t f = start; f < start + n; ++f) { out[f][1] = out[f][0]; }
		}
		return out;
	}
	if (unit == "tube-curve")
	{
		// The static curve alone: the tube's transfer function and where it clips
		const auto model = static_cast<GainStage::Model>(gainModel(c["settings"].toObject()["drive"].toString()));
		for (auto& frame : out) { frame = SampleFrame{GainStage::tube(model, frame[0]), GainStage::tubeClipLevel(model)}; }
		return out;
	}
	const auto then = c["then"].toObject();
	const auto changeAt = then.isEmpty() ? frames : static_cast<std::int64_t>(then["at"].toDouble());
	auto strip = AnalogChannelStrip{nullptr};
	auto master = Master{};
	auto tolerance = ChannelTolerance::none();
	auto settings = c["settings"].toObject();
	auto set = [&](const QJsonObject& s) {
		if (unit == "strip")
		{
			apply(strip, s);
			const int channel = s["channel"].toInt();
			tolerance = channel > 0 ? ChannelTolerance::forChannel(channel) : ChannelTolerance::none();
		}
		else { master.apply(s); }
	};
	set(settings);
	for (std::int64_t start = 0; start < frames; start += block)
	{
		if (start == changeAt) { set(then["settings"].toObject()); }
		const auto count = std::min(block, frames - start);
		auto span = std::span<SampleFrame>{out.data() + start, static_cast<std::size_t>(count)};
		if (unit == "strip") { strip.process(span, Rate, true, tolerance); }
		else { master.process(span); }
	}
	return out;
}

} // namespace

auto main(int argc, char** argv) -> int
{
	if (argc != 3)
	{
		std::fprintf(stderr, "usage: legacy-harness <cases.json> <reference directory>\n");
		return 2;
	}
	// The strip's and the master's controls are the legacy program's models, which need its engine (set up as its
	// tests do, with an isolated configuration: TestEnvironment.cpp is linked in)
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
		const auto out = run(c);
		auto reference = QFile{QString{"%1/%2.f32"}.arg(argv[2], name)};
		if (!reference.open(QIODevice::WriteOnly)) { std::fprintf(stderr, "cannot write %s\n", qPrintable(name)); return 1; }
		for (const auto& frame : out)
		{
			const auto pair = std::array{frame[0], frame[1]};
			// Little-endian on every target the legacy program builds for
			reference.write(reinterpret_cast<const char*>(pair.data()), sizeof(pair));
		}
		std::printf("%s\n", qPrintable(name));
	}
	Engine::destroy();
	return 0;
}
