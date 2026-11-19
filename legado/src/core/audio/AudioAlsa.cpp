/*
 * AudioAlsa.cpp - ALSA driver: duplex, on the card's own format, exclusive
 *
 * Copyright (c) 2004-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
 *
 * This file is part of LinuxMusic, a fork of LMMS - https://lmms.io
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

#include "AudioAlsa.h"

#ifdef LMMS_HAVE_ALSA

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <fcntl.h>
#include <unistd.h>

#include "AudioDeviceReservation.h"
#include "AudioEngine.h"
#include "ConfigManager.h"

namespace lmms
{

namespace
{

//! The card's formats we take, the best first
constexpr snd_pcm_format_t Formats[] = {
	SND_PCM_FORMAT_FLOAT, SND_PCM_FORMAT_S32, SND_PCM_FORMAT_S24_3LE, SND_PCM_FORMAT_S24, SND_PCM_FORMAT_S16
};

//! The most channels we capture from one device
constexpr unsigned MaxCaptureChannels = 64;

//! Forked processes (e.g. plugin hosts) must not inherit the device's file descriptors
void closeOnExec(snd_pcm_t* pcm)
{
	const int count = snd_pcm_poll_descriptors_count(pcm);
	if (count <= 0) { return; }
	auto descriptors = std::vector<pollfd>(static_cast<std::size_t>(count));
	snd_pcm_poll_descriptors(pcm, descriptors.data(), static_cast<unsigned>(count));
	for (const auto& descriptor : descriptors)
	{
		const int flags = fcntl(descriptor.fd, F_GETFD, 0);
		if (flags >= 0) { fcntl(descriptor.fd, F_SETFD, flags | FD_CLOEXEC); }
	}
}

} // namespace




AudioAlsa::AudioAlsa(bool& successful, AudioEngine* audioEngine)
	: AudioDevice(DEFAULT_CHANNELS, audioEngine)
{
	successful = false;

	if (setenv("PULSE_ALSA_HOOK_CONF", "/dev/null", 0))
	{
		fprintf(stderr, "Could not avoid possible interception by PulseAudio\n");
	}

	const auto playbackDevice = probeDevice();
	const auto captureDevice = probeCaptureDevice();

	// A hardware device is exclusive: ask the sound server to give the card up first
	reserve(playbackDevice);
	if (!captureDevice.isEmpty()) { reserve(captureDevice); }

	if (!openStream(m_playback, playbackDevice, SND_PCM_STREAM_PLAYBACK, DEFAULT_CHANNELS, 2))
	{
		closeStream(m_playback);
		m_reservations.clear();
		return;
	}
	setChannels(static_cast<ch_cnt_t>(std::min<unsigned>(m_playback.channels, DEFAULT_CHANNELS)));

	if (!captureDevice.isEmpty())
	{
		// The capture runs in step with the playback: the same rate and period
		if (!openStream(m_capture, captureDevice, SND_PCM_STREAM_CAPTURE, 0, 4))
		{
			fprintf(stderr, "ALSA: no capture from %s; going on without input\n", captureDevice.toLocal8Bit().constData());
			closeStream(m_capture);
		}
		else if (m_capture.period != m_playback.period)
		{
			fprintf(stderr, "ALSA: the capture's period (%lu) differs from the playback's (%lu); going on without "
				"input\n", static_cast<unsigned long>(m_capture.period), static_cast<unsigned long>(m_playback.period));
			closeStream(m_capture);
		}
		else
		{
			// Linked, they start, stop and recover together, on one clock if they are on one card
			m_linked = snd_pcm_link(m_capture.pcm, m_playback.pcm) == 0;
			if (!m_linked)
			{
				fprintf(stderr, "ALSA: the capture and playback devices cannot be linked; they may drift apart\n");
			}
			setInputChannels(static_cast<ch_cnt_t>(m_capture.channels));
		}
	}

	// Planar scratch for the engine, one period
	const auto period = static_cast<std::size_t>(m_playback.period);
	m_outputStorage.assign(static_cast<std::size_t>(channels()) * period, 0.f);
	m_outputPointers.resize(channels());
	for (ch_cnt_t channel = 0; channel < channels(); ++channel)
	{
		m_outputPointers[channel] = m_outputStorage.data() + channel * period;
	}
	m_inputStorage.assign(static_cast<std::size_t>(m_capture.channels) * period, 0.f);
	m_inputPointers.resize(m_capture.channels);
	for (unsigned channel = 0; channel < m_capture.channels; ++channel)
	{
		m_inputPointers[channel] = m_inputStorage.data() + channel * period;
	}

	// What goes out waits behind the playback buffer; what comes in reaches us a period after it was captured
	m_outputLatency = static_cast<f_cnt_t>(m_playback.buffer);
	m_inputLatency = m_capture.pcm ? static_cast<f_cnt_t>(m_capture.period) : 0;

	successful = true;
}




AudioAlsa::~AudioAlsa()
{
	stopProcessing();
	if (m_linked) { snd_pcm_unlink(m_capture.pcm); }
	closeStream(m_capture);
	closeStream(m_playback);
	m_reservations.clear();
}




QString AudioAlsa::probeDevice()
{
	QString dev = ConfigManager::inst()->value( "audioalsa", "device" );
	if( dev == "" )
	{
		if( getenv( "AUDIODEV" ) != nullptr )
		{
			return getenv( "AUDIODEV" );
		}
		return "default";
	}
	return dev;
}




QString AudioAlsa::probeCaptureDevice()
{
	const auto device = ConfigManager::inst()->value("audioalsa", "inputdevice");
	if (device == NoCapture) { return {}; }
	return device.isEmpty() ? probeDevice() : device;
}




AudioAlsa::DeviceInfoCollection AudioAlsa::getAvailableDevices()
{
	DeviceInfoCollection deviceInfos;

	char** hints = nullptr;

	/* Enumerate sound devices */
	int err = snd_device_name_hint(-1, "pcm", (void***)&hints);
	if (err != 0)
	{
		return deviceInfos;
	}

	char** n = hints;
	while (*n != nullptr)
	{
		char *name = snd_device_name_get_hint(*n, "NAME");
		char *description = snd_device_name_get_hint(*n, "DESC");
		// "Input", "Output", or none for both
		char *direction = snd_device_name_get_hint(*n, "IOID");

		if (name != 0 && description != 0)
		{
			const bool input = direction == nullptr || std::strcmp(direction, "Input") == 0;
			const bool output = direction == nullptr || std::strcmp(direction, "Output") == 0;
			deviceInfos.push_back(DeviceInfo(QString(name), QString(description), output, input));
		}

		free(name);
		free(description);
		free(direction);

		n++;
	}

	//Free the hint buffer
	snd_device_name_free_hint((void**)hints);

	return deviceInfos;
}




