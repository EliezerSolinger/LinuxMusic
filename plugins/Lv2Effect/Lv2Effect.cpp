/*
 * Lv2Effect.cpp - implementation of LV2 effect
 *
 * Copyright (c) 2018-2023 Johannes Lorenz <jlsf2013$users.sourceforge.net, $=@>
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

#include "Lv2Effect.h"

#include <QDebug>

#include "Lv2SubPluginFeatures.h"

#include "embed.h"
#include "plugin_export.h"


namespace lmms
{


extern "C"
{

Plugin::Descriptor PLUGIN_EXPORT lv2effect_plugin_descriptor =
{
	LMMS_STRINGIFY(PLUGIN_NAME),
	"LV2",
	QT_TRANSLATE_NOOP("PluginBrowser",
		"plugin for using arbitrary LV2-effects inside LinuxMusic."),
	"Johannes Lorenz <jlsf2013$$$users.sourceforge.net, $$$=@>",
	0x0100,
	Plugin::Type::Effect,
	new PluginPixmapLoader("logo"),
	nullptr,
	new Lv2SubPluginFeatures(Plugin::Type::Effect)
};

}




Lv2Effect::Lv2Effect(Model* parent, const Descriptor::SubPluginFeatures::Key *key) :
	Effect(&lv2effect_plugin_descriptor, parent, key),
	m_controls(this, key->attributes["uri"])
{
}




auto Lv2Effect::activate(double sampleRate, f_cnt_t maxFrameCount) -> bool
{
	(void)maxFrameCount;

	// TODO: once lv2 options are implemented,
	//       plugins that support it might allow changing their samplerate
	//       through it instead of reloading
	if (static_cast<sample_rate_t>(sampleRate) != m_controls.sampleRate()) { m_controls.reload(); }
	return true;
}




auto Lv2Effect::channelCount(bool isInput) const -> ch_cnt_t
{
	return m_controls.audioChannelCount(isInput);
}




auto Lv2Effect::channelGroupCount(bool isInput) const -> group_cnt_t
{
	return m_controls.audioChannelGroupCount(isInput);
}




auto Lv2Effect::getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup& group) const
	-> ch_cnt_t
{
	(void)group;
	return m_controls.audioChannelGroup(index, isInput);
}




auto Lv2Effect::getInplacePair(ch_cnt_t inputIndex) const -> std::optional<ch_cnt_t>
{
	// The plugin has its own port buffers, and the input is needed for the dry signal afterwards
	(void)inputIndex;
	return std::nullopt;
}




auto Lv2Effect::process(ProcessContext& context) -> ProcessStatus
{
	const AudioBuffer& in = context.in();
	AudioBuffer& out = context.out();
	const auto frames = out.frames();

	m_controls.copyBuffersFromCore(in, frames);
	m_controls.copyModelsFromLmms();

	m_controls.run(frames);

	m_controls.copyModelsToLmms();
	m_controls.copyBuffersToCore(out, frames);

	// Mix the dry signal of the main inputs into the outputs; sidechain inputs are not part of the dry
	// signal, and outputs without a matching main input have none
	bool corrupt = wetLevel() < 0; // #3261 - if w < 0, bash w := 0, d := 1
	const float d = corrupt ? 1 : dryLevel();
	const float w = corrupt ? 0 : wetLevel();
	if (d != 0.f || w != 1.f)
	{
		const auto mainInputs = m_controls.mainInputCount();
		for (ch_cnt_t channel = 0; channel < out.channels(); ++channel)
		{
			auto wet = out.buffer(channel);
			if (channel < mainInputs)
			{
				const auto dry = in.buffer(channel);
				for (f_cnt_t f = 0; f < frames; ++f) { wet[f] = d * dry[f] + w * wet[f]; }
			}
			else
			{
				for (f_cnt_t f = 0; f < frames; ++f) { wet[f] *= w; }
			}
		}
	}

	return ProcessStatus::ContinueIfNotQuiet;
}




extern "C"
{

// necessary for getting instance out of shared lib
PLUGIN_EXPORT Plugin *lmms_plugin_main(Model *_parent, void *_data)
{
	using KeyType = Plugin::Descriptor::SubPluginFeatures::Key;
	try {
		return new Lv2Effect(_parent, static_cast<const KeyType*>(_data));
	} catch (const std::runtime_error& e) {
		qCritical() << e.what();
		return nullptr;
	}
}

}


} // namespace lmms
