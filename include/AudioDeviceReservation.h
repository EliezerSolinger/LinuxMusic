/*
 * AudioDeviceReservation.h - asks the desktop's sound server for a sound card (org.freedesktop.ReserveDevice1)
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

#ifndef LMMS_AUDIO_DEVICE_RESERVATION_H
#define LMMS_AUDIO_DEVICE_RESERVATION_H

#include "lmmsconfig.h"

#ifdef LMMS_HAVE_QTDBUS

#include <QObject>
#include <QString>

namespace lmms
{

/**
 * The device reservation protocol (http://git.0pointer.net/reserve.git/tree/reserve.txt): a program owns the D-Bus
 * name org.freedesktop.ReserveDevice1.Audio<card> on the session bus while it uses the card. PipeWire and
 * PulseAudio own the names of the cards they use, with a low priority, and give a card up when a program with a
 * higher priority asks for it through RequestRelease() (as JACK does). They take it back when the name is free.
 *
 * While reserved, the card is ours: RequestRelease() from others is refused.
 */
class AudioDeviceReservation : public QObject
{
	Q_OBJECT
	Q_CLASSINFO("D-Bus Interface", "org.freedesktop.ReserveDevice1")
	Q_PROPERTY(int Priority READ priority)
	Q_PROPERTY(QString ApplicationName READ applicationName)
	Q_PROPERTY(QString ApplicationDeviceName READ applicationDeviceName)

public:
	//! @param card the ALSA card index (as in hw:<card>)
	AudioDeviceReservation(int card, QString deviceName);
	~AudioDeviceReservation() override;

	/**
	 * Reserves the card: asks whoever holds it to give it up
	 * @returns false if the holder refused (the card is busy); true if the card is ours, or if there is no session
	 *          bus (then nobody can hold it this way)
	 */
	auto acquire() -> bool;

	//! Frees the card, which the sound server then takes back
	void release();

	auto priority() const -> int { return Priority; }
	auto applicationName() const -> QString;
	auto applicationDeviceName() const -> QString { return m_deviceName; }

	//! The priority we ask with: above PipeWire's and PulseAudio's (0), as JACK's
	static constexpr int Priority = 10;

	//! @returns the card index of an ALSA device name (hw:1, hw:CARD=USB,DEV=0, plughw:1,0), or -1 if not a card
	static auto cardOfDevice(const QString& device) -> int;

public slots:
	//! D-Bus: another program asks for the card. We keep it while we use it.
	bool RequestRelease(int priority);

private:
	auto serviceName() const -> QString;
	auto objectPath() const -> QString;

	int m_card;
	QString m_deviceName;
	bool m_owned = false;
};

} // namespace lmms

#endif // LMMS_HAVE_QTDBUS

#endif // LMMS_AUDIO_DEVICE_RESERVATION_H
