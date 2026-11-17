/*
 * AudioJack.cpp - support for JACK transport
 *
 * Copyright (c) 2005-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "AudioJack.h"

#ifdef LMMS_HAVE_JACK

#include <QStringList>

#include "AudioEngine.h"
#include "ConfigManager.h"
#include "MidiJack.h"
#include "lmmsversion.h"
#include "UserNotice.h"

#include <algorithm>
#include <cstdio>

namespace lmms {
namespace {

void printJackStatus(jack_status_t status)
{
	std::fprintf(stderr, "Status: 0x%2.0x\n", static_cast<unsigned int>(status));

	if (status & JackFailure)
	{
		std::fprintf(stderr, "Overall operation failed. JACK dependencies might need to be installed.\n");
	}

	if (status & JackServerFailed)
	{
		std::fprintf(stderr, "Could not connect to JACK server.\n");
	}
}

} // namespace

using namespace JackSettings;

auto JackSettings::inputCount() -> int
{
	return std::clamp(ConfigManager::inst()->value(audioJackClass, inputsKey, "2").toInt(), 0, MaxInputs);
}

AudioJack::AudioJack(bool& successful, AudioEngine* audioEngineParam)
	: AudioDevice(
		DEFAULT_CHANNELS,
		audioEngineParam)
	, m_client(nullptr)
	, m_active(false)
	, m_midiClient(nullptr)
	, m_tempOutBufs(new jack_default_audio_sample_t*[channels()])
{
	successful = initJackClient();
	if (successful) {
		connect(this, SIGNAL(zombified()), this, SLOT(restartAfterZombified()), Qt::QueuedConnection);
	}
}




AudioJack::~AudioJack()
{
#ifdef AUDIO_BUS_HANDLE_SUPPORT
	while (m_portMap.size())
	{
		unregisterPort(m_portMap.begin().key());
	}
#endif

	if (m_client != nullptr)
	{
		if (m_active) { jack_deactivate(m_client); }
		jack_client_close(m_client);
	}

	delete[] m_tempOutBufs;
}




void AudioJack::tellUser(const QString& title, const QString& text)
{
	// (from any thread: the notice reaches the user, if there is one, on the main thread)
	UserNotice::information(title, text);
}




void AudioJack::restartAfterZombified()
{
	if (initJackClient())
	{
		m_active = false;
		startProcessing();
		tellUser(tr("JACK client restarted"),
			tr(	"LinuxMusic was kicked by JACK for some reason. "
				"Therefore the JACK backend of LinuxMusic has been "
				"restarted. You will have to make manual "
				"connections again."));
	}
	else
	{
		tellUser(tr("JACK server down"),
			tr(	"The JACK server seems to have been shutdown "
				"and starting a new instance failed. "
				"Therefore LinuxMusic is unable to proceed. "
				"You should save your project and restart "
				"JACK and LinuxMusic."));
	}
}




AudioJack* AudioJack::addMidiClient(MidiJack* midiClient)
{
	if (m_client == nullptr) { return nullptr; }

	m_midiClient = midiClient;

	return this;
}


bool AudioJack::initJackClient()
{
	QString clientName = ConfigManager::inst()->value(audioJackClass, clientNameKey);
	if (clientName.isEmpty()) { clientName = LMMS_PROJECT_ID; }

	// Will attempt to start the JACK server
	jack_status_t status;
	m_client = jack_client_open(clientName.toLatin1().constData(), JackNullOption, &status);
	if (m_client == nullptr)
	{
		std::fprintf(stderr, "jack_client_open() failed, ");
		printJackStatus(status);

		return false;
	}
	if (status & JackNameNotUnique)
	{
		std::printf("there's already a client with name '%s', so unique "
					"name '%s' was assigned\n",
					clientName.toLatin1().constData(), jack_get_client_name(m_client));
	}

	// set process-callback
	jack_set_process_callback(m_client, staticProcessCallback, this);

	// set shutdown-callback
	jack_on_shutdown(m_client, shutdownCallback, this);

	if (jack_get_sample_rate(m_client) != sampleRate()) { setSampleRate(jack_get_sample_rate(m_client)); }

	for (ch_cnt_t ch = 0; ch < channels(); ++ch)
	{
		const QString name = buildOutputName(ch);
		m_outputPorts.push_back(
			jack_port_register(m_client, name.toLatin1().constData(), JACK_DEFAULT_AUDIO_TYPE, JackPortIsOutput, 0));
		if (m_outputPorts.back() == nullptr)
		{
			std::fprintf(stderr, "no more JACK-ports available!\n");
			return false;
		}
	}

	// As many inputs as the setup asks for: the tracks record from them
	for (int ch = 0; ch < inputCount(); ++ch)
	{
		const QString inputName = buildInputName(static_cast<ch_cnt_t>(ch));
		m_inputPorts.push_back(
			jack_port_register(m_client, inputName.toLatin1().constData(), JACK_DEFAULT_AUDIO_TYPE, JackPortIsInput, 0));
		if (m_inputPorts.back() == nullptr)
		{
			std::fprintf(stderr, "no more JACK-ports available!\n");
			m_inputPorts.pop_back();
			break;
		}
	}

	// The input ports are read in the same cycle as the output ports are written
	m_tempInBufs.assign(m_inputPorts.size(), nullptr);
	setInputChannels(static_cast<ch_cnt_t>(m_inputPorts.size()));

	return true;
}




void AudioJack::attemptToConnect(size_t index, const char *lmms_port_type, const char *source_port, const char *destination_port)
{
	std::printf("Attempting to reconnect %s port %u: %s -> %s", lmms_port_type, static_cast<unsigned int>(index), source_port, destination_port);
	if (!jack_connect(m_client, source_port, destination_port))
	{
		std::printf(" - Success!\n");
	}
	else
	{
		std::printf(" - Failure\n");
	}
}

void AudioJack::attemptToReconnectOutput(size_t outputIndex, const QString& targetPort)
{
	if (outputIndex >= m_outputPorts.size()) { return; }

	if (targetPort == disconnectedRepresentation)
	{
		std::fprintf(stderr, "Output port %u is not connected.\n", static_cast<unsigned int>(outputIndex));
		return;
	}

	auto outputName = jack_port_name(m_outputPorts[outputIndex]);
	auto targetName = targetPort.toLatin1();

	attemptToConnect(outputIndex, "output", outputName, targetName.constData());
}

void AudioJack::attemptToReconnectInput(size_t inputIndex, const QString& sourcePort)
{
	if (inputIndex >= m_inputPorts.size()) { return; }

	if (sourcePort == disconnectedRepresentation)
	{
		std::fprintf(stderr, "Input port %u is not connected.\n", static_cast<unsigned int>(inputIndex));
		return;
	}

	auto inputName = jack_port_name(m_inputPorts[inputIndex]);
	auto sourceName = sourcePort.toLatin1();

	attemptToConnect(inputIndex, "input", sourceName.constData(), inputName);
}


void AudioJack::startProcessingImpl()
{
	if (m_active || m_client == nullptr) { return; }

	if (jack_activate(m_client))
	{
		std::fprintf(stderr, "cannot activate client\n");
		return;
	}

	m_active = true;

	// try to sync JACK's and LMMS's buffer-size
	//	jack_set_buffer_size( m_client, audioEngine()->framesPerPeriod() );

	const auto cm = ConfigManager::inst();

	const auto numberOfChannels = channels();
	for (size_t i = 0; i < numberOfChannels; ++i)
	{
		attemptToReconnectOutput(i, cm->value(audioJackClass, getOutputKeyByChannel(i)));
	}

	for (size_t i = 0; i < m_inputPorts.size(); ++i)
	{
		attemptToReconnectInput(i, cm->value(audioJackClass, getInputKeyByChannel(i)));
	}
}




auto AudioJack::inputChannelName(ch_cnt_t channel) const -> QString
{
	if (channel >= m_inputPorts.size()) { return AudioDevice::inputChannelName(channel); }

	// What feeds the port says most, e.g. "system:capture_3"; otherwise the port's own name
	auto name = QString::fromLatin1(jack_port_short_name(m_inputPorts[channel]));
	if (const char** connections = jack_port_get_connections(m_inputPorts[channel]))
	{
		name = QString::fromLatin1(connections[0]);
		jack_free(connections);
	}
	return name;
}




auto AudioJack::inputLatency() const -> f_cnt_t
{
	// The capture latency of what is connected to our inputs, as JACK propagates it
	auto latency = jack_nframes_t{0};
	for (auto* port : m_inputPorts)
	{
		auto range = jack_latency_range_t{};
		jack_port_get_latency_range(port, JackCaptureLatency, &range);
		latency = std::max(latency, range.max);
	}
	return latency;
}




auto AudioJack::outputLatency() const -> f_cnt_t
{
	// JACK's playback latency of our outputs: from the cycle that writes them to the physical outputs
	auto latency = jack_nframes_t{0};
	for (auto* port : m_outputPorts)
	{
		auto range = jack_latency_range_t{};
		jack_port_get_latency_range(port, JackPlaybackLatency, &range);
		latency = std::max(latency, range.max);
	}
	return latency;
}




void AudioJack::stopProcessingImpl()
{
}

void AudioJack::registerPort(AudioBusHandle* port)
{
#ifdef AUDIO_BUS_HANDLE_SUPPORT
	// make sure, port is not already registered
	unregisterPort(port);
	const QString name[2] = {port->name() + " L", port->name() + " R"};

	for (ch_cnt_t ch = 0; ch < DEFAULT_CHANNELS; ++ch)
	{
		m_portMap[port].ports[ch] = jack_port_register(
			m_client, name[ch].toLatin1().constData(), JACK_DEFAULT_AUDIO_TYPE, JackPortIsOutput, 0);
	}
#else
	(void)port;
#endif
}




void AudioJack::unregisterPort(AudioBusHandle* port)
{
#ifdef AUDIO_BUS_HANDLE_SUPPORT
	if (m_portMap.contains(port))
	{
		for (ch_cnt_t ch = 0; ch < DEFAULT_CHANNELS; ++ch)
		{
			if (m_portMap[port].ports[ch] != nullptr) { jack_port_unregister(m_client, m_portMap[port].ports[ch]); }
		}
		m_portMap.erase(m_portMap.find(port));
	}
#else
	(void)port;
#endif
}

void AudioJack::renamePort(AudioBusHandle* port)
{
#ifdef AUDIO_BUS_HANDLE_SUPPORT
	if (m_portMap.contains(port))
	{
		const QString name[2] = {port->name() + " L", port->name() + " R"};
		for (ch_cnt_t ch = 0; ch < DEFAULT_CHANNELS; ++ch)
		{
#ifdef LMMS_HAVE_JACK_PRENAME
			jack_port_rename(m_client, m_portMap[port].ports[ch], name[ch].toLatin1().constData());
#else
			jack_port_set_name(m_portMap[port].ports[ch], name[ch].toLatin1().constData());
#endif
		}
	}
#else
	(void)port;
#endif // AUDIO_BUS_HANDLE_SUPPORT
}




int AudioJack::processCallback(jack_nframes_t nframes)
{
	// do midi processing first so that midi input can
	// add to the following sound processing
	if (m_midiClient && nframes > 0)
	{
		m_midiClient.load()->JackMidiRead(nframes);
		m_midiClient.load()->JackMidiWrite(nframes);
	}

	for (int c = 0; c < channels(); ++c)
	{
		m_tempOutBufs[c] = (jack_default_audio_sample_t*)jack_port_get_buffer(m_outputPorts[c], nframes);
	}

#ifdef AUDIO_BUS_HANDLE_SUPPORT
	const int frames = std::min<int>(nframes, audioEngine()->framesPerAudioBuffer());
	for (JackPortMap::iterator it = m_portMap.begin(); it != m_portMap.end(); ++it)
	{
		for (ch_cnt_t ch = 0; ch < channels(); ++ch)
		{
			if (it.value().ports[ch] == nullptr) { continue; }
			jack_default_audio_sample_t* buf
				= (jack_default_audio_sample_t*)jack_port_get_buffer(it.value().ports[ch], nframes);
			for (int frame = 0; frame < frames; ++frame)
			{
				buf[frame] = it.key()->buffer()[frame][ch];
			}
		}
	}
#endif

	if (!isRunning())
	{
		for (int c = 0; c < channels(); ++c)
		{
			std::fill_n(m_tempOutBufs[c], nframes, 0.f);
		}
	}
	else
	{
		for (std::size_t c = 0; c < m_inputPorts.size(); ++c)
		{
			m_tempInBufs[c] = static_cast<jack_default_audio_sample_t*>(jack_port_get_buffer(m_inputPorts[c], nframes));
		}
		const auto input = PlanarBufferView<const float>{m_tempInBufs.data(),
			static_cast<ch_cnt_t>(m_tempInBufs.size()), nframes};
		audioEngine()->renderNextBuffer(input, PlanarBufferView<float>{m_tempOutBufs, channels(), nframes});
	}
	return 0;
}




int AudioJack::staticProcessCallback(jack_nframes_t nframes, void* udata)
{
	return static_cast<AudioJack*>(udata)->processCallback(nframes);
}




void AudioJack::shutdownCallback(void* udata)
{
	auto thisClass = static_cast<AudioJack*>(udata);
	thisClass->m_client = nullptr;
	emit thisClass->zombified();
}

} // namespace lmms

#endif // LMMS_HAVE_JACK
