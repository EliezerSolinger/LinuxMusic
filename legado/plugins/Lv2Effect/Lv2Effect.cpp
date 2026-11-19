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

#include <chrono>

#include <QDebug>
#include <lilv/lilv.h>

#include "AudioEngine.h"
#include "Engine.h"
#include "ErrorLog.h"
#include "Host.h"
#include "Lv2Isolation.h"
#include "Lv2Manager.h"
#include "Lv2Proc.h"
#include "Lv2SubPluginFeatures.h"
#include "PluginGuard.h"
#include "RemotePlugin.h"
#include "UserNotice.h"

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
	// (a plugin the PluginGuard suspects comes isolated, see Plugin::instantiateWithKey(): it is not even made here)
	m_isolated(key->attributes.value("isolated") == "1"),
	m_controls(this, key->attributes["uri"], !m_isolated)
{
	if (m_isolated && !startIsolated()) { setOkay(false); }
}




Lv2Effect::~Lv2Effect() { stopIsolated(); }




void Lv2Effect::setIsolated(bool isolated)
{
	if (isolated == m_isolated) { return; }
	m_isolated = isolated;
	// The host moves the buffers into or out of shared memory and activates it again, which makes the plugin again
	// here or there (see activate())
	if (host()) { host()->useRemotePluginBuffers(isolated); }
}




void Lv2Effect::onSharedMemoryChange(const SharedBufferInfo& info)
{
	m_sharedBufferKey = std::string{info.key};
	m_sharedBufferInfo = info;
	m_sharedBufferInfo.key = m_sharedBufferKey;
	if (m_remote) { m_remote->setSharedAudioBuffer(m_sharedBufferInfo); }
}




auto Lv2Effect::startIsolated() -> bool
{
	using namespace Lv2Isolation;
	stopIsolated();
	const auto fail = [this](const QString& why) {
		UserNotice::warning(tr("Effect"), tr("The LV2 plugin %1 could not run in a process of its own: %2")
			.arg(displayName(), why));
		return false;
	};
	const auto* plugin = m_controls.getPlugin();
	if (!plugin) { return fail(tr("it is unknown.")); }

	// The plan: each instance's ports, in port order, connected as the program connects them (the channels counted
	// as Lv2ControlBase counts them: all main inputs, then all sidechain inputs; the outputs)
	auto plans = std::vector<Plan>{};
	auto place = std::size_t{1}; // (0: the period's frames)
	auto firstMain = ch_cnt_t{0};
	auto firstSidechain = m_controls.mainInputCount();
	auto firstOutput = ch_cnt_t{0};
	m_controlInputs.clear();
	m_controlOutputs.clear();
	m_cvInputs.clear();
	for (std::size_t which = 0; which < m_controls.procCount(); ++which)
	{
		auto* proc = m_controls.control(which);
		auto plan = Plan{};
		auto main = ch_cnt_t{0};
		auto sidechain = ch_cnt_t{0};
		auto output = ch_cnt_t{0};
		for (const auto& port : proc->ports())
		{
			auto planned = Port{};
			const bool input = port->m_flow == Lv2Ports::Flow::Input;
			if (auto* audio = dynamic_cast<Lv2Ports::Audio*>(port.get()))
			{
				planned.kind = input ? Kind::AudioIn : Kind::AudioOut;
				planned.index = !input ? static_cast<std::size_t>(firstOutput + output++)
					: audio->isSideChain() ? static_cast<std::size_t>(firstSidechain + sidechain++)
					: static_cast<std::size_t>(firstMain + main++);
			}
			else if (auto* control = dynamic_cast<Lv2Ports::Control*>(port.get()))
			{
				planned.kind = input ? Kind::ControlIn : Kind::ControlOut;
				planned.index = place++;
				(input ? m_controlInputs : m_controlOutputs).emplace_back(control, planned.index);
			}
			else if (auto* cv = dynamic_cast<Lv2Ports::Cv*>(port.get()))
			{
				planned.kind = input ? Kind::CvIn : Kind::CvOut;
				if (input)
				{
					planned.index = place++;
					m_cvInputs.emplace_back(cv, planned.index);
				}
			}
			else if (auto* events = dynamic_cast<Lv2Ports::AtomSeq*>(port.get()))
			{
				planned.kind = input ? Kind::EventsIn : Kind::EventsOut;
				planned.eventCapacity = events->m_capacity;
			}
			plan.push_back(planned);
		}
		plans.push_back(std::move(plan));
		firstMain += proc->mainInputCount();
		firstSidechain += proc->sidechainInputCount();
		firstOutput += proc->outputCount();
	}

	try { m_controlBlock.create(place); }
	catch (const std::exception& error) { return fail(QString::fromUtf8(error.what())); }
	// The controls as they are now, the latency until the plugin says
	for (const auto& [control, at] : m_controlInputs) { m_controlBlock[at] = control->m_val; }
	for (const auto& [cv, at] : m_cvInputs) { m_controlBlock[at] = cv->m_buffer.empty() ? 0.f : cv->m_buffer.front(); }

	// The features the program offers without data, offered there too
	auto staticFeatures = std::string{};
	for (const auto& uri : Engine::getLv2Manager()->supportedFeatureURIs())
	{
		if (uri == LV2_URID__map || uri == LV2_URID__unmap || uri == LV2_OPTIONS__options || uri == LV2_WORKER__schedule)
		{
			continue;
		}
		staticFeatures += std::string{uri} + ' ';
	}
	auto* bundlePath = lilv_file_uri_parse(lilv_node_as_uri(lilv_plugin_get_bundle_uri(plugin)), nullptr);
	const auto bundle = std::string{bundlePath ? bundlePath : ""};
	lilv_free(bundlePath);

	auto remote = std::make_unique<RemotePlugin>();
	remote->setGuardIdentity(PluginGuard::identity(*this));
	// (init() says whether it failed)
	if (remote->init("RemoteLv2Host", false)) { return fail(tr("its process did not start.")); }
	remote->lock();
	remote->waitForInitDone(false);
	remote->sendMessage(RemotePlugin::message{IdLv2Load}
		.addString(bundle)
		.addString(lilv_node_as_uri(lilv_plugin_get_uri(plugin)))
		.addInt(static_cast<int>(m_controls.sampleRate()))
		.addInt(static_cast<int>(Engine::audioEngine()->framesPerPeriod()))
		.addString(encode(plans))
		.addString(m_controlBlock.key())
		.addInt(Engine::audioEngine()->renderOnly() ? 0 : 1)
		.addString(staticFeatures));
	const auto answer = remote->waitForMessage(RemotePlugin::message{IdLv2Loaded}, std::chrono::seconds{30});
	remote->unlock();
	if (answer.id != IdLv2Loaded) { return fail(tr("its process did not answer.")); }
	if (answer.getInt(0) != 1) { return fail(QString::fromStdString(answer.getString(1))); }

	m_remote = std::move(remote);
	// (the host may have put the buffers in shared memory already)
	if (!m_sharedBufferKey.empty()) { m_remote->setSharedAudioBuffer(m_sharedBufferInfo); }
	return true;
}




