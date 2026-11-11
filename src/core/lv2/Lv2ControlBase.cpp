/*
 * Lv2ControlBase.cpp - Lv2 control base class
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

#include "Lv2ControlBase.h"

#ifdef LMMS_HAVE_LV2

#include <algorithm>
#include <QDebug>
#include <QtGlobal>

#include "AudioEngine.h"
#include "Engine.h"
#include "lmms_constants.h"
#include "Lv2Manager.h"
#include "Lv2Proc.h"


namespace lmms
{


Plugin::Type Lv2ControlBase::check(const LilvPlugin *plugin,
	std::vector<PluginIssue> &issues)
{
	// for some reason, all checks can be done by one processor...
	return Lv2Proc::check(plugin, issues);
}




Lv2ControlBase::Lv2ControlBase(Model* that, const QString &uri) :
	m_plugin(Engine::getLv2Manager()->getPlugin(uri))
{
	if (m_plugin)
	{
		init(that);
	}
	else
	{
		qCritical() << "No Lv2 plugin found for URI" << uri;
		throw std::runtime_error("No Lv2 plugin found for given URI");
	}
}




Lv2ControlBase::~Lv2ControlBase() = default;




void Lv2ControlBase::init(Model* meAsModel)
{
	m_sampleRate = Engine::audioEngine()->outputSampleRate();

	// Mono plugins run once per stereo channel, all others once with all of their channels
	m_procs.push_back(std::make_unique<Lv2Proc>(m_plugin, meAsModel));
	if (Lv2Proc::isMono(m_procs[0]->mainInputCount(), m_procs[0]->outputCount()))
	{
		for (ch_cnt_t proc = 1; proc < DEFAULT_CHANNELS; ++proc)
		{
			m_procs.push_back(std::make_unique<Lv2Proc>(m_plugin, meAsModel));
		}
	}
	linkAllModels();
}




void Lv2ControlBase::shutdown()
{
	// currently nothing to do here
}




void Lv2ControlBase::reload()
{
	// The plugin instances are created again, so the audio thread must not use them meanwhile
	Engine::audioEngine()->requestChangeInModel();
	m_sampleRate = Engine::audioEngine()->outputSampleRate();
	for (const auto& c : m_procs) { c->reload(); }
	Engine::audioEngine()->doneChangeInModel();
}




auto Lv2ControlBase::latency() const -> f_cnt_t
{
	return m_procs.empty() ? 0 : m_procs.front()->latency();
}




auto Lv2ControlBase::audioChannelCount(bool isInput) const -> ch_cnt_t
{
	if (m_procs.empty()) { return 0; }
	const auto& proc = *m_procs.front();
	const auto perProc = isInput
		? proc.mainInputCount() + proc.sidechainInputCount()
		: proc.outputCount();
	return static_cast<ch_cnt_t>(m_procs.size() * perProc);
}




auto Lv2ControlBase::mainInputCount() const -> ch_cnt_t
{
	return m_procs.empty() ? 0 : static_cast<ch_cnt_t>(m_procs.size() * m_procs.front()->mainInputCount());
}




auto Lv2ControlBase::audioChannelGroupCount(bool isInput) const -> group_cnt_t
{
	if (!isInput) { return audioChannelCount(false) > 0 ? 1 : 0; }

	const auto sidechainInputs = audioChannelCount(true) - mainInputCount();
	return static_cast<group_cnt_t>((mainInputCount() > 0) + (sidechainInputs > 0));
}




auto Lv2ControlBase::audioChannelGroup(group_cnt_t index, bool isInput) const -> ch_cnt_t
{
	if (!isInput) { return audioChannelCount(false); }

	const auto mainInputs = mainInputCount();
	const auto sidechainInputs = static_cast<ch_cnt_t>(audioChannelCount(true) - mainInputs);
	return index == 0 && mainInputs > 0 ? mainInputs : sidechainInputs;
}




LinkedModelGroup *Lv2ControlBase::getGroup(std::size_t idx)
{
	return (m_procs.size() > idx) ? m_procs[idx].get() : nullptr;
}




const LinkedModelGroup *Lv2ControlBase::getGroup(std::size_t idx) const
{
	return (m_procs.size() > idx) ? m_procs[idx].get() : nullptr;
}




void Lv2ControlBase::copyModelsFromLmms() {
	for (const auto& c : m_procs) { c->copyModelsFromCore(); }
}




void Lv2ControlBase::copyModelsToLmms() const
{
	for (const auto& c : m_procs) { c->copyModelsToCore(); }
}




void Lv2ControlBase::copyBuffersFromCore(const AudioBuffer& in, f_cnt_t frames)
{
	// tell the procs which channels they shall read from
	auto firstMain = ch_cnt_t{0};
	auto firstSidechain = mainInputCount();
	for (const auto& c : m_procs)
	{
		c->copyBuffersFromCore(in, firstMain, firstSidechain, frames);
		firstMain += c->mainInputCount();
		firstSidechain += c->sidechainInputCount();
	}
}




void Lv2ControlBase::copyBuffersToCore(AudioBuffer& out, f_cnt_t frames) const
{
	// tell the procs which channels they shall write to
	auto first = ch_cnt_t{0};
	for (const auto& c : m_procs)
	{
		c->copyBuffersToCore(out, first, frames);
		first += c->outputCount();
	}
}




void Lv2ControlBase::run(f_cnt_t frames) {
	for (const auto& c : m_procs) { c->run(frames); }
}




void Lv2ControlBase::saveSettings(QDomDocument &doc, QDomElement &that)
{
	LinkedModelGroups::saveSettings(doc, that);
	
	// TODO: save state if supported by plugin
}




void Lv2ControlBase::loadSettings(const QDomElement &that)
{
	LinkedModelGroups::loadSettings(that);
	
	// TODO: load state if supported by plugin
}




void Lv2ControlBase::loadFile(const QString &file)
{
	(void)file;
}




std::size_t Lv2ControlBase::controlCount() const {
	std::size_t res = 0;
	for (const auto& c : m_procs) { res += c->controlCount(); }
	return res;
}




bool Lv2ControlBase::hasNoteInput() const
{
	return std::any_of(m_procs.begin(), m_procs.end(),
		[](const auto& c) { return c->hasNoteInput(); });
}




void Lv2ControlBase::handleMidiInputEvent(const MidiEvent &event,
	const TimePos &time, f_cnt_t offset)
{
	for (const auto& c : m_procs) { c->handleMidiInputEvent(event, time, offset); }
}


} // namespace lmms


#endif // LMMS_HAVE_LV2
