/*
 * MidiClient.h - base-class for MIDI clients like ALSA-sequencer-client
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

#ifndef LMMS_MIDI_CLIENT_H
#define LMMS_MIDI_CLIENT_H

#include <QStringList>
#include <vector>


#include "MidiByteParser.h"
#include "MidiEvent.h"

class QObject;


namespace lmms
{

class MidiPort;
class TimePos;


// base-class for all MIDI-clients
class MidiClient
{
public:
	MidiClient() = default;
	virtual ~MidiClient();

	// to be implemented by sub-classes
	virtual void processOutEvent( const MidiEvent & _me,
						const TimePos & _time,
						const MidiPort * _port ) = 0;

	// inheriting classes can re-implement this for being able to update
	// their internal port-structures etc.
	virtual void applyPortMode( MidiPort * _port );
	virtual void applyPortName( MidiPort * _port );

	virtual void addPort( MidiPort * _port );
	//! The ports registered with it
	auto ports() const -> const std::vector<MidiPort*>& { return m_midiPorts; }

	// re-implemented methods HAVE to call removePort() of base-class!!
	virtual void removePort( MidiPort * _port );


	// returns whether client works with raw-MIDI, only needs to be
	// re-implemented by MidiClientRaw for returning true
	virtual bool isRaw() const
	{
		return false;
	}

	// if not raw-client, return all readable/writable ports
	virtual QStringList readablePorts() const
	{
		return QStringList();
	}
	virtual QStringList writablePorts() const
	{
		return QStringList();
	}

	// return name of port which specified MIDI event came from
	virtual QString sourcePortName( const MidiEvent & ) const
	{
		return QString();
	}


	// (un)subscribe given MidiPort to/from destination-port
	virtual void subscribeReadablePort( MidiPort * _port,
						const QString & _dest,
						bool _subscribe = true );
	virtual void subscribeWritablePort( MidiPort * _port,
						const QString & _dest,
						bool _subscribe = true );

	// qobject-derived classes can use this for make a slot being
	// connected to signal of non-raw-MIDI-client if port-lists change
	virtual void connectRPChanged( QObject *, const char * )
	{
	}

	virtual void connectWPChanged( QObject *, const char * )
	{
	}

	// tries to open either MIDI-driver from config-file or (if it fails)
	// any other working
	static MidiClient * openMidiClient();

protected:
	std::vector<MidiPort *> m_midiPorts;

} ;




class MidiClientRaw : public MidiClient
{
public:
	MidiClientRaw() = default;
	~MidiClientRaw() override = default;

	// we are raw-clients for sure!
	bool isRaw() const override
	{
		return true;
	}


protected:
	// generic raw-MIDI-parser which generates appropriate MIDI-events
	void parseData( const unsigned char c );

	// to be implemented by actual client-implementation
	virtual void sendByte( const unsigned char c ) = 0;


private:
	//! Hands @p event to every port
	void processParsedEvent(const MidiEvent& event);
	void processOutEvent( const MidiEvent& event, const TimePos& time, const MidiPort* port ) override;


	//! The events of the stream received (see MidiByteParser)
	MidiByteParser m_parser;


} ;

} // namespace lmms

#endif // LMMS_MIDI_CLIENT_H