void Lv2Effect::stopIsolated()
{
	m_remote.reset();
	m_remoteActive = false;
	m_controlInputs.clear();
	m_controlOutputs.clear();
	m_cvInputs.clear();
	m_controlBlock.detach();
}




void Lv2Effect::deactivate()
{
	if (!m_remote || !m_remoteActive) { return; }
	m_remote->lock();
	m_remote->sendMessage(RemotePlugin::message{Lv2Isolation::IdLv2Deactivate});
	m_remote->unlock();
	m_remoteActive = false;
}




auto Lv2Effect::processIsolated(f_cnt_t frames) -> bool
{
	if (!m_remote) { return false; }
	// The models' values into the ports (as for a plugin here), from there into the control block
	m_controls.copyModelsFromLmms();
	for (const auto& [control, at] : m_controlInputs) { m_controlBlock[at] = control->m_val; }
	for (const auto& [cv, at] : m_cvInputs) { m_controlBlock[at] = cv->m_buffer.empty() ? 0.f : cv->m_buffer.front(); }
	m_controlBlock[0] = static_cast<float>(frames);
	const bool done = m_remote->process();
	// What the plugin reports (its latency, its meters)
	for (const auto& [control, at] : m_controlOutputs) { control->m_val = m_controlBlock[at]; }
	return done;
}




auto Lv2Effect::activate(double sampleRate, f_cnt_t maxFrameCount) -> bool
{
	(void)maxFrameCount;

	// TODO: once lv2 options are implemented,
	//       plugins that support it might allow changing their samplerate
	//       through it instead of reloading
	const bool rateChanged = static_cast<sample_rate_t>(sampleRate) != m_controls.sampleRate();
	if (m_isolated != (m_remote != nullptr) || (m_isolated && rateChanged))
	{
		// Isolated, or back here (see setIsolated()), or a new rate there: made again where it runs now
		stopIsolated();
		if (rateChanged) { m_controls.reloadWhileInactive(); }
		m_controls.setLocal(!m_isolated);
		if (m_isolated && !startIsolated()) { return true; } // (silent: told to the user)
	}
	else if (rateChanged) { m_controls.reloadWhileInactive(); }

	if (m_remote && !m_remoteActive)
	{
		// Active, then once on silence for the latency, as here (see Lv2Proc::measureLatency())
		m_remote->lock();
		m_remote->sendMessage(RemotePlugin::message{Lv2Isolation::IdLv2Activate});
		m_remote->sendMessage(RemotePlugin::message{Lv2Isolation::IdLv2MeasureLatency});
		m_remote->waitForMessage(RemotePlugin::message{Lv2Isolation::IdLv2Measured}, std::chrono::seconds{5});
		m_remote->unlock();
		for (const auto& [control, at] : m_controlOutputs) { control->m_val = m_controlBlock[at]; }
		m_remoteActive = true;
	}
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

	if (m_isolated)
	{
		// The channels are in the shared buffers already; a plugin process that is gone or hung leaves it silent
		if (!processIsolated(frames)) { out.silenceAllChannels(); }
	}
	else
	{
		m_controls.copyBuffersFromCore(in, frames);
		m_controls.copyModelsFromLmms();

		m_controls.run(frames);

		m_controls.copyModelsToLmms();
		m_controls.copyBuffersToCore(out, frames);
	}

	// The dry signal of the main inputs; sidechain inputs are not part of it
	mixDrySignal(in, out, m_controls.mainInputCount());

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
