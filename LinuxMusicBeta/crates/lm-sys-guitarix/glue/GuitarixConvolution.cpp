/*
 * GuitarixConvolution.cpp - Guitarix's convolving units: the cabinet, the amp impulse (preamp), the presence
 *
 * This file is part of LinuxMusic, based on LMMS - https://lmms.io
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

#include <algorithm>
#include <array>
#include <cmath>
#include <cstring>
#include <memory>
#include <vector>

#include "GuitarixUnits.h"

// Guitarix's: its plugin interface, the zita-convolver and resampler it convolves with, its impulse responses and
// the Faust filters it shapes them with (gx/engine, gx/faust: as they are)
#include "gx_compiler.h"
#include "gx_faust_support.h"
#include "gx_plugin.h"
#include "gx_resampler.h"
#include "zita-convolver.h"

namespace
{

// The impulse responses' description, and the data (gx_internal_plugins.cpp)
struct CabDesc
{
	int ir_count;
	int ir_sr;
	float ir_data[];
};

template<int tab_size>
struct CabDesc_imp
{
	int ir_count;
	int ir_sr;
	float ir_data[tab_size];
	operator CabDesc&() { return *(CabDesc*)this; }
};

#include "gx_cabinet_data.cc"

struct PreDesc
{
	int ir_count;
	int ir_sr;
	float ir_data[];
};

template<int tab_size>
struct PreDesc_imp
{
	int ir_count;
	int ir_sr;
	float ir_data[tab_size];
	operator PreDesc&() { return *(PreDesc*)this; }
};

#include "gx_preamp_data.cc"

} // namespace

#include "cabinet_impulse_former.h"
#include "cabinet_impulse_former.cc"
#include "preamp_impulse_former.h"
#include "preamp_impulse_former.cc"
#include "presence_level.h"
#include "presence_level.cc"

namespace lmms::guitarix
{

namespace
{

struct IrEntry
{
	const char* value_id;
	const char* value_label;
	int count;
	int rate;
	const float* data;
};

template<typename Desc>
auto entry(const char* id, const char* label, Desc& desc) -> IrEntry
{
	return {id, label, desc.ir_count, desc.ir_sr, desc.ir_data};
}

// Guitarix's tables (gx_internal_plugins.cpp: cab_table, pre_table), in its order
const auto s_cabinets = std::array{
	entry("4x12", N_("4x12"), cab_data_4x12),
	entry("2x12", N_("2x12"), cab_data_2x12),
	entry("1x12", N_("1x12"), cab_data_1x12),
	entry("4x10", N_("4x10"), cab_data_4x10),
	entry("2x10", N_("2x10"), cab_data_2x10),
	entry("HighGain", N_("HighGain Style"), cab_data_HighGain),
	entry("Twin", N_("Twin Style"), cab_data_Twin),
	entry("Bassman", N_("Bassman Style"), cab_data_Bassman),
	entry("Marshall", N_("Marshall Style"), cab_data_Marshall),
	entry("AC-30", N_("AC-30 Style"), cab_data_AC30),
	entry("Princeton", N_("Princeton Style"), cab_data_Princeton),
	entry("A2", N_("A2 Style"), cab_data_A2),
	entry("1x15", N_("1x15"), cab_data_1x15),
	entry("Mesa Boogie", N_("Mesa Boogie Style"), cab_data_mesa),
	entry("Briliant", N_("Briliant"), cab_data_briliant),
	entry("Vitalize", N_("Vitalize"), cab_data_vitalize),
	entry("Charisma", N_("Charisma"), cab_data_charisma),
};

const auto s_preamps = std::array{
	entry("AC30", N_("AC30 Style"), pre_data_ac30),
	entry("Bassman", N_("Bassman Style"), pre_data_bassman),
	entry("Tube", N_("Tube Style"), pre_data_tube),
	entry("Fender", N_("Fender Style"), pre_data_fender),
	entry("JCM800", N_("JCM800 Style"), pre_data_jcm800),
	entry("JTM45", N_("JTM45 Style"), pre_data_jtm45),
	entry("Mesa Boogie", N_("Mesa Boogie Style"), pre_data_mesaboogie),
	entry("Boutique", N_("Boutique Style"), pre_data_boutique),
	entry("Ampeg", N_("Ampeg Style"), pre_data_ampeg),
	entry("Rectifier", N_("Rectifier Style"), pre_data_rectifier),
};

//! The rate Guitarix convolves at: the rate times the integer factor that brings it nearest 96 kHz, from below
//! (FixedBaseConvolver::init: bz = 96000 / rate)
auto upsampling(unsigned int rate) -> unsigned int
{
	return (std::max)(1u, 96000u / (std::max)(1u, rate));
}

/**
 * Guitarix's GxSimpleConvolver (gx_convolver.cpp) on the zita-convolver: an impulse response (resampled to the rate
 * it convolves at) convolved with what comes, in place. Here in one partition level, the block's size (its quantum,
 * its smallest and largest partition the same): the zita-convolver then does all of it in the calling thread, with
 * no threads of its own to wait for (an export is the same however fast it renders). A block too small for a
 * partition (under 64 frames) uses partitions of 64 and the convolver's threads, waited for (sync).
 */
