/*
 * SamplePlayHandle.cpp - implementation of class SamplePlayHandle
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

#include "SamplePlayHandle.h"
#include "AudioEngine.h"
#include "AudioBusHandle.h"
#include "Engine.h"
#include "lmms_math.h"
#include "PatternTrack.h"
#include "SampleClip.h"
#include "SampleTrack.h"

namespace lmms
{

SamplePlayHandle::SamplePlayHandle(std::unique_ptr<Sample> sample)
	: PlayHandle(Type::SamplePlayHandle)
	, m_sample(sample.get())
	, m_ownSample(std::move(sample))
	, m_ownBus(std::make_unique<AudioBusHandle>("SamplePlayHandle", false))
{
	setAudioBusHandle(m_ownBus.get());
}




SamplePlayHandle::SamplePlayHandle( const QString& sampleFile ) :
	SamplePlayHandle(std::make_unique<Sample>(SampleBuffer::fromFile(sampleFile)))
{
}




SamplePlayHandle::SamplePlayHandle()
	: PlayHandle(Type::SamplePlayHandle)
{
}




SamplePlayHandle::SamplePlayHandle( SampleClip* clip ) :
	PlayHandle(Type::SamplePlayHandle)
{
	m_sample = &clip->sample();
	m_clip = clip;
	m_track = clip->getTrack();
	setAudioBusHandle(((SampleTrack *)clip->getTrack())->audioBusHandle());
}




void SamplePlayHandle::restart(SampleClip* clip, VoicePool<SamplePlayHandle>* pool)
{
	m_state.restart(0);
	m_frame = 0;
	m_sample = &clip->sample();
	m_clip = clip;
	m_track = clip->getTrack();
	m_patternTrack = nullptr;
	m_doneMayReturnTrue = true;
	m_pool = pool;
	setOffset(0);
	releaseBuffer();
	setAudioBusHandle(static_cast<SampleTrack*>(clip->getTrack())->audioBusHandle());
	// Finished by the engine on the thread it plays on now (it was made on the main thread)
	setAffinityToCurrentThread();
}


void SamplePlayHandle::dispose()
{
	if (m_pool)
	{
		m_clip = nullptr;
		m_pool->give(this);
	}
	else { delete this; }
}


SamplePlayHandle::~SamplePlayHandle() = default;




void SamplePlayHandle::play(std::span<SampleFrame> buffer)
{
	//play( 0, _try_parallelizing );
	if( framesDone() >= totalFrames() )
	{
		zeroSampleFrames(buffer.data(), buffer.size());
		return;
	}

	SampleFrame* workingBuffer = buffer.data();
	f_cnt_t frames = buffer.size();

	// apply offset for the first period
	if( framesDone() == 0 )
	{
		zeroSampleFrames(buffer.data(), offset());
		workingBuffer += offset();
		frames -= offset();
	}

	const f_cnt_t initialFrameIndex = m_state.frameIndex();

	if( !( m_track && m_track->isSilenced() )
				&& !(m_patternTrack && m_patternTrack->isSilenced()))
	{
/*		StereoVolumeVector v =
			{ { m_volumeModel->value() / DefaultVolume,
				m_volumeModel->value() / DefaultVolume } };*/
		// SamplePlayHandle always plays the sample at its original pitch;
		// it is used only for previews, SampleTracks and the metronome.
		if (!m_sample->play(workingBuffer, &m_state, frames))
		{
			zeroSampleFrames(workingBuffer, frames);
		}

		// Apply crossfade if this play handle is for a clip
		if (m_clip && (m_clip->startCrossfadeLength() != 0 || m_clip->endCrossfadeLength() != 0))
		{
			const int framesPerTick = Engine::framesPerTick(Engine::audioEngine()->outputSampleRate());
			const int startCrossfadeFrames = m_clip->startCrossfadeLength() * framesPerTick;
			const int endCrossfadeFrames = m_clip->endCrossfadeLength() * framesPerTick;
			const int startOffsetFrames = m_clip->startTimeOffset() * framesPerTick;
			const int lengthFrames = m_clip->length() * framesPerTick;
			const float startPower = std::log(1.0f - m_clip->startCrossfadeTension()) / -std::numbers::ln2;
			const float endPower = std::log(1.0f - m_clip->endCrossfadeTension()) / -std::numbers::ln2;

			for (f_cnt_t f = 0; f < frames; ++f)
			{
				const f_cnt_t frameIndex = initialFrameIndex + f;
				const int framesRelativeToClipStart = frameIndex + startOffsetFrames;
				const int framesRelativeToClipEnd = lengthFrames - framesRelativeToClipStart;
				if (framesRelativeToClipStart < 0 || framesRelativeToClipEnd < 0) { workingBuffer[f] = SampleFrame(); continue; }

				if (framesRelativeToClipStart < startCrossfadeFrames && framesRelativeToClipEnd < endCrossfadeFrames)
				{
					workingBuffer[f] *=
							fastPow(static_cast<float>(framesRelativeToClipStart) / static_cast<float>(startCrossfadeFrames), startPower)
							* fastPow(static_cast<float>(framesRelativeToClipEnd) / static_cast<float>(endCrossfadeFrames), endPower);
				}
				else if (framesRelativeToClipStart < startCrossfadeFrames)
				{
					workingBuffer[f] *= fastPow(static_cast<float>(framesRelativeToClipStart) / static_cast<float>(startCrossfadeFrames), startPower);
				}
				else if (framesRelativeToClipEnd < endCrossfadeFrames)
				{
					workingBuffer[f] *= fastPow(static_cast<float>(framesRelativeToClipEnd) / static_cast<float>(endCrossfadeFrames), endPower);
				}
			}
		}
	}

	m_frame += frames;
}




bool SamplePlayHandle::isFinished() const
{
	return framesDone() >= totalFrames() && m_doneMayReturnTrue == true;
}




bool SamplePlayHandle::isFromTrack( const Track * _track ) const
{
	return m_track == _track || m_patternTrack == _track;
}




f_cnt_t SamplePlayHandle::totalFrames() const
{
	return (m_sample->endFrame() - m_sample->startFrame()) *
			(static_cast<float>(Engine::audioEngine()->outputSampleRate()) / m_sample->sampleRate());
}


} // namespace lmms
