/*
 * AudioEngine.h - device-independent audio engine for LMMS
 *
 * Copyright (c) 2004-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_AUDIO_ENGINE_H
#define LMMS_AUDIO_ENGINE_H

#include <mutex>

#include <QThread>
#include <samplerate.h>

#include <memory>
#include <optional>
#include <algorithm>
#include <array>
#include <atomic>
#include <cstdint>
#include <vector>

#include "AudioBufferView.h"
#include "AudioDevice.h"
#include "LmmsTypes.h"
#include "TripleBuffer.h"
#include "SampleFrame.h"
#include "LocklessList.h"
#include "AudioEngineProfiler.h"
#include "PlayHandle.h"



namespace lmms
{

class MidiClient;
class AudioBusHandle;  // IWYU pragma: keep
class AudioEngineWorkerThread;

constexpr f_cnt_t MINIMUM_BUFFER_SIZE = 32;
constexpr f_cnt_t DEFAULT_BUFFER_SIZE = 256;
constexpr f_cnt_t MAXIMUM_BUFFER_SIZE = 4096;

constexpr int BYTES_PER_SAMPLE = sizeof(sample_t);
constexpr int BYTES_PER_INT_SAMPLE = sizeof(int_sample_t);
constexpr int BYTES_PER_FRAME = sizeof(SampleFrame);

constexpr float OUTPUT_SAMPLE_MULTIPLIER = 32767.0f;

constexpr auto SUPPORTED_SAMPLERATES = std::array{44100, 48000, 88200, 96000, 176400, 192000};
constexpr auto SUPPORTED_BITRATES = std::array{64, 128, 160, 192, 256, 320};

class LMMS_EXPORT AudioEngine : public QObject
{
	Q_OBJECT
public:
	/**
	 * @brief RAII helper for requestChangesInModel.
	 * Used by AudioEngine::requestChangesGuard.
	 */
	class RequestChangesGuard {
		friend class AudioEngine;

	private:
		RequestChangesGuard(AudioEngine* audioEngine)
			: m_audioEngine{audioEngine}
		{
			m_audioEngine->requestChangeInModel();
		}
	public:

		RequestChangesGuard()
			: m_audioEngine{nullptr}
		{
		}

		RequestChangesGuard(RequestChangesGuard&& other)
			: RequestChangesGuard()
		{
			std::swap(other.m_audioEngine, m_audioEngine);
		}

		// Disallow copy.
		RequestChangesGuard(const RequestChangesGuard&) = delete;
		RequestChangesGuard& operator=(const RequestChangesGuard&) = delete;

		~RequestChangesGuard() {
			if (m_audioEngine) {
				m_audioEngine->doneChangeInModel();
			}
		}

	private:
		AudioEngine* m_audioEngine;
	};

	void initDevices();
	void clear();
	void clearNewPlayHandles();


	// audio-device-stuff

	bool renderOnly() const { return m_renderOnly; }
	// Returns the current audio device's name. This is not necessarily
	// the user's preferred audio device, in case you were thinking that.
	inline const QString & audioDevName() const
	{
		return m_audioDevName;
	}
	inline bool audioDevStartFailed() const
	{
		return m_audioDevStartFailed;
	}

	//! Set new audio device. Old device will be deleted,
	//! unless it's stored using storeAudioDevice
	void setAudioDevice(AudioDevice* _dev, bool startNow);

	/**
	 * Opens the audio device again with the settings saved (its interface, sample rate and buffer), without
	 * restarting the program (docs/gravacao-levantamento.md §6). The engine's period stays as it is (it is at most
	 * DEFAULT_BUFFER_SIZE frames and the device's buffer is cut into periods). @p sampleRate asks for a rate other
	 * than the settings' (a project's). Main thread, with no change in the model under way.
	 * @returns what to tell: why it could not (while recording), or what it opened instead of what was asked (another
	 *          interface, another rate); empty when it opened what was asked
	 */
	auto reopenAudioDevice(std::optional<sample_rate_t> sampleRate = std::nullopt) -> QString;
	void storeAudioDevice();
	void restoreAudioDevice();
	inline AudioDevice * audioDev() { return m_audioDev; }
	inline const AudioDevice* audioDev() const { return m_audioDev; }

	bool captureDeviceAvailable() const;


	// audio-bus-handle-stuff
	inline void addAudioBusHandle(AudioBusHandle* busHandle)
	{
		requestChangeInModel();
		m_audioBusHandles.push_back(busHandle);
		doneChangeInModel();
	}

	void removeAudioBusHandle(AudioBusHandle* busHandle);


	// MIDI-client-stuff
	inline const QString & midiClientName() const
	{
		return m_midiClientName;
	}

	inline MidiClient * midiClient()
	{
		return m_midiClient;
	}


	// play-handle stuff
	bool addPlayHandle( PlayHandle* handle );

	void removePlayHandle( PlayHandle* handle );

	inline PlayHandleList& playHandles()
	{
		return m_playHandles;
	}

	void removePlayHandlesOfTypes(Track * track, PlayHandle::Types types);

	AudioEngineProfiler& profiler()
	{
		return m_profiler;
	}

	int cpuLoad() const
	{
		return m_profiler.cpuLoad();
	}

	int detailLoad(const AudioEngineProfiler::DetailType type) const
	{
		return m_profiler.detailLoad(type);
	}

	sample_rate_t baseSampleRate() const { return m_baseSampleRate; }


	sample_rate_t outputSampleRate() const
	{
		return m_audioDev != nullptr ? m_audioDev->sampleRate() : m_baseSampleRate;
	}
	

	sample_rate_t inputSampleRate() const	
	{
		return m_audioDev != nullptr ? m_audioDev->sampleRate() : m_baseSampleRate;
	}


	inline float masterGain() const
	{
		return m_masterGain;
	}

	inline void setMasterGain(const float mo)
	{
		m_masterGain = mo;
	}


	static inline sample_t clip(const sample_t s)
	{
		if (s > 1.0f)
		{
			return 1.0f;
		}
		else if (s < -1.0f)
		{
			return -1.0f;
		}
		return s;
	}


	bool criticalXRuns() const;

	//! Starts/stops the audio device's processing. Tests stop it to render periods themselves.
	void startProcessing() { m_audioDev->startProcessing(); }
	void stopProcessing() { m_audioDev->stopProcessing(); }

	/**
	 * @returns the input the audio device captured for the period being rendered: one planar buffer of
	 *          framesPerPeriod() frames per input channel of the device (none if it does not capture). Only on the
	 *          audio thread, while a period is rendered.
	 *
	 * The input of a period is what the device handed the engine together with the output of the period before
	 * (see renderNextBuffer(input, output)): it was captured the length of a period before the output of the period
	 * being rendered is handed to the device.
	 */
	auto capture() const -> PlanarBufferView<const float>
	{
		const auto& pointers = m_capturePointers[1 - m_captureFill];
		return {pointers.data(), m_captureChannels, m_captureChannels > 0 ? m_framesPerPeriod : 0};
	}

	/**
	 * Main thread: opens or closes the audio device's input, for devices that capture on demand (briefly stopping
	 * the device); the engine keeps the setting across device changes
	 */
	void setCaptureEnabled(bool enabled);
	auto captureEnabled() const -> bool { return m_captureEnabled; }

	//! @returns the number of periods rendered since the engine started (any thread)
	auto periodsRendered() const -> std::int64_t { return m_periodsRendered.load(std::memory_order_relaxed); }

	/**
	 * @returns The internal buffer size used by audio plugins and other processing done within the audio engine.
	 * Its value is @ref DEFAULT_BUFFER_SIZE or @ref framesPerAudioBuffer(), whichever is lower.
	 * @see renderNextPeriod()
	 */
	f_cnt_t framesPerPeriod() const { return m_framesPerPeriod; }

	/**
	 * @returns The buffer size used by the configured @ref AudioDevice "output audio device". It is in the range
	 * of @ref MINIMUM_BUFFER_SIZE and @ref MAXIMUM_BUFFER_SIZE.
	 * @note This should not be needed for a majority of cases. Currently, it's only being used to set up the
	 * audio devices. This member function may be removed in a later refactor.
	 */
	f_cnt_t framesPerAudioBuffer() const { return m_framesPerAudioBuffer; }

	/**
	 * @brief Renders the next audio period.
	 *
	 * An audio period is a fixed-size chunk of audio the engine generates, and represents a single cycle of the
	 * engine's output. The rendering is chunked into smaller periods to timely handle per-buffer updates like
	 * non-sample-accurate automation, as well as to improve memory cache performance.
	 *
	 * The audio period generated is interleaved and stereo.
	 *
	 * @note The audio period returned is non-owning and will be changed on subsequent calls to @ref renderNextPeriod()
	 * and @ref renderNextBuffer(). Callers must copy the data into their own local buffers if they need it to persist.
	 *
	 * @returns A non-owning buffer to the next audio period.
	 */
	std::span<const SampleFrame> renderNextPeriod();

	/**
	 * @brief Renders an audio buffer into @a dst.
	 *
	 * Renders @ref renderNextPeriod() "audio periods" into @a dst. If @a dst is not a multiple of the period
	 * size, the remaining frames are partially rendered (an extra period may be rendered in such cases, which can
	 * degrade performance).
	 *
	 * If @a dst has 1 channel, the channels are averaged to mono.
	 * If @a dst has 2 channels, the channels are directly copied.
	 * If @a dst has more than 2 channels, the stereo channels are copied and the rest are zero-filled.
	 *
	 * @param dst An audio buffer view to write into. Both interleaved and planar overloads are provided.
	 */
	void renderNextBuffer(InterleavedBufferView<float> dst) { renderNextBuffer({}, dst); }

	//! @copydoc renderNextBuffer(InterleavedBufferView<float>)
	void renderNextBuffer(PlanarBufferView<float> dst) { renderNextBuffer({}, dst); }

	/**
	 * @brief Renders an audio buffer into @a dst, taking the device's input for the same frames
	 *
	 * For devices that capture: @p input holds the frames the device captured in the same callback, one planar
	 * buffer per input channel, as many frames as @p dst. They become capture() for the next period, so the input
	 * stays on the same clock as the output. Channels beyond the device's inputChannels() are ignored; missing ones
	 * are silence.
	 */
	void renderNextBuffer(PlanarBufferView<const float> input, InterleavedBufferView<float> dst)
	{
		renderNextBufferImpl(input, dst);
	}

	//! @copydoc renderNextBuffer(PlanarBufferView<const float>, InterleavedBufferView<float>)
	void renderNextBuffer(PlanarBufferView<const float> input, PlanarBufferView<float> dst)
	{
		renderNextBufferImpl(input, dst);
	}

	//! Block until a change in model can be done (i.e. wait for audio thread)
	void requestChangeInModel();
	void doneChangeInModel();

	RequestChangesGuard requestChangesGuard()
	{
		return RequestChangesGuard{this};
	}

	static bool isAudioDevNameValid(QString name);
	static bool isMidiDevNameValid(QString name);

	/**
	 * The latest period of the master output, for displays (e.g. the oscilloscope): the audio thread publishes each
	 * period without waiting, and the display takes the latest one when it repaints. There must be only one reader,
	 * on the main thread.
	 */
	auto outputMonitor() -> TripleBuffer<std::vector<SampleFrame>>& { return m_outputMonitor; }

	//! @returns true if sanitization of inf/nan values is enabled
	bool sanitizationEnabled() { return m_sanitizationEnabled.load(std::memory_order_relaxed); }

	//! Enable/disable sanitization of inf/nan values
	void setSanitizationEnabled(bool enabled) { m_sanitizationEnabled.store(enabled, std::memory_order_relaxed); }

