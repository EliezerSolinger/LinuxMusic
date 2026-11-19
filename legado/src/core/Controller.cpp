/*
 * Controller.cpp - implementation of class controller which handles
 *                  remote-control of AutomatableModels
 *
 * Copyright (c) 2008 Paul Giblock <drfaygo/at/gmail.com>
 * Copyright (c) 2014 Lukas W <lukaswhl/at/gmail.com>
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

#include <QDomElement>

#include <vector>

#include "AudioEngine.h"
#include "PluginChecks.h"
#include "ControllerConnection.h"
#include "LfoController.h"
#include "MidiController.h"
#include "PeakController.h"

namespace lmms
{


std::atomic<long> Controller::s_periods = 0;
std::vector<Controller*> Controller::s_controllers;
std::vector<Controller*> Controller::s_allControllers;



Controller::Controller( ControllerType _type, Model * _parent,
					const QString & _display_name ) :
	Model( _parent, _display_name ),
	JournallingObject(),
	m_valueBuffer( Engine::audioEngine()->framesPerPeriod() ),
	m_bufferLastUpdated( -1 ),
	m_connectionCount( 0 ),
	m_type( _type )
{
	// A concrete controller is put in the lists the audio thread goes through by its own constructor, once whole
	// (publish()); the dummy one is this class itself
	if (_type == ControllerType::Dummy) { publish(); }
	updateValueBuffer();
}




void Controller::publish()
{
	// (the audio thread goes through the lists every period)
	const auto guard = AudioEngine::changesGuard();
	if (std::find(s_allControllers.begin(), s_allControllers.end(), this) != s_allControllers.end()) { return; }
	s_allControllers.push_back(this);
	if( m_type != ControllerType::Dummy && m_type != ControllerType::Midi )
	{
		s_controllers.push_back(this);
		// Determine which name to use
		for ( uint i=s_controllers.size(); ; i++ )
		{
			QString new_name = QString( tr( "Controller %1" ) )
					.arg( i );

			// Check if name is already in use
			bool name_used = false;
			for (Controller * controller : s_controllers)
			{
				if ( controller->name() == new_name )
				{
					name_used = true;
					break;
				}
			}
			if ( ! name_used )
			{
				m_name = new_name;
				break;
			}
		}
	}
}




void Controller::withdraw()
{
	const auto guard = AudioEngine::changesGuard();
	std::erase(s_allControllers, this);
	std::erase(s_controllers, this);
}



Controller::~Controller()
{
	// (taken out already by the concrete controller's destructor, while it was whole: withdraw())
	withdraw();

	m_valueBuffer.clear();
	// Remove connections by destroyed signal
}



// Get current value, with an offset into the current buffer for sample exactness
float Controller::currentValue( int _offset )
{
	if( _offset == 0 || isSampleExact() )
	{
		const auto value = fittedValue(this->value(_offset));
		m_currentValue.store(value, std::memory_order_relaxed);
		return value;
	}

	return m_currentValue.load(std::memory_order_relaxed);
}



float Controller::value( int offset )
{
	// Only the thread rendering the period brings the values up to date (see updateAllValueBuffers()): the worker
	// threads find them so, and other threads (the GUI) read the last ones
	// (other threads, the GUI, take the value published at the end of the last update: the buffer may be being
	// written meanwhile)
	if (!ThreadCheck::isAudioThread()) { return m_publishedValue.load(std::memory_order_relaxed); }
	if (AudioEngine::isRenderingThread() && m_bufferLastUpdated != s_periods)
	{
		refresh();
	}
	return m_valueBuffer.values()[ offset ];
}
	

ValueBuffer * Controller::valueBuffer()
{
	// (as value(); for the audio threads only)
	if (AudioEngine::isRenderingThread() && m_bufferLastUpdated != s_periods)
	{
		refresh();
	}
	return &m_valueBuffer;
}


void Controller::refresh()
{
	updateValueBuffer();
	if (const auto length = m_valueBuffer.length(); length > 0)
	{
		m_publishedValue.store(m_valueBuffer.values()[length - 1], std::memory_order_relaxed);
	}
}




void Controller::updateValueBuffer()
{
	m_valueBuffer.fill(0.5f);
	m_bufferLastUpdated = s_periods;
}


// Get position in frames
unsigned int Controller::runningFrames()
{
	return s_periods * Engine::audioEngine()->framesPerPeriod();
}



// Get position in seconds
float Controller::runningTime()
{
	return runningFrames() / Engine::audioEngine()->outputSampleRate();
}



void Controller::triggerFrameCounter()
{
	for (Controller * controller : s_controllers)
	{
		// This signal is for updating values for both stubborn knobs and for
		// painting.  If we ever get all the widgets to use or at least check
		// currentValue() then we can throttle the signal and only use it for
		// GUI.
		emit controller->valueChanged();
	}

	s_periods ++;
	//emit s_signaler.triggerValueChanged();
}



void Controller::updateAllValueBuffers()
{
	for (auto* controller : s_allControllers)
	{
		if (controller->m_bufferLastUpdated != s_periods) { controller->refresh(); }
	}
}



void Controller::resetFrameCounter()
{
	for (Controller * controller : s_controllers)
	{
		controller->m_bufferLastUpdated = 0;
	}
	s_periods = 0;
}



Controller * Controller::create( ControllerType _ct, Model * _parent )
{
	static Controller * dummy = nullptr;
	Controller * c = nullptr;

	switch( _ct )
	{
		case ControllerType::Dummy:
			if (!dummy)
				dummy = new Controller( ControllerType::Dummy, nullptr,
								QString() );
			c = dummy;
			break;

		case ControllerType::Lfo:
			c = new class LfoController( _parent );
			break;

		case ControllerType::Peak:
			//Already instantiated in EffectChain::loadSettings()
			Q_ASSERT( false );
			break;

		case ControllerType::Midi:
			c = new class MidiController( _parent );
			break;

		default: 
			break;
	}

	return( c );
}



Controller * Controller::create( const QDomElement & _this, Model * _parent )
{
	const auto controllerType = static_cast<ControllerType>(_this.attribute("type").toInt());
	auto controller = controllerType == ControllerType::Peak
		? PeakController::getControllerBySetting(_this)
		: create(controllerType, _parent);
	if (controller) { controller->restoreState(_this); }
	return controller;
}



bool Controller::hasModel( const Model * m ) const
{
	for (QObject * c : children())
	{
		auto am = qobject_cast<AutomatableModel*>(c);
		if( am != nullptr )
		{
			if( am == m )
			{
				return true;
			}

			ControllerConnection * cc = am->controllerConnection();
			if( cc != nullptr && cc->getController()->hasModel( m ) )
			{
				return true;
			}
		}
	}

	return false;
}



void Controller::saveSettings( QDomDocument & _doc, QDomElement & _this )
{
	_this.setAttribute( "type", static_cast<int>(type()) );
	_this.setAttribute( "name", name() );
}



void Controller::loadSettings( const QDomElement & _this )
{
	if( static_cast<ControllerType>(_this.attribute( "type" ).toInt()) != type() )
	{
		qWarning( "controller-type does not match controller-type of "
							"settings-node!\n" );
	}

	setName( _this.attribute( "name" ) );
}


QString Controller::nodeName() const
{
	return( "Controller" );
}


void Controller::addConnection( ControllerConnection * )
{
	m_connectionCount++;
}




void Controller::removeConnection( ControllerConnection * )
{
	m_connectionCount--;
	Q_ASSERT( m_connectionCount >= 0 );
}




int Controller::connectionCount() const{
	return m_connectionCount;
}


} // namespace lmms




