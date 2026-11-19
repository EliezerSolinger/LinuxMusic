/*
 * ControllerConnection.h - declaration of a controller connect, which
 *              provides a definition of the link between a controller and
 *              model, also handles deferred creation of links while
 *              loading project
 *
 * Copyright (c) 2008 Paul Giblock <pgllama/at/gmail.com>
 * Copyright (c) 2010 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_CONTROLLER_CONNECTION_H
#define LMMS_CONTROLLER_CONNECTION_H

#include <memory>
#include <vector>

#include <QObject>

#include "Controller.h"
#include "JournallingObject.h"
#include "ValueBuffer.h"

namespace lmms
{

class ControllerConnection;

using ControllerConnectionVector = std::vector<ControllerConnection*>;

/**
 * The link from a model to the controller that drives it. The model owns the connection (see
 * AutomatableModel::setControllerConnection()). The controller is one of three:
 * - one of the song's controllers (an LFO or a peak controller), only observed: when it goes, the connection
 *   emits orphaned() and its model deletes it;
 * - a MIDI controller of its own, owned here;
 * - the shared dummy controller, while there is none (a connection loaded before its controller, see
 *   finalizeConnections()).
 * The audio thread reads the controller: it changes under the audio engine's change lock.
 */
class LMMS_EXPORT ControllerConnection : public QObject, public JournallingObject
{
	Q_OBJECT
public:
	//! To one of the song's controllers (observed); null for none yet
	explicit ControllerConnection(Controller* controller);
	//! To a MIDI controller of its own
	explicit ControllerConnection(std::unique_ptr<Controller> ownController);
	//! To the song's controller with this index, found by finalizeConnections() once the song is loaded
	explicit ControllerConnection(int controllerId);

	ControllerConnection(const ControllerConnection&) = delete;
	ControllerConnection& operator=(const ControllerConnection&) = delete;

	~ControllerConnection() override;

	//! Never null: the dummy controller while there is none
	Controller* getController() { return m_controller; }

	//! One of the song's controllers (observed); null for none
	void setController(Controller* controller);
	//! A MIDI controller of its own; null for none
	void setController(std::unique_ptr<Controller> ownController);

	float currentValue( int _offset )
	{
		return m_controller->currentValue( _offset );
	}

	ValueBuffer * valueBuffer()
	{
		return m_controller->valueBuffer();
	}

	inline QString targetName() const
	{
		return m_targetName;
	}

	inline bool isFinalized()
	{
		return m_controllerId < 0;
	}

	//! Once the song is loaded: connects what was loaded to the song's controllers, and orphans what is left
	//! without one
	static void finalizeConnections();

	void saveSettings( QDomDocument & _doc, QDomElement & _this ) override;
	void loadSettings( const QDomElement & _this ) override;

	static inline const QString classNodeName()
	{
		return "connection";
	}

	QString nodeName() const override
	{
		return classNodeName();
	}

signals:
	// The value changed while the audio engine isn't running (i.e: MIDI CC)
	void valueChanged();
	//! There is no controller for this connection any more: its model deletes it (nothing may follow this)
	void orphaned();

private slots:
	//! The observed controller is going away
	void controllerDestroyed();

private:
	//! Leaves the current controller (deleting it when it is our own) and takes @p controller (never null)
	void attach(Controller* controller, std::unique_ptr<Controller> ownController);

	Controller* m_controller;
	std::unique_ptr<Controller> m_ownController; //!< When the controller is our own (MIDI)
	QString m_targetName;
	int m_controllerId;

	static ControllerConnectionVector s_connections;
};


} // namespace lmms

#endif // LMMS_CONTROLLER_CONNECTION_H