class Convolver : private Convproc
{
public:
	~Convolver() { stop(); }

	//! Sets it up for @p count samples of @p ir at @p irRate, convolving at @p rate in blocks of @p block [main thread]
	auto configure(int count, const float* ir, unsigned int irRate, unsigned int rate, unsigned int block) -> bool
	{
		stop();
		auto resampled = std::unique_ptr<float[]>{};
		auto data = const_cast<float*>(ir);
		if (irRate != rate)
		{
			auto resampler = gx_resample::BufferResampler{};
			resampled.reset(resampler.process(static_cast<int>(irRate), count, data, static_cast<int>(rate), &count));
			if (!resampled) { return false; }
			data = resampled.get();
		}
		m_block = block;
		const auto partition = std::max<unsigned int>(block, Convproc::MINPART);
		m_sync = partition != block;
		if (Convproc::configure(1, 1, static_cast<unsigned int>(count), block, partition, partition) != 0) { return false; }
		if (impdata_create(0, 0, 1, data, 0, count) != 0) { return false; }
		if (start_process(0, 0) != 0) { return false; }
		return true;
	}

	//! Convolves @p count samples (a multiple of its block) in place [audio thread]
	void compute(int count, float* buffer)
	{
		if (state() != Convproc::ST_PROC || m_block == 0) { return; }
		for (int done = 0; done + static_cast<int>(m_block) <= count; done += static_cast<int>(m_block))
		{
			std::memcpy(inpdata(0), buffer + done, m_block * sizeof(float));
			process(m_sync);
			std::memcpy(buffer + done, outdata(0), m_block * sizeof(float));
		}
	}

	//! Sets @p count samples of @p ir anew, the same length it was set up for [main thread, no period rendered]
	auto update(int count, const float* ir, unsigned int irRate, unsigned int rate) -> bool
	{
		if (state() == Convproc::ST_IDLE) { return false; }
		auto resampled = std::unique_ptr<float[]>{};
		auto data = const_cast<float*>(ir);
		if (irRate != rate)
		{
			auto resampler = gx_resample::BufferResampler{};
			resampled.reset(resampler.process(static_cast<int>(irRate), count, data, static_cast<int>(rate), &count));
			if (!resampled) { return false; }
			data = resampled.get();
		}
		return impdata_update(0, 0, 1, data, 0, count) == 0;
	}

	//! Forgets the signal it holds (LinuxMusicBeta: a panic's silence) [audio thread: no allocation]
	void clear()
	{
		if (state() == Convproc::ST_PROC) { Convproc::reset(); }
	}

private:
	void stop()
	{
		if (state() == Convproc::ST_PROC) { stop_process(); }
		if (state() != Convproc::ST_IDLE) { cleanup(); }
		m_block = 0;
	}

