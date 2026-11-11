/*
 * EffectChain.cpp - class for processing and effects chain
 *
 * Copyright (c) 2006-2008 Danny McRae <khjklujn/at/users.sourceforge.net>
 * Copyright (c) 2008-2009 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "EffectChain.h"

#include <QDomElement>
#include <algorithm>
#include <cassert>

#include "AudioBuffer.h"
#include "Effect.h"
#include "DummyEffect.h"
#include "MixHelpers.h"
#include "PluginInstance.h"

namespace lmms
{


EffectChain::EffectChain( Model * _parent ) :
	Model( _parent ),
	SerializingObject(),
	m_enabledModel( false, nullptr, tr( "Effects enabled" ) )
{
}




EffectChain::~EffectChain()
{
	clear();
}




void EffectChain::saveSettings( QDomDocument & _doc, QDomElement & _this )
{
	m_enabledModel.saveSettings( _doc, _this, "enabled" );
	_this.setAttribute("numofeffects", static_cast<int>(m_effects.size()));

	for (auto& instance : m_effects)
	{
		saveEffect(*instance, _doc, _this);
	}
}




auto EffectChain::saveEffect(PluginInstance& instance, QDomDocument& doc, QDomElement& parent) -> QDomElement
{
	Effect& effect = instance.effectRef();
	if (auto dummy = dynamic_cast<DummyEffect*>(&effect))
	{
		// What could not be loaded is kept as it was
		auto original = dummy->originalPluginData();
		parent.appendChild(original);
		return original;
	}

	QDomElement ef = effect.saveState(doc, parent);
	ef.setAttribute("name", QString::fromUtf8(effect.descriptor()->name));
	ef.appendChild(effect.key().saveXML(doc));
	instance.savePins(doc, ef);
	return ef;
}




void EffectChain::loadSettings( const QDomElement & _this )
{
	clear();

	// TODO This method should probably also lock the audio engine

	m_enabledModel.loadSettings( _this, "enabled" );

	const int plugin_cnt = _this.attribute( "numofeffects" ).toInt();

	QDomNode node = _this.firstChild();
	int fx_loaded = 0;
	while( !node.isNull() && fx_loaded < plugin_cnt )
	{
		if( node.isElement() && node.nodeName() == "effect" )
		{
			m_effects.push_back(loadEffect(node.toElement()));
			++fx_loaded;
		}
		node = node.nextSibling();
	}

	emit dataChanged();
}




auto EffectChain::loadEffect(const QDomElement& effectData) -> std::unique_ptr<PluginInstance>
{
	const QString name = effectData.attribute("name");
	EffectKey key(effectData.elementsByTagName("key").item(0).toElement());

	auto e = std::make_unique<PluginInstance>(Plugin::Type::Effect, name, this, &key);

	// Before loading the pins, so that pins to auxiliary track channels exist
	e->setTrackChannelCount(m_trackChannels);

	if (e->effect() && e->effectRef().isOkay() && e->effectRef().nodeName() == effectData.nodeName())
	{
		e->effectRef().restoreState(effectData);
		e->loadPins(effectData);
	}
	else
	{
		e->convertToDummyEffect(parentModel(), effectData);
	}
	return e;
}




void EffectChain::prependEffects(const QDomElement& savedChain)
{
	// The saved chain's effects are created in this chain, so that they belong to it from the start
	auto loaded = EffectList{};
	const int count = savedChain.attribute("numofeffects").toInt();
	for (auto node = savedChain.firstChildElement("effect"); !node.isNull() && static_cast<int>(loaded.size()) < count;
		node = node.nextSiblingElement("effect"))
	{
		loaded.push_back(loadEffect(node));
	}
	if (loaded.empty()) { return; }

	Engine::audioEngine()->requestChangeInModel();
	m_effects.insert(m_effects.begin(), std::make_move_iterator(loaded.begin()), std::make_move_iterator(loaded.end()));
	Engine::audioEngine()->doneChangeInModel();

	m_enabledModel.setValue(true);
	emit dataChanged();
}




void EffectChain::setTrackChannelCount(ch_cnt_t channels)
{
	m_trackChannels = channels;
	for (auto& effect : m_effects) { effect->setTrackChannelCount(channels); }
}




void EffectChain::appendEffect(std::unique_ptr<PluginInstance> effect)
{
	Engine::audioEngine()->requestChangeInModel();
	effect->setTrackChannelCount(m_trackChannels);
	m_effects.push_back(std::move(effect));
	Engine::audioEngine()->doneChangeInModel();

	m_enabledModel.setValue( true );

	emit dataChanged();
}




void EffectChain::insertEffect(std::size_t index, std::unique_ptr<PluginInstance> effect)
{
	Engine::audioEngine()->requestChangeInModel();
	effect->setTrackChannelCount(m_trackChannels);
	m_effects.insert(m_effects.begin() + static_cast<std::ptrdiff_t>(std::min(index, m_effects.size())),
		std::move(effect));
	Engine::audioEngine()->doneChangeInModel();

	m_enabledModel.setValue(true);

	emit dataChanged();
}




void EffectChain::replaceEffect(std::size_t index, std::unique_ptr<PluginInstance> effect)
{
	if (index >= m_effects.size()) { return; }

	Engine::audioEngine()->requestChangeInModel();
	effect->setTrackChannelCount(m_trackChannels);
	m_effects[index] = std::move(effect);
	Engine::audioEngine()->doneChangeInModel();

	emit dataChanged();
}




void EffectChain::moveEffect(std::size_t from, std::size_t to)
{
	if (from >= m_effects.size() || from == to) { return; }
	to = std::min(to, m_effects.size() - 1);

	Engine::audioEngine()->requestChangeInModel();
	auto effect = std::move(m_effects[from]);
	m_effects.erase(m_effects.begin() + static_cast<std::ptrdiff_t>(from));
	m_effects.insert(m_effects.begin() + static_cast<std::ptrdiff_t>(to), std::move(effect));
	Engine::audioEngine()->doneChangeInModel();

	emit dataChanged();
}




auto EffectChain::find(const Effect* effect) -> EffectList::iterator
{
	return std::find_if(m_effects.begin(), m_effects.end(), [effect](const std::unique_ptr<PluginInstance>& instance) {
		return instance->plugin() == effect;
	});
}




auto EffectChain::instanceOf(const Effect* effect) -> PluginInstance*
{
	const auto it = find(effect);
	return it != m_effects.end() ? it->get() : nullptr;
}




void EffectChain::removeEffect( Effect * _effect )
{
	Engine::audioEngine()->requestChangeInModel();

	const auto found = find(_effect);
	if( found == m_effects.end() )
	{
		Engine::audioEngine()->doneChangeInModel();
		return;
	}

	// Deletes the effect
	m_effects.erase( found );

	Engine::audioEngine()->doneChangeInModel();

	if (m_effects.empty())
	{
		m_enabledModel.setValue( false );
	}

	emit dataChanged();
}




void EffectChain::moveDown( Effect * _effect )
{
	const auto it = find(_effect);
	if (it != m_effects.end() && std::next(it) != m_effects.end())
	{
		Engine::audioEngine()->requestChangeInModel();
		std::iter_swap(it, std::next(it));
		Engine::audioEngine()->doneChangeInModel();
	}
}




void EffectChain::moveUp( Effect * _effect )
{
	const auto it = find(_effect);
	if (it != m_effects.end() && it != m_effects.begin())
	{
		Engine::audioEngine()->requestChangeInModel();
		std::iter_swap(it, std::prev(it));
		Engine::audioEngine()->doneChangeInModel();
	}
}




bool EffectChain::processAudioBuffer(AudioBuffer& buffer)
{
	if( m_enabledModel.value() == false )
	{
		return false;
	}

	bool moreEffects = false;
	for (auto& effect : m_effects)
	{
		moreEffects |= effect->process(buffer);
	}

	return moreEffects;
}




auto EffectChain::latency() const -> f_cnt_t
{
	if (!m_enabledModel.value()) { return 0; }

	f_cnt_t total = 0;
	for (const auto& effect : m_effects) { total += effect->latency(); }
	return total;
}




void EffectChain::clear()
{
	emit aboutToClear();

	Engine::audioEngine()->requestChangeInModel();
	m_effects.clear();
	Engine::audioEngine()->doneChangeInModel();

	m_enabledModel.setValue( false );
}

} // namespace lmms
