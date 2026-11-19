/*
 * MidiApple.cpp - the macOS MIDI client (CoreMIDI)
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

#include "MidiApple.h"

#ifdef LMMS_BUILD_APPLE

#include <algorithm>
#include <array>

#include <QMetaObject>

#include "MidiEventToByteSeq.h"
#include "MidiPort.h"
#include "PluginChecks.h"


namespace lmms
{


namespace
{

//! Room for the ports a source or destination is subscribed by, taken on CoreMIDI's or the audio thread (no allocation)
constexpr auto MaxSubscribers = std::size_t{64};

auto stringProperty(MIDIObjectRef object, CFStringRef property) -> QString
{
	auto value = CFStringRef{nullptr};
	if (MIDIObjectGetStringProperty(object, property, &value) != noErr || value == nullptr) { return {}; }
	const auto text = QString::fromCFString(value);
	CFRelease(value);
	return text;
}

auto isOnline(MIDIObjectRef object) -> bool
{
	auto offline = SInt32{0};
	return MIDIObjectGetIntegerProperty(object, kMIDIPropertyOffline, &offline) != noErr || offline == 0;
}

//! Locks @p mutex on the main thread; elsewhere only tries (see MidiApple's documentation)
auto routingLock(std::mutex& mutex) -> std::unique_lock<std::mutex>
{
	if (ThreadCheck::isMainThread()) { return std::unique_lock{mutex}; }
	return std::unique_lock{mutex, std::try_to_lock};
}

} // namespace




MidiApple::MidiApple()
{
	MIDIClientCreate(CFSTR("LinuxMusic"), &MidiApple::notifyProc, this, &m_client);
	if (m_client)
	{
		MIDIInputPortCreate(m_client, CFSTR("LinuxMusic in"), &MidiApple::readProc, this, &m_inputPort);
		MIDIOutputPortCreate(m_client, CFSTR("LinuxMusic out"), &m_outputPort);
	}
	rescan();
}




MidiApple::~MidiApple()
{
	// (no callback runs after its port is gone, so the sources may go afterwards)
	if (m_inputPort) { MIDIPortDispose(m_inputPort); }
	if (m_outputPort) { MIDIPortDispose(m_outputPort); }
	if (m_client) { MIDIClientDispose(m_client); }
}




auto MidiApple::endpointName(MIDIEndpointRef endpoint) -> QString
{
	// A device's endpoint: "device:endpoint", as projects saved it; another program's (virtual): its own name
	const auto name = stringProperty(endpoint, kMIDIPropertyName);
	auto entity = MIDIEntityRef{0};
	auto device = MIDIDeviceRef{0};
	if (MIDIEndpointGetEntity(endpoint, &entity) == noErr && entity
		&& MIDIEntityGetDevice(entity, &device) == noErr && device)
	{
		return stringProperty(device, kMIDIPropertyName) + ":" + name;
	}
	return name.isEmpty() ? stringProperty(endpoint, kMIDIPropertyDisplayName) : name;
}




void MidiApple::rescan()
{
	// Names made unique (two of the same device): the second one numbered
	const auto unique = [](const QMap<QString, MIDIEndpointRef>& taken, QString name) {
		if (!taken.contains(name)) { return name; }
		for (int n = 2;; ++n)
		{
			if (const auto numbered = QString{"%1 (%2)"}.arg(name).arg(n); !taken.contains(numbered)) { return numbered; }
		}
	};

	auto inputs = QMap<QString, MIDIEndpointRef>{};
	auto seen = std::vector<MIDIUniqueID>{};
	for (ItemCount i = 0, count = MIDIGetNumberOfSources(); i < count; ++i)
	{
		const auto ref = MIDIGetSource(i);
		if (!ref || !isOnline(ref)) { continue; }
		const auto name = unique(inputs, endpointName(ref));
		inputs.insert(name, ref);

		auto id = MIDIUniqueID{0};
		MIDIObjectGetIntegerProperty(ref, kMIDIPropertyUniqueID, &id);
		seen.push_back(id);
		auto& source = m_sources[id];
		if (source && source->name != name)
		{
			// Renamed: a new one, the old one kept (a callback may still be reading its name)
			if (source->connected) { MIDIPortDisconnectSource(m_inputPort, source->ref); }
			m_renamedSources.push_back(std::move(source));
		}
		if (!source)
		{
			source = std::make_unique<Source>();
			source->name = name;
		}
		if (!source->connected || source->ref != ref)
		{
			if (source->connected) { MIDIPortDisconnectSource(m_inputPort, source->ref); }
			source->ref = ref;
			source->connected = m_inputPort && MIDIPortConnectSource(m_inputPort, ref, source.get()) == noErr;
		}
	}
	// The sources gone (unplugged)
	for (auto& [id, source] : m_sources)
	{
		if (source->connected && std::find(seen.begin(), seen.end(), id) == seen.end())
		{
			MIDIPortDisconnectSource(m_inputPort, source->ref);
			source->connected = false;
		}
	}

	auto outputs = QMap<QString, MIDIEndpointRef>{};
	for (ItemCount i = 0, count = MIDIGetNumberOfDestinations(); i < count; ++i)
	{
		const auto ref = MIDIGetDestination(i);
		if (!ref || !isOnline(ref)) { continue; }
		outputs.insert(unique(outputs, endpointName(ref)), ref);
	}

	auto readableChanged = false;
	auto writableChanged = false;
	{
		const auto lock = std::lock_guard{m_routingMutex};
		readableChanged = inputs.keys() != m_inputDevices.keys();
		writableChanged = outputs.keys() != m_outputDevices.keys();
		m_inputDevices = std::move(inputs);
		m_outputDevices = std::move(outputs);
	}
	if (readableChanged) { emit readablePortsChanged(); }
	if (writableChanged) { emit writablePortsChanged(); }
}




void MidiApple::notifyProc(const MIDINotification* message, void* client)
{
	// (on the main thread's run loop, where the client was made) Devices plugged in or out: looked at again once
	// CoreMIDI is done telling
	switch (message->messageID)
	{
	case kMIDIMsgSetupChanged:
	case kMIDIMsgObjectAdded:
	case kMIDIMsgObjectRemoved:
	case kMIDIMsgPropertyChanged:
	{
		auto* self = static_cast<MidiApple*>(client);
		QMetaObject::invokeMethod(self, [self] { self->rescan(); }, Qt::QueuedConnection);
		break;
	}
	default:
		break;
	}
}




void MidiApple::readProc(const MIDIPacketList* packets, void* client, void* sourceContext)
{
	auto* self = static_cast<MidiApple*>(client);
	auto* source = static_cast<Source*>(sourceContext);
	if (!self || !source) { return; }

	// The events queued for the ports subscribed to this source (the audio thread hands them over: see MidiClient),
	// under the routing lock: while the routing holds a port, it is alive
	const auto lock = routingLock(self->m_routingMutex);
	if (!lock.owns_lock()) { return; }
	const auto it = self->m_inputSubs.constFind(source->name);
	const auto* packet = &packets->packet[0];
	for (UInt32 i = 0; i < packets->numPackets; ++i)
	{
		for (UInt16 b = 0; b < packet->length; ++b)
		{
			// (the parser goes on even with nobody subscribed: it keeps the running status)
			source->parser.feed(packet->data[b], &source->name, [&](const MidiEvent& event) {
				if (it == self->m_inputSubs.cend()) { return; }
				for (auto* port : *it) { self->queueInEvent(port, event); }
			});
		}
		packet = MIDIPacketNext(packet);
	}
}




void MidiApple::processOutEvent(const MidiEvent& event, const TimePos&, const MidiPort* port)
{
	auto bytes = std::array<std::uint8_t, 3>{};
	const auto size = writeToByteSeq(event, bytes.data(), bytes.size());
	if (size == 0 || !m_outputPort) { return; }

	// The destinations @p port is subscribed to (from any thread: see the class's documentation)
	auto destinations = std::array<MIDIEndpointRef, MaxSubscribers>{};
	auto count = std::size_t{0};
	{
		const auto lock = routingLock(m_routingMutex);
		if (!lock.owns_lock()) { return; }
		for (auto it = m_outputSubs.cbegin(); it != m_outputSubs.cend(); ++it)
		{
			if (!it.value().contains(const_cast<MidiPort*>(port))) { continue; }
			if (const auto ref = m_outputDevices.value(it.key(), 0); ref && count < destinations.size())
			{
				destinations[count++] = ref;
			}
		}
	}

	auto buffer = std::array<Byte, 64>{};
	auto* list = reinterpret_cast<MIDIPacketList*>(buffer.data());
	auto* packet = MIDIPacketListInit(list);
	packet = MIDIPacketListAdd(list, buffer.size(), packet, 0, size, bytes.data()); // (time 0: now)
	if (!packet) { return; }
	for (std::size_t d = 0; d < count; ++d) { MIDISend(m_outputPort, destinations[d], list); }
}




QStringList MidiApple::readablePorts() const
{
	const auto lock = std::lock_guard{m_routingMutex};
	return m_inputDevices.keys();
}




QStringList MidiApple::writablePorts() const
{
	const auto lock = std::lock_guard{m_routingMutex};
	return m_outputDevices.keys();
}




QString MidiApple::sourcePortName(const MidiEvent& event) const
{
	// (the events received point to their source's name, which lives as long as this client, see Source)
	if (const auto* name = static_cast<const QString*>(event.sourcePort())) { return *name; }
	return MidiClient::sourcePortName(event);
}




void MidiApple::applyPortMode(MidiPort* port)
{
	// No subscription the port's mode does not allow anymore
	const auto lock = std::lock_guard{m_routingMutex};
	if (!port->isInputEnabled())
	{
		for (auto& ports : m_inputSubs) { ports.removeAll(port); }
	}
	if (!port->isOutputEnabled())
	{
		for (auto& ports : m_outputSubs) { ports.removeAll(port); }
	}
}




void MidiApple::removePort(MidiPort* port)
{
	{
		const auto lock = std::lock_guard{m_routingMutex};
		for (auto& ports : m_inputSubs) { ports.removeAll(port); }
		for (auto& ports : m_outputSubs) { ports.removeAll(port); }
	}
	MidiClient::removePort(port);
}




void MidiApple::subscribeReadablePort(MidiPort* port, const QString& dest, bool subscribe)
{
	if (subscribe && !port->isInputEnabled())
	{
		qWarning("port %s can't be (un)subscribed!", qPrintable(port->displayName()));
		return;
	}
	// (kept by name: a device plugged in later is subscribed when it comes)
	const auto lock = std::lock_guard{m_routingMutex};
	auto& ports = m_inputSubs[dest];
	ports.removeAll(port);
	if (subscribe) { ports.push_back(port); }
	if (ports.empty()) { m_inputSubs.remove(dest); }
}




void MidiApple::subscribeWritablePort(MidiPort* port, const QString& dest, bool subscribe)
{
	if (subscribe && !port->isOutputEnabled())
	{
		qWarning("port %s can't be (un)subscribed!", qPrintable(port->displayName()));
		return;
	}
	const auto lock = std::lock_guard{m_routingMutex};
	auto& ports = m_outputSubs[dest];
	ports.removeAll(port);
	if (subscribe) { ports.push_back(port); }
	if (ports.empty()) { m_outputSubs.remove(dest); }
}


} // namespace lmms

#endif // LMMS_BUILD_APPLE
