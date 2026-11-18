/*
 * GuitarixEffect.cpp - a Guitarix unit as a LinuxMusic effect
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

#include "GuitarixEffect.h"

#include <QCoreApplication>
#include <QDomElement>
#include <QLabel>
#include <algorithm>
#include <cmath>

#include "AudioEngine.h"
#include "Engine.h"
#include "GuitarixControlDialog.h"
#include "GuitarixUnits.h"
#include "Mixer.h"
#include "embed.h"
#include "plugin_export.h"

#include "gx_plugin.h"

namespace lmms
{

namespace
{

//! Lists the units as the plugin's sub-plugins, grouped as Guitarix groups them
class GuitarixSubPluginFeatures : public Plugin::Descriptor::SubPluginFeatures
{
public:
	GuitarixSubPluginFeatures() : SubPluginFeatures(Plugin::Type::Effect) {}

	void listSubPluginKeys(const Plugin::Descriptor* desc, KeyList& keys) const override
	{
		for (const auto& unit : guitarix::units())
		{
			auto attributes = Key::AttributeMap{};
			attributes["unit"] = unit.id;
			keys.push_back(Key{desc, nameOf(unit), attributes});
		}
	}

	void fillDescriptionWidget(QWidget* parent, const Key* key) const override
	{
		const auto* unit = guitarix::unit(key->attributes.value("unit").toUtf8().constData());
		auto text = QString{"<p><b>%1</b></p><p>%2</p><p>%3</p>"}.arg(
			unit ? nameOf(*unit) : key->name,
			unit ? categoryOf(*unit) : QString{},
			QCoreApplication::translate("GuitarixEffect", "From Guitarix (guitarix.org), by Hermann Meyer, James "
				"Warden, Andreas Degert, Pete Shorthose and its other authors: its own code, built in."));
		auto label = new QLabel(text, parent);
		label->setWordWrap(true);
	}

	//! The unit's category, as Guitarix gives it (distortion, fuzz, modulation...), translated
	static auto categoryOf(const guitarix::Unit& unit) -> QString
	{
		auto* def = unit.make();
		const auto category = QCoreApplication::translate("GuitarixEffect", def->category);
		def->delete_instance(def);
		return category;
	}

	//! The unit's name, as Guitarix gives it
	static auto nameOf(const guitarix::Unit& unit) -> QString
	{
		auto* def = unit.make();
		const auto name = QString::fromUtf8(def->name);
		def->delete_instance(def);
		return name;
	}

private:
	auto displayName(const Key& key) const -> QString override
	{
		if (const auto* unit = guitarix::unit(key.attributes.value("unit").toUtf8().constData())) { return nameOf(*unit); }
		return key.name;
	}
};

} // namespace


extern "C"
{

Plugin::Descriptor PLUGIN_EXPORT guitarix_plugin_descriptor =
{
	LMMS_STRINGIFY(PLUGIN_NAME),
	"Guitarix",
	QT_TRANSLATE_NOOP("PluginBrowser", "Guitarix's pedals, amps and cabinets, built in"),
	"Hermann Meyer, James Warden, Andreas Degert, Pete Shorthose (Guitarix)",
	0x0100,
	Plugin::Type::Effect,
	new PixmapLoader("lmms-plugin-logo"),
	nullptr,
	new GuitarixSubPluginFeatures,
};

}


namespace
{

//! Holds a unit's file state's lock (spinning: on the audio thread; held for an effect's period at most), if it
//! has one
class FileStateLock
{
public:
	explicit FileStateLock(const guitarix::FileState* state) : m_lock(state ? state->lock : nullptr)
	{
		if (!m_lock) { return; }
		while (m_lock->test_and_set(std::memory_order_acquire)) {}
	}
	~FileStateLock()
	{
		if (m_lock) { m_lock->clear(std::memory_order_release); }
	}
	FileStateLock(const FileStateLock&) = delete;
	auto operator=(const FileStateLock&) -> FileStateLock& = delete;

private:
	std::atomic_flag* m_lock;
};

//! The parameters being registered (Guitarix's ParamReg takes plain functions: the effect being made, on the main
//! thread, and which instance)
GuitarixControls* s_registering = nullptr;

auto parameterFor(const char* id) -> GuitarixParameter*
{
	for (auto& parameter : s_registering->parameters())
	{
		if (parameter.id == QString::fromUtf8(id)) { return &parameter; }
	}
	return nullptr;
}

auto newParameter(const char* id, const char* name, const char* kind, float value, float low, float up, float step,
	Model* parent) -> GuitarixParameter&
{
	auto parameter = GuitarixParameter{};
	parameter.id = QString::fromUtf8(id);
	// (as Guitarix names one registered without a name: the id's last part, its first letter upper case)
	parameter.name = name && *name ? QString::fromUtf8(name) : parameter.id.section('.', -1);
	if (!(name && *name) && !parameter.name.isEmpty()) { parameter.name[0] = parameter.name[0].toUpper(); }
	parameter.kind = QString::fromUtf8(kind ? kind : "");
	parameter.lower = low;
	parameter.upper = up;
	parameter.step = step;
	// (its options after the first letter: O output, not saved; o output, saved; s not saved; L log; N no MIDI)
	if (parameter.kind.mid(1).contains('O') || parameter.kind.mid(1).contains('o'))
	{
		// (an output: what the unit measures, not something to set, automate or save)
		// (at the bottom of its range until the unit writes it: a meter that has measured nothing)
		parameter.output = std::make_unique<std::atomic<float>>(std::min(low, up));
		return s_registering->parameters().emplace_back(std::move(parameter));
	}
	// (a log slider's step is a factor, not a step: steps of a hundredth of the range)
	const auto linearStep = parameter.kind.contains('L') || step <= 0 ? (up - low) / 100.f : step;
	parameter.model = std::make_unique<FloatModel>(value, low, up, linearStep, parent, parameter.name);
	if (parameter.kind.contains('L') && low > 0) { parameter.model->setScaleLogarithmic(true); }
	return s_registering->parameters().emplace_back(std::move(parameter));
}

auto registerFloat(const char* id, const char* name, const char* kind, const char*, float* var, float value,
	float low, float up, float step, const value_pair* values) -> float*
{
	auto* parameter = parameterFor(id);
	if (!parameter)
	{
		parameter = &newParameter(id, name, kind, value, low, up, step, s_registering);
		for (auto pair = values; pair && pair->value_id; ++pair)
		{
			parameter->labels << QString::fromUtf8(pair->value_label ? pair->value_label : pair->value_id);
		}
	}
	*var = value;
	parameter->vars.push_back(var);
	return var;
}

auto registerInt(const char* id, const char* name, const char* kind, const char*, int* var, int value, int low,
	int up, const value_pair* values) -> int*
{
	auto* parameter = parameterFor(id);
	if (!parameter)
	{
		parameter = &newParameter(id, name, kind, static_cast<float>(value), static_cast<float>(low),
			static_cast<float>(up), 1.f, s_registering);
		for (auto pair = values; pair && pair->value_id; ++pair)
		{
			parameter->labels << QString::fromUtf8(pair->value_label ? pair->value_label : pair->value_id);
		}
	}
	*var = value;
	parameter->intVars.push_back(var);
	return var;
}

auto registerBool(const char* id, const char* name, const char* kind, const char*, bool* var, bool value) -> bool*
{
	auto* parameter = parameterFor(id);
	if (!parameter) { parameter = &newParameter(id, name, kind, value ? 1.f : 0.f, 0.f, 1.f, 1.f, s_registering); }
	*var = value;
	parameter->boolVars.push_back(var);
	return var;
}

} // namespace


GuitarixControls::GuitarixControls(GuitarixEffect* effect) :
	EffectControls(effect),
	m_effect(effect)
{
}


auto GuitarixControls::controlCount() -> int
{
	return static_cast<int>(std::count_if(m_parameters.begin(), m_parameters.end(),
		[](const auto& parameter) { return parameter.model != nullptr; }));
}


auto GuitarixControls::parameter(const QString& id) -> GuitarixParameter*
{
	const auto found = std::find_if(m_parameters.begin(), m_parameters.end(),
		[&id](const auto& parameter) { return parameter.id == id; });
	return found == m_parameters.end() ? nullptr : &*found;
}


void GuitarixControls::saveSettings(QDomDocument& doc, QDomElement& parent)
{
	for (auto& parameter : m_parameters)
	{
		// (not those Guitarix does not save)
		if (parameter.model && !parameter.kind.mid(1).contains('s'))
		{
			parameter.model->saveSettings(doc, parent, parameter.id);
		}
	}
}


void GuitarixControls::loadSettings(const QDomElement& parent)
{
	for (auto& parameter : m_parameters)
	{
		if (!parameter.model) { continue; }
		// (one not saved keeps Guitarix's default)
		if (parent.hasAttribute(parameter.id) || !parent.firstChildElement(parameter.id).isNull())
		{
			parameter.model->loadSettings(parent, parameter.id);
		}
	}
}


gui::EffectControlDialog* GuitarixControls::createView()
{
	return new gui::GuitarixControlDialog(this);
}




GuitarixEffect::GuitarixEffect(Model* parent, const Descriptor::SubPluginFeatures::Key* key) :
	LegacyEffect(&guitarix_plugin_descriptor, parent, key),
	m_controls(this)
{
	// (no unit named, as when made without a key: the first)
	m_unit = key ? guitarix::unit(key->attributes.value("unit").toUtf8().constData()) : nullptr;
	if (!m_unit && (!key || !key->attributes.contains("unit"))) { m_unit = &guitarix::units().front(); }
	if (!m_unit) { return; }

	makeInstances();

	// Before a rest (an export, a new project), on this thread: made anew there, the audio thread's reset() finds
	// them at rest
	QObject::connect(Engine::mixer(), &Mixer::processingStateDiscarding, &m_controls, [this] {
		if (m_sampleRate == 0) { return; }
		const auto guard = AudioEngine::changesGuard();
		// (made anew from the factory: not all of Guitarix's units clear all of their state when started again)
		makeInstances();
		initialize(m_sampleRate);
	});

	initialize(Engine::audioEngine()->outputSampleRate());
}


GuitarixEffect::~GuitarixEffect()
{
	deleteInstances();
}


void GuitarixEffect::deleteInstances()
{
	for (auto*& instance : m_instances)
	{
		if (!instance) { continue; }
		if (instance->activate_plugin) { instance->activate_plugin(false, instance); }
		instance->delete_instance(instance);
		instance = nullptr;
	}
}


void GuitarixEffect::makeInstances()
{
	deleteInstances();
	// The parameters are bound to the new instances' variables (the models stay: the values are the user's)
	for (auto& parameter : m_controls.m_parameters)
	{
		parameter.vars.clear();
		parameter.intVars.clear();
		parameter.boolVars.clear();
	}

	// Two instances of a mono unit, one of a stereo one; their parameters in the models (the same ids)
	m_instances[0] = m_unit->make();
	m_stereo = m_instances[0]->stereo_audio != nullptr;
	if (!m_stereo) { m_instances[1] = m_unit->make(); }
	s_registering = &m_controls;
	for (auto* instance : m_instances)
	{
		if (!instance || !instance->register_params) { continue; }
		auto registrar = ParamReg{};
		registrar.plugin = instance;
		registrar.registerFloatVar = &registerFloat;
		registrar.registerIntVar = &registerInt;
		registrar.registerBoolVar = &registerBool;
		instance->register_params(registrar);
	}
	s_registering = nullptr;
	if (m_unit->fileState)
	{
		for (auto& state : m_fileStates) { state.assign(m_unit->fileState->size, std::byte{0}); }
	}
}


auto GuitarixEffect::unitName() const -> QString
{
	return m_instances[0] ? QString::fromUtf8(m_instances[0]->name) : QString{};
}


auto GuitarixEffect::activate(double sampleRate, f_cnt_t maxFrameCount) -> bool
{
	(void)maxFrameCount;
	const auto rate = static_cast<unsigned int>(sampleRate);
	if (rate != m_sampleRate) { initialize(rate); }
	return true;
}


void GuitarixEffect::initialize(unsigned int sampleRate)
{
	// (with no period being rendered: some units fill tables of their file when started, tables the same units in
	// other effects are reading as they process; the same values, but not while they read)
	const auto guard = AudioEngine::changesGuard();
	m_sampleRate = sampleRate;
	for (auto* instance : m_instances)
	{
		if (!instance) { continue; }
		// (as Guitarix starts a unit: the rate, which clears its state, then its activation)
		if (instance->set_samplerate) { instance->set_samplerate(sampleRate, instance); }
		if (instance->activate_plugin) { instance->activate_plugin(true, instance); }
	}
	// (the state its code keeps in its file, as starting it left it: each instance's own from now on)
	if (const auto* state = m_unit ? m_unit->fileState : nullptr)
	{
		const auto lock = FileStateLock{state};
		for (auto& copy : m_fileStates) { state->save(copy.data()); }
	}
}


void GuitarixEffect::applyParameters()
{
	for (auto& parameter : m_controls.m_parameters)
	{
		if (!parameter.model) { continue; }
		const auto value = parameter.model->value();
		for (auto* var : parameter.vars) { *var = value; }
		for (auto* var : parameter.intVars) { *var = static_cast<int>(std::lround(value)); }
		for (auto* var : parameter.boolVars) { *var = value >= 0.5f; }
	}
}


auto GuitarixEffect::process(ProcessContext& context) -> ProcessStatus
{
	if (!m_instances[0]) { return ProcessStatus::Continue; }
	// (a unit with state in its file: its controls set and its code run with no other effect's in between)
	const auto* state = m_unit->fileState;
	const auto lock = FileStateLock{state};
	applyParameters();

	const auto dry = dryLevel();
	const auto wet = wetLevel();
	auto inOut = context.inOut().interleavedBuffer();
	const auto frames = inOut.frames();
	for (f_cnt_t start = 0; start < frames; start += Chunk)
	{
		const auto count = std::min(Chunk, frames - start);
		for (f_cnt_t f = 0; f < count; ++f)
		{
			const auto& frame = inOut.sampleFrameAt(start + f);
			m_input[0][f] = frame.left();
			m_input[1][f] = frame.right();
		}
		if (m_stereo)
		{
			if (state) { state->restore(m_fileStates[0].data()); }
			m_instances[0]->stereo_audio(static_cast<int>(count), m_input[0].data(), m_input[1].data(),
				m_output[0].data(), m_output[1].data(), m_instances[0]);
			if (state) { state->save(m_fileStates[0].data()); }
		}
		else
		{
			for (int ch = 0; ch < 2; ++ch)
			{
				if (state) { state->restore(m_fileStates[ch].data()); }
				m_instances[ch]->mono_audio(static_cast<int>(count), m_input[ch].data(), m_output[ch].data(),
					m_instances[ch]);
				if (state) { state->save(m_fileStates[ch].data()); }
			}
		}
		for (f_cnt_t f = 0; f < count; ++f)
		{
			auto& frame = inOut.sampleFrameAt(start + f);
			frame = frame * dry + SampleFrame{m_output[0][f], m_output[1][f]} * wet;
		}
	}
	collectOutputs();
	return ProcessStatus::ContinueIfNotQuiet;
}


void GuitarixEffect::collectOutputs()
{
	for (auto& parameter : m_controls.m_parameters)
	{
		if (!parameter.output || parameter.vars.empty()) { continue; }
		auto highest = *parameter.vars.front();
		for (auto* var : parameter.vars) { highest = std::max(highest, *var); }
		parameter.output->store(highest, std::memory_order_relaxed);
	}
}


extern "C"
{

// necessary for getting instance out of shared lib
PLUGIN_EXPORT Plugin* lmms_plugin_main(Model* parent, void* data)
{
	return new GuitarixEffect(parent, static_cast<const Plugin::Descriptor::SubPluginFeatures::Key*>(data));
}

}

} // namespace lmms
