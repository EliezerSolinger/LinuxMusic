/*
 * shim.cpp - the C interface of lm-sys-guitarix: Guitarix's units (GuitarixUnits.cpp) for Rust
 *
 * This file is part of LinuxMusic - GPL-2.0-or-later (the crate it builds is GPL-3.0-or-later: see Cargo.toml).
 *
 * Every function is a thin call of a unit's PluginDef. C++ exceptions never cross: nothing here throws (the units'
 * code does not use exceptions; a std::bad_alloc while starting would end the process, as in the legacy program).
 */

#include <cstddef>
#include <mutex>

#include <fftw3.h>

#include "GuitarixUnits.h"
#include "gx_plugin.h"

extern "C"
{

//! A parameter as the unit registers it (one call per registration: a unit may register an id more than once)
struct LmgxParam
{
	const char* id;
	const char* name;
	const char* kind; //!< Guitarix's tp ("S", "SL", "B", "SLMO"...)
	int type; //!< 0: float, 1: int, 2: bool
	void* var;
	float value;
	float low;
	float up;
	float step;
	const value_pair* values; //!< a selector's values (ends with a null id), or null
};

typedef void (*LmgxRegisterFn)(void* context, const LmgxParam* param);

} // extern "C"

namespace
{

//! FFTW's planner, which zita-convolver and the Detune call when a unit starts, is not thread safe, and effects are
//! built on several threads: made so for the whole process, once, before any unit is made (fftw3f_threads)
void plannerThreadSafe()
{
	static std::once_flag once;
	std::call_once(once, [] { fftwf_make_planner_thread_safe(); });
}

//! The registration under way, on this thread
thread_local LmgxRegisterFn t_register = nullptr;
thread_local void* t_context = nullptr;

void report(const char* id, const char* name, const char* kind, int type, void* var, float value, float low,
	float up, float step, const value_pair* values)
{
	auto param = LmgxParam{id, name, kind ? kind : "", type, var, value, low, up, step, values};
	if (t_register) { t_register(t_context, &param); }
}

auto registerFloat(const char* id, const char* name, const char* kind, const char*, float* var, float value,
	float low, float up, float step, const value_pair* values) -> float*
{
	*var = value;
	report(id, name, kind, 0, var, value, low, up, step, values);
	return var;
}

auto registerInt(const char* id, const char* name, const char* kind, const char*, int* var, int value, int low,
	int up, const value_pair* values) -> int*
{
	*var = value;
	report(id, name, kind, 1, var, static_cast<float>(value), static_cast<float>(low), static_cast<float>(up), 1.f,
		values);
	return var;
}

auto registerBool(const char* id, const char* name, const char* kind, const char*, bool* var, bool value) -> bool*
{
	*var = value;
	report(id, name, kind, 2, var, value ? 1.f : 0.f, 0.f, 1.f, 1.f, nullptr);
	return var;
}

} // namespace

extern "C"
{

int lmgx_unit_count()
{
	return static_cast<int>(lmms::guitarix::units().size());
}

const char* lmgx_unit_id(int unit)
{
	return lmms::guitarix::units()[static_cast<std::size_t>(unit)].id;
}

int lmgx_unit_has_update(int unit)
{
	return lmms::guitarix::units()[static_cast<std::size_t>(unit)].update != nullptr;
}

PluginDef* lmgx_make(int unit)
{
	plannerThreadSafe();
	return lmms::guitarix::units()[static_cast<std::size_t>(unit)].make();
}

void lmgx_describe(PluginDef* def, const char** name, const char** category, int* stereo)
{
	*name = def->name ? def->name : def->id;
	*category = def->category ? def->category : "";
	*stereo = def->stereo_audio != nullptr;
}

void lmgx_register(PluginDef* def, void* context, LmgxRegisterFn fn)
{
	if (!def->register_params) { return; }
	t_register = fn;
	t_context = context;
	auto reg = ParamReg{};
	reg.plugin = def;
	reg.registerFloatVar = &registerFloat;
	reg.registerIntVar = &registerInt;
	reg.registerBoolVar = &registerBool;
	def->register_params(reg);
	t_register = nullptr;
	t_context = nullptr;
}

//! As GuitarixEffect::initialize: the rate (which clears the state), then the activation, in blocks of @p block
void lmgx_start(PluginDef* def, unsigned int rate, unsigned int block)
{
	lmms::guitarix::setBlockFrames(block);
	if (def->set_samplerate) { def->set_samplerate(rate, def); }
	if (def->activate_plugin) { def->activate_plugin(true, def); }
}

//! The unit's update (a new impulse response, a new latency setting), in blocks of @p block
void lmgx_update(int unit, PluginDef* def, unsigned int block)
{
	const auto update = lmms::guitarix::units()[static_cast<std::size_t>(unit)].update;
	if (!update) { return; }
	lmms::guitarix::setBlockFrames(block);
	update(def);
}

void lmgx_mono(PluginDef* def, int count, float* input, float* output)
{
	def->mono_audio(count, input, output, def);
}

void lmgx_stereo(PluginDef* def, int count, float* input0, float* input1, float* output0, float* output1)
{
	def->stereo_audio(count, input0, input1, output0, output1, def);
}

int lmgx_clear(PluginDef* def)
{
	if (!def->clear_state) { return 0; }
	def->clear_state(def);
	return 1;
}

void lmgx_delete(PluginDef* def)
{
	if (def->activate_plugin) { def->activate_plugin(false, def); }
	def->delete_instance(def);
}

} // extern "C"
