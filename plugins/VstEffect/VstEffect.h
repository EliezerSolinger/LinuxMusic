/*
 * VstEffect.h - class for handling VST effect plugins
 *
 * Copyright (c) 2006-2009 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef _VST_EFFECT_H
#define _VST_EFFECT_H

#include <string>

#include <QMutex>
#include <QSharedPointer>

#include "Effect.h"
#include "VstEffectControls.h"

namespace lmms
{

class VstPlugin;

/**
 * Hosts a VST effect, which runs in a RemoteVstPlugin process.
 *
 * The plugin's audio inputs and outputs, as the remote process reports them, are exposed to the pin
 * connector. The remote process reads and writes the host's buffers in shared memory.
 */
class VstEffect : public Effect
{
public:
	VstEffect( Model * _parent,
			const Descriptor::SubPluginFeatures::Key * _key );
	~VstEffect() override = default;

	EffectControls * controls() override
	{
		return &m_vstControls;
	}

	// AudioProcessor implementation
	auto process(ProcessContext& context) -> ProcessStatus override;
	auto channelCount(bool isInput) const -> ch_cnt_t override;
	auto channelGroupCount(bool isInput) const -> group_cnt_t override { return channelCount(isInput) > 0 ? 1 : 0; }
	auto getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup& group) const
		-> ch_cnt_t override
	{
		(void)index;
		(void)group;
		return channelCount(isInput);
	}
	auto getInplacePair(ch_cnt_t inputIndex) const -> std::optional<ch_cnt_t> override
	{
		(void)inputIndex;
		return std::nullopt;
	}
	auto usesSharedMemoryBuffers() const -> bool override { return m_plugin != nullptr; }
	void onSharedMemoryChange(const SharedBufferInfo& info) override;

private:
	//! Returns true if plugin was loaded (m_plugin != nullptr)
	bool openPlugin(const QString& plugin);
	void closePlugin();

	//! Whether the buffers the host set up match the channel counts the remote process reported
	auto buffersMatchRemote() const -> bool;

	QSharedPointer<VstPlugin> m_plugin;
	QMutex m_pluginMutex;
	EffectKey m_key;

	//! Where the host put the audio buffers for the remote process
	std::string m_sharedBufferKey;
	SharedBufferInfo m_sharedBufferInfo{};

	VstEffectControls m_vstControls;


	friend class VstEffectControls;
	friend class gui::VstEffectControlDialog;
	friend class gui::ManageVSTEffectView;

} ;


} // namespace lmms

#endif