void AudioAlsa::reserve(const QString& device)
{
#ifdef LMMS_HAVE_QTDBUS
	const auto card = AudioDeviceReservation::cardOfDevice(device);
	if (card < 0) { return; }
	for (const auto& reservation : m_reservations)
	{
		if (AudioDeviceReservation::cardOfDevice(reservation->applicationDeviceName()) == card) { return; }
	}
	auto reservation = std::make_unique<AudioDeviceReservation>(card, device);
	if (!reservation->acquire())
	{
		fprintf(stderr, "ALSA: card %d is in use by another program\n", card);
		return;
	}
	m_reservations.push_back(std::move(reservation));
#else
	(void)device;
#endif
}




auto AudioAlsa::openStream(Stream& stream, const QString& device, snd_pcm_stream_t direction, unsigned channels,
	unsigned periods) -> bool
{
	const bool capture = direction == SND_PCM_STREAM_CAPTURE;
	const char* what = capture ? "capture" : "playback";
	if (int err = snd_pcm_open(&stream.pcm, device.toLocal8Bit().constData(), direction, 0); err < 0)
	{
		fprintf(stderr, "ALSA: cannot open %s for %s: %s\n", device.toLocal8Bit().constData(), what, snd_strerror(err));
		stream.pcm = nullptr;
		return false;
	}

	snd_pcm_hw_params_t* hw = nullptr;
	snd_pcm_hw_params_alloca(&hw);
	if (int err = snd_pcm_hw_params_any(stream.pcm, hw); err < 0)
	{
		fprintf(stderr, "ALSA: no configuration for %s: %s\n", what, snd_strerror(err));
		return false;
	}

	// Interleaved if the card takes it, otherwise one buffer per channel
	if (snd_pcm_hw_params_set_access(stream.pcm, hw, SND_PCM_ACCESS_RW_INTERLEAVED) == 0) { stream.interleaved = true; }
	else if (snd_pcm_hw_params_set_access(stream.pcm, hw, SND_PCM_ACCESS_RW_NONINTERLEAVED) == 0)
	{
		stream.interleaved = false;
	}
	else
	{
		fprintf(stderr, "ALSA: no read/write access for %s\n", what);
		return false;
	}

	// The card's own format, the best it has
	stream.format = SND_PCM_FORMAT_UNKNOWN;
	for (const auto format : Formats)
	{
		if (snd_pcm_hw_params_test_format(stream.pcm, hw, format) == 0
			&& snd_pcm_hw_params_set_format(stream.pcm, hw, format) == 0)
		{
			stream.format = format;
			break;
		}
	}
	if (stream.format == SND_PCM_FORMAT_UNKNOWN)
	{
		fprintf(stderr, "ALSA: no sample format we know for %s\n", what);
		return false;
	}

	// Channels: as wished for the playback (a card may need more), all the card has for the capture
	auto wanted = channels;
	if (wanted == 0)
	{
		unsigned most = 0;
		snd_pcm_hw_params_get_channels_max(hw, &most);
		wanted = std::clamp(most, 1u, MaxCaptureChannels);
	}
	if (int err = snd_pcm_hw_params_set_channels_near(stream.pcm, hw, &wanted); err < 0)
	{
		fprintf(stderr, "ALSA: cannot set %u channels for %s: %s\n", wanted, what, snd_strerror(err));
		return false;
	}

	// The capture runs at the playback's rate and period
	unsigned rate = sampleRate(); // the playback's, once the playback is open
	if (int err = snd_pcm_hw_params_set_rate_near(stream.pcm, hw, &rate, nullptr); err < 0)
	{
		fprintf(stderr, "ALSA: cannot set the rate for %s: %s\n", what, snd_strerror(err));
		return false;
	}
	auto period = capture && m_playback.pcm
		? m_playback.period
		: static_cast<snd_pcm_uframes_t>(audioEngine()->framesPerAudioBuffer());
	int dir = 0;
	if (int err = snd_pcm_hw_params_set_period_size_near(stream.pcm, hw, &period, &dir); err < 0)
	{
		fprintf(stderr, "ALSA: cannot set the period for %s: %s\n", what, snd_strerror(err));
		return false;
	}
	unsigned periodCount = periods;
	dir = 0;
	if (snd_pcm_hw_params_set_periods_near(stream.pcm, hw, &periodCount, &dir) < 0)
	{
		auto buffer = period * periods;
		if (int err = snd_pcm_hw_params_set_buffer_size_near(stream.pcm, hw, &buffer); err < 0)
		{
			fprintf(stderr, "ALSA: cannot set the buffer for %s: %s\n", what, snd_strerror(err));
			return false;
		}
	}
	if (int err = snd_pcm_hw_params(stream.pcm, hw); err < 0)
	{
		fprintf(stderr, "ALSA: cannot configure %s: %s\n", what, snd_strerror(err));
		return false;
	}

	// What the card gave
	snd_pcm_hw_params_get_channels(hw, &stream.channels);
	snd_pcm_hw_params_get_rate(hw, &rate, nullptr);
	dir = 0;
	snd_pcm_hw_params_get_period_size(hw, &stream.period, &dir);
	snd_pcm_hw_params_get_buffer_size(hw, &stream.buffer);
	if (stream.buffer < 2 * stream.period)
	{
		fprintf(stderr, "ALSA: the %s buffer is too small (%lu frames)\n", what, static_cast<unsigned long>(stream.buffer));
		return false;
	}
	if (!capture && rate != sampleRate()) { setSampleRate(rate); }
	if (capture && rate != sampleRate())
	{
		fprintf(stderr, "ALSA: the capture runs at %u Hz, the playback at %u Hz\n", rate, sampleRate());
		return false;
	}

	// Transfers of a period; the playback starts once its buffer is full, the capture with it (or at its first read)
	snd_pcm_sw_params_t* sw = nullptr;
	snd_pcm_sw_params_alloca(&sw);
	snd_pcm_sw_params_current(stream.pcm, sw);
	snd_pcm_sw_params_set_avail_min(stream.pcm, sw, stream.period);
	snd_pcm_sw_params_set_start_threshold(stream.pcm, sw, capture ? 1 : stream.buffer);
	if (int err = snd_pcm_sw_params(stream.pcm, sw); err < 0)
	{
		fprintf(stderr, "ALSA: cannot set the software parameters for %s: %s\n", what, snd_strerror(err));
		return false;
	}

	stream.bytesPerSample = snd_pcm_format_physical_width(stream.format) / 8;
	const auto channelBytes = static_cast<std::size_t>(stream.period) * stream.bytesPerSample;
	stream.raw.assign(channelBytes * stream.channels, 0);
	stream.channelAreas.resize(stream.interleaved ? 0 : stream.channels);
	stream.partAreas.resize(stream.channelAreas.size());
	for (std::size_t channel = 0; channel < stream.channelAreas.size(); ++channel)
	{
		stream.channelAreas[channel] = stream.raw.data() + channel * channelBytes;
	}

	closeOnExec(stream.pcm);
	return true;
}




