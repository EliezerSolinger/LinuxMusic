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
		Effect& effect = instance->effectRef();
		if (auto dummy = dynamic_cast<DummyEffect*>(&effect))
		{
			_this.appendChild(dummy->originalPluginData());
		}
		else
		{
			QDomElement ef = effect.saveState(_doc, _this);
			ef.setAttribute("name", QString::fromUtf8(effect.descriptor()->name));
			ef.appendChild(effect.key().saveXML(_doc));
			instance->savePins(_doc, ef);
		}
	}
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
			QDomElement effectData = node.toElement();

			const QString name = effectData.attribute( "name" );
			EffectKey key( effectData.elementsByTagName( "key" ).item( 0 ).toElement() );

			auto e = std::make_unique<PluginInstance>(Plugin::Type::Effect, name, this, &key);

			if (e->effect() && e->effectRef().isOkay() && e->effectRef().nodeName() == node.nodeName())
			{
				e->effectRef().restoreState(effectData);
				e->loadPins(effectData);
			}
			else
			{
				e->convertToDummyEffect(parentModel(), effectData);
			}

			m_effects.push_back(std::move(e));
			++fx_loaded;
		}
		node = node.nextSibling();
	}

	emit dataChanged();
}




void EffectChain::appendEffect(std::unique_ptr<PluginInstance> effect)
{
	Engine::audioEngine()->requestChangeInModel();
	m_effects.push_back(std::move(effect));
	Engine::audioEngine()->doneChangeInModel();

	m_enabledModel.setValue( true );

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




void EffectChain::clear()
{
	emit aboutToClear();

	Engine::audioEngine()->requestChangeInModel();
	m_effects.clear();
	Engine::audioEngine()->doneChangeInModel();

	m_enabledModel.setValue( false );
}

} // namespace lmms