signals:
	void qualitySettingsChanged();
	void sampleRateChanged();


private:
	void renderNextBufferImpl(PlanarBufferView<const float> input, AudioBufferView<float> auto dst)
	{
		assert(input.channels() == 0 || input.frames() == dst.frames());
		const auto inputChannels = std::min(input.channels(), m_captureChannels);
		for (auto frame = f_cnt_t{0}; frame < dst.frames(); ++frame)
		{
			if (m_outputBufferReadIndex == m_framesPerPeriod) { m_outputBufferReadIndex = 0; }
			if (m_outputBufferReadIndex == 0)
			{
				// The input taken with the last period's output is the input of the period to render
				m_captureFill = 1 - m_captureFill;
				renderNextPeriod();
			}

			// Take the device's input for this frame
			auto& fill = m_capturePointers[m_captureFill];
			for (ch_cnt_t channel = 0; channel < inputChannels; ++channel)
			{
				fill[channel][m_outputBufferReadIndex] = input.sample(channel, frame);
			}
			for (ch_cnt_t channel = inputChannels; channel < m_captureChannels; ++channel)
			{
				fill[channel][m_outputBufferReadIndex] = 0.f;
			}

			switch (dst.channels())
			{
			case 0:
				assert(false);
				break;
			case 1:
				dst.sample(0, frame) = m_outputBufferRead[m_outputBufferReadIndex].average();
				break;
			case 2:
				dst.sample(0, frame) = m_outputBufferRead[m_outputBufferReadIndex][0];
				dst.sample(1, frame) = m_outputBufferRead[m_outputBufferReadIndex][1];
				break;
			default:
				dst.sample(0, frame) = m_outputBufferRead[m_outputBufferReadIndex][0];
				dst.sample(1, frame) = m_outputBufferRead[m_outputBufferReadIndex][1];
				for (auto channel = 2; channel < dst.channels(); ++channel)
				{
					dst.sample(channel, frame) = 0.f;
				}
				break;
			}

			++m_outputBufferReadIndex;
		}
	}

	AudioEngine( bool renderOnly );
	~AudioEngine() override;



	AudioDevice * tryAudioDevices();
	MidiClient * tryMidiClients();

	void renderStageNoteSetup();
	void renderStageInstruments();
	void renderStageEffects();
	void renderStageMix();


	void swapBuffers();

	//! Sets the capture buffers up for the audio device's inputs (not on the audio thread)
	void prepareCapture();

	//! Opens or closes the device's input as captureEnabled() says, now or as soon as it is safe
	void applyCaptureEnabled();

	//! Moves the play handles added since the last period into the list of play handles (audio thread, or holding
	//! the change lock)
	void adoptNewPlayHandles();

	void clearInternal();

	bool m_renderOnly;

	std::vector<AudioBusHandle*> m_audioBusHandles;

	f_cnt_t m_framesPerAudioBuffer;
	f_cnt_t m_framesPerPeriod;
	sample_rate_t m_baseSampleRate;

	/**
	 * The device's input, by period: one half is filled with the input of the period being output, the other is
	 * capture() for the period being rendered. Allocated when a device is set, never on the audio thread.
	 */
	std::array<std::vector<float>, 2> m_captureStorage;
	std::array<std::vector<float*>, 2> m_capturePointers;
	int m_captureFill = 0; //!< the half being filled
	ch_cnt_t m_captureChannels = 0;
	bool m_captureEnabled = false;
	std::atomic<std::int64_t> m_periodsRendered = 0;

	std::unique_ptr<SampleFrame[]> m_outputBufferRead;
	std::unique_ptr<SampleFrame[]> m_outputBufferWrite;
	TripleBuffer<std::vector<SampleFrame>> m_outputMonitor;
	f_cnt_t m_outputBufferReadIndex;

	// worker thread stuff
	std::vector<AudioEngineWorkerThread *> m_workers;
	int m_numWorkers;

	// playhandle stuff
	PlayHandleList m_playHandles;
	// place where new playhandles are added temporarily
	LocklessList<PlayHandle *> m_newPlayHandles;
	ConstPlayHandleList m_playHandlesToRemove;

	float m_masterGain;

	// audio device stuff
	void doSetAudioDevice( AudioDevice *_dev );
	AudioDevice * m_audioDev;
	AudioDevice * m_oldAudioDev;
	QString m_audioDevName;
	bool m_audioDevStartFailed;

	// MIDI device stuff
	MidiClient * m_midiClient;
	QString m_midiClientName;

	AudioEngineProfiler m_profiler;

	bool m_clearSignal;
	std::atomic<bool> m_sanitizationEnabled = false;

	std::recursive_mutex m_changeMutex;

	friend class Engine;
	friend class AudioEngineWorkerThread;
	friend class ProjectRenderer;

} ;

} // namespace lmms

#endif // LMMS_AUDIO_ENGINE_H
