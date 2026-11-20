/*
 * legacy-harness.cpp - plays lm-plugins-stk's A/B cases (cases.txt) on the legacy Mallets and writes the references
 *
 * Built by build-references.sh against the legacy build: the legacy MalletsSynth (Mallets.cpp's object) around the
 * STK voices (STK 5.0.1, the library the legacy plugin links), driven as MalletsInstrument::playNoteImpl drives them.
 * playNoteImpl itself belongs to the instrument, which needs a running program (a track, a note play handle); what it
 * does around the MalletsSynth, the randomness, the voice and the level when a note starts and the live controls in
 * each period, is written out below line by line from Mallets.cpp.
 *
 * Output: <dir>/<case>.f32, the plugin's own output (before the track's volume), f32 little endian: interleaved
 * stereo when the case spreads the sides, else the left side alone (the right is the same, sample for sample).
 *
 * This file is part of LinuxMusic - GPL-2.0-or-later.
 */

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <map>
#include <sstream>
#include <string>
#include <vector>

#include <stk/BandedWG.h>
#include <stk/ModalBar.h>
#include <stk/Stk.h>
#include <stk/TubeBell.h>

#include "Mallets.h"
#include "lmms_math.h"
#include "volume.h"

using namespace lmms;

namespace
{

constexpr double Rate = 44100;
constexpr int Period = 32;

const std::vector<std::string> Presets = {"marimba", "vibraphone", "agogo", "wood1", "reso", "wood2", "beats",
	"two-fixed", "clump", "tubular-bells", "uniform-bar", "tuned-bar", "glass", "tibetan-bowl"};

//! Mallets.cpp's BandedWGAtRest (in an anonymous namespace there), the same three lines
class BandedWGAtRest : public stk::BandedWG
{
public:
	BandedWGAtRest()
	{
		velocityInput_ = 0;
		maxVelocity_ = 0;
	}
};

struct Change
{
	int frame;
	std::string knob;
	float value;
};

struct Case
{
	std::string name;
	std::map<std::string, float> knobs = {{"hardness", 64}, {"position", 64}, {"vibratoGain", 0}, {"vibratoFreq", 0},
		{"stickMix", 0}, {"modulator", 64}, {"crossfade", 64}, {"lfoSpeed", 64}, {"lfoDepth", 64}, {"adsr", 64},
		{"pressure", 64}, {"speed", 64}, {"bowed", 1}, {"spread", 0}, {"randomness", 0}, {"pitch", 0},
		{"levelled", 1}, {"oldVersion", 0}};
	int preset = 0;
	int key = 69;
	int volume = 100;
	int frames = 0;
	unsigned long seed = 0;
	std::vector<Change> changes;
};

std::vector<Case> readCases(const char* path)
{
	std::ifstream in(path);
	std::vector<Case> cases;
	std::string line;
	while (std::getline(in, line))
	{
		if (line.empty() || line[0] == '#') { continue; }
		std::istringstream words(line);
		Case c;
		words >> c.name;
		std::string word;
		while (words >> word)
		{
			const auto eq = word.find('=');
			const auto key = word.substr(0, eq);
			const auto value = word.substr(eq + 1);
			if (key == "preset")
			{
				c.preset = static_cast<int>(std::find(Presets.begin(), Presets.end(), value) - Presets.begin());
			}
			else if (key == "key") { c.key = std::stoi(value); }
			else if (key == "volume") { c.volume = std::stoi(value); }
			else if (key == "frames") { c.frames = std::stoi(value); }
			else if (key == "seed") { c.seed = std::stoul(value); }
			else if (key == "change")
			{
				const auto a = value.find(':');
				const auto b = value.find(':', a + 1);
				c.changes.push_back({std::stoi(value.substr(0, a)), value.substr(a + 1, b - a - 1),
					std::stof(value.substr(b + 1))});
			}
			else { c.knobs[key] = std::stof(value); }
		}
		cases.push_back(c);
	}
	return cases;
}

//! The note's frequency as the plugin's instrument functions work it out (pitch in cents, no bend, no glide)
float frequency(int key, float pitchCents)
{
	const float pitch = pitchCents / 100.0f + 0.0f * 2.0f;
	const float keys = static_cast<float>(key) - 69.0f + 0.0f + pitch;
	return 440.0f * std::exp2(keys / 12.0f);
}

std::vector<float> play(Case& c)
{
	auto& k = c.knobs;
	const int p = c.preset;
	std::vector<float> out(2 * static_cast<size_t>(c.frames));

	// MalletsInstrument's m_scalers
	const std::array<sample_t, 14> scalers = {4.0, 4.0, 5.0, 4.0, 2.5, 5.0, 20.0, 5.0, 4.0, 1.8f, 25.0, 10.0, 16.0, 7.0};
	const bool isOldVersion = k["oldVersion"] != 0;
	const bool levelled = k["levelled"] != 0;
	const volume_t noteVolume = static_cast<volume_t>(c.volume);

	fastRandState() = c.seed;
	float freq = frequency(c.key, k["pitch"]);

	// playNoteImpl, when the note starts
	MalletsSynth* ps = nullptr;
	{
		float velocityAdjust = isOldVersion ? 100.0 : 200.0;
		const float vel = noteVolume / velocityAdjust;

		const float random = k["randomness"];
		float hardness = k["hardness"];
		float position = k["position"];
		float modulator = k["modulator"];
		float crossfade = k["crossfade"];
		float pressure = k["pressure"];
		float speed = k["speed"];

		if (p < 9)
		{
			hardness += random * fastRandInc(-64.f, 64.f);
			hardness = std::clamp(hardness, 0.0f, 128.0f);

			position += random * fastRandInc(-32.f, 32.f);
			position = std::clamp(position, 0.0f, 64.0f);
		}
		else if (p == 9)
		{
			modulator += random * fastRandInc(-64.f, 64.f);
			modulator = std::clamp(modulator, 0.0f, 128.0f);

			crossfade += random * fastRandInc(-64.f, 64.f);
			crossfade = std::clamp(crossfade, 0.0f, 128.0f);
		}
		else
		{
			pressure += random * fastRandInc(-64.f, 64.f);
			pressure = std::clamp(pressure, 0.0f, 128.0f);

			speed += random * fastRandInc(-64.f, 64.f);
			speed = std::clamp(speed, 0.0f, 128.0f);
		}

		// MalletsInstrument::makeVoice
		stk::Instrmnt* voice = nullptr;
		if (p < 9) { voice = new stk::ModalBar(); }
		else if (p == 9) { voice = new stk::TubeBell(); }
		else { voice = new BandedWGAtRest(); }

		const auto spread = static_cast<uint8_t>(k["spread"]);
		if (p < 9)
		{
			ps = new MalletsSynth(voice, freq, vel, k["stickMix"], hardness, position, k["vibratoGain"],
				k["vibratoFreq"], p, spread);
		}
		else if (p == 9)
		{
			ps = new MalletsSynth(voice, freq, vel, p, k["lfoDepth"], modulator, crossfade, k["lfoSpeed"], k["adsr"],
				spread);
		}
		else
		{
			ps = new MalletsSynth(voice, freq, vel, pressure, speed, p - 10, k["bowed"] != 0,
				static_cast<uint8_t>(spread));
		}
		ps->setPresetIndex(p);
		if (levelled)
		{
			static constexpr auto Trims = std::array{-7.f, -8.5f, -11.f, -6.f, -5.f, -6.f, -16.5f, -12.5f, -6.5f,
				-0.5f, -3.f, -5.f, -3.f};
			auto level = dbfsToAmp(Trims[std::clamp<std::size_t>(p, 0, Trims.size() - 1)]);
			if (p <= 9) { level *= DefaultVolume / std::max(static_cast<float>(noteVolume), 1.f); }
			ps->setLevel(level);
		}
	}

	// playNoteImpl, in each period
	for (int start = 0; start < c.frames; start += Period)
	{
		for (const auto& change : c.changes)
		{
			if (change.frame == start)
			{
				k[change.knob] = change.value;
				if (change.knob == "pitch") { freq = frequency(c.key, k["pitch"]); }
			}
		}
		const int frames = std::min(Period, c.frames - start);
		ps->setFrequency(freq);

		const int pi = ps->presetIndex();
		if (pi < 9)
		{
			ps->setVibratoGain(k["vibratoGain"]);
			ps->setVibratoFreq(k["vibratoFreq"]);
		}
		else if (pi == 9)
		{
			ps->setADSR(k["adsr"]);
			ps->setLFODepth(k["lfoDepth"]);
			ps->setLFOSpeed(k["lfoSpeed"]);
		}

		sample_t add_scale = 0.0f;
		if (pi == 10 && isOldVersion)
		{
			add_scale = static_cast<sample_t>(k["bowed"] != 0) * freq * 2.5f;
		}

		for (int frame = start; frame < start + frames; ++frame)
		{
			out[2 * frame] = ps->nextSampleLeft() * (scalers[pi] + add_scale) * ps->level();
			out[2 * frame + 1] = ps->nextSampleRight() * (scalers[pi] + add_scale) * ps->level();
		}
	}
	stk::Instrmnt* voice = ps->voice();
	delete ps;
	delete voice;
	return out;
}

} // namespace

int main(int argc, char** argv)
{
	if (argc != 4)
	{
		std::fprintf(stderr, "usage: %s cases.txt rawwave-folder output-folder\n", argv[0]);
		return 2;
	}
	stk::Stk::setSampleRate(Rate);
	stk::Stk::setRawwavePath(argv[2]);
	stk::Stk::showWarnings(false);
	for (auto& c : readCases(argv[1]))
	{
		const bool stereo = c.knobs["spread"] != 0;
		auto out = play(c);
		if (!stereo)
		{
			for (size_t i = 0; i < out.size() / 2; ++i)
			{
				if (out[2 * i + 1] != out[2 * i]) { std::fprintf(stderr, "%s: the sides differ\n", c.name.c_str()); return 1; }
				out[i] = out[2 * i];
			}
			out.resize(out.size() / 2);
		}
		const auto path = std::string(argv[3]) + "/" + c.name + ".f32";
		std::FILE* f = std::fopen(path.c_str(), "wb");
		if (!f) { return 1; }
		std::fwrite(out.data(), sizeof(float), out.size(), f);
		std::fclose(f);
		std::printf("%s: %d frames\n", c.name.c_str(), c.frames);
	}
	return 0;
}
