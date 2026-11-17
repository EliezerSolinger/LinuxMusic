/*
 * ControllerConnection.cpp - implementation of class controller connection
 *            which handles the link between AutomatableModels and controllers
 *
 * Copyright (c) 2008 Paul Giblock <drfaygo/at/gmail.com>
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

#include "ControllerConnection.h"

#include <algorithm>

#include <QDomElement>

#include "AudioEngine.h"
#include "Engine.h"
#include "ErrorLog.h"
#include "MidiController.h"
#include "Song.h"

namespace lmms
{


ControllerConnectionVector ControllerConnection::s_connections;

namespace
{

//! The audio thread reads the controller (the song's models go after the audio engine, at the end)
auto changesGuard() -> AudioEngine::RequestChangesGuard
{
	return Engine::audioEngine() ? Engine::audioEngine()->requestChangesGuard() : AudioEngine::RequestChangesGuard{};
}

auto dummyController() -> Controller*
{
	return Controller::create(Controller::ControllerType::Dummy, nullptr);
}

} // namespace




ControllerConnection::ControllerConnection(Controller* controller)
	: m_controller{dummyController()}
	, m_controllerId{-1}
{
	s_connections.push_back(this);
	setController(controller);
}




ControllerConnection::ControllerConnection(std::unique_ptr<Controller> ownController)
	: m_controller{dummyController()}
	, m_controllerId{-1}
{
	s_connections.push_back(this);
	setController(std::move(ownController));
}




ControllerConnection::ControllerConnection(int controllerId)
	: m_controller{dummyController()}
	, m_controllerId{controllerId}
{
	s_connections.push_back(this);
}




ControllerConnection::~ControllerConnection()
{
	attach(dummyController(), nullptr);
	std::erase(s_connections, this);
}




void ControllerConnection::attach(Controller* controller, std::unique_ptr<Controller> ownController)
{
	// The audio thread reads the controller
	auto previousOwn = std::unique_ptr<Controller>{};
	{
		const auto guard = changesGuard();
		if (m_controller->type() != Controller::ControllerType::Dummy) { m_controller->removeConnection(this); }
		QObject::disconnect(m_controller, nullptr, this, nullptr);
		previousOwn = std::move(m_ownController);
		m_controller = controller;
		m_ownController = std::move(ownController);
	}
	// (deleted outside the lock: a MIDI controller unsubscribes its ports)
	previousOwn.reset();

	if (m_controller->type() == Controller::ControllerType::Dummy) { return; }
	m_controller->addConnection(this);
	QObject::connect(m_controller, &Controller::valueChanged, this, &ControllerConnection::valueChanged,
		Qt::DirectConnection);
	if (!m_ownController)
	{
		QObject::connect(m_controller, &QObject::destroyed, this, &ControllerConnection::controllerDestroyed);
	}
}




void ControllerConnection::setController(Controller* controller)
{
	m_controllerId = -1;
	attach(controller ? controller : dummyController(), nullptr);
}




void ControllerConnection::setController(std::unique_ptr<Controller> ownController)
{
	m_controllerId = -1;
	if (!ownController)
	{
		attach(dummyController(), nullptr);
		return;
	}
	auto* controller = ownController.get();
	attach(controller, std::move(ownController));
}




void ControllerConnection::controllerDestroyed()
{
	// The controller is in its destructor: nothing of it may be used now
	{
		const auto guard = changesGuard();
		m_controller = dummyController();
	}
	emit orphaned();
}




/*
 * A connection may not be finalized.  This means, the connection should exist,
 * but the controller does not yet exist.  This happens when loading.  Even
 * loading connections last won't help, since there can be connections BETWEEN
 * controllers. So, we remember the controller-ID and use a dummyController
 * instead.  Once the song is loaded, finalizeConnections() connects to the proper controllers
 */
void ControllerConnection::finalizeConnections()
{
	auto orphans = ControllerConnectionVector{};
	for (auto* c : s_connections)
	{
		if (!c->isFinalized() && static_cast<std::size_t>(c->m_controllerId) < Engine::getSong()->controllers().size())
		{
			c->setController(Engine::getSong()->controllers().at(c->m_controllerId));
		}
		else if (c->getController()->type() == Controller::ControllerType::Dummy)
		{
			orphans.push_back(c);
		}
	}
	// Each one's model deletes it
	for (auto* c : orphans)
	{
		if (std::find(s_connections.begin(), s_connections.end(), c) != s_connections.end()) { emit c->orphaned(); }
	}
}




void ControllerConnection::saveSettings( QDomDocument & _doc, QDomElement & _this )
{
	if( Engine::getSong() )
	{
		if (m_ownController)
		{
			m_controller->saveState( _doc, _this );
		}
		else
		{
			const auto& controllers = Engine::getSong()->controllers();
			const auto it = std::find(controllers.begin(), controllers.end(), m_controller);
			if (it != controllers.end())
			{
				const int id = std::distance(controllers.begin(), it);
				_this.setAttribute("id", id);
			}
		}
	}
}




void ControllerConnection::loadSettings( const QDomElement & _this )
{
	QDomNode node = _this.firstChild();
	if( !node.isNull() )
	{
		// Only a MIDI controller is saved in its connection (see saveSettings())
		const auto element = node.toElement();
		if (static_cast<Controller::ControllerType>(element.attribute("type").toInt())
			!= Controller::ControllerType::Midi)
		{
			ErrorLog::report(ErrorLog::Severity::Warning, "ControllerConnection::loadSettings",
				"a connection held a controller other than MIDI: left without one");
			setController(nullptr);
			return;
		}
		auto controller = std::make_unique<MidiController>(nullptr);
		controller->restoreState(element);
		setController(std::move(controller));
	}
	else
	{
		setController(nullptr);
		m_controllerId = _this.attribute( "id", "-1" ).toInt();
		if( m_controllerId < 0 )
		{
			if (_this.hasAttribute("id"))
			{
				ErrorLog::report(ErrorLog::Severity::Warning, "ControllerConnection::loadSettings",
					"a controller index was invalid", m_controllerId);
			}
			m_controllerId = -1;
		}

		if (!Engine::getSong()->isLoadingProject()
			&& m_controllerId != -1
			&& static_cast<std::size_t>(m_controllerId) < Engine::getSong()->controllers().size())
		{
			setController( Engine::getSong()->
					controllers().at( m_controllerId ) );
		}
	}
}



} // namespace lmms
