/*
 * GuitarixEffect.h - a Guitarix unit as a LinuxMusic effect
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

#ifndef LMMS_GUITARIX_EFFECT_H
#define LMMS_GUITARIX_EFFECT_H

#include <array>
#include <atomic>
#include <memory>
#include <vector>

#include "AutomatableModel.h"
#include "Effect.h"
#include "EffectControls.h"

struct PluginDef;

namespace lmms
{

namespace guitarix
{
struct Unit;
}

namespace gui
{
class GuitarixControlDialog;
}

class GuitarixEffect;

//! A Guitarix unit's parameter, as its register_par() gave it (see GuitarixEffect)
struct GuitarixParameter
{
	QString id; //!< Guitarix's (it names the attribute it is saved in)
	QString name;
	QString kind; //!< Guitarix's "tp": S slider, SL log slider, B switch, O output (a meter's level), ...
	//! What the user sets (none for an output: the unit writes it)
	std::unique_ptr<FloatModel> model;
	//! An output's value, as the unit last wrote it (its instances' highest), for the view to show
	std::unique_ptr<std::atomic<float>> output;
	//! Where the instances of the unit read it (a unit may register an id more than once, for more variables)
	std::vector<float*> vars;
	std::vector<int*> intVars; //!< (an integer parameter, a selector)
	std::vector<bool*> boolVars; //!< (a switch)
	QStringList labels; //!< a selector's values
};

class GuitarixControls : public EffectControls
{
	Q_OBJECT
public:
	explicit GuitarixControls(GuitarixEffect* effect);
	~GuitarixControls() override = default;

	void saveSettings(QDomDocument& doc, QDomElement& parent) override;
	void loadSettings(const QDomElement& parent) override;
	QString nodeName() const override { return "GuitarixControls"; }
	gui::EffectControlDialog* createView() override;
	int controlCount() override;

	auto parameters() -> std::vector<GuitarixParameter>& { return m_parameters; }
	//! The one registered as @p id (none if there is none)
	auto parameter(const QString& id) -> GuitarixParameter*;
	//! The unit (its UI: Guitarix's own layout for it)
	auto effect() const -> GuitarixEffect* { return m_effect; }

private:
	GuitarixEffect* m_effect;
	std::vector<GuitarixParameter> m_parameters;

	friend class GuitarixEffect;
	friend class gui::GuitarixControlDialog;
};


/**
 * A unit of Guitarix (its pedals, amps, cabinets...) as an effect, its DSP Guitarix's own code, run in this process
 * on the audio thread. Each unit is one of the plugin's sub-plugins (the key's "unit"). A mono unit (most) runs in
 * two instances, one for each channel; a stereo one in one.
 *
 * The units are made on the main thread, and made again (initialized at the rate) there when the rate changes and
 * before a rest (Mixer::processingStateDiscarding): their resamplers keep what came before, and setting them up
 * allocates. On the audio thread they only process.
 */
class GuitarixEffect : public LegacyEffect
{
public:
	GuitarixEffect(Model* parent, const Descriptor::SubPluginFeatures::Key* key);
	~GuitarixEffect() override;

	EffectControls* controls() override { return &m_controls; }

	auto activate(double sampleRate, f_cnt_t maxFrameCount) -> bool override;

	//! The unit (none for a key it does not know)
	auto unit() const -> const guitarix::Unit* { return m_unit; }
	//! Its name, as Guitarix gives it
	auto unitName() const -> QString;
	//! Its first instance, as Guitarix made it (its UI is asked of it) [main thread]
	auto definition() const -> PluginDef* { return m_instances[0]; }

private:
	auto process(ProcessContext& context) -> ProcessStatus override;
	//! Initializes the instances at @p sampleRate (anew: from rest) [main thread]
	void initialize(unsigned int sampleRate);
	//! The models' values to the instances' parameters [audio thread]
	void applyParameters();
	//! The instances' outputs to the parameters' (for the view) [audio thread]
	void collectOutputs();

	const guitarix::Unit* m_unit = nullptr;
	std::array<PluginDef*, 2> m_instances = {};
	bool m_stereo = false;
	unsigned int m_sampleRate = 0;

	//! A chunk of each channel, its input and output (Guitarix processes channels apart)
	static constexpr f_cnt_t Chunk = 256;
	std::array<std::array<float, Chunk>, 2> m_input = {};
	std::array<std::array<float, Chunk>, 2> m_output = {};

	GuitarixControls m_controls;

	friend class GuitarixControls;
};

} // namespace lmms

#endif // LMMS_GUITARIX_EFFECT_H
