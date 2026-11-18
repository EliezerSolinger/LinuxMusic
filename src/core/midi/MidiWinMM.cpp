/*
 * MidiWinMM.cpp - WinMM MIDI client
 *
 * Copyright (c) 2008-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "MidiWinMM.h"

#ifdef LMMS_HAVE_WINMM


namespace lmms
{


MidiWinMM::MidiWinMM() :
	MidiClient(),
	m_inputDevices(),
	m_outputDevices(),
	m_inputSubs(),
	m_outputSubs()
{
	openDevices();
}




MidiWinMM::~MidiWinMM()
{
	closeDevices();
}




void MidiWinMM::processOutEvent( const MidiEvent& event, const TimePos& time, const MidiPort* port )
{
	const DWORD shortMsg = ( event.type() + event.channel() ) +
				( ( event.param( 0 ) & 0xff ) << 8 ) +
				( ( event.param( 1 ) & 0xff ) << 16 );

	// (from the audio thread too: the routing changes on the main thread, under the lock)
	const auto lock = std::lock_guard{m_routingMutex};

	QStringList outDevs;
	for( SubMap::ConstIterator it = m_outputSubs.begin(); it != m_outputSubs.end(); ++it )
	{
		for( MidiPortList::ConstIterator jt = it.value().begin(); jt != it.value().end(); ++jt )
		{
			if( *jt == port )
			{
				outDevs += it.key();
				break;
			}
		}
	}

	for( QMap<HMIDIOUT, QString>::Iterator it = m_outputDevices.begin(); it != m_outputDevices.end(); ++it )
	{
		if( outDevs.contains( *it ) )
		{
			midiOutShortMsg( it.key(), shortMsg );
		}
	}
}




void MidiWinMM::applyPortMode( MidiPort* port )
{
	const auto lock = std::lock_guard{m_routingMutex};
	// make sure no subscriptions exist which are not possible with
	// current port-mode
	if( !port->isInputEnabled() )
	{
		for( SubMap::Iterator it = m_inputSubs.begin(); it != m_inputSubs.end(); ++it )
		{
			it.value().removeAll( port );
		}
	}

	if( !port->isOutputEnabled() )
	{
		for( SubMap::Iterator it = m_outputSubs.begin(); it != m_outputSubs.end(); ++it )
		{
			it.value().removeAll( port );
		}
	}
}




void MidiWinMM::removePort( MidiPort* port )
{
	{
		// (out of the routing under the lock: the input callback queues events for the ports there)
		const auto lock = std::lock_guard{m_routingMutex};
		for( SubMap::Iterator it = m_inputSubs.begin(); it != m_inputSubs.end(); ++it )
		{
			it.value().removeAll( port );
		}

		for( SubMap::Iterator it = m_outputSubs.begin(); it != m_outputSubs.end(); ++it )
		{
			it.value().removeAll( port );
		}
	}

	MidiClient::removePort( port );
}




QString MidiWinMM::sourcePortName( const MidiEvent& event ) const
{
	if( event.sourcePort() )
	{
		// (the source is the device's handle itself)
		const auto lock = std::lock_guard{m_routingMutex};
		return m_inputDevices.value(static_cast<HMIDIIN>(const_cast<void*>(event.sourcePort())));
	}

	return MidiClient::sourcePortName( event );
}




void MidiWinMM::subscribeReadablePort( MidiPort* port, const QString& dest, bool subscribe )
{
	if( subscribe && port->isInputEnabled() == false )
	{
		qWarning( "port %s can't be (un)subscribed!\n", port->displayName().toLatin1().constData() );
		return;
	}

	const auto lock = std::lock_guard{m_routingMutex};
	m_inputSubs[dest].removeAll( port );
	if( subscribe )
	{
		m_inputSubs[dest].push_back( port );
	}
}




void MidiWinMM::subscribeWritablePort( MidiPort* port, const QString& dest, bool subscribe )
{
	if( subscribe && port->isOutputEnabled() == false )
	{
		qWarning( "port %s can't be (un)subscribed!\n", port->displayName().toLatin1().constData() );
		return;
	}

	const auto lock = std::lock_guard{m_routingMutex};
	m_outputSubs[dest].removeAll( port );
	if( subscribe )
	{
		m_outputSubs[dest].push_back( port );
	}
}




void WINAPI CALLBACK MidiWinMM::inputCallback( HMIDIIN hm, UINT msg, DWORD_PTR inst, DWORD_PTR param1, DWORD_PTR param2 )
{
	if( msg == MIM_DATA )
	{
		( (MidiWinMM *) inst )->handleInputEvent( hm, param1 );
	}
}




void MidiWinMM::handleInputEvent( HMIDIIN hm, DWORD ev )
{
	const int cmd = ev & 0xff;
	if( cmd == MidiActiveSensing )
	{
		return;
	}
	const int par1 = ( ev >> 8 ) & 0xff;
	const int par2 = ev >> 16;
	const MidiEventTypes cmdtype = static_cast<MidiEventTypes>( cmd & 0xf0 );
	const int chan = cmd & 0x0f;

	// The event queued for each port subscribed to the device (the audio thread hands it over: see MidiClient), under
	// the lock: the routing changes under it, and while it holds a port, the port is alive
	const auto lock = std::lock_guard{m_routingMutex};
	const QString d = m_inputDevices.value( hm );
	const auto subs = m_inputSubs.constFind(d);
	if (d.isEmpty() || subs == m_inputSubs.cend())
	{
		return;
	}

	// (the source: the device's handle, which lasts while it is open, unlike the callback's argument)
	const auto source = static_cast<const void*>(hm);
	for (auto* port : subs.value())
	{
		switch (cmdtype)
		{
			case MidiNoteOn:
			case MidiNoteOff:
			case MidiKeyPressure:
			case MidiControlChange:
			case MidiProgramChange:
			case MidiChannelPressure:
				queueInEvent(port, MidiEvent(cmdtype, chan, par1, par2 & 0xff, source));
				break;

			case MidiPitchBend:
				queueInEvent(port, MidiEvent(cmdtype, chan, par1 + par2 * 128, 0, source));
				break;

			default:
				qWarning("MidiWinMM: unhandled input event %d\n", cmdtype);
				break;
		}
	}
}




void MidiWinMM::updateDeviceList()
{
	closeDevices();
	openDevices();

	emit readablePortsChanged();
	emit writablePortsChanged();
}



void MidiWinMM::closeDevices()
{
	{
		const auto lock = std::lock_guard{m_routingMutex};
		m_inputSubs.clear();
		m_outputSubs.clear();
	}

	// (closed outside the lock: closing waits for the device's callback, which takes it)
	QMapIterator<HMIDIIN, QString> i( m_inputDevices );

	HMIDIIN hInDev;
	while( i.hasNext() )
	{
		hInDev = i.next().key();
		midiInReset( hInDev );
		midiInClose( hInDev );
	}

	QMapIterator<HMIDIOUT, QString> o( m_outputDevices );
	while( o.hasNext() )
	{
		midiOutClose( o.next().key() );
	}

	const auto lock = std::lock_guard{m_routingMutex};
	m_inputDevices.clear();
	m_outputDevices.clear();
}




void MidiWinMM::openDevices()
{
	{
		const auto lock = std::lock_guard{m_routingMutex};
		m_inputDevices.clear();
		m_outputDevices.clear();
	}
	for( unsigned int i = 0; i < midiInGetNumDevs(); ++i )
	{
		MIDIINCAPSW c;
		midiInGetDevCapsW(i, &c, sizeof(c));
		HMIDIIN hm = 0;
		MMRESULT res = midiInOpen( &hm, i, (DWORD_PTR) &inputCallback,
						(DWORD_PTR) this,
							CALLBACK_FUNCTION );
		if( res == MMSYSERR_NOERROR )
		{
			{
				const auto lock = std::lock_guard{m_routingMutex};
				m_inputDevices[hm] = QString::fromWCharArray(c.szPname);
			}
			midiInStart( hm );
		}
	}

	for( unsigned int i = 0; i < midiOutGetNumDevs(); ++i )
	{
		MIDIOUTCAPSW c;
		midiOutGetDevCapsW(i, &c, sizeof(c));
		HMIDIOUT hm = 0;
		MMRESULT res = midiOutOpen( &hm, i, 0, 0, CALLBACK_NULL );
		if( res == MMSYSERR_NOERROR )
		{
			const auto lock = std::lock_guard{m_routingMutex};
			m_outputDevices[hm] = QString::fromWCharArray(c.szPname);
		}
	}
}


} // namespace lmms

#endif // LMMS_HAVE_WINMM
