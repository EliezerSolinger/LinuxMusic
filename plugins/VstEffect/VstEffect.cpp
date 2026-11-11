/*
 * VstEffect.cpp - class for handling VST effect plugins
 *
 * Copyright (c) 2006-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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


#include "VstEffect.h"

#include <QMutexLocker>

#include "GuiApplication.h"
#include "Host.h"
#include "Song.h"
#include "TextFloat.h"
#include "VstPlugin.h"
#include "VstSubPluginFeatures.h"

#include "embed.h"
#include "plugin_export.h"

namespace lmms
{


extern "C"
{

Plugin::Descriptor PLUGIN_EXPORT vsteffect_plugin_descriptor =
{
	LMMS_STRINGIFY( PLUGIN_NAME ),
	"VST",
	QT_TRANSLATE_NOOP( "PluginBrowser",
				"plugin for using arbitrary VST effects inside LinuxMusic." ),
	"Tobias Doerffel <tobydox/at/users.sf.net>",
	0x0200,
	Plugin::Type::Effect,
	new PluginPixmapLoader("logo"),
	nullptr,
	new VstSubPluginFeatures( Plugin::Type::Effect )
} ;

}


VstEffect::VstEffect( Model * _parent,
			const Descriptor::SubPluginFeatures::Key * _key ) :
	Effect(&vsteffect_plugin_descriptor, _parent, _key),
	m_pluginMutex(),
	m_key( *_key ),
	m_vstControls( this )
{
	bool loaded = false;
	if( !m_key.attributes["file"].isEmpty() )
	{
		loaded = openPlugin(m_key.attributes["file"]);
	}
	setDisplayName( m_key.attributes["file"].section( ".dll", 0, 0 ).isEmpty()
		? m_key.name : m_key.attributes["file"].section( ".dll", 0, 0 ) );

	setDontRun(!loaded);
}




auto VstEffect::channelCount(bool isInput) const -> ch_cnt_t
{
	// Without a plugin, keep the stereo pins
	if (!m_plugin) { return DEFAULT_CHANNELS; }
	return isInput ? m_plugin->remoteChannelsIn() : m_plugin->remoteChannelsOut();
}




auto VstEffect::buffersMatchRemote() const -> bool
{
	return m_plugin
		&& m_sharedBufferInfo.channels == m_plugin->remoteChannelsIn() + m_plugin->remoteChannelsOut();
}




void VstEffect::onSharedMemoryChange(const SharedBufferInfo& info)
{
	m_sharedBufferKey = std::string{info.key};
	m_sharedBufferInfo = info;
	m_sharedBufferInfo.key = m_sharedBufferKey;

	const auto lock = QMutexLocker{&m_pluginMutex};
	if (m_plugin) { m_plugin->setSharedAudioBuffer(m_sharedBufferInfo); }
}




auto VstEffect::process(ProcessContext& context) -> ProcessStatus
{
	auto& out = context.out();

	// The plugin may be being replaced; while exporting, wait for it
	if (!m_pluginMutex.tryLock(Engine::getSong()->isExporting() ? -1 : 0))
	{
		out.silenceAllChannels();
		return ProcessStatus::Continue;
	}

	// Until the host has set up buffers for the channel counts the remote process reported
	// (see channelCountsChanged), the remote process cannot process
	const bool processed = buffersMatchRemote() && m_plugin->process();
	m_pluginMutex.unlock();

	if (!processed)
	{
		out.silenceAllChannels();
		return ProcessStatus::Sleep;
	}

	const auto in = channelCount(true) > 0 ? &context.in() : nullptr;

	// Lastly, perform wet/dry mixing on the output channels.
	// This assumes that the first 1-2 output channels are the main output channels,
	// and the dry signal for those output channels are the first 1-2 input channels.
	// Wet/dry mixing only applies to those channels and any additional
	// channels remain as-is.

	if (!in || in->channels() == 0)
	{
		// Do not process wet/dry for an instrument loaded as an effect
		// TODO: Prevent instruments from loading as effects?
		return ProcessStatus::ContinueIfNotQuiet;
	}

	const float w = wetLevel();
	const float d = dryLevel();

	const auto mixableOutputs = std::min<ch_cnt_t>(out.channels(), 2);
	for (ch_cnt_t channel = 0; channel < mixableOutputs; ++channel)
	{
		auto wetBuffer = out.buffer(channel);
		auto dryBuffer = in->buffer(std::min<ch_cnt_t>(channel, in->channels() - 1));
		for (f_cnt_t f = 0; f < out.frames(); ++f)
		{
			wetBuffer[f] = w * wetBuffer[f] + d * dryBuffer[f];
		}
	}

	return ProcessStatus::ContinueIfNotQuiet;
}




bool VstEffect::openPlugin(const QString& plugin)
{
	gui::TextFloat* tf = nullptr;
	if( gui::getGUI() != nullptr )
	{
		tf = gui::TextFloat::displayMessage(
			VstPlugin::tr( "Loading plugin" ),
			VstPlugin::tr( "Please wait while loading VST plugin..." ),
				PLUGIN_NAME::getIconPixmap( "logo", 24, 24 ), 0 );
	}

	{
		QMutexLocker ml( &m_pluginMutex ); Q_UNUSED( ml );
		m_plugin = QSharedPointer<VstPlugin>(new VstPlugin{plugin});
		if( m_plugin->failed() )
		{
			m_plugin.clear();
			delete tf;
			collectErrorForUI(VstPlugin::tr("The VST plugin %1 could not be loaded.").arg(plugin));
			return false;
		}
	}

	// The remote process reports its channel counts, also later on (e.g. when the plugin changes them);
	// the host then sets up new buffers (in the main thread, since this may come from the audio thread)
	connect(m_plugin.data(), &RemotePlugin::channelCountsChanged, this, [this] {
		if (host()) { host()->rescanAudioPorts(Host::AudioPortsRescanFlag::ChannelCount); }
	}, Qt::QueuedConnection);

	delete tf;

	m_key.attributes["file"] = plugin;

	// New channel counts and a new remote process which needs to know where the buffers are
	if (host()) { host()->rescanAudioPorts(Host::AudioPortsRescanFlag::ChannelCount); }
	return true;
}




extern "C"
{

// necessary for getting instance out of shared lib
PLUGIN_EXPORT Plugin * lmms_plugin_main( Model * _parent, void * _data )
{
	return new VstEffect( _parent,
		static_cast<const Plugin::Descriptor::SubPluginFeatures::Key *>(
								_data ) );
}

}


} // namespace lmms