void AudioAlsa::closeStream(Stream& stream)
{
	if (stream.pcm) { snd_pcm_close(stream.pcm); }
	stream = Stream{};
}




auto AudioAlsa::sampleAt(Stream& stream, unsigned channel, snd_pcm_uframes_t frame) -> unsigned char*
{
	const auto index = stream.interleaved
		? frame * stream.channels + channel
		: channel * stream.period + frame;
	return stream.raw.data() + index * stream.bytesPerSample;
}




auto AudioAlsa::readSample(const unsigned char* sample, snd_pcm_format_t format) -> float
{
	switch (format)
	{
	case SND_PCM_FORMAT_FLOAT:
	{
		float value;
		std::memcpy(&value, sample, sizeof(value));
		return value;
	}
	case SND_PCM_FORMAT_S32:
	{
		std::int32_t value;
		std::memcpy(&value, sample, sizeof(value));
		return static_cast<float>(value / 2147483648.0);
	}
	case SND_PCM_FORMAT_S24:
	{
		// 24 bits in the low bytes of 32
		std::uint32_t raw;
		std::memcpy(&raw, sample, sizeof(raw));
		const auto value = static_cast<std::int32_t>(raw << 8) >> 8;
		return static_cast<float>(value / 8388608.0);
	}
	case SND_PCM_FORMAT_S24_3LE:
	{
		const auto raw = static_cast<std::uint32_t>(sample[0]) | (static_cast<std::uint32_t>(sample[1]) << 8)
			| (static_cast<std::uint32_t>(sample[2]) << 16);
		const auto value = static_cast<std::int32_t>(raw << 8) >> 8;
		return static_cast<float>(value / 8388608.0);
	}
	case SND_PCM_FORMAT_S16:
	{
		std::int16_t value;
		std::memcpy(&value, sample, sizeof(value));
		return static_cast<float>(value / 32768.0);
	}
	default:
		return 0.f;
	}
}




