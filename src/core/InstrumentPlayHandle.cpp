/*
 * InstrumentPlayHandle.cpp - play-handle for driving an instrument
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


#include "InstrumentPlayHandle.h"

#include "Hardware.h"
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "Engine.h"
#include "AudioEngine.h"

namespace lmms
{


InstrumentPlayHandle::InstrumentPlayHandle(Instrument * instrument, InstrumentTrack* instrumentTrack) :
	PlayHandle(Type::InstrumentPlayHandle),
	m_instrument(instrument)
{
	setAudioBusHandle(instrumentTrack->audioBusHandle());
}

void InstrumentPlayHandle::play(std::span<SampleFrame> buffer)
{
	InstrumentTrack * instrumentTrack = m_instrument->instrumentTrack();

	// ensure that all our nph's have been processed first
	auto nphv = NotePlayHandle::nphsOfInstrumentTrack(instrumentTrack, true);

	// A note queued for this period is processed here if no worker took it yet, and waited for if one did. Its own
	// state says so; asking the note whether it is finished instead read it while a worker was playing it, and could
	// let the instrument render before the note was done.
	bool nphsLeft;
	do
	{
		nphsLeft = false;
		for (const auto& handle : nphv)
		{
			const auto state = handle->state();
			if (state == ThreadableJob::ProcessingState::Queued)
			{
				const_cast<NotePlayHandle*>(handle)->process();
				nphsLeft = nphsLeft || handle->state() != ThreadableJob::ProcessingState::Done;
			}
			else if (state == ThreadableJob::ProcessingState::InProgress) { nphsLeft = true; }
		}
		if (nphsLeft) { busyWaitHint(); }
	}
	while (nphsLeft);

	instrumentTrack->playInstrument(m_instrument, buffer);

	// Process the audio buffer that the instrument has just worked on...
	instrumentTrack->processAudioBuffer(buffer.data(), buffer.size(), nullptr);
}

bool InstrumentPlayHandle::isFromTrack(const Track* track) const
{
	return m_instrument->isFromTrack(track);
}


} // namespace lmms