	unsigned int m_block = 0;
	bool m_sync = false;
};


/**
 * What Guitarix's FixedBaseConvolver and its cabinet and preamp make of it (gx_internal_plugins.cpp): the signal
 * upsampled (to near 96 kHz), convolved with the chosen impulse response, downsampled, then through a Faust filter
 * (bass, treble, level). The impulse response is set when the unit starts, and again when another is chosen
 * (Guitarix checks every 200 ms on its UI's thread: here the effect calls update() on the main thread, the audio
 * thread kept out).
 */
template<typename Former>
class IrUnit : public PluginDef
{
public:
	IrUnit(const char* id, const char* name, const char* selectId, const char* levelId, float levelLow,
		float levelUp, float levelStep, const char* bassId, const char* trebleId, const IrEntry* table, int size,
		int (*loader)(const UiBuilder&, int)) :
		PluginDef(),
		m_selectId(selectId), m_levelId(levelId), m_bassId(bassId), m_trebleId(trebleId),
		m_levelLow(levelLow), m_levelUp(levelUp), m_levelStep(levelStep),
		m_table(table), m_size(size)
	{
		version = PLUGINDEF_VERSION;
		this->id = id;
		this->name = name;
		category = N_("Tone Control");
		load_ui = loader;
		mono_audio = &IrUnit::process;
		set_samplerate = &IrUnit::initialize;
		register_params = &IrUnit::registerParameters;
		delete_instance = &IrUnit::destroy;
		clear_state = &IrUnit::clear; // (LinuxMusicBeta)
		for (int i = 0; i < size; ++i) { m_names.push_back({table[i].value_id, table[i].value_label}); }
		m_names.push_back({nullptr, nullptr});
	}

	//! Its impulse response anew if another is chosen [main thread, no period rendered]
	static void update(PluginDef* plugin)
	{
		auto& self = *static_cast<IrUnit*>(plugin);
		if (self.m_rate != 0 && self.m_current != self.chosen()) { self.configure(); }
	}

private:
	auto chosen() const -> int { return std::clamp(m_selected, 0, m_size - 1); }

	void configure()
	{
		m_current = chosen();
		const auto& ir = m_table[m_current];
		m_former.clear_state_f();
		m_convolver.configure(ir.count, ir.data, static_cast<unsigned int>(ir.rate), m_rate * m_factor,
			m_block * m_factor);
	}

	static void initialize(unsigned int rate, PluginDef* plugin)
	{
		auto& self = *static_cast<IrUnit*>(plugin);
		self.m_rate = rate;
		self.m_factor = upsampling(rate);
		self.m_block = blockFrames();
		self.m_resampler.setup(static_cast<int>(rate), static_cast<int>(self.m_factor * rate));
		self.m_former.init(rate);
		// (room for the most a call upsamples to: the effect's chunk)
		self.m_upsampled.assign(static_cast<std::size_t>(self.m_resampler.max_out_count(256)) + 16, 0.f);
		self.configure();
	}

	static void process(int count, float* input, float* output, PluginDef* plugin)
	{
		auto& self = *static_cast<IrUnit*>(plugin);
		// (Guitarix's chain processes in place: its run_cab_conf takes the output as its input)
		if (input != output) { std::copy_n(input, count, output); }
		const auto upsampled = self.m_resampler.up(count, output, self.m_upsampled.data());
		self.m_convolver.compute(upsampled, self.m_upsampled.data());
		self.m_resampler.down(self.m_upsampled.data(), output);
		self.m_former.compute(count, output, output);
	}