void AudioAlsa::writeSample(unsigned char* sample, snd_pcm_format_t format, float value)
{
	if (format == SND_PCM_FORMAT_FLOAT)
	{
		std::memcpy(sample, &value, sizeof(value));
		return;
	}
	const double clipped = std::clamp(static_cast<double>(value), -1.0, 1.0);
	switch (format)
	{
	case SND_PCM_FORMAT_S32:
	{
		const auto integer = static_cast<std::int32_t>(std::lround(clipped * 2147483647.0));
		std::memcpy(sample, &integer, sizeof(integer));
		break;
	}
	case SND_PCM_FORMAT_S24:
	{
		const auto integer = static_cast<std::int32_t>(std::lround(clipped * 8388607.0));
		std::memcpy(sample, &integer, sizeof(integer));
		break;
	}
	case SND_PCM_FORMAT_S24_3LE:
	{
		const auto integer = static_cast<std::uint32_t>(static_cast<std::int32_t>(std::lround(clipped * 8388607.0)));
		sample[0] = static_cast<unsigned char>(integer & 0xff);
		sample[1] = static_cast<unsigned char>((integer >> 8) & 0xff);
		sample[2] = static_cast<unsigned char>((integer >> 16) & 0xff);
		break;
	}
	case SND_PCM_FORMAT_S16:
	{
		const auto integer = static_cast<std::int16_t>(std::lround(clipped * 32767.0));
		std::memcpy(sample, &integer, sizeof(integer));
		break;
	}
	default:
		break;
	}
}




auto AudioAlsa::transfer(Stream& stream, bool capture) -> snd_pcm_sframes_t
{
	snd_pcm_uframes_t done = 0;
	while (done < stream.period)
	{
		const auto left = stream.period - done;
		snd_pcm_sframes_t count;
		if (stream.interleaved)
		{
			auto* where = stream.raw.data() + done * stream.channels * stream.bytesPerSample;
			count = capture ? snd_pcm_readi(stream.pcm, where, left) : snd_pcm_writei(stream.pcm, where, left);
		}
		else
		{
			for (std::size_t channel = 0; channel < stream.channelAreas.size(); ++channel)
			{
				stream.partAreas[channel] = static_cast<unsigned char*>(stream.channelAreas[channel])
					+ done * stream.bytesPerSample;
			}
			count = capture ? snd_pcm_readn(stream.pcm, stream.partAreas.data(), left)
				: snd_pcm_writen(stream.pcm, stream.partAreas.data(), left);
		}
		if (count == -EAGAIN || count == -EINTR) { continue; }
		if (count < 0) { return count; }
		done += static_cast<snd_pcm_uframes_t>(count);
	}
	return static_cast<snd_pcm_sframes_t>(done);
}




