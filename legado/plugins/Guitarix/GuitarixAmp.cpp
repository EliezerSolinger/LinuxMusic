/*
 * GuitarixAmp.cpp - Guitarix's amp head: its fixed chain, as its engine runs it, as one unit
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

#include "GuitarixUnits.h"

// Guitarix's factories, as Guitarix declares them
#include "gx_faust_plugins.h"
#include "gx_faust_support.h"

namespace lmms::guitarix
{

namespace
{

using namespace gx_engine;

/**
 * Guitarix's amp: what its engine puts around the rack (gx_engine.cpp: load_static_plugins), as one unit. Its mono
 * chain, here once for each channel: the noise gate's input level (NoiseGate), the noise shaper, the amps (its
 * ampstack: 19 tube models), the soft clip, the bass booster, the amp's output, the presence (its contrast
 * convolver) and the noise gate's output (OutPutGate); then its stereo chain: the feedback reverb (gxfeed), the
 * master level, the balance, the fizz remover, the output level meters (MaxLevel) and the limiter. A module with an
 * on/off (its "<id>.on_off") runs when on, its state cleared as it is turned on (as Guitarix's engine clears the
 * chain's); they start as Guitarix's factory default preset has them. Its UI is Guitarix's own: its amp box and the
 * amp details under it.
 */
class AmpHead : public PluginDef
{
public:
	AmpHead() :
		PluginDef() // (value-initialized: its function pointers not set here are null)
	{
		version = PLUGINDEF_VERSION;
		id = "ampstack";
		name = N_("Amp");
		category = N_("Guitar Effects");
		load_ui = &AmpHead::ui;
		stereo_audio = &AmpHead::process;
		set_samplerate = &AmpHead::initialize;
		register_params = &AmpHead::registerParameters;
		delete_instance = [](PluginDef* plugin) { delete static_cast<AmpHead*>(plugin); };
		for (auto& channel : m_channels)
		{
			channel.shaper = gx_effects::noise_shaper::plugin();
			channel.amps = makeAmpstack();
			channel.clip = gx_effects::softclip::plugin();
			channel.bassBooster = gx_effects::bassbooster::plugin();
			channel.ampOut = gx_effects::gx_ampout::plugin();
			channel.presence = makeContrast();
		}
		m_feed = gx_effects::gxfeed::plugin();
		m_master = gx_effects::gx_outputlevel::plugin();
		m_balance = balance::plugin();
		m_fizz = fizz_remover::plugin();
		m_limiter = hardlim::plugin();
	}

	~AmpHead()
	{
		for (auto* module : modules()) { module->delete_instance(module); }
	}

	AmpHead(const AmpHead&) = delete;
	auto operator=(const AmpHead&) -> AmpHead& = delete;

	//! The presence's response, shaped anew if its level changed [main thread]
	static void update(PluginDef* plugin)
	{
		for (auto& channel : static_cast<AmpHead*>(plugin)->m_channels) { updateContrast(channel.presence); }
	}

private:
	struct Channel
	{
		PluginDef* shaper = nullptr;
		PluginDef* amps = nullptr;
		PluginDef* clip = nullptr;
		PluginDef* bassBooster = nullptr;
		PluginDef* ampOut = nullptr;
		PluginDef* presence = nullptr;
		float gate = 1.f; //!< (NoiseGate's ngate)
	};

	//! A module's on/off, as Guitarix registers it ("<id>.on_off"), and whether it was on (to clear it when on)
	struct Switch
	{
		const char* id;
		bool on;
		bool wasOn;
	};

	auto modules() -> std::vector<PluginDef*>
	{
		auto all = std::vector<PluginDef*>{};
		for (auto& channel : m_channels)
		{
			all.insert(all.end(),
				{channel.shaper, channel.amps, channel.clip, channel.bassBooster, channel.ampOut, channel.presence});
		}
		all.insert(all.end(), {m_feed, m_master, m_balance, m_fizz, m_limiter});
		return all;
	}

	static auto ui(const UiBuilder& builder, int format) -> int
	{
		if (!(format & UI_FORM_GLADE)) { return -1; }
		// (two parts of Guitarix's main window: "<file>#<object>", see GuitarixLayout.cpp)
		builder.load_glade_file("ampbox.glade#amp_background");
		builder.load_glade_file("mainpanel.glade#ampdetail_normal");
		return 0;
	}

	static void initialize(unsigned int rate, PluginDef* plugin)
	{
		auto& self = *static_cast<AmpHead*>(plugin);
		for (auto* module : self.modules())
		{
			if (module->set_samplerate) { module->set_samplerate(rate, module); }
		}
		for (auto& channel : self.m_channels) { channel.gate = 1.f; }
		self.m_maxLevel = {};
	}

