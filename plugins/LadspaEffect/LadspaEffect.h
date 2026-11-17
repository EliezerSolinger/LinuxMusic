/*
 * LadspaEffect.h - class for handling LADSPA effect plugins
 *
 * Copyright (c) 2006-2008 Danny McRae <khjklujn/at/users.sourceforge.net>
 * Copyright (c) 2009 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_LADSPA_EFFECT_H
#define LMMS_LADSPA_EFFECT_H

#include <vector>

#include <memory>
#include <string>

#include "Effect.h"
#include "LadspaPorts.h"
#include "SharedMemory.h"
#include "ladspa.h"
#include "LadspaControls.h"
#include "LadspaManager.h"

namespace lmms
{

class RemotePlugin;
struct port_desc_t;
using multi_proc_t = QVector<port_desc_t*>;

/**
 * Hosts a LADSPA effect plugin.
 *
 * The plugin's audio ports are exposed to the pin connector as they are: a plugin with 1 input and
 * 2 outputs gets 1 input and 2 outputs, a 4-channel plugin gets 4 inputs and 4 outputs, and so on.
 * Mono plugins (1 input and 1 output) are the exception: as in LMMS, they run as two instances
 * ("processors"), one per stereo channel, whose controls can be linked.
 *
 * The plugin's audio ports are connected straight to the host's planar buffers, so no audio is copied
 * in or out of the plugin.
 *
 * Isolated (an option of each effect, and what a plugin the PluginGuard suspects gets), the plugin runs in a process
 * of its own (RemoteLadspaHost): a crash there leaves the program running. Its channels are then in shared memory
 * (usesSharedMemoryBuffers()), and its control ports' values in a shared control block (see LadspaPorts::Layout).
 */
class LadspaEffect : public Effect
{
	Q_OBJECT
	//! (the effect's menu offers it, see EffectView)
	Q_PROPERTY(bool isolated READ isolated WRITE setIsolated)
public:
	LadspaEffect( Model * _parent,
			const Descriptor::SubPluginFeatures::Key * _key );
	~LadspaEffect() override;

	void setControl( int _control, LADSPA_Data _data );

	//! Whether it runs in a process of its own (see the class); changing it makes the plugin again [main thread]
	auto isolated() const -> bool { return m_isolated; }
	Q_INVOKABLE void setIsolated(bool isolated);

	EffectControls * controls() override
	{
		return m_controls;
	}

	inline const multi_proc_t & getPortControls()
	{
		return m_portControls;
	}

	ch_cnt_t processorCount() const
	{
		return m_processors;
	}

	// AudioProcessor implementation
	auto activate(double sampleRate, f_cnt_t maxFrameCount) -> bool override;
	void deactivate() override;
	auto process(ProcessContext& context) -> ProcessStatus override;
	auto latency() const -> f_cnt_t override;

	auto channelCount(bool isInput) const -> ch_cnt_t override;
	auto channelGroupCount(bool isInput) const -> group_cnt_t override;
	auto getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup& group) const
		-> ch_cnt_t override;
	auto getInplacePair(ch_cnt_t inputIndex) const -> std::optional<ch_cnt_t> override;
	auto usesSharedMemoryBuffers() const -> bool override { return m_isolated; }
	void onSharedMemoryChange(const SharedBufferInfo& info) override;

private:
	//! Instantiates the plugin again for a new sample rate, keeping the control values. [main thread]
	void changeSampleRate(sample_rate_t sampleRate);

	void pluginInstantiation(sample_rate_t sampleRate, f_cnt_t maxFrames);
	void pluginDestruction();

	//! Starts the plugin's process and makes the instances there; false (and why, to the user) when it could not
	auto startIsolated(sample_rate_t sampleRate, f_cnt_t maxFrames) -> bool;

	void activateHandles();

	//! Connects the ports to @p in and @p out and the controls, and runs the processors
	//! (isolated: false when the plugin process did not run the period)
	auto runProcessors(const AudioBuffer& in, AudioBuffer& out) -> bool;

	/**
	 * Plugins report their latency while they run, so a plugin with a latency port runs once on silence after
	 * being activated, and is then activated afresh (as Ardour does): the latency is known before the first period.
	 */
	void measureLatency();
	void deactivateHandles();

	LadspaControls * m_controls;

	ladspa_key_t m_key;
	int m_portCount = 0;

	const LADSPA_Descriptor * m_descriptor = nullptr;
	QVector<LADSPA_Handle> m_handles;

	QVector<multi_proc_t> m_ports;
	multi_proc_t m_portControls;

	//! The output control port named "latency" of the first processor, if any
	const port_desc_t* m_latencyPort = nullptr;

	//! Number of plugin instances (2 for mono plugins, otherwise 1)
	ch_cnt_t m_processors = 0;

	//! Audio inputs and outputs of each plugin instance
	ch_cnt_t m_inputsPerProcessor = 0;
	ch_cnt_t m_outputsPerProcessor = 0;

	//! The sample rate the plugin instances were created with
	sample_rate_t m_sampleRate = 0;
	f_cnt_t m_maxFrames = 0;

	//! Whether the plugin instances were activated (LADSPA activate())
	bool m_handlesActive = false;

	//! The buffer each audio port is currently connected to, to only call connect_port() when it changes
	std::vector<LADSPA_Data*> m_connectedAudioBuffers;

	//! Isolated: wanted (see setIsolated()), and the plugin's process with its control block
	bool m_isolated = false;
	std::unique_ptr<RemotePlugin> m_remote;
	SharedMemory<float[]> m_controlBlock;
	SharedBufferInfo m_sharedBufferInfo{};
	std::string m_sharedBufferKey;
};


} // namespace lmms

#endif // LMMS_LADSPA_EFFECT_H
