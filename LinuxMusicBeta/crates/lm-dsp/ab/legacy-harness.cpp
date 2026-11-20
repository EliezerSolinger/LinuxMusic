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
#include "BasicFilters.h"
#include "AutomatableModel.h"
#include "AudioEngine.h"
#include "EffectChain.h"
#include "EnvelopeAndLfoParameters.h"
#include "Engine.h"
#include "lmms_math.h"
#include "DrawnWave.h"
#include "Graph.h"
#include "KickerOsc.h"
#include "VibratingString.h"
#include "PluginInstance.h"
#include "DeskTolerance.h"
#include "GainStage.h"
#include "MasterSection.h"
#include "Oscillator.h"
#include "Sample.h"

// The plugins' DSP (built from their sources by build-references.sh)
#include "Lfo.h"
#include "StereoDelay.h"
extern "C"
{
#include "base.h"
#include "dcblock.h"
#include "revsc.h"
}
#include "SampleFrame.h"

// The LB302's synth, driven as its notes drive it: its state opened to the harness (its own header only, after every
// header it includes)
#include <QScopeGuard>
#include "InstrumentTrack.h"
#include "Song.h"
#include "Track.h"
#include "Hardware.h"
#include "Instrument.h"
#include "InstrumentView.h"
#include "NotePlayHandle.h"
#define private public
#define protected public
#include "Lb302.h"
#include "Sfxr.h"
#include "Nes.h"
#undef protected
#undef private

using namespace lmms;