	static auto registerParameters(const ParamReg& reg) -> int
	{
		auto& self = *static_cast<IrUnit*>(reg.plugin);
		reg.registerIntVar(self.m_selectId, "select", "B", "", &self.m_selected, 0, 0, 0, self.m_names.data());
		reg.registerFloatVar(self.m_levelId, N_("Level"), "S", N_("Level"), &self.m_level, 1.0f, self.m_levelLow,
			self.m_levelUp, self.m_levelStep, nullptr);
		reg.registerFloatVar(self.m_bassId, N_("Bass"), "S", N_("Bass"), &self.m_bass, 0.f, -10.f, 10.f, 0.5f,
			nullptr);
		reg.registerFloatVar(self.m_trebleId, N_("Treble"), "S", N_("Treble"), &self.m_treble, 0.f, -10.f, 10.f,
			0.5f, nullptr);
		self.m_former.register_par(reg);
		return 0;
	}

	static void destroy(PluginDef* plugin) { delete static_cast<IrUnit*>(plugin); }

	//! (LinuxMusicBeta: a panic's silence) the filter's and the convolver's memory
	static void clear(PluginDef* plugin)
	{
		auto& self = *static_cast<IrUnit*>(plugin);
		self.m_former.clear_state_f();
		self.m_convolver.clear();
	}

	const char* m_selectId;
	const char* m_levelId;
	const char* m_bassId;
	const char* m_trebleId;
	float m_levelLow, m_levelUp, m_levelStep;
	const IrEntry* m_table;
	int m_size;
	std::vector<value_pair> m_names;

	int m_selected = 0;
	int m_current = -1;
	float m_level = 1.f;
	float m_bass = 0.f;
	float m_treble = 0.f;

	unsigned int m_rate = 0;
	unsigned int m_factor = 1;
	unsigned int m_block = 256;
	gx_resample::FixedRateResampler m_resampler;
	std::vector<float> m_upsampled;
	Convolver m_convolver;
	Former m_former;
};


/**
 * Guitarix's ContrastConvolver (gx_internal_plugins.cpp), the amp head's presence: its impulse response shaped by the
 * presence level (a Faust filter over it), convolved at near 96 kHz, crossfaded with the dry signal by the level (its
 * DELT_PRESENCE_HACK). A new level shapes the response anew [main thread: update()].
 */
class Contrast : public PluginDef
{
public:
	Contrast() :
		PluginDef() // (value-initialized: its function pointers not set here are null)
	{
		version = PLUGINDEF_VERSION;
		id = "con";
		name = N_("Contrast convolver");
		mono_audio = &Contrast::process;
		set_samplerate = &Contrast::initialize;
		register_params = &Contrast::registerParameters;
		delete_instance = [](PluginDef* plugin) { delete static_cast<Contrast*>(plugin); };
		// (LinuxMusicBeta: a panic's silence) the convolver's memory
		clear_state = [](PluginDef* plugin) { static_cast<Contrast*>(plugin)->m_convolver.clear(); };
	}

	static void update(PluginDef* plugin)
	{
		auto& self = *static_cast<Contrast*>(plugin);
		if (self.m_rate != 0 && std::abs(self.m_sum - self.m_level) > 0.01f) { self.shape(false); }
	}

private:
	static constexpr float NoSum = 1e10f;

	//! The response shaped by the level, set up anew (@p configure) or updated
	void shape(bool configure)
	{
		auto shaped = std::vector<float>(static_cast<std::size_t>(contrast_ir_desc.ir_count));
		m_presence.compute(contrast_ir_desc.ir_count, contrast_ir_desc.ir_data, shaped.data());
		const auto irRate = static_cast<unsigned int>(contrast_ir_desc.ir_sr);
		if (configure)
		{
			m_convolver.configure(contrast_ir_desc.ir_count, shaped.data(), irRate, m_rate * m_factor,
				m_block * m_factor);
		}
		else { m_convolver.update(contrast_ir_desc.ir_count, shaped.data(), irRate, m_rate * m_factor); }
		m_sum = m_level;
	}

	static void initialize(unsigned int rate, PluginDef* plugin)
	{
		auto& self = *static_cast<Contrast*>(plugin);
		self.m_rate = rate;
		self.m_factor = upsampling(rate);
		self.m_block = blockFrames();
		self.m_resampler.setup(static_cast<int>(rate), static_cast<int>(self.m_factor * rate));
		self.m_presence.init(static_cast<unsigned int>(contrast_ir_desc.ir_sr));
		const auto room = static_cast<std::size_t>(self.m_resampler.max_out_count(256)) + 16;
		self.m_wet.assign(room, 0.f);
		self.m_dry.assign(room, 0.f);
		self.shape(true);
	}

