/*
 * GuitarixSelectors.cpp - Guitarix's units that are a choice among others: crybaby, wah, tonestack, power amp
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
#include <vector>

#include "GuitarixUnits.h"

// Guitarix's factories, as Guitarix declares them
#include "gx_faust_plugins.h"
#include "gx_faust_support.h"

namespace lmms::guitarix
{

namespace
{

using Creator = PluginDef* (*)();

/**
 * Guitarix's ModuleSelectorFromList (gx_engine_audio.cpp) as a unit of its own: its modules (Guitarix's units, made
 * with it), its selector (an integer parameter whose values are the modules' ids and names, registered with the
 * modules' parameters: their ids are shared among them, "SA"), and the chosen module processing. Guitarix's engine
 * puts the chosen module in the chain when the selector changes, and clears the chain's modules' state; here the
 * module chosen is cleared as it is chosen (they have no activate_plugin: clear_state does no allocation).
 */
class ModuleSelector : public PluginDef
{
public:
	ModuleSelector(const char* id, const char* name, const char* category, std::vector<Creator> creators,
		const char* selectId, const char* selectName, int (*loader)(const UiBuilder&, int)) :
		PluginDef(),
		m_selectId(selectId),
		m_selectName(selectName)
	{
		version = PLUGINDEF_VERSION;
		this->id = id;
		this->name = name;
		this->category = category;
		load_ui = loader;
		mono_audio = &ModuleSelector::process;
		set_samplerate = &ModuleSelector::initialize;
		register_params = &ModuleSelector::registerParameters;
		delete_instance = &ModuleSelector::destroy;
		for (const auto create : creators) { m_modules.push_back(create()); }
		m_values.reserve(m_modules.size() + 1);
		for (auto* module : m_modules) { m_values.push_back({module->id, module->name}); }
		m_values.push_back({nullptr, nullptr});
	}

	~ModuleSelector()
	{
		for (auto* module : m_modules) { module->delete_instance(module); }
	}

	ModuleSelector(const ModuleSelector&) = delete;
	auto operator=(const ModuleSelector&) -> ModuleSelector& = delete;

private:
	static auto self(PluginDef* plugin) -> ModuleSelector& { return *static_cast<ModuleSelector*>(plugin); }

	static auto registerParameters(const ParamReg& reg) -> int
	{
		auto& selector = self(reg.plugin);
		reg.registerIntVar(selector.m_selectId, selector.m_selectName, "S", "", &selector.m_selected, 0, 0, 0,
			selector.m_values.data());
		for (auto* module : selector.m_modules)
		{
			if (!module->register_params) { continue; }
			auto moduleReg = reg;
			moduleReg.plugin = module;
			module->register_params(moduleReg);
		}
		return 0;
	}

	static void initialize(unsigned int sampleRate, PluginDef* plugin)
	{
		for (auto* module : self(plugin).m_modules)
		{
			if (module->set_samplerate) { module->set_samplerate(sampleRate, module); }
		}
	}

	static void process(int count, float* input, float* output, PluginDef* plugin)
	{
		auto& selector = self(plugin);
		if (selector.m_modules.empty()) { return; }
		const auto chosen = std::clamp(selector.m_selected, 0, static_cast<int>(selector.m_modules.size()) - 1);
		auto* module = selector.m_modules[chosen];
		if (chosen != selector.m_running)
		{
			if (module->clear_state) { module->clear_state(module); }
			selector.m_running = chosen;
		}
		if (module->mono_audio) { module->mono_audio(count, input, output, module); }
		else { std::copy_n(input, count, output); }
	}

	static void destroy(PluginDef* plugin) { delete &self(plugin); }

	const char* m_selectId;
	const char* m_selectName;
	std::vector<PluginDef*> m_modules;
	std::vector<value_pair> m_values;
	int m_selected = 0;
	int m_running = 0; //!< (the module that ran last: its state is as it left it, the others' cleared when chosen)
};


// Their UIs, as Guitarix's engine gives them (gx_engine.cpp: load_crybaby_ui...): their glade files

auto crybabyUi(const UiBuilder& builder, int format) -> int
{
	if (format & UI_FORM_GLADE) { builder.load_glade_file("crybaby_ui.glade"); return 0; }
	return -1;
}