// The instruments whose headers cannot share this file (each its own: legacy-harness-<name>.cpp)
auto runMonstro(const QJsonObject& c, std::int64_t frames, std::int64_t block) -> std::vector<SampleFrame>;
auto runWatsyn(const QJsonObject& c, std::int64_t frames, std::int64_t block) -> std::vector<SampleFrame>;
auto runPatman(const QJsonObject& c, std::int64_t frames, std::int64_t block) -> std::vector<SampleFrame>;
auto runSlicerT(const QJsonObject& c, std::int64_t frames, std::int64_t block) -> std::vector<SampleFrame>;
auto runSlicerTAnalysis(const QJsonObject& c, std::int64_t frames) -> std::vector<SampleFrame>;

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
	if (unit == "monstro") { return runMonstro(c, frames, block); }
	if (unit == "watsyn") { return runWatsyn(c, frames, block); }
	if (unit == "patman") { return runPatman(c, frames, block); }
	if (unit == "slicert") { return runSlicerT(c, frames, block); }
	if (unit == "slicert-analysis") { return runSlicerTAnalysis(c, frames); }
	if (unit == "nes")
	{
		// The Nescaline's note (NesObject compiled in from the plugin) on a note of a track of the song, its frequency
		// set to the case's, the legacy fastRand seeded for the dither
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
		auto instrument = std::make_unique<NesInstrument>(track);
		const auto s = c["settings"].toObject();
		for (auto* model : instrument->findChildren<AutomatableModel*>())
		{
			if (s.contains(model->displayName())) { model->setValue(static_cast<float>(s[model->displayName()].toDouble())); }
		}
		// (the knobs' changes reach what the notes read once a period, before they render)
		AutomatableModel::incrementPeriodCounter();
		for (auto* model : instrument->findChildren<AutomatableModel*>()) { model->valueBuffer(); }
		static_cast<Instrument*>(instrument.get())->preparePeriod();
		auto* note = NotePlayHandleManager::acquire(track, 0, static_cast<f_cnt_t>(frames), Note{});
		const_cast<float&>(note->frequency()) = static_cast<float>(c["freq"].toDouble());
		fastRandState() = static_cast<unsigned long>(c["seed"].toDouble());
		auto nes = NesObject{instrument.get(), static_cast<sample_rate_t>(c["rate"].toDouble()), note};
		for (std::int64_t start = 0; start < frames; start += block)
		{
			nes.renderOutput(out.data() + start, static_cast<f_cnt_t>(std::min(block, frames - start)));
		}
		NotePlayHandleManager::release(note);
		return out;
	}
	if (unit == "sfxr")
	{
		// The sfxr's synth at its own rate, its sound's knobs set by their names, the legacy fastRand seeded
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
		auto instrument = std::make_unique<SfxrInstrument>(track);
		const auto s = c["settings"].toObject();
		for (auto* model : instrument->findChildren<AutomatableModel*>())
		{
			if (s.contains(model->displayName())) { model->setValue(static_cast<float>(s[model->displayName()].toDouble())); }
		}
		fastRandState() = static_cast<unsigned long>(c["seed"].toDouble());
		auto synth = SfxrSynth{instrument.get()};
		for (std::int64_t start = 0; start < frames; start += block)
		{
			synth.update(out.data() + start, static_cast<std::int32_t>(std::min(block, frames - start)));
		}
		return out;
	}
	if (unit == "lb302")
	{
		// The LB302 with no track (its knobs set by their names, its notes started as processNote starts them, the
		// held note's end given), a period at a time: preparePeriod(), the notes, render()
		// (a track of the song for its play handle, which the stopped engine never plays: the handle removed with the
		// synth, the track left to the song)
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
		auto synth = std::make_unique<Lb302Synth>(track);
		const auto cleanup = qScopeGuard([&] {
			Engine::audioEngine()->removePlayHandlesOfTypes(track, PlayHandle::Type::InstrumentPlayHandle);
			synth.reset();
		});
		const auto settingsOf = [&](const QJsonObject& s) {
			for (auto* model : synth->findChildren<AutomatableModel*>())
			{
				if (s.contains(model->displayName())) { model->setValue(static_cast<float>(s[model->displayName()].toDouble())); }
			}
		};
		settingsOf(c["settings"].toObject());
		const auto then = c["then"].toObject();
		const auto at = then.isEmpty() ? frames : static_cast<std::int64_t>(then["at"].toDouble());
		const auto notes = c["notes"].toArray();
		const auto rate = static_cast<float>(Engine::audioEngine()->outputSampleRate());
		for (std::int64_t start = 0; start < frames; start += block)
		{
			AutomatableModel::incrementPeriodCounter();
			if (start == at) { settingsOf(then["settings"].toObject()); }
			for (auto* model : synth->findChildren<AutomatableModel*>()) { model->valueBuffer(); }
			synth->preparePeriod();
			for (const auto& value : notes)
			{
				const auto n = value.toObject();
				if (static_cast<std::int64_t>(n["at"].toDouble()) != start) { continue; }
				synth->m_newFreq = true;
				synth->m_trueFreq = static_cast<float>(n["freq"].toDouble());
				const auto inc = synth->m_trueFreq / rate;
				if (synth->m_slideToggle.value()) { synth->m_slideBase = inc; } else { synth->m_vcoInc = inc; }
			}
			synth->m_releaseFrame = static_cast<f_cnt_t>(c["release"].toDouble());
			synth->render(out.data() + start, static_cast<f_cnt_t>(std::min(block, frames - start)));
		}
		return out;
	}
	if (unit == "vibed")
	{
		// The Vibed's note: its strings as its StringContainer makes them (VibratingString compiled in from the
		// plugin), the legacy fastRand seeded so that their randomness is drawn the same, mixed as playNote mixes them
		const auto s = c["settings"].toObject();
		fastRandState() = static_cast<unsigned long>(s["seed"].toDouble());
		constexpr auto octave = std::array{0.25f, 0.5f, 1.0f, 2.0f, 3.0f, 4.0f, 5.0f, 6.0f, 7.0f};
		const auto freq = static_cast<float>(s["freq"].toDouble());
		const auto rate = static_cast<sample_rate_t>(c["rate"].toDouble());
		struct Voice { std::unique_ptr<VibratingString> string; float pan; float volume; };
		auto voices = std::vector<Voice>{};
		for (const auto& value : s["strings"].toArray())
		{
			const auto o = value.toObject();
			auto shape = std::vector<float>{};
			for (const auto& p : o["shape"].toArray()) { shape.push_back(static_cast<float>(p.toDouble())); }
			const auto f = [&](const char* key) { return static_cast<float>(o[key].toDouble()); };
			voices.push_back(Voice{std::make_unique<VibratingString>(freq * octave[o["harmonic"].toInt()], f("pick"),
				f("pickup"), shape.data(), static_cast<int>(shape.size()), rate, o["length"].toInt(), f("fuzziness"),
				f("stiffness"), f("detune"), o["impulse"].toBool()), f("pan"), f("volume")});
		}
		for (std::int64_t i = 0; i < frames; ++i)
		{
			out[i] = SampleFrame{0.f, 0.f};
			for (auto& v : voices)
			{
				const float pan = (v.pan + 1) / 2.0f;
				const sample_t sample = v.string->nextSample() * v.volume / 100.0f;
				out[i][0] += (1.0f - pan) * sample;
				out[i][1] += pan * sample;
			}
		}
		return out;
	}
	if (unit == "organic")
	{
		// The Organic's note: its eight oscillators on each side made as Organic.cpp makes them (the volumes and
		// detunings by its formulas, copied here: the plugin's library is not linked), mixed through the core's
		// Oscillator, band limited, then its waveshaper and volume
		const auto s = c["settings"].toObject();
		const auto rate = static_cast<double>(Engine::audioEngine()->outputSampleRate());
		const auto oscs = s["oscs"].toArray();
		const auto shapes = std::array{Oscillator::WaveShape::Sine, Oscillator::WaveShape::Saw,
			Oscillator::WaveShape::Square, Oscillator::WaveShape::Triangle, Oscillator::WaveShape::MoogSaw,
			Oscillator::WaveShape::Exponential};
		constexpr auto Ratios = std::array<double, 18>{0.5, 0.75, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16};
		const auto count = static_cast<std::size_t>(oscs.size());
		auto algo = IntModel{static_cast<int>(Oscillator::ModulationAlgo::SignalMix), 0, 4};
		std::vector<std::unique_ptr<IntModel>> shapeModels;
		std::array<std::vector<std::unique_ptr<Oscillator>>, 2> chains;
		std::array<std::vector<float>, 2> volumes, phases;
		std::array<std::vector<double>, 2> detunings;
		for (auto side = 0; side < 2; ++side)
		{
			chains[side].resize(count);
			volumes[side].resize(count);
			phases[side].resize(count);
			detunings[side].resize(count);
		}
		const auto freq = static_cast<float>(s["freq"].toDouble());
		for (auto i = count; i-- > 0;)
		{
			const auto o = oscs[static_cast<int>(i)].toObject();
			shapeModels.push_back(std::make_unique<IntModel>(static_cast<int>(shapes[o["shape"].toInt()]), 0, 7));
			const auto vol = static_cast<float>(o["volume"].toDouble());
			const auto pan = static_cast<float>(o["pan"].toDouble());
			const auto num = static_cast<int>(count);
			volumes[0][i] = (1.0f - pan / 100.f) * vol / num / 100.0f;
			volumes[1][i] = (1.0f + pan / 100.f) * vol / num / 100.0f;
			const auto ratio = Ratios[o["harmonic"].toInt()];
			const auto detune = static_cast<double>(static_cast<float>(o["detune"].toDouble())) * (1.0f / 1200.0f);
			detunings[0][i] = ratio * std::exp2(detune) / rate;
			detunings[1][i] = ratio * std::exp2(-detune) / rate;
			phases[0][i] = static_cast<float>(o["phaseLeft"].toDouble());
			phases[1][i] = static_cast<float>(o["phaseRight"].toDouble());
			for (auto side = 0; side < 2; ++side)
			{
				chains[side][i] = std::make_unique<Oscillator>(shapeModels.back().get(), &algo, freq, detunings[side][i],
					phases[side][i], volumes[side][i], i + 1 < count ? chains[side][i + 1].get() : nullptr);
				chains[side][i]->setUseWaveTable(true);
			}
		}
		const auto amount = static_cast<float>(s["distortion"].toDouble());
		const auto volume = static_cast<float>(s["volume"].toDouble());
		const auto shape = [](float in, float a) {
			const float k = 2.0f * a / (1.0f - a);
			return (1.0f + k) * in / (1.0f + k * std::abs(in));
		};
		for (std::int64_t start = 0; start < frames; start += block)
		{
			const auto n = std::min(block, frames - start);
			chains[0][0]->update(out.data() + start, n, 0);
			chains[1][0]->update(out.data() + start, n, 1);
			for (auto f = start; f < start + n; ++f)
			{
				out[f][0] = shape(out[f][0], amount) * volume / 100.0f;
				out[f][1] = shape(out[f][1], amount) * volume / 100.0f;
			}
		}
		return out;
	}
	if (unit == "plugin-effect")
	{
		// A legacy effect plugin, loaded and run as the mixer runs it (wet only), its knobs set by their names
		const auto settingsOf = [&](const QJsonObject& s, Plugin* plugin) {
			for (auto* model : plugin->findChildren<AutomatableModel*>())
			{
				const auto key = model->displayName();
				if (s.contains(key)) { model->setValue(static_cast<float>(s[key].toDouble())); }
			}
		};
		if (static_cast<std::int64_t>(Engine::audioEngine()->framesPerPeriod()) != block)
		{
			std::fprintf(stderr, "the engine's period is %d\n", static_cast<int>(Engine::audioEngine()->framesPerPeriod()));
			std::exit(1);
		}
		auto chain = EffectChain{nullptr};
		chain.appendEffect(std::make_unique<PluginInstance>(Plugin::Type::Effect, c["plugin"].toString(), &chain, nullptr));
		auto* plugin = chain.effects().back()->plugin();
		settingsOf(c["settings"].toObject(), plugin);
		// The plugin's graphs (the Multitap Echo's taps), in the order it makes them, after the knobs that size them
		const auto graphs = plugin->findChildren<graphModel*>();
		const auto shapes = c["graphs"].toArray();
		for (qsizetype g = 0; g < std::min(shapes.size(), graphs.size()); ++g)
		{
			auto values = std::vector<float>{};
			for (const auto& v : shapes[g].toArray()) { values.push_back(static_cast<float>(v.toDouble())); }
			graphs[g]->setSamples(std::span<const float>{values});
		}
		auto buffer = AudioBuffer{static_cast<f_cnt_t>(block), 2, 2, true};
		buffer.setGroups(1, [](group_cnt_t, AudioBuffer::ChannelGroup&) { return ch_cnt_t{2}; });
		const auto then = c["then"].toObject();
		const auto at = then.isEmpty() ? frames : static_cast<std::int64_t>(then["at"].toDouble());
		for (std::int64_t start = 0; start < frames; start += block)
		{
			// A period starts, as the engine counts them (the models' value buffers are per period)
			AutomatableModel::incrementPeriodCounter();
			if (start == at) { settingsOf(then["settings"].toObject(), plugin); }
			// Each knob's value buffer worked out once at the period's start: a knob that changed reads as changed
			// in this period only (left to the effect, isValueChanged() worked it out a period late, and a change
			// counted in two periods: a quirk of the legacy plumbing, not of the DSP)
			for (auto* model : plugin->findChildren<AutomatableModel*>()) { model->valueBuffer(); }
			auto interleaved = buffer.interleavedBuffer().asSampleFrames();
			std::copy_n(out.data() + start, block, interleaved.data());
			buffer.interleavedToMainPair();
			buffer.assumeNonSilent(0);
			buffer.assumeNonSilent(1);
			chain.effects().back()->process(buffer); // as EffectChain::processAudioBuffer does, enabled
			std::copy_n(buffer.interleavedBuffer().asSampleFrames().data(), block, out.data() + start);
		}
		return out;
	}
	if (unit == "sample")
	{
		// Sample::play, as a note of the AudioFileProcessor plays it: the sample is the signal
		const auto s = c["settings"].toObject();
		auto data = std::vector<SampleFrame>(out.begin(), out.end());
		auto buffer = std::make_shared<const SampleBuffer>(std::move(data), static_cast<int>(s["rate"].toDouble()));
		auto sample = Sample{buffer};
		const auto bounds = s["bounds"].toArray();
		sample.setAllPointFrames(bounds[0].toInt(), bounds[1].toInt(), bounds[2].toInt(), bounds[3].toInt());
		sample.setAmplification(static_cast<float>(s["amp"].toDouble()));
		sample.setFrequency(static_cast<float>(s["frequency"].toDouble()));
		sample.setReversed(s["reversed"].toBool());
		sample.setDeclick(s["declick"].toBool());
		const auto modes = QStringList{"zoh", "linear", "sinc-fastest", "sinc-medium", "sinc-best"};
		auto state = Sample::PlaybackState{static_cast<AudioResampler::Mode>(modes.indexOf(s["interpolation"].toString()))};
		const auto loops = QStringList{"off", "on", "ping-pong"};
		const auto loop = static_cast<Sample::Loop>(loops.indexOf(s["loop"].toString()));
		const auto ratio = s["ratio"].toDouble();
		const auto length = static_cast<std::int64_t>(s["frames"].toDouble());
		auto played = std::vector<SampleFrame>(static_cast<std::size_t>(length));
		for (std::int64_t start = 0; start < length; start += block)
		{
			const auto n = std::min(block, length - start);
			sample.play(played.data() + start, &state, static_cast<std::size_t>(n), loop, ratio);
		}
		return played;
	}
	if (unit == "basic-filter")
	{
		// BasicFilters, as an instrument's sound shaping drives it: the type, then the frequency and the resonance
		// once a period
		const auto s = c["settings"].toObject();
		auto filter = BasicFilters<2>{Rate};
		filter.setFilterType(static_cast<BasicFilters<2>::FilterType>(s["type"].toInt()));
		const auto then = c["then"].toObject();
		const auto at = then.isEmpty() ? frames : static_cast<std::int64_t>(then["at"].toDouble());
		for (std::int64_t start = 0; start < frames; start += block)
		{
			const auto& now = start < at ? s : then["settings"].toObject();
			filter.calcFilterCoeffs(static_cast<float>(now["freq"].toDouble()), static_cast<float>(now["q"].toDouble()));
			for (auto f = start; f < std::min(start + block, frames); ++f)
			{
				out[f] = SampleFrame{filter.update(out[f][0], 0), filter.update(out[f][1], 1)};
			}
		}
		return out;
	}
	if (unit == "drawn-wave")
	{
		// A drawn wave's band-limited tables, played as a note gliding from one frequency to another: each frame its
		// table for the frequency, at the phase
		const auto s = c["settings"].toObject();
		auto points = std::vector<float>{};
		for (const auto& p : s["points"].toArray()) { points.push_back(static_cast<float>(p.toDouble())); }
		const auto wave = DrawnWave{points, s["hold"].toString() == "step" ? DrawnWave::Hold::Step : DrawnWave::Hold::Linear};
		const auto rate = static_cast<float>(c["rate"].toDouble());
		const auto from = static_cast<float>(s["from"].toDouble());
		const auto to = static_cast<float>(s["to"].toDouble());
		auto phase = 0.f;
		for (std::int64_t f = 0; f < frames; ++f)
		{
			const auto freq = from + (to - from) * (static_cast<float>(f) / static_cast<float>(frames));
			const auto* table = wave.tableFor(freq, rate);
			const auto v = table ? DrawnWave::at(*table, phase) : 0.f;
			out[f] = SampleFrame{v, v};
			phase += freq / rate;
			if (phase >= 1.f) { phase -= 1.f; }
		}
		return out;
	}
	if (unit == "kicker-osc")
	{
		// The Kicker's oscillator as a note builds it (its noise off: the legacy noise is the global generator's)
		const auto s = c["settings"].toObject();
		const auto v = [&](const char* key) { return static_cast<float>(s[key].toDouble()); };
		using DistFX = DspEffectLibrary::Distortion;
		auto osc = KickerOsc<DspEffectLibrary::MonoToStereoAdaptor<DistFX>>{DistFX{v("dist"), v("gain")}, v("start"),
			v("end"), 0.f, v("offset"), v("slope"), v("env"), v("dist"), v("distEnd"), v("length"), v("attack")};
		for (std::int64_t start = 0; start < frames; start += block)
		{
			osc.update(out.data() + start, std::min(block, frames - start), static_cast<float>(c["rate"].toDouble()));
		}
		return out;
	}
	if (unit == "envelope")
	{
		// An instrument's envelope and LFO: its level for a note from its start, released at "release", the LFO
		// moving on after each period as the engine moves it
		const auto s = c["settings"].toObject();
		auto envelope = EnvelopeAndLfoParameters{static_cast<float>(s["zero"].toDouble()), nullptr};
		for (auto* model : envelope.findChildren<AutomatableModel*>())
		{
			const auto key = model->displayName();
			if (s.contains(key)) { model->setValue(static_cast<float>(s[key].toDouble())); }
		}
		envelope.updateSampleVars();
		const auto release = static_cast<f_cnt_t>(s["release"].toDouble());
		auto level = std::vector<float>(static_cast<std::size_t>(block));
		for (std::int64_t start = 0; start < frames; start += block)
		{
			envelope.fillLevel(level.data(), static_cast<f_cnt_t>(start), release, static_cast<f_cnt_t>(block));
			for (std::int64_t f = 0; f < block; ++f) { out[start + f] = SampleFrame{level[f], level[f]}; }
			EnvelopeAndLfoParameters::instances()->trigger();
		}
		return out;
	}
	if (unit == "delay")
	{
		// DelayEffect::process, wet only, on the plugin's StereoDelay and Lfo
		const auto rate = static_cast<float>(Rate);
		auto delay = StereoDelay{20, static_cast<int>(Rate)};
		auto lfo = Lfo{static_cast<int>(Rate)};
		lfo.restart();
		auto apply = [&](std::int64_t from, std::int64_t to, const QJsonObject& s) {
			const float length = static_cast<float>(s["time"].toDouble());
			const float amplitude = static_cast<float>(s["lfoAmount"].toDouble()) * rate;
			const float lfoTime = 1.0 / static_cast<float>(s["lfoTime"].toDouble());
			const float feedback = static_cast<float>(s["feedback"].toDouble());
			const float outGain = dbfsToAmp(static_cast<float>(s["outGain"].toDouble()));
			for (auto f = from; f < to; ++f)
			{
				delay.setFeedback(feedback);
				lfo.setFrequency(lfoTime);
				const int currentLength = static_cast<int>(length * rate);
				delay.setLength(currentLength + (amplitude * static_cast<float>(lfo.tick())));
				delay.tick(out[f]);
				out[f] *= outGain;
			}
		};
		const auto then = c["then"].toObject();
		const auto at = then.isEmpty() ? frames : static_cast<std::int64_t>(then["at"].toDouble());
		apply(0, at, c["settings"].toObject());
		if (at < frames) { apply(at, frames, then["settings"].toObject()); }
		return out;
	}
	if (unit == "reverbsc")
	{
		// ReverbSCEffect::process, wet only, on Soundpipe's revsc and dcblock
		sp_data* sp = nullptr;
		sp_create(&sp);
		sp->sr = static_cast<int>(Rate);
		sp_revsc* revsc = nullptr;
		sp_revsc_create(&revsc);
		sp_revsc_init(sp, revsc);
		std::array<sp_dcblock*, 2> dcblk{};
		for (auto& d : dcblk) { sp_dcblock_create(&d); sp_dcblock_init(sp, d, 1); }
		auto apply = [&](std::int64_t from, std::int64_t to, const QJsonObject& s) {
			const auto inGain = fastPow10f<SPFLOAT>(static_cast<float>(s["inputGain"].toDouble()) / 20.f);
			const auto outGain = fastPow10f<SPFLOAT>(static_cast<float>(s["outputGain"].toDouble()) / 20.f);
			for (auto f = from; f < to; ++f)
			{
				auto in = std::array{out[f][0] * inGain, out[f][1] * inGain};
				revsc->feedback = static_cast<SPFLOAT>(s["size"].toDouble());
				revsc->lpfreq = static_cast<SPFLOAT>(s["color"].toDouble());
				SPFLOAT l, r, dl, dr;
				sp_revsc_compute(sp, revsc, &in[0], &in[1], &l, &r);
				sp_dcblock_compute(sp, dcblk[0], &l, &dl);
				sp_dcblock_compute(sp, dcblk[1], &r, &dr);
				out[f] = SampleFrame{dl * outGain, dr * outGain};
			}
		};
		const auto then = c["then"].toObject();
		const auto at = then.isEmpty() ? frames : static_cast<std::int64_t>(then["at"].toDouble());
		apply(0, at, c["settings"].toObject());
		if (at < frames) { apply(at, frames, then["settings"].toObject()); }
		sp_revsc_destroy(&revsc);
		for (auto& d : dcblk) { sp_dcblock_destroy(&d); }
		sp_destroy(&sp);
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
