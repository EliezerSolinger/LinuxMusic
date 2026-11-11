/*
 * EffectChain.h - class for processing and effects chain
 *
 * Copyright (c) 2006-2008 Danny McRae <khjklujn/at/users.sourceforge.net>
 * Copyright (c) 2008-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
 *
 * This file is part of LMMS - https://lmms.io
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

#ifndef LMMS_EFFECT_CHAIN_H
#define LMMS_EFFECT_CHAIN_H

#include "Model.h"
#include "lmms_constants.h"
#include <memory>
#include <vector>

#include "SerializingObject.h"
#include "AutomatableModel.h"

namespace lmms
{

class AudioBuffer;
class Effect;
class PluginInstance;

namespace gui
{

class EffectRackView;

} // namespace gui


class LMMS_EXPORT EffectChain : public Model, public SerializingObject
{
	Q_OBJECT
public:
	EffectChain( Model * _parent );
	~EffectChain() override;

	void saveSettings( QDomDocument & _doc, QDomElement & _parent ) override;
	void loadSettings( const QDomElement & _this ) override;

	inline QString nodeName() const override
	{
		return "fxchain";
	}

	void appendEffect(std::unique_ptr<PluginInstance> effect);

	//! Loads the effects of a saved chain (an element saved by saveState()) in front of this chain's effects
	void prependEffects(const QDomElement& savedChain);
	void removeEffect( Effect * _effect );
	void moveDown( Effect * _effect );
	void moveUp( Effect * _effect );
	bool processAudioBuffer(AudioBuffer& buffer);

	//! The latency of the effects that process audio, added up (see PluginInstance::latency()) [audio thread]
	auto latency() const -> f_cnt_t;

	void clear();


	using EffectList = std::vector<std::unique_ptr<PluginInstance>>;

	//! The effects in processing order
	auto effects() const -> const EffectList& { return m_effects; }

	//! @returns the PluginInstance holding @p effect, or nullptr if it is not in this chain
	auto instanceOf(const Effect* effect) -> PluginInstance*;

	/**
	 * Sets how many channels the chain's track (e.g. mixer channel) has, for the pins of all effects,
	 * including the ones added later. [main thread, audio engine paused]
	 */
	void setTrackChannelCount(ch_cnt_t channels);
	auto trackChannelCount() const -> ch_cnt_t { return m_trackChannels; }

private:
	auto find(const Effect* effect) -> EffectList::iterator;

	//! Creates the effect saved in @p effectData in this chain (a DummyEffect if it cannot be loaded)
	auto loadEffect(const QDomElement& effectData) -> std::unique_ptr<PluginInstance>;

	EffectList m_effects;

	BoolModel m_enabledModel;

	ch_cnt_t m_trackChannels = DEFAULT_CHANNELS;


	friend class gui::EffectRackView;


signals:
	void aboutToClear();

} ;

} // namespace lmms

#endif // LMMS_EFFECT_CHAIN_H