void AudioAlsa::prefill()
{
	// Silence for the whole playback buffer: the playback (and the linked capture) starts when it is full
	std::fill(m_playback.raw.begin(), m_playback.raw.end(), 0);
	for (auto filled = snd_pcm_uframes_t{0}; filled < m_playback.buffer; filled += m_playback.period)
	{
		if (transfer(m_playback, false) < 0) { break; }
	}
	if (m_capture.pcm && !m_linked) { snd_pcm_start(m_capture.pcm); }
}




auto AudioAlsa::restart() -> bool
{
	reportXrun();

	// A suspended card (e.g. the computer slept) must be resumed first
	for (auto* pcm : {m_playback.pcm, m_capture.pcm})
	{
		if (pcm && snd_pcm_state(pcm) == SND_PCM_STATE_SUSPENDED)
		{
			int err;
			while ((err = snd_pcm_resume(pcm)) == -EAGAIN) { usleep(100'000); }
		}
	}

	// Both streams again from silence, in step
	snd_pcm_drop(m_playback.pcm);
	if (m_capture.pcm && !m_linked) { snd_pcm_drop(m_capture.pcm); }
	if (int err = snd_pcm_prepare(m_playback.pcm); err < 0)
	{
		fprintf(stderr, "ALSA: cannot recover the playback: %s\n", snd_strerror(err));
		return false;
	}
	if (m_capture.pcm && !m_linked)
	{
		if (int err = snd_pcm_prepare(m_capture.pcm); err < 0)
		{
			fprintf(stderr, "ALSA: cannot recover the capture: %s\n", snd_strerror(err));
			return false;
		}
	}
	prefill();
	return true;
}




void AudioAlsa::startProcessingImpl()
{
	start(QThread::HighPriority);
}




void AudioAlsa::stopProcessingImpl()
{
	stopProcessingThread( this );
}




void AudioAlsa::run()
{
	const auto period = static_cast<f_cnt_t>(m_playback.period);
	const auto inputs = static_cast<ch_cnt_t>(m_capture.pcm ? m_capture.channels : 0);
	const auto outputs = channels();

	snd_pcm_prepare(m_playback.pcm);
	if (m_capture.pcm && !m_linked) { snd_pcm_prepare(m_capture.pcm); }
	prefill();

	while (AudioDevice::isRunning())
	{
		// One period of input, in the card's format, made planar
		if (inputs > 0)
		{
			if (const auto read = transfer(m_capture, true); read < 0)
			{
				if (!restart()) { usleep(10'000); }
				continue;
			}
			for (ch_cnt_t channel = 0; channel < inputs; ++channel)
			{
				auto* planar = m_inputPointers[channel];
				for (f_cnt_t frame = 0; frame < period; ++frame)
				{
					planar[frame] = readSample(sampleAt(m_capture, channel, frame), m_capture.format);
				}
			}
		}

		audioEngine()->renderNextBuffer(
			PlanarBufferView<const float>{const_cast<const float* const*>(m_inputPointers.data()), inputs, period},
			PlanarBufferView<float>{m_outputPointers.data(), outputs, period});

		// The output in the card's format; the card's channels past ours are silent
		for (unsigned channel = 0; channel < m_playback.channels; ++channel)
		{
			const float* planar = channel < outputs ? m_outputPointers[channel] : nullptr;
			for (f_cnt_t frame = 0; frame < period; ++frame)
			{
				writeSample(sampleAt(m_playback, channel, frame), m_playback.format, planar ? planar[frame] : 0.f);
			}
		}
		if (const auto written = transfer(m_playback, false); written < 0)
		{
			if (!restart()) { usleep(10'000); }
		}
	}

	snd_pcm_drop(m_playback.pcm);
	if (m_capture.pcm && !m_linked) { snd_pcm_drop(m_capture.pcm); }
}

} // namespace lmms

#endif // LMMS_HAVE_ALSA
