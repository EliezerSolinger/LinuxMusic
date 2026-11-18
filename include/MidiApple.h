/*
 * MidiApple.h - the macOS MIDI client (CoreMIDI)
 *
 * Copyright (c) 2005-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
 * Copyright (c) 2015 Maurizio Lo Bosco (rageboge on github)
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

#ifndef LMMS_MIDI_APPLE_H
#define LMMS_MIDI_APPLE_H

#include "lmmsconfig.h"

#ifdef LMMS_BUILD_APPLE

#include <map>
#include <memory>
#include <mutex>
#include <vector>

#include <CoreMIDI/CoreMIDI.h>

#include "MidiByteParser.h"
#include "MidiClient.h"
#include "MidiPort.h"


namespace lmms
{


/**
 * The macOS MIDI client (CoreMIDI). Every source and destination online is a port to subscribe to: a device's are
 * named "device:endpoint" (as projects saved them), a virtual one (another program's) by its own name. Devices
 * plugged in or out while the program runs come and go (hotplug); a track's subscriptions are kept by name, so a
 * device plugged in again is subscribed again.
 *
 * Threads: the lists and subscriptions change on the main thread, under m_routingMutex. CoreMIDI hands the bytes
 * received to a thread of its own (readProc()), which queues the events for their ports (the audio thread delivers
 * them: see MidiClient); the events sent may come from the audio thread. Those threads take
 * nothing but the routing they need, under the mutex, which a thread other than the main one only tries: while the
 * main thread changes the routing (a moment), an event there is dropped rather than the thread waiting. Nothing is
 * allocated on them.
 */
class MidiApple : public QObject, public MidiClient
{
	Q_OBJECT
public:
	MidiApple();
	~MidiApple() override;

	inline static QString probeDevice()
	{
		return QString(); // no midi device name
	}

	inline static QString name()
	{
		return QT_TRANSLATE_NOOP( "MidiSetupWidget", "Apple MIDI" );
	}
	inline static QString configSection()
	{
		return QString(); // no configuration settings
	}

	void processOutEvent(const MidiEvent& event, const TimePos& time, const MidiPort* port) override;

	void applyPortMode(MidiPort* port) override;
	void removePort(MidiPort* port) override;

	// list devices as ports
	QStringList readablePorts() const override;
	QStringList writablePorts() const override;

	//! The name of the source @p event came from (for learning a controller)
	QString sourcePortName(const MidiEvent& event) const override;

	// (un)subscribe given MidiPort to/from destination-port
	void subscribeReadablePort(MidiPort* port, const QString& dest, bool subscribe = true) override;
	void subscribeWritablePort(MidiPort* port, const QString& dest, bool subscribe = true) override;

	void connectRPChanged(QObject* receiver, const char* member) override
	{
		connect(this, SIGNAL(readablePortsChanged()), receiver, member);
	}

	void connectWPChanged(QObject* receiver, const char* member) override
	{
		connect(this, SIGNAL(writablePortsChanged()), receiver, member);
	}

	bool isRaw() const override
	{
		return false;
	}

	//! Looks at the sources and destinations online again (CoreMIDI says when they change; tests call it too)
	void rescan();

signals:
	void readablePortsChanged();
	void writablePortsChanged();

private:
	//! A source seen since the client was made (by CoreMIDI's unique id): it stays as long as the client, as its
	//! address is its connection's context and its name the events' source (see sourcePortName())
	struct Source
	{
		QString name;
		MIDIEndpointRef ref = 0;
		bool connected = false;
		MidiByteParser parser; //!< (CoreMIDI's thread's)
	};

	static void readProc(const MIDIPacketList* packets, void* client, void* source);
	static void notifyProc(const MIDINotification* message, void* client);
	static auto endpointName(MIDIEndpointRef endpoint) -> QString;

	MIDIClientRef m_client = 0;
	MIDIPortRef m_inputPort = 0;
	MIDIPortRef m_outputPort = 0;

	std::map<MIDIUniqueID, std::unique_ptr<Source>> m_sources; //!< (main thread)
	std::vector<std::unique_ptr<Source>> m_renamedSources;	//!< sources whose name changed: kept, see Source

	// The routing (see the class's documentation)
	mutable std::mutex m_routingMutex;
	QMap<QString, MIDIEndpointRef> m_inputDevices;
	QMap<QString, MIDIEndpointRef> m_outputDevices;
	using SubMap = QMap<QString, MidiPortList>;
	SubMap m_inputSubs;
	SubMap m_outputSubs;
} ;


} // namespace lmms

#endif // LMMS_BUILD_APPLE

#endif // LMMS_MIDI_APPLE_H
