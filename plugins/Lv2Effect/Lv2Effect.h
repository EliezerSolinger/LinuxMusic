/*
 * Lv2Effect.h - implementation of LV2 effect
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

#ifndef LV2_EFFECT_H
#define LV2_EFFECT_H

#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "Effect.h"
#include "Lv2Ports.h"
#include "SharedMemory.h"
#include "Lv2FxControls.h"

namespace lmms
{

class RemotePlugin;

/**
 * Hosts an LV2 effect plugin.
 *
 * The plugin's audio ports are exposed to the pin connector as they are, including sidechain inputs,
 * which follow the main inputs in a separate channel group. Mono plugins run twice, once per stereo
 * channel (see Lv2ControlBase).
 *
 * Isolated (an option of each effect, and what a plugin the PluginGuard suspects gets), the plugin runs in a process
 * of its own (RemoteLv2Host): a crash there leaves the program running. Its channels are then in shared memory, its
 * controls in a shared control block, and the ports are connected there as planned here (see Lv2Isolation.h).
 */
class Lv2Effect : public Effect
{
	Q_OBJECT
	//! (the effect's menu offers it, see EffectView)
	Q_PROPERTY(bool isolated READ isolated WRITE setIsolated)

public:
	/*
		initialization
	*/
	Lv2Effect(Model* parent, const Descriptor::SubPluginFeatures::Key* _key);
	~Lv2Effect() override;

	EffectControls* controls() override { return &m_controls; }

	Lv2FxControls* lv2Controls() { return &m_controls; }
	const Lv2FxControls* lv2Controls() const { return &m_controls; }

	// AudioProcessor implementation
	auto activate(double sampleRate, f_cnt_t maxFrameCount) -> bool override;
	auto process(ProcessContext& context) -> ProcessStatus override;
	auto latency() const -> f_cnt_t override { return m_controls.latency(); }
	auto channelCount(bool isInput) const -> ch_cnt_t override;
	auto channelGroupCount(bool isInput) const -> group_cnt_t override;
	auto getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup& group) const
		-> ch_cnt_t override;
	auto getInplacePair(ch_cnt_t inputIndex) const -> std::optional<ch_cnt_t> override;
	auto usesSharedMemoryBuffers() const -> bool override { return m_isolated; }
	void onSharedMemoryChange(const SharedBufferInfo& info) override;
	void deactivate() override;

	//! Whether it runs in a process of its own (see the class); changing it makes the plugin again [main thread]
	auto isolated() const -> bool { return m_isolated; }
	Q_INVOKABLE void setIsolated(bool isolated);

private:
	//! Starts the plugin's process with the plan of its ports; false (and why, to the user) when it could not
	auto startIsolated() -> bool;
	void stopIsolated();
	//! One period in the plugin's process: the controls there and back, the audio in the shared buffers
	auto processIsolated(f_cnt_t frames) -> bool;

	bool m_isolated;
	Lv2FxControls m_controls;

	std::unique_ptr<RemotePlugin> m_remote;
	SharedMemory<float[]> m_controlBlock;
	SharedBufferInfo m_sharedBufferInfo{};
	std::string m_sharedBufferKey;
	//! The controls and their places in the control block
	std::vector<std::pair<Lv2Ports::Control*, std::size_t>> m_controlInputs;
	std::vector<std::pair<Lv2Ports::Control*, std::size_t>> m_controlOutputs;
	std::vector<std::pair<Lv2Ports::Cv*, std::size_t>> m_cvInputs;
	bool m_remoteActive = false;
};


} // namespace lmms

#endif