	static void process(int count, float* input, float* output, PluginDef* plugin)
	{
		auto& self = *static_cast<Contrast*>(plugin);
		if (input != output) { std::copy_n(input, count, output); }
		const auto total = self.m_resampler.max_out_count(count);
		const auto upsampled = self.m_resampler.up(count, output, self.m_wet.data());
		std::copy_n(self.m_wet.data(), total, self.m_dry.data());
		// (crossfade over the knob's whole range, 0...5: smoothstep, equal power)
		const auto t = (std::min)(self.m_level * 0.2f, 1.0f);
		const auto mix = t * t * (3.0f - 2.0f * t);
		const auto halfPi = 1.57079632679f;
		const auto dryGain = std::cos(halfPi * mix);
		const auto wetGain = std::sin(halfPi * mix) / 2;
		self.m_convolver.compute(upsampled, self.m_wet.data());
		for (int i = 0; i < total; ++i) { self.m_wet[i] = self.m_dry[i] * dryGain + self.m_wet[i] * wetGain; }
		self.m_resampler.down(self.m_wet.data(), output);
	}

	static auto registerParameters(const ParamReg& reg) -> int
	{
		auto& self = *static_cast<Contrast*>(reg.plugin);
		// (Guitarix's: its granularity changed from 0.5 to 0.1, "too coarse and jumpy")
		reg.registerFloatVar("con.Level", "", "S", "", &self.m_level, 1.0f, 0.1f, 5.0f, 0.1f, nullptr);
		self.m_presence.register_par(reg);
		return 0;
	}

	float m_level = 1.f;
	float m_sum = NoSum;
	unsigned int m_rate = 0;
	unsigned int m_factor = 1;
	unsigned int m_block = 256;
	gx_resample::FixedRateResampler m_resampler;
	std::vector<float> m_wet;
	std::vector<float> m_dry;
	Convolver m_convolver;
	presence_level::Dsp m_presence;
};


// Their UIs, as Guitarix gives them (gx_internal_plugins.cpp: cab_load_ui, pre_load_ui): their glade files

auto cabinetUi(const UiBuilder& builder, int format) -> int
{
	if (format & UI_FORM_GLADE) { builder.load_glade_file("cabinet_ui.glade"); return 0; }
	return -1;
}

auto preampUi(const UiBuilder& builder, int format) -> int
{
	if (format & UI_FORM_GLADE) { builder.load_glade_file("ampimpulse_ui.glade"); return 0; }
	return -1;
}

using Cabinet = IrUnit<cabinet_impulse_former::Dsp>;
using Preamp = IrUnit<preamp_impulse_former::Dsp>;

} // namespace


auto makeCabinet() -> PluginDef*
{
	return new Cabinet("cab", N_("Cabinet"), "cab.select", "cab.Level", 0.5f, 5.f, 0.5f, "cab.bass", "cab.treble",
		s_cabinets.data(), static_cast<int>(s_cabinets.size()), cabinetUi);
}

auto makePreamp() -> PluginDef*
{
	return new Preamp("pre", N_("Amp Impulse"), "pre.select", "pre.Level", 0.1f, 2.1f, 0.1f, "pre.bass",
		"pre.treble", s_preamps.data(), static_cast<int>(s_preamps.size()), preampUi);
}

auto makeContrast() -> PluginDef*
{
	return new Contrast;
}

void updateContrast(PluginDef* plugin)
{
	Contrast::update(plugin);
}

void updateCabinet(PluginDef* plugin)
{
	Cabinet::update(plugin);
}

void updatePreamp(PluginDef* plugin)
{
	Preamp::update(plugin);
}

} // namespace lmms::guitarix