	static auto registerParameters(const ParamReg& reg) -> int
	{
		auto& self = *static_cast<AmpHead*>(reg.plugin);
		for (auto& on : self.m_switches)
		{
			reg.registerBoolVar(on.id, N_("on/off"), "B", "", &on.on, on.on);
		}
		// (NoiseGate's, MaxLevel's: gx_internal_plugins.cpp, gx_internal_ui_plugins.cpp)
		reg.registerFloatVar("noise_gate.threshold", N_("Threshold"), "S", "", &self.m_threshold, 0.017f, 0.01f,
			0.31f, 0.001f, nullptr);
		reg.registerFloatVar("maxlevel.left", "", "SLMO", N_("Overall Rack output"), &self.m_maxLevel[0], 0.f, 0.f,
			1.f, 0.f, nullptr);
		reg.registerFloatVar("maxlevel.right", "", "SLMO", N_("Overall Rack output"), &self.m_maxLevel[1], 0.f, 0.f,
			1.f, 0.f, nullptr);
		for (auto* module : self.modules())
		{
			if (!module->register_params) { continue; }
			auto moduleReg = reg;
			moduleReg.plugin = module;
			module->register_params(moduleReg);
		}
		return 0;
	}

	//! Whether switch @p index is on; turned on since the last period, @p modules' state is cleared first
	auto running(std::size_t index, std::initializer_list<PluginDef*> modules) -> bool
	{
		auto& on = m_switches[index];
		if (on.on && !on.wasOn)
		{
			for (auto* module : modules)
			{
				if (module && module->clear_state) { module->clear_state(module); }
			}
		}
		on.wasOn = on.on;
		return on.on;
	}

	static void mono(PluginDef* module, int count, float* buffer)
	{
		if (module->mono_audio) { module->mono_audio(count, buffer, buffer, module); }
	}

	static void process(int count, float* input0, float* input1, float* output0, float* output1, PluginDef* plugin)
	{
		auto& self = *static_cast<AmpHead*>(plugin);
		std::copy_n(input0, count, output0);
		std::copy_n(input1, count, output1);
		auto& left = self.m_channels[0];
		auto& right = self.m_channels[1];
		const auto gate = self.running(NoiseGate, {});
		const auto shaper = self.running(Shaper, {left.shaper, right.shaper});
		const auto clip = self.running(Clip, {left.clip, right.clip});
		const auto bassBoost = self.running(BassBoost, {left.bassBooster, right.bassBooster});
		const auto ampOut = self.running(AmpOut, {left.ampOut, right.ampOut});
		const auto presence = self.running(Presence, {left.presence, right.presence});
		const auto fizz = self.running(Fizz, {self.m_fizz});

		std::array<float*, 2> outputs = {output0, output1};
		for (std::size_t c = 0; c < 2; ++c)
		{
			auto& channel = self.m_channels[c];
			auto* buffer = outputs[c];
			// NoiseGate::inputlevel_process: the gate opens on a period louder than the threshold, else closes slowly
			if (gate)
			{
				auto sum = 0.f;
				for (int i = 0; i < count; ++i) { sum += buffer[i] * buffer[i]; }
				const auto level = self.m_threshold * 0.1f;
				if (sum / static_cast<float>(count) > level * level) { channel.gate = 1.f; }
				else if (channel.gate > 0.01f) { channel.gate *= 0.996f; }
			}
			if (shaper) { mono(channel.shaper, count, buffer); }
			mono(channel.amps, count, buffer);
			if (clip) { mono(channel.clip, count, buffer); }
			if (bassBoost) { mono(channel.bassBooster, count, buffer); }
			if (ampOut) { mono(channel.ampOut, count, buffer); }
			if (presence) { mono(channel.presence, count, buffer); }
			// OutPutGate::outputgate_process
			if (gate)
			{
				for (int i = 0; i < count; ++i) { buffer[i] *= channel.gate; }
			}
		}

		const auto stereo = [&](PluginDef* module) {
			if (module->stereo_audio) { module->stereo_audio(count, output0, output1, output0, output1, module); }
		};
		stereo(self.m_feed);
		stereo(self.m_master);
		stereo(self.m_balance);
		if (fizz) { stereo(self.m_fizz); }
		// MaxLevel::process: the loudest since the meters read it
		for (std::size_t c = 0; c < 2; ++c)
		{
			for (int i = 0; i < count; ++i)
			{
				self.m_maxLevel[c] = (std::max)(self.m_maxLevel[c], std::abs(outputs[c][i]));
			}
		}
		stereo(self.m_limiter);
	}

	enum : std::size_t { NoiseGate, Shaper, Clip, BassBoost, AmpOut, Presence, Fizz };
	// (as Guitarix's factory default preset has them)
	std::array<Switch, 7> m_switches = {{
		{"noise_gate.on_off", false, false},
		{"shaper.on_off", false, false},
		{"amp.clip.on_off", false, false},
		{"amp.bass_boost.on_off", true, false},
		{"amp.on_off", false, false},
		{"con.on_off", true, false},
		{"antyfizz.on_off", false, false},
	}};
	float m_threshold = 0.017f;
	std::array<float, 2> m_maxLevel = {};

	std::array<Channel, 2> m_channels;
	PluginDef* m_feed = nullptr;
	PluginDef* m_master = nullptr;
	PluginDef* m_balance = nullptr;
	PluginDef* m_fizz = nullptr;
	PluginDef* m_limiter = nullptr;
};

} // namespace


auto makeAmpHead() -> PluginDef*
{
	return new AmpHead;
}

void updateAmpHead(PluginDef* plugin)
{
	AmpHead::update(plugin);
}

} // namespace lmms::guitarix
