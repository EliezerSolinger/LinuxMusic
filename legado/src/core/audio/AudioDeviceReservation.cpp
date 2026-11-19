/*
 * AudioDeviceReservation.cpp - asks the desktop's sound server for a sound card (org.freedesktop.ReserveDevice1)
 *
 * This file is part of LinuxMusic, a fork of LMMS - https://lmms.io
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

#include "AudioDeviceReservation.h"

#ifdef LMMS_HAVE_QTDBUS

#include <QCoreApplication>
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusInterface>
#include <QDBusReply>
#include <QRegularExpression>
#include <QtDebug>

#ifdef LMMS_HAVE_ALSA
#include <alsa/asoundlib.h>
#endif

namespace lmms
{

AudioDeviceReservation::AudioDeviceReservation(int card, QString deviceName)
	: m_card{card}
	, m_deviceName{std::move(deviceName)}
{
}




AudioDeviceReservation::~AudioDeviceReservation()
{
	release();
}




auto AudioDeviceReservation::applicationName() const -> QString
{
	return QCoreApplication::applicationName();
}




auto AudioDeviceReservation::serviceName() const -> QString
{
	return QString{"org.freedesktop.ReserveDevice1.Audio%1"}.arg(m_card);
}




auto AudioDeviceReservation::objectPath() const -> QString
{
	return QString{"/org/freedesktop/ReserveDevice1/Audio%1"}.arg(m_card);
}




auto AudioDeviceReservation::acquire() -> bool
{
	if (m_owned) { return true; }
	auto bus = QDBusConnection::sessionBus();
	if (!bus.isConnected()) { return true; }
	auto* busInterface = bus.interface();

	const auto take = [&](QDBusConnectionInterface::ServiceQueueOptions queue) {
		const auto reply = busInterface->registerService(serviceName(), queue,
			QDBusConnectionInterface::DontAllowReplacement);
		return reply.isValid() && reply.value() == QDBusConnectionInterface::ServiceRegistered;
	};

	if (!take(QDBusConnectionInterface::DontQueueService))
	{
		// Someone holds the card: ask them to give it up
		auto holder = QDBusInterface{serviceName(), objectPath(), "org.freedesktop.ReserveDevice1", bus};
		holder.setTimeout(5000);
		const QDBusReply<bool> released = holder.call("RequestRelease", Priority);
		if (!released.isValid() || !released.value())
		{
			qWarning("ALSA: the sound server would not give up card %d", m_card);
			return false;
		}
		if (!take(QDBusConnectionInterface::ReplaceExistingService))
		{
			qWarning("ALSA: could not reserve card %d", m_card);
			return false;
		}
	}

	bus.registerObject(objectPath(), this, QDBusConnection::ExportAllSlots | QDBusConnection::ExportAllProperties);
	m_owned = true;
	return true;
}




void AudioDeviceReservation::release()
{
	if (!m_owned) { return; }
	auto bus = QDBusConnection::sessionBus();
	bus.unregisterObject(objectPath());
	bus.interface()->unregisterService(serviceName());
	m_owned = false;
}




bool AudioDeviceReservation::RequestRelease(int)
{
	return false;
}




auto AudioDeviceReservation::cardOfDevice(const QString& device) -> int
{
	static const auto hardware = QRegularExpression{"^(?:plug)?hw:(?:CARD=)?([^,]+)"};
	const auto match = hardware.match(device);
	if (!match.hasMatch()) { return -1; }
#ifdef LMMS_HAVE_ALSA
	return snd_card_get_index(match.captured(1).toLocal8Bit().constData());
#else
	bool isNumber = false;
	const auto card = match.captured(1).toInt(&isNumber);
	return isNumber ? card : -1;
#endif
}

} // namespace lmms

#endif // LMMS_HAVE_QTDBUS
