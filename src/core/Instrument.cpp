/*
 * Instrument.cpp - base-class for all instrument-plugins (synths, samplers etc)
 *
 * Copyright (c) 2005-2009 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "Instrument.h"

#include <cmath>
#include <numbers>

#include "AudioEngine.h"
#include "DummyInstrument.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "LmmsTypes.h"

namespace lmms
{


Instrument::Instrument(const Descriptor* _descriptor,
			InstrumentTrack* _instrument_track,
			const Descriptor::SubPluginFeatures::Key* key,
			Flags flags) :
	Plugin(_descriptor, nullptr/* _instrument_track*/, key),
	m_instrumentTrack( _instrument_track ),
	m_flags(flags)
{
}




auto Instrument::instantiate(const QString &_plugin_name,
	InstrumentTrack *_instrument_track, const Descriptor::SubPluginFeatures::Key *key, bool keyFromDnd)
	-> std::unique_ptr<Instrument>
{
	if(keyFromDnd)
		Q_ASSERT(!key);
	// copy from above // TODO! common cleaner func
	if (auto instrument = pluginAs<Instrument>(
		Plugin::instantiateWithKey(_plugin_name, _instrument_track, key, keyFromDnd)))
	{
		return instrument;
	}
	return std::make_unique<DummyInstrument>(_instrument_track);
}




bool Instrument::isFromTrack( const Track * _track ) const
{
	return( m_instrumentTrack == _track );
}

// helper function for Instrument::applyFadeIn
static int countZeroCrossings(SampleFrame* buf, f_cnt_t start, f_cnt_t frames)
{
	// zero point crossing counts of all channels
	auto zeroCrossings = std::array<int, DEFAULT_CHANNELS>{};
	// maximum zero point crossing of all channels
	int maxZeroCrossings = 0;

	// determine the zero point crossing counts
	for (f_cnt_t f = start; f < frames; ++f)
	{
		for (ch_cnt_t ch = 0; ch < DEFAULT_CHANNELS; ++ch)
		{
			// we don't want to count [-1, 0, 1] as two crossings
			if ((buf[f - 1][ch] <= 0.0 && buf[f][ch] > 0.0) ||
					(buf[f - 1][ch] >= 0.0 && buf[f][ch] < 0.0))
			{
				++zeroCrossings[ch];
				if (zeroCrossings[ch] > maxZeroCrossings)
				{
					maxZeroCrossings = zeroCrossings[ch];
				}
			}
		}
	}

	return maxZeroCrossings;
}

// helper function for Instrument::applyFadeIn
static f_cnt_t getFadeInLength(float maxLength, f_cnt_t frames, int zeroCrossings, float blockFrames)
{
	// calculate the length of the fade in
	// Length is inversely proportional to the max of zeroCrossings,
	// because for low frequencies, we need a longer fade in to
	// prevent clicking.
	return (f_cnt_t) (maxLength  / ((float) zeroCrossings / ((float) frames / blockFrames) + 1.0f));
}


void Instrument::applyFadeIn(SampleFrame* buf, NotePlayHandle * n)
{
	// At most 85 frames at 44.1 kHz (about 2 ms), shorter the more often the note crosses zero (counted per 128
	// frames at 44.1 kHz): the same time at any sample rate
	const float rateFactor = Engine::audioEngine()->outputSampleRate() / 44100.f;
	const float maxFadeInLength = 85.f * rateFactor;
	const float blockFrames = 128.f * rateFactor;
	f_cnt_t total = n->totalFramesPlayed();
	if (total == 0)
	{
		const f_cnt_t frames = n->framesLeftForCurrentPeriod();
		const f_cnt_t offset = n->offset();

		// We need to skip the first sample because it almost always
		// produces a zero crossing; it's not helpful while
		// determining the fade in length. Hence 1
		int maxZeroCrossings = countZeroCrossings(buf, offset + 1, offset + frames);

		f_cnt_t length = getFadeInLength(maxFadeInLength, frames, maxZeroCrossings, blockFrames);
		n->m_fadeInLength = length;

		// apply fade in
		length = length < frames ? length : frames;
		for (f_cnt_t f = 0; f < length; ++f)
		{
			for (ch_cnt_t ch = 0; ch < DEFAULT_CHANNELS; ++ch)
			{
				buf[offset + f][ch] *= 0.5 - 0.5 * std::cos(std::numbers::pi_v<float> * static_cast<float>(f) / static_cast<float>(n->m_fadeInLength));
			}
		}
	}
	else if (total < n->m_fadeInLength)
	{
		const f_cnt_t frames = n->framesLeftForCurrentPeriod();

		int new_zc = countZeroCrossings(buf, 1, frames);
		f_cnt_t new_length = getFadeInLength(maxFadeInLength, frames, new_zc, blockFrames);

		for (f_cnt_t f = 0; f < frames; ++f)
		{
			const float currentLength = n->m_fadeInLength * (1.0f - (float) f / frames) + new_length * ((float) f / frames);
			if (total + f >= currentLength)
			{
				// Faded in (both channels alike)
				n->m_fadeInLength = currentLength;
				return;
			}
			const float gain = 0.5f - 0.5f * std::cos(std::numbers::pi_v<float> * static_cast<float>(total + f) / currentLength);
			for (ch_cnt_t ch = 0; ch < DEFAULT_CHANNELS; ++ch) { buf[f][ch] *= gain; }
		}
		n->m_fadeInLength = new_length;
	}
}

void Instrument::applyRelease( SampleFrame* buf, const NotePlayHandle * _n )
{
	const auto fpp = Engine::audioEngine()->framesPerPeriod();
	const auto releaseFrames = desiredReleaseFrames();

	const auto endFrame = _n->framesLeft();
	const auto startFrame = endFrame - std::min(endFrame, releaseFrames);

	for (auto f = startFrame; f < endFrame && f < fpp; f++)
	{
		const float fac = (float)(endFrame - f) / (float)releaseFrames;
		for (ch_cnt_t ch = 0; ch < DEFAULT_CHANNELS; ch++)
		{
			buf[f][ch] *= fac;
		}
	}
}

float Instrument::computeReleaseTimeMsByFrameCount(f_cnt_t frames) const
{
	return frames / getSampleRate() * 1000.;
}


sample_rate_t Instrument::getSampleRate() const
{
	return Engine::audioEngine()->outputSampleRate();
}


QString Instrument::fullDisplayName() const
{
	return instrumentTrack()->displayName();
}


} // namespace lmms
