/*
 * Metronome.cpp
 *
 * Copyright (c) 2024 saker
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


#include "Metronome.h"

#include "AudioBusHandle.h"
#include "AudioEngine.h"
#include "Engine.h"
#include "Sample.h"
#include "SampleBuffer.h"
#include "SamplePlayHandle.h"

namespace lmms {

Metronome::Metronome() = default;
Metronome::~Metronome() = default;




void Metronome::setActive(bool active)
{
	if (active) { prepare(); }
	m_active = active;
}




void Metronome::prepare()
{
	if (m_prepared) { return; }
	m_strongClick = std::make_unique<Sample>(SampleBuffer::fromFile("misc/metronome02.ogg"));
	m_weakClick = std::make_unique<Sample>(SampleBuffer::fromFile("misc/metronome01.ogg"));
	m_bus = std::make_unique<AudioBusHandle>("Metronome", false);
	m_prepared = true;
}




void Metronome::release()
{
	m_prepared = false;
	m_bus.reset();
	m_strongClick.reset();
	m_weakClick.reset();
}




void Metronome::processTick(int currentTick, int ticksPerBar, int beatsPerBar, size_t bufferOffset, bool force)
{
	const auto ticksPerBeat = ticksPerBar / beatsPerBar;
	if (ticksPerBeat <= 0 || currentTick % ticksPerBeat != 0 || !(m_active || force) || !m_prepared) { return; }

	// The handle plays a click prepared on the main thread, on the metronome's own bus
	auto* click = currentTick % ticksPerBar == 0 ? m_strongClick.get() : m_weakClick.get();
	const auto handle = new SamplePlayHandle(click, false);
	handle->setAudioBusHandle(m_bus.get());
	handle->setOffset(bufferOffset);
	Engine::audioEngine()->addPlayHandle(handle);
}
} // namespace lmms
