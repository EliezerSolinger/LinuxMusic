/*
 * Metronome.h
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

#ifndef LMMS_METRONOME_H
#define LMMS_METRONOME_H

#include <atomic>
#include <cstddef>
#include <memory>

namespace lmms {

class AudioBusHandle;
class Sample;

class Metronome
{
public:
	Metronome();
	~Metronome();

	bool active() const { return m_active; }

	//! Main thread: switches the metronome on or off (on, it is prepared first)
	void setActive(bool active);

	/**
	 * Main thread: loads the clicks and makes the metronome's bus, once. The audio thread only plays what is
	 * prepared: it neither reads the click files nor makes a bus.
	 */
	void prepare();

	//! Audio thread: clicks on the beats while active, or while @p force (e.g. a count-in)
	void processTick(int currentTick, int ticksPerBar, int beatsPerBar, size_t bufferOffset, bool force = false);

private:
	std::atomic<bool> m_active = false;
	std::atomic<bool> m_prepared = false;
	std::unique_ptr<Sample> m_strongClick; //!< on the first beat of a bar
	std::unique_ptr<Sample> m_weakClick; //!< on the others
	std::unique_ptr<AudioBusHandle> m_bus;
};
} // namespace lmms

#endif // LMMS_METRONOME_H