auto wahUi(const UiBuilder& builder, int format) -> int
{
	if (format & UI_FORM_GLADE) { builder.load_glade_file("wah_ui.glade"); return 0; }
	return -1;
}

auto tonestackUi(const UiBuilder& builder, int format) -> int
{
	if (format & UI_FORM_GLADE) { builder.load_glade_file("amp.tonestack_ui.glade"); return 0; }
	return -1;
}

auto powerampUi(const UiBuilder& builder, int format) -> int
{
	if (format & UI_FORM_GLADE) { builder.load_glade_file("poweramps_ui.glade"); return 0; }
	return -1;
}

using namespace gx_engine;

} // namespace


// As Guitarix's engine makes them (gx_engine.cpp: GxEngine's constructor), their modules in its order

auto makeCrybaby() -> PluginDef*
{
	return new ModuleSelector("crybaby", N_("Crybaby"), N_("Guitar Effects"),
		{gx_effects::crybaby::plugin, gx_effects::autowah::plugin}, "crybaby.autowah", N_("select"), crybabyUi);
}

auto makeWah() -> PluginDef*
{
	return new ModuleSelector("wah", N_("Wah"), N_("Guitar Effects"),
		{gx_effects::colwah::plugin, gx_effects::dallaswah::plugin, gx_effects::foxwah::plugin,
			gx_effects::jenwah::plugin, gx_effects::maestrowah::plugin, gx_effects::selwah::plugin,
			gx_effects::voxwah::plugin, gx_effects::rolwah::plugin, gx_effects::colbwah::plugin,
			gx_effects::jenbasswah::plugin},
		"wah.select", N_("select"), wahUi);
}

auto makeTonestack() -> PluginDef*
{
	using namespace gx_tonestacks;
	return new ModuleSelector("amp.tonestack", N_("Tonestack"), N_("Tone Control"),
		{tonestack_default::plugin, tonestack_bassman::plugin, tonestack_twin::plugin, tonestack_princeton::plugin,
			tonestack_jcm800::plugin, tonestack_jcm2000::plugin, tonestack_mlead::plugin, tonestack_m2199::plugin,
			tonestack_ac30::plugin, tonestack_soldano::plugin, tonestack_mesa::plugin, tonestack_jtm45::plugin,
			tonestack_ac15::plugin, tonestack_peavey::plugin, tonestack_ibanez::plugin, tonestack_roland::plugin,
			tonestack_ampeg::plugin, tonestack_ampeg_rev::plugin, tonestack_sovtek::plugin, tonestack_bogner::plugin,
			tonestack_groove::plugin, tonestack_crunch::plugin, tonestack_fender_blues::plugin,
			tonestack_fender_default::plugin, tonestack_fender_deville::plugin, tonestack_gibsen::plugin,
			tonestack_engl::plugin},
		"amp.tonestack.select", N_("select"), tonestackUi);
}

auto makeAmpstack() -> PluginDef*
{
	// (the amp head's: GuitarixAmp.cpp; its UI is the head's)
	using namespace gx_amps;
	return new ModuleSelector("ampstack", N_("Amp"), "",
		{gxamp::plugin, gxamp3::plugin, gxamp14::plugin, gxamp10::plugin, gxamp18::plugin, gxamp2::plugin,
			gxamp9::plugin, gxamp11::plugin, gxamp17::plugin, gxamp13::plugin, gxamp5::plugin, gxamp4::plugin,
			gxamp15::plugin, gxamp12::plugin, gxamp7::plugin, gxamp8::plugin, gxamp16::plugin, gxamp6::plugin,
			gxnoamp::plugin},
		"tube.select", N_("select"), nullptr);
}

auto makePoweramp() -> PluginDef*
{
	using namespace gx_poweramps;
	return new ModuleSelector("poweramp", N_("PowerAmp"), N_("Guitar Effects"),
		{champ::plugin, epiphone::plugin, princeton::plugin, orangedarkterror::plugin, plexiel34::plugin,
			supersonic::plugin},
		"poweramp.mode", N_("select"), powerampUi);
}

} // namespace lmms::guitarix
