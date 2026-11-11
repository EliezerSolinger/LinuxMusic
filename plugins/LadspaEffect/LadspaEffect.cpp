/*
 * LadspaEffect.cpp - class for processing LADSPA effects
 *
 * Copyright (c) 2006-2008 Danny McRae <khjklujn/at/users.sourceforge.net>
 * Copyright (c) 2009-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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


#include <algorithm>
#include <cstring>

#include <QMessageBox>

#include "LadspaEffect.h"
#include "DataFile.h"
#include "AudioEngine.h"
#include "Ladspa2LMMS.h"
#include "LadspaBase.h"
#include "LadspaControl.h"
#include "LadspaSubPluginFeatures.h"
#include "AutomationClip.h"
#include "ValueBuffer.h"
#include "Song.h"

#include "embed.h"

#include "plugin_export.h"

namespace lmms
{


extern "C"
{

Plugin::Descriptor PLUGIN_EXPORT ladspaeffect_plugin_descriptor =
{
	LMMS_STRINGIFY( PLUGIN_NAME ),
	"LADSPA",
	QT_TRANSLATE_NOOP( "PluginBrowser",
				"plugin for using arbitrary LADSPA-effects "
				"inside LinuxMusic." ),
	"Danny McRae <khjklujn/at/users.sourceforge.net>",
	0x0100,
	Plugin::Type::Effect,
	new PluginPixmapLoader("logo"),
	nullptr,
	new LadspaSubPluginFeatures( Plugin::Type::Effect )
} ;

}

LadspaEffect::LadspaEffect(Model* _parent, const Descriptor::SubPluginFeatures::Key* _key)
	: Effect(&ladspaeffect_plugin_descriptor, _parent, _key)
	, m_controls(nullptr)
	, m_key(LadspaSubPluginFeatures::subPluginKeyToLadspaKey(_key))
{
	Ladspa2LMMS * manager = Engine::getLADSPAManager();
	if( manager->getDescription( m_key ) == nullptr )
	{
		Engine::getSong()->collectError(tr( "Unknown LADSPA plugin %1 requested." ).arg(
											m_key.second ) );
		setOkay( false );
		return;
	}

	setDisplayName( manager->getShortName( m_key ) );

	pluginInstantiation(Engine::audioEngine()->outputSampleRate(), Engine::audioEngine()->framesPerPeriod());
}




LadspaEffect::~LadspaEffect()
{
	pluginDestruction();
}




void LadspaEffect::changeSampleRate(sample_rate_t sampleRate)
{
	DataFile dataFile( DataFile::Type::EffectSettings );
	m_controls->saveState( dataFile, dataFile.content() );

	LadspaControls * controls = m_controls;
	m_controls = nullptr;

	pluginDestruction();
	pluginInstantiation(sampleRate, m_maxFrames);

	controls->effectModelChanged( m_controls );
	delete controls;

	m_controls->restoreState( dataFile.content().firstChild().toElement() );

	// the IDs of re-created controls have been saved and now need to be
	// resolved again
	AutomationClip::resolveAllIDs();
}




auto LadspaEffect::activate(double sampleRate, f_cnt_t maxFrameCount) -> bool
{
	if (!isOkay() || m_descriptor == nullptr) { return true; }

	const auto rate = static_cast<sample_rate_t>(sampleRate);
	if (maxFrameCount > m_maxFrames)
	{
		// The audio-rate control ports need bigger buffers
		m_maxFrames = maxFrameCount;
		changeSampleRate(rate);
	}
	else if (rate != m_sampleRate)
	{
		changeSampleRate(rate);
	}

	if (!isOkay()) { return true; }

	activateHandles();
	return true;
}




void LadspaEffect::deactivate()
{
	deactivateHandles();
}




void LadspaEffect::activateHandles()
{
	if (m_handlesActive) { return; }

	Ladspa2LMMS * manager = Engine::getLADSPAManager();
	for (const auto handle : m_handles)
	{
		manager->activate(m_key, handle);
	}
	m_handlesActive = true;
}




void LadspaEffect::deactivateHandles()
{
	if (!m_handlesActive) { return; }

	Ladspa2LMMS * manager = Engine::getLADSPAManager();
	for (const auto handle : m_handles)
	{
		manager->deactivate(m_key, handle);
	}
	m_handlesActive = false;
}




auto LadspaEffect::latency() const -> f_cnt_t
{
	if (!m_latencyPort) { return 0; }

	// Written by the plugin while processing; ignore nonsense
	const float frames = m_latencyPort->buffer[0];
	return frames >= 1.f && frames < 10.f * m_sampleRate ? static_cast<f_cnt_t>(frames + 0.5f) : 0;
}




auto LadspaEffect::channelCount(bool isInput) const -> ch_cnt_t
{
	return static_cast<ch_cnt_t>(m_processors * (isInput ? m_inputsPerProcessor : m_outputsPerProcessor));
}




auto LadspaEffect::channelGroupCount(bool isInput) const -> group_cnt_t
{
	(void)isInput;
	return 1;
}




auto LadspaEffect::getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup& group) const
	-> ch_cnt_t
{
	// All of the plugin's channels form one group
	(void)index;
	(void)group;
	return channelCount(isInput);
}




auto LadspaEffect::getInplacePair(ch_cnt_t inputIndex) const -> std::optional<ch_cnt_t>
{
	// The input is still needed after running the plugin for the dry signal, so the
	// inputs and outputs never share buffers
	(void)inputIndex;
	return std::nullopt;
}




ProcessStatus LadspaEffect::process(ProcessContext& context)
{
	const AudioBuffer& in = context.in();
	AudioBuffer& out = context.out();
	const auto frames = std::min(out.frames(), m_maxFrames);

	Ladspa2LMMS * manager = Engine::getLADSPAManager();

	// Connect the audio ports to the host's buffers and set the control ports.
	std::size_t audioPort = 0;
	for( ch_cnt_t proc = 0; proc < processorCount(); ++proc )
	{
		ch_cnt_t input = proc * m_inputsPerProcessor;
		ch_cnt_t output = proc * m_outputsPerProcessor;
		for( int port = 0; port < m_portCount; ++port )
		{
			port_desc_t * pp = m_ports.at( proc ).at( port );
			switch( pp->rate )
			{
				case BufferRate::ChannelIn:
				case BufferRate::ChannelOut:
				{
					// LADSPA does not have const input ports, but plugins must not write to their inputs
					auto buffer = pp->rate == BufferRate::ChannelIn
						? const_cast<LADSPA_Data*>(in.buffer(input++).data())
						: out.buffer(output++).data();
					if (m_connectedAudioBuffers[audioPort] != buffer)
					{
						manager->connectPort(m_key, m_handles[proc], port, buffer);
						m_connectedAudioBuffers[audioPort] = buffer;
					}
					++audioPort;
					break;
				}
				case BufferRate::AudioRateInput:
				{
					ValueBuffer * vb = pp->control->valueBuffer();
					if( vb )
					{
						std::memcpy(pp->buffer, vb->values(), frames * sizeof(float));
					}
					else
					{
						pp->value = static_cast<LADSPA_Data>(
											pp->control->value() / pp->scale );
						// This only supports control rate ports, so the audio rates are
						// treated as though they were control rate by setting the
						// port buffer to all the same value.
						std::fill_n(pp->buffer, frames, pp->value);
					}
					break;
				}
				case BufferRate::ControlRateInput:
					if( pp->control == nullptr )
					{
						break;
					}
					pp->value = static_cast<LADSPA_Data>(
										pp->control->value() / pp->scale );
					pp->buffer[0] =
						pp->value;
					break;
				case BufferRate::AudioRateOutput:
				case BufferRate::ControlRateOutput:
					break;
				default:
					break;
			}
		}
	}

	// Process the buffers.
	for( ch_cnt_t proc = 0; proc < processorCount(); ++proc )
	{
		(m_descriptor->run)(m_handles[proc], frames);
	}

	// Mix the dry signal into the outputs. Outputs without a matching input have no dry signal.
	const float d = dryLevel();
	const float w = wetLevel();
	if (d != 0.f || w != 1.f)
	{
		const auto inputs = static_cast<ch_cnt_t>(in.channels());
		for (ch_cnt_t channel = 0; channel < out.channels(); ++channel)
		{
			auto wet = out.buffer(channel);
			if (channel < inputs)
			{
				const auto dry = in.buffer(channel);
				for (f_cnt_t frame = 0; frame < frames; ++frame)
				{
					wet[frame] = d * dry[frame] + w * wet[frame];
				}
			}
			else
			{
				for (f_cnt_t frame = 0; frame < frames; ++frame)
				{
					wet[frame] *= w;
				}
			}
		}
	}

	return ProcessStatus::ContinueIfNotQuiet;
}




void LadspaEffect::setControl( int _control, LADSPA_Data _value )
{
	if( !isOkay() )
	{
		return;
	}
	m_portControls[_control]->value = _value;
}




void LadspaEffect::pluginInstantiation(sample_rate_t sampleRate, f_cnt_t maxFrames)
{
	Ladspa2LMMS * manager = Engine::getLADSPAManager();
	m_sampleRate = sampleRate;
	m_maxFrames = maxFrames;

	// Mono plugins run once per stereo channel; all others run once with all of their channels.
	const auto description = manager->getDescription( m_key );
	m_inputsPerProcessor = description->inputChannels;
	m_outputsPerProcessor = description->outputChannels;
	m_processors = m_inputsPerProcessor == 1 && m_outputsPerProcessor == 1 ? DEFAULT_CHANNELS : 1;

	// Categorize the ports, and create the buffers. The audio channels use the host's buffers.
	m_portCount = manager->getPortCount( m_key );

	for( ch_cnt_t proc = 0; proc < processorCount(); proc++ )
	{
		multi_proc_t ports;
		for( int port = 0; port < m_portCount; port++ )
		{
			auto p = new port_desc_t;

			p->name = manager->getPortName( m_key, port );
			p->proc = proc;
			p->port_id = port;
			p->control = nullptr;
			p->buffer = nullptr;

			// Determine the port's category.
			if( manager->isPortAudio( m_key, port ) )
			{
				// Same classification as LadspaManager::getPluginInputs() and getPluginOutputs()
				if( p->name.toUpper().contains( "IN" ) &&
					manager->isPortInput( m_key, port ) )
				{
					p->rate = BufferRate::ChannelIn;
				}
				else if( p->name.toUpper().contains( "OUT" ) &&
					manager->isPortOutput( m_key, port ) )
				{
					p->rate = BufferRate::ChannelOut;
				}
				else if( manager->isPortInput( m_key, port ) )
				{
					p->rate = BufferRate::AudioRateInput;
					p->buffer = new LADSPA_Data[maxFrames];
				}
				else
				{
					p->rate = BufferRate::AudioRateOutput;
					p->buffer = new LADSPA_Data[maxFrames];
				}
			}
			else
			{
				p->buffer = new LADSPA_Data[1];

				if( manager->isPortInput( m_key, port ) )
				{
					p->rate = BufferRate::ControlRateInput;
				}
				else
				{
					p->rate = BufferRate::ControlRateOutput;
					p->buffer[0] = 0.f;
					// The usual way for LADSPA plugins to report their latency
					if (proc == 0 && p->name.compare("latency", Qt::CaseInsensitive) == 0) { m_latencyPort = p; }
				}
			}

			p->scale = 1.0f;
			if( manager->isEnum( m_key, port ) )
			{
				p->data_type = BufferDataType::Enum;
			}
			else if( manager->isPortToggled( m_key, port ) )
			{
				p->data_type = BufferDataType::Toggled;
			}
			else if( manager->isInteger( m_key, port ) )
			{
				p->data_type = BufferDataType::Integer;
			}
			else if( p->name.toUpper().contains( "(SECONDS)" ) )
			{
				p->data_type = BufferDataType::Time;
				p->scale = 1000.0f;
				int loc = p->name.toUpper().indexOf(
								"(SECONDS)" );
				p->name.replace( loc, 9, "(ms)" );
			}
			else if( p->name.toUpper().contains( "(S)" ) )
			{
				p->data_type = BufferDataType::Time;
				p->scale = 1000.0f;
				int loc = p->name.toUpper().indexOf( "(S)" );
				p->name.replace( loc, 3, "(ms)" );
			}
			else if( p->name.toUpper().contains( "(MS)" ) )
			{
				p->data_type = BufferDataType::Time;
				int loc = p->name.toUpper().indexOf( "(MS)" );
				p->name.replace( loc, 4, "(ms)" );
			}
			else
			{
				p->data_type = BufferDataType::Floating;
			}

			// Get the range and default values.
			p->max = manager->getUpperBound( m_key, port );
			if( p->max == NOHINT )
			{
				p->max = p->name.toUpper() == "GAIN" ? 10.0f :
					1.0f;
			}

			if( manager->areHintsSampleRateDependent(
								m_key, port ) )
			{
				p->max *= sampleRate;
			}

			p->min = manager->getLowerBound( m_key, port );
			if( p->min == NOHINT )
			{
				p->min = 0.0f;
			}

			if( manager->areHintsSampleRateDependent(
								m_key, port ) )
			{
				p->min *= sampleRate;
			}

			p->def = manager->getDefaultSetting( m_key, port );
			if( p->def == NOHINT )
			{
				if( p->data_type != BufferDataType::Toggled )
				{
					p->def = ( p->min + p->max ) / 2.0f;
				}
				else
				{
					p->def = 1.0f;
				}
			}
			else if( manager->areHintsSampleRateDependent( m_key, port ) )
			{
				p->def *= sampleRate;
			}


			p->max *= p->scale;
			p->min *= p->scale;
			p->def *= p->scale;

			p->value = p->def;

			p->suggests_logscale = manager->isLogarithmic( m_key, port );

			ports.append( p );

	// For convenience, keep a separate list of the ports that are used
	// to control the processors.
			if( p->rate == BufferRate::AudioRateInput ||
					p->rate == BufferRate::ControlRateInput )
			{
				p->control_id = m_portControls.count();
				m_portControls.append( p );
			}
		}
		m_ports.append( ports );
	}

	// The controls only need the ports, so they exist even if the plugin fails to instantiate below
	m_controls = new LadspaControls( this );

	// Instantiate the processing units.
	m_descriptor = manager->getDescriptor( m_key );
	if( m_descriptor == nullptr )
	{
		QMessageBox::warning( 0, "Effect",
			"Can't get LADSPA descriptor function: " + m_key.second,
			QMessageBox::Ok, QMessageBox::NoButton );
		setOkay( false );
		return;
	}
	if( m_descriptor->run == nullptr )
	{
		QMessageBox::warning( 0, "Effect",
			"Plugin has no processor: " + m_key.second,
			QMessageBox::Ok, QMessageBox::NoButton );
		setDontRun( true );
	}
	for( ch_cnt_t proc = 0; proc < processorCount(); proc++ )
	{
		LADSPA_Handle effect = manager->instantiate(m_key, sampleRate);
		if( effect == nullptr )
		{
			QMessageBox::warning( 0, "Effect",
				"Can't get LADSPA instance: " + m_key.second,
				QMessageBox::Ok, QMessageBox::NoButton );
			setOkay( false );
			return;
		}
		m_handles.append( effect );
	}

	// Connect the control ports. The audio ports are connected to the host's buffers in process().
	m_connectedAudioBuffers.clear();
	for( ch_cnt_t proc = 0; proc < processorCount(); proc++ )
	{
		for( int port = 0; port < m_portCount; port++ )
		{
			port_desc_t * pp = m_ports.at( proc ).at( port );
			if (pp->rate == BufferRate::ChannelIn || pp->rate == BufferRate::ChannelOut)
			{
				m_connectedAudioBuffers.push_back(nullptr);
				continue;
			}
			if( !manager->connectPort( m_key,
						m_handles[proc],
						port,
						pp->buffer ) )
			{
				QMessageBox::warning( 0, "Effect",
				"Failed to connect port: " + m_key.second,
				QMessageBox::Ok, QMessageBox::NoButton );
				setDontRun( true );
				return;
			}
		}
	}

	// The processing units are activated in activate()
}




void LadspaEffect::pluginDestruction()
{
	delete m_controls;
	m_controls = nullptr;
	m_latencyPort = nullptr;

	// LADSPA requires deactivate() before cleanup() for activated instances
	deactivateHandles();

	Ladspa2LMMS * manager = Engine::getLADSPAManager();
	for( ch_cnt_t proc = 0; proc < m_ports.size(); proc++ )
	{
		if (proc < m_handles.size())
		{
			manager->cleanup( m_key, m_handles[proc] );
		}
		for (const auto pp : m_ports[proc])
		{
			// The audio channel ports do not own a buffer
			delete[] pp->buffer;
			delete pp;
		}
		m_ports[proc].clear();
	}
	m_connectedAudioBuffers.clear();
	m_ports.clear();
	m_handles.clear();
	m_portControls.clear();
}

extern "C"
{

// necessary for getting instance out of shared lib
PLUGIN_EXPORT Plugin * lmms_plugin_main( Model * _parent, void * _data )
{
	return new LadspaEffect( _parent,
		static_cast<const Plugin::Descriptor::SubPluginFeatures::Key *>(
								_data ) );
}

}


} // namespace lmms
