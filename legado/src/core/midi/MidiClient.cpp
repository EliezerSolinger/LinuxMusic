/*
 * MidiClient.cpp - base-class for MIDI-clients like ALSA-sequencer-client
 *
 * Copyright (c) 2005-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
 * This file partly contains code from Fluidsynth, Peter Hanappe
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

#include "MidiClient.h"

#include "MidiEventToByteSeq.h"

#include <algorithm>
#include <array>

#include "MidiPort.h"

namespace lmms
{

MidiClient::~MidiClient()
{
	//TODO: noteOffAll(); / clear all ports
	for (MidiPort* port : m_midiPorts)
	{
		port->invalidateCilent();
	}
}




void MidiClient::applyPortMode( MidiPort* )
{
}




void MidiClient::applyPortName( MidiPort* )
{
}




void MidiClient::addPort( MidiPort* port )
{
	m_midiPorts.push_back( port );
}




void MidiClient::removePort( MidiPort* port )
{
	if( ! port )
	{
		return;
	}

	auto it = std::find(m_midiPorts.begin(), m_midiPorts.end(), port);
	if( it != m_midiPorts.end() )
	{
		m_midiPorts.erase( it );
	}
}




void MidiClient::queueInEvent(MidiPort* port, const MidiEvent& event, const TimePos& time)
{
	const auto item = QueuedInEvent{port, port ? port->serial() : 0, event, time};
	if (!m_inEvents.tryPush(item)) { m_droppedInEvents.fetch_add(1, std::memory_order_relaxed); }
}




void MidiClient::deliverInEvents()
{
	while (const auto item = m_inEvents.tryPop())
	{
		if (!item->port)
		{
			for (auto* port : m_midiPorts) { port->processInEvent(item->event, item->time); }
			continue;
		}
		// (a port deleted since, or another made at its address, is not it)
		if (std::find(m_midiPorts.begin(), m_midiPorts.end(), item->port) != m_midiPorts.end()
			&& item->port->serial() == item->serial)
		{
			item->port->processInEvent(item->event, item->time);
		}
	}
}




void MidiClient::subscribeReadablePort( MidiPort*, const QString& , bool )
{
}




void MidiClient::subscribeWritablePort( MidiPort* , const QString& , bool )
{
}







void MidiClientRaw::parseData( const unsigned char c )
{
	m_parser.feed(c, nullptr, [this](const MidiEvent& event) { processParsedEvent(event); });
}




void MidiClientRaw::processParsedEvent(const MidiEvent& event)
{
	// (to every port, by the audio thread: see MidiClient)
	queueInEvent(nullptr, event);
}




void MidiClientRaw::processOutEvent(const MidiEvent& event, const TimePos&, const MidiPort* port)
{
	(void)port;
	// TODO: also evaluate _time and queue event if necessary
	auto bytes = std::array<uint8_t, 3>{};
	const auto size = writeToByteSeq(event, bytes.data(), bytes.size());
	for (std::size_t i = 0; i < size; ++i) { sendByte(bytes[i]); }
}

} // namespace lmms
