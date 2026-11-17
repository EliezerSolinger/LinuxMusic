/*
 * Controller.h - declaration of class controller, which provides a
 *                standard for all controllers and controller plugins
 *
 * Copyright (c) 2008-2009 Paul Giblock <pgllama/at/gmail.com>
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

#ifndef LMMS_CONTROLLER_H
#define LMMS_CONTROLLER_H

#include "lmms_export.h"
#include "Engine.h"

#include <atomic>
#include "Model.h"
#include "JournallingObject.h"
#include "ValueBuffer.h"

namespace lmms
{

class Controller;
class ControllerConnection;

namespace gui
{

class ControllerDialog;

} // namespace gui

using ControllerVector = std::vector<Controller*>;

class LMMS_EXPORT Controller : public Model, public JournallingObject
{
	Q_OBJECT
public:
	enum class ControllerType
	{
		Dummy,
		Lfo,
		Midi,
		Peak,
		/*
		XY,
		Equation
		*/
	} ;

	Controller( ControllerType _type, Model * _parent,
						const QString & _display_name );

	~Controller() override;

	virtual float currentValue( int _offset );
	// The per-controller get-value-in-buffers function
	virtual ValueBuffer * valueBuffer();

	inline bool isSampleExact() const
	{
		return m_sampleExact;
	}

	void setSampleExact( bool _exact )
	{
		m_sampleExact = _exact;
	}

	inline ControllerType type() const
	{
		return( m_type );
	}

	// return whether this controller updates models frequently - used for
	// determining when to update GUI
	inline bool frequentUpdates() const
	{
		switch( m_type )
		{
			case ControllerType::Lfo: return( true );
			case ControllerType::Peak: return( true );
			default:
				break;
		}
		return( false );
	}

	virtual const QString & name() const
	{
		return( m_name );
	}


	void saveSettings( QDomDocument & _doc, QDomElement & _this ) override;
	void loadSettings( const QDomElement & _this ) override;
	QString nodeName() const override;

	static Controller * create( ControllerType _tt, Model * _parent );
	static Controller * create( const QDomElement & _this,
							Model * _parent );

	inline static float fittedValue( float _val )
	{
		return std::clamp(_val, 0.0f, 1.0f);
	}

	static long runningPeriods()
	{
		return s_periods;
	}
	static unsigned int runningFrames();
	static float runningTime();

	static void triggerFrameCounter();
	static void resetFrameCounter();

	/**
	 * Brings every controller's values up to the period about to render, once, on the audio thread before the
	 * worker threads start: they then only read them. (Each controller used to update itself on its first read in
	 * a period, from whichever worker thread read it first, while others read.)
	 */
	static void updateAllValueBuffers();

	//Accepts a ControllerConnection * as it may be used in the future.
	void addConnection( ControllerConnection * );
	void removeConnection( ControllerConnection * );
	int connectionCount() const;

	bool hasModel( const Model * m ) const;

public slots:
	virtual gui::ControllerDialog * createDialog( QWidget * _parent );

	virtual void setName( const QString & _new_name )
	{
		m_name = _new_name;
	}


protected:
	// The internal per-controller get-value function
	virtual float value( int _offset );

	virtual void updateValueBuffer();

	// buffer for storing sample-exact values in case there
	// are more than one model wanting it, so we don't have to create it
	// again every time
	ValueBuffer m_valueBuffer;
	// when we last updated the valuebuffer - so we know if we have to update it
	long m_bufferLastUpdated;

	//! The last value of the last period updated, for threads other than the audio threads (the GUI)
	std::atomic<float> m_publishedValue = 0.5f;
	//! updateValueBuffer(), then publishes its last value
	void refresh();

	//! The last value asked for (by any thread: the GUI and the worker threads)
	std::atomic<float> m_currentValue = 0.f;
	bool  m_sampleExact;
	int m_connectionCount;

	QString m_name;
	ControllerType m_type;

	static ControllerVector s_controllers;
	//! Every controller, for updateAllValueBuffers() (changed under the audio engine's change lock)
	static ControllerVector s_allControllers;

	//! The periods rendered (counted by the audio thread, read by any)
	static std::atomic<long> s_periods;


signals:
	// The value changed while the audio engine isn't running (i.e: MIDI CC)
	void valueChanged();

	friend class gui::ControllerDialog;

} ;


} // namespace lmms

#endif // LMMS_CONTROLLER_H
