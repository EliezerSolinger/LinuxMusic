/*
 * Sample.h
 *
 * Copyright (c) 2025 saker <sakertooth@gmail.com>
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

#ifndef LMMS_SAMPLE_H
#define LMMS_SAMPLE_H

#include <memory>

#include "AudioResampler.h"
#include "AudioStream.h"
#include "Note.h"
#include "SampleBuffer.h"
#include "lmms_export.h"

namespace lmms {
class LMMS_EXPORT Sample
{
public:
	enum class Loop
	{
		Off,
		On,
		PingPong
	};

	class LMMS_EXPORT PlaybackState
	{
	public:
		PlaybackState(AudioResampler::Mode interpolationMode = AudioResampler::Mode::Linear, int frameIndex = 0)
			: m_resampler(interpolationMode)
			, m_frameIndex(frameIndex)
		{
		}

		auto frameIndex() const -> int { return m_frameIndex; }
		auto backwards() const -> bool { return m_backwards; }

		void setFrameIndex(int index) { m_frameIndex = index; }
		void setBackwards(bool backwards) { m_backwards = backwards; }
		//! Plays from @p frameIndex as from nothing before (a new sound: the resampler forgets the last one).
		//! Allocates nothing (for the audio thread)
		void restart(int frameIndex)
		{
			m_resampler.reset();
			m_bufferView = {};
			m_frameIndex = frameIndex;
			m_backwards = false;
			m_direct = false;
		}

	private:
		AudioResampler m_resampler;
		std::array<SampleFrame, DEFAULT_BUFFER_SIZE> m_buffer;
		std::span<SampleFrame> m_bufferView;
		int m_frameIndex = 0;
		bool m_backwards = false;
		bool m_direct = false; //!< played without the resampler (at a ratio of exactly 1) the last time
		friend class Sample;
	};

	Sample() = default;

	Sample(const SampleFrame* data, size_t numFrames, int sampleRate = Engine::audioEngine()->outputSampleRate());
	Sample(const Sample& other);
	Sample(Sample&& other) noexcept;
	explicit Sample(std::shared_ptr<const SampleBuffer> buffer);
	//! A sample played from disk (AudioStream): it has no buffer, only the stream
	explicit Sample(std::shared_ptr<AudioStream> stream);

	auto operator=(const Sample&) -> Sample&;
	auto operator=(Sample&&) noexcept -> Sample&;

	auto play(SampleFrame* dst, PlaybackState* state, size_t numFrames, Loop loopMode = Loop::Off,
		double ratio = 1.0) const -> bool;
	//! Plays @p numFrames frames as they are in the sample, without resampling (for a sample at the engine's rate:
	//! exactly its frames), silence past its end. @returns the frames of the sample played
	auto copyFrames(SampleFrame* dst, PlaybackState* state, std::size_t numFrames) const -> std::size_t;

	auto sampleDuration() const -> std::chrono::milliseconds;
	auto sampleFile() const -> const QString& { return m_stream ? m_stream->file() : m_buffer->audioFile(); }
	auto sampleRate() const -> int { return m_stream ? m_stream->sampleRate() : static_cast<int>(m_buffer->sampleRate()); }
	auto sampleSize() const -> size_t { return m_stream ? m_stream->frames() : m_buffer->size(); }
	//! Whether it plays from disk (then buffer() is empty)
	auto isStreamed() const -> bool { return m_stream != nullptr; }
	auto stream() const -> const std::shared_ptr<AudioStream>& { return m_stream; }

	auto toBase64() const -> QString { return m_buffer->toBase64(); }

	auto buffer() const -> const SampleBuffer& { return *m_buffer; }
	auto shareBuffer() const -> std::shared_ptr<const SampleBuffer> { return m_buffer; }

	auto data() const -> const SampleFrame* { return m_buffer->data(); }
	auto startFrame() const -> int { return m_startFrame.load(std::memory_order_relaxed); }
	auto endFrame() const -> int { return m_endFrame.load(std::memory_order_relaxed); }
	auto loopStartFrame() const -> int { return m_loopStartFrame.load(std::memory_order_relaxed); }
	auto loopEndFrame() const -> int { return m_loopEndFrame.load(std::memory_order_relaxed); }
	auto amplification() const -> float { return m_amplification.load(std::memory_order_relaxed); }
	auto frequency() const -> float { return m_frequency.load(std::memory_order_relaxed); }
	auto reversed() const -> bool { return m_reversed.load(std::memory_order_relaxed); }

	void setStartFrame(int startFrame) { m_startFrame.store(startFrame, std::memory_order_relaxed); }
	void setEndFrame(int endFrame) { m_endFrame.store(endFrame, std::memory_order_relaxed); }
	void setLoopStartFrame(int loopStartFrame) { m_loopStartFrame.store(loopStartFrame, std::memory_order_relaxed); }
	void setLoopEndFrame(int loopEndFrame) { m_loopEndFrame.store(loopEndFrame, std::memory_order_relaxed); }
	void setAllPointFrames(int startFrame, int endFrame, int loopStartFrame, int loopEndFrame);
	void setAmplification(float amplification) { m_amplification.store(amplification, std::memory_order_relaxed); }
	void setFrequency(float frequency) { m_frequency.store(frequency, std::memory_order_relaxed); }
	void setReversed(bool reversed) { m_reversed.store(reversed, std::memory_order_relaxed); }

private:
	f_cnt_t render(SampleFrame* dst, f_cnt_t size, PlaybackState* state, Loop loop) const;
	std::shared_ptr<const SampleBuffer> m_buffer = SampleBuffer::emptyBuffer();
	std::shared_ptr<AudioStream> m_stream;
	std::atomic<int> m_startFrame = 0;
	std::atomic<int> m_endFrame = 0;
	std::atomic<int> m_loopStartFrame = 0;
	std::atomic<int> m_loopEndFrame = 0;
	std::atomic<float> m_amplification = 1.0f;
	std::atomic<float> m_frequency = DefaultBaseFreq;
	std::atomic<bool> m_reversed = false;
};
} // namespace lmms
#endif
