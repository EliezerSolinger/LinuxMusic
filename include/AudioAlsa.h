/*
 * AudioAlsa.h - ALSA driver: duplex, on the card's own format, exclusive
 *
 * Copyright (c) 2004-2009 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_AUDIO_ALSA_H
#define LMMS_AUDIO_ALSA_H

#include "lmmsconfig.h"

#ifdef LMMS_HAVE_ALSA

// older ALSA-versions might require this
#define ALSA_PCM_NEW_HW_PARAMS_API

#include <alsa/asoundlib.h>
#include <memory>
#include <vector>
#include <QThread>

#include "AudioDevice.h"

namespace lmms
{

class AudioDeviceReservation;

/**
 * The ALSA driver (docs/plano-gravacao.md, "Drivers"):
 * - duplex: the capture and the playback stream are linked (snd_pcm_link()), so they start together and run on the
 *   card's clock; each cycle reads one period of input, renders it with the output (renderNextBuffer(input, output))
 *   and writes one period of output;
 * - on the card's own format: float, 32, 24 (in 3 or 4 bytes) or 16 bits, interleaved or not, converted here; with
 *   a hw: device nothing in between converts or resamples;
 * - exclusive: for a hw: device, the card is asked from PipeWire or PulseAudio over D-Bus
 *   (org.freedesktop.ReserveDevice1) before it is opened, and given back when it is closed;
 * - an xrun is counted and both streams are recovered together, the output refilled with silence;
 * - the latencies are the buffers': the playback buffer on the way out, a period on the way in.
 */
class AudioAlsa : public QThread, public AudioDevice
{
	Q_OBJECT
public:
	/**
	 * @brief Contains the relevant information about available ALSA devices
	 */
	class DeviceInfo
	{
	public:
		DeviceInfo(QString const & deviceName, QString const & deviceDescription, bool playback = true,
			bool capture = true) :
			m_deviceName(deviceName),
			m_deviceDescription(deviceDescription),
			m_playback(playback),
			m_capture(capture)
		{}
		~DeviceInfo() = default;

		QString const & getDeviceName() const { return m_deviceName; }
		QString const & getDeviceDescription() const { return m_deviceDescription; }
		bool canPlay() const { return m_playback; }
		bool canCapture() const { return m_capture; }

	private:
		QString m_deviceName;
		QString m_deviceDescription;
		bool m_playback;
		bool m_capture;
	};

	using DeviceInfoCollection = std::vector<DeviceInfo>;

public:
	AudioAlsa( bool & _success_ful, AudioEngine* audioEngine );
	~AudioAlsa() override;

	inline static QString name()
	{
		return QT_TRANSLATE_NOOP( "AudioDeviceSetupWidget",
			"ALSA (Advanced Linux Sound Architecture)" );
	}

	//! @returns the playback device: the configured one, $AUDIODEV, or "default"
	static QString probeDevice();

	//! @returns the capture device: the configured one, the playback device if none is configured, or an empty
	//!          string for no capture
	static QString probeCaptureDevice();

	//! The configuration's value of the capture device for no capture
	static constexpr const char* NoCapture = "none";

	//! @brief Creates a list of all available devices, with whether each plays and captures.
	//!
	//! Uses the hints API of ALSA to collect all devices: the plugs (default, PipeWire, PulseAudio, ...), which
	//! share the card with other programs and convert, and the hardware devices (hw:CARD=...,DEV=...), which are
	//! exclusive and hand over the card's own format.
	static DeviceInfoCollection getAvailableDevices();

	auto inputLatency() const -> f_cnt_t override { return m_inputLatency; }
	auto outputLatency() const -> f_cnt_t override { return m_outputLatency; }

private:
	//! One direction of the card
	struct Stream
	{
		snd_pcm_t* pcm = nullptr;
		snd_pcm_format_t format = SND_PCM_FORMAT_UNKNOWN;
		bool interleaved = true;
		unsigned channels = 0;
		snd_pcm_uframes_t period = 0;
		snd_pcm_uframes_t buffer = 0;
		int bytesPerSample = 0;
		std::vector<unsigned char> raw; //!< one period in the card's format
		std::vector<void*> channelAreas; //!< for non-interleaved access: each channel's part of raw
		std::vector<void*> partAreas; //!< for non-interleaved access: where the rest of a partial transfer goes
	};

	void startProcessingImpl() override;
	void stopProcessingImpl() override;
	void run() override;

	//! Opens @p stream on @p device with the card's own format; @p channels is the wish (0: all the card has)
	auto openStream(Stream& stream, const QString& device, snd_pcm_stream_t direction, unsigned channels,
		unsigned periods) -> bool;
	static void closeStream(Stream& stream);

	auto transfer(Stream& stream, bool capture) -> snd_pcm_sframes_t;
	auto restart() -> bool;
	void prefill();
	void reserve(const QString& device);

	static auto readSample(const unsigned char* sample, snd_pcm_format_t format) -> float;
	static void writeSample(unsigned char* sample, snd_pcm_format_t format, float value);
	auto sampleAt(Stream& stream, unsigned channel, snd_pcm_uframes_t frame) -> unsigned char*;

	Stream m_playback;
	Stream m_capture;
	bool m_linked = false;

	std::vector<std::unique_ptr<AudioDeviceReservation>> m_reservations;

	//! Planar scratch for the engine, one period
	std::vector<float> m_inputStorage;
	std::vector<float> m_outputStorage;
	std::vector<float*> m_inputPointers;
	std::vector<float*> m_outputPointers;

	f_cnt_t m_inputLatency = 0;
	f_cnt_t m_outputLatency = 0;
} ;

} // namespace lmms

#endif // LMMS_HAVE_ALSA

#endif // LMMS_AUDIO_ALSA_H
