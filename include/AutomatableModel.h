/*
 * AutomatableModel.h - declaration of class AutomatableModel
 *
 * Copyright (c) 2007-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_AUTOMATABLE_MODEL_H
#define LMMS_AUTOMATABLE_MODEL_H

#include <atomic>
#include <cmath>
#include <algorithm>
#include <utility>
#include <vector>
#include <QMap>
#include <QMutex>

#include "JournallingObject.h"
#include "Model.h"
#include "TimePos.h"
#include "ValueBuffer.h"
#include "ModelVisitor.h"


namespace lmms
{

// simple way to map a property of a view to a model
#define mapPropertyFromModelPtr(type,getfunc,setfunc,modelname)	\
		public:													\
			type getfunc() const								\
			{													\
				return (type) modelname->value();				\
			}													\
		public slots:											\
			void setfunc( const type val )						\
			{													\
				modelname->setValue( val );						\
			}

#define mapPropertyFromModel(type,getfunc,setfunc,modelname)	\
		public:													\
			type getfunc() const								\
			{													\
				return (type) modelname.value();				\
			}													\
		public slots:											\
			void setfunc( const type val )						\
			{													\
				modelname.setValue( (float) val );				\
			}

// use this to make subclasses visitable
#define MODEL_IS_VISITABLE \
	void accept(ModelVisitor& v) override { v.visit(*this); } \
	void accept(ConstModelVisitor& v) const override { v.visit(*this); }



class ControllerConnection;

class LMMS_EXPORT AutomatableModel : public Model, public JournallingObject
{
	Q_OBJECT
public:
	enum class ScaleType
	{
		Linear,
		Logarithmic,
		Decibel
	};


	~AutomatableModel() override;

	// Implement those by using the MODEL_IS_VISITABLE macro
	virtual void accept(ModelVisitor& v) = 0;
	virtual void accept(ConstModelVisitor& v) const = 0;

public:
	/**
	   @brief Return this class casted to Target
	   @test AutomatableModelTest.cpp
	   @param doThrow throw an assertion if the cast fails, instead of
	     returning a nullptr
	   @return the casted class if Target is the exact or a base class of
	     *this, nullptr otherwise
	*/
	template<class Target>
	Target* dynamicCast(bool doThrow = false)
	{
		DCastVisitor<Target> vis; accept(vis);
		if (doThrow && !vis.result) { Q_ASSERT(false); }
		return vis.result;
	}

	//! const overload, see overloaded function
	template<class Target>
	const Target* dynamicCast(bool doThrow = false) const
	{
		ConstDCastVisitor<Target> vis; accept(vis);
		if (doThrow && !vis.result) { Q_ASSERT(false); }
		return vis.result;
	}

	bool isAutomated() const;
	bool isAutomatedOrControlled() const
	{
		return isAutomated() || m_controllerConnection != nullptr;
	}

	ControllerConnection* controllerConnection() const
	{
		return m_controllerConnection;
	}


	void setControllerConnection( ControllerConnection* c );


	template<class T>
	static T castValue( const float v )
	{
		if constexpr (std::is_same_v<T, int>)
		{
			return static_cast<int>(std::round(v));
		}
		else if constexpr (std::is_same_v<T, bool>)
		{
			return std::round(v) != 0;
		}
		else
		{
			return static_cast<T>(v);
		}
	}

	float oldValue() const { return m_oldValue.load(std::memory_order_relaxed); }

	template<class T>
	inline T value( int frameOffset = 0 ) const
	{
		// TODO
		// The `m_value` should only be updated whenever the Controller value changes,
		// instead of the Model calling `controller->currentValue()` every time.
		// This becomes even worse in the case of linked Models, where it has to
		// loop through the list of all links.

		if (m_useControllerValue.load(std::memory_order_relaxed))
		{
			if (m_controllerConnection)
			{
				return castValue<T>(controllerValue(frameOffset));
			}
			for (auto next = m_nextLink; next != this; next = next->m_nextLink)
			{
				if (next->controllerConnection() && next->useControllerValue())
				{
					return castValue<T>(fittedValue(next->controllerValue(frameOffset)));
				}
			}
		}
		return castValue<T>(m_value.load(std::memory_order_relaxed));
	}

	float controllerValue( int frameOffset ) const;

	//! @brief Function that returns sample-exact data as a ValueBuffer
	//! @return pointer to model's valueBuffer when s.ex.data exists, NULL otherwise
	ValueBuffer * valueBuffer();

	template<class T>
	T initValue() const
	{
		return castValue<T>( m_initValue );
	}

	bool isAtInitValue() const
	{
		return m_value.load(std::memory_order_relaxed) == m_initValue;
	}

	template<class T>
	T minValue() const
	{
		return castValue<T>( m_minValue );
	}

	template<class T>
	T maxValue() const
	{
		return castValue<T>( m_maxValue );
	}

	template<class T>
	T step() const
	{
		return castValue<T>( m_step );
	}

	//! @brief Returns value scaled with the scale type and min/max values of this model
	float scaledValue( float value ) const;
	//! @brief Returns value applied with the inverse of this model's scale type
	float inverseScaledValue( float value ) const;
	//! @brief Returns value inverse-scaled and normalized into the range of 0..1
	float normValue(float value) const;
	//! @brief Returns value transformed from normalized to direct representation
	float inverseNormValue(float value) const;

	void setInitValue( const float value );

	void setValue(const float value, const bool isAutomated = false);

	void incValue( int steps )
	{
		setValue(m_value.load(std::memory_order_relaxed) + steps * m_step);
	}

	float range() const
	{
		return m_range;
	}

	void setRange( const float min, const float max, const float step = 1 );
	void setScaleType( ScaleType sc ) {
		m_scaleType = sc;
	}
	void setScaleLogarithmic( bool setToTrue = true )
	{
		setScaleType( setToTrue ? ScaleType::Logarithmic : ScaleType::Linear );
	}
	bool isScaleLogarithmic() const
	{
		return m_scaleType == ScaleType::Logarithmic;
	}

	void setStep( const float step );

	float centerValue() const
	{
		return m_centerValue;
	}

	void setCenterValue( const float centerVal )
	{
		m_centerValue = centerVal;
	}

	//! link this to @p model, copying the value from @p model
	void linkToModel(AutomatableModel* model);
	//! @return number of other models linked to this
	size_t countLinks() const;

	//! @brief Saves settings of AutomatableModel into a DOM element
	//!
	//! Saves settings (value, automation links and controller connections) of AutomatableModel into DOM element
	//! @p element using @p name as attribute/node name.
	//! 
	//! @param doc TODO
	//! @param element Where this option shall be saved. Depending on the model, this can be done in an attribute or in a
	//! subnode.
	//! @param name Name to store this model as.
	virtual void saveSettings(QDomDocument& doc, QDomElement& element, const QString& name);

	//! @brief Loads settings of AutomatableModel from a DOM element
	//!
	//! Loads settings (value, automation links and controller connections) of AutomatableModel from DOM element
	//! @p element using @p name as attribute/node name.
	virtual void loadSettings(const QDomElement& element, const QString& name);

	QString nodeName() const override
	{
		return "automatablemodel";
	}

	virtual QString displayValue( const float val ) const = 0;

	bool isLinked() const
	{
		return m_nextLink != this;
	}

	// a way to track changed values in the model and avoid using signals/slots - useful for speed-critical code.
	// note that this method should only be called once per period since it resets the state of the variable - so if your model
	// has to be accessed by more than one object, then this function shouldn't be used.
	bool isValueChanged()
	{
		if (m_valueChanged.exchange(false, std::memory_order_relaxed) || valueBuffer())
		{
			return true;
		}
		return false;
	}

	float globalAutomationValueAt( const TimePos& time );

	void setStrictStepSize( const bool b )
	{
		m_hasStrictStepSize = b;
	}

	static void incrementPeriodCounter()
	{
		s_periodCounter.fetch_add(1, std::memory_order_relaxed);
	}

	static void resetPeriodCounter()
	{
		s_periodCounter.store(0, std::memory_order_relaxed);
	}

	bool useControllerValue() const
	{
		return m_useControllerValue;
	}

	static bool mustQuoteName(const QString &name);

public slots:
	virtual void reset();
	void unlink();
	void unlinkControllerConnection();
	void setUseControllerValue(bool b = true);


protected:
	AutomatableModel(
						const float val = 0,
						const float min = 0,
						const float max = 0,
						const float step = 0,
						Model* parent = nullptr,
						const QString& displayName = QString(),
						bool defaultConstructed = false );
	//! returns a value which is in range between min() and
	//! max() and aligned according to the step size (step size 0.05 -> value
	//! 0.12345 becomes 0.10 etc.). You should always call it at the end after
	//! doing your own calculations.
	float fittedValue( float value ) const;

private:
	// dynamicCast implementation
	template<class Target>
	struct DCastVisitor : public ModelVisitor
	{
		Target* result = nullptr;
		void visit(Target& tar) { result = &tar; }
	};

	// dynamicCast implementation
	template<class Target>
	struct ConstDCastVisitor : public ConstModelVisitor
	{
		const Target* result = nullptr;
		void visit(const Target& tar) { result = &tar; }
	};

	void saveSettings( QDomDocument& doc, QDomElement& element ) override
	{
		saveSettings( doc, element, "value" );
	}

	void loadSettings( const QDomElement& element ) override
	{
		loadSettings( element, "value" );
	}

	void setValueInternal(const float value);

	//! linking is stored in a linked list ring
	//! @return the model whose `m_nextLink` is `this`,
	//! or `this` if there are no linked models
	AutomatableModel* getLastLinkedModel() const;

	//! @return true if the `model` is in the linked list
	bool isLinkedToModel(AutomatableModel* model) const;
	
	//! @brief Scales @p value from linear to logarithmic.
	//! @param value A linear value. Should be within [0,1].
	template<class T> T logToLinearScale(T value) const;

	//! @brief Rounds @p value to @p where if it is close to it
	//! @param[out] value Will be modified to rounded value
	//! @param[in] where The number to round @p value to, if it is near enough
	template<class T> void roundAt(T& value, const T& where) const;


	ScaleType m_scaleType; //!< scale type, linear by default
	//! The value (written by whoever sets it, the GUI or the automation on the audio thread; read by any thread)
	std::atomic<float> m_value;
	float m_initValue = 0.f;
	float m_minValue;
	float m_maxValue;
	float m_step;
	float m_range;
	float m_centerValue;

	std::atomic<bool> m_valueChanged;
	std::atomic<float> m_oldValue; //!< used by valueBuffer for interpolation

	// used to determine if step size should be applied strictly (ie. always)
	// or only when value set from gui (default)
	bool m_hasStrictStepSize;

	//! an `AutomatableModel` can be linked together with others in a linked list
	//! the list has no end, the last model is connected to the first forming a ring
	AutomatableModel* m_nextLink;


	//! NULL if not appended to controller, otherwise connection info
	ControllerConnection* m_controllerConnection;


	ValueBuffer m_valueBuffer;
	long m_lastUpdatedPeriod;
	//! Counts the audio periods (written by the audio thread, read by any thread)
	static std::atomic<long> s_periodCounter;

	bool m_hasSampleExactData;

	// prevent several threads from attempting to write the same vb at the same time
	QMutex m_valueBufferMutex;

	std::atomic<bool> m_useControllerValue;

signals:
	void initValueChanged( float val );
	void destroyed( lmms::jo_id_t id );

} ;




template <typename T> class LMMS_EXPORT TypedAutomatableModel : public AutomatableModel
{
public:
	using AutomatableModel::AutomatableModel;
	T value( int frameOffset = 0 ) const
	{
		return AutomatableModel::value<T>( frameOffset );
	}

	T initValue() const
	{
		return AutomatableModel::initValue<T>();
	}

	T minValue() const
	{
		return AutomatableModel::minValue<T>();
	}

	T maxValue() const
	{
		return AutomatableModel::maxValue<T>();
	}
};


// some typed AutomatableModel-definitions

class LMMS_EXPORT FloatModel : public TypedAutomatableModel<float>
{
	Q_OBJECT
	MODEL_IS_VISITABLE
public:
	FloatModel( float val = 0, float min = 0, float max = 0, float step = 0,
				Model * parent = nullptr,
				const QString& displayName = QString(),
				bool defaultConstructed = false ) :
		TypedAutomatableModel( val, min, max, step, parent, displayName, defaultConstructed )
	{
	}
	float getRoundedValue() const;
	int getDigitCount() const;
	QString displayValue( const float val ) const override;
} ;


class LMMS_EXPORT IntModel : public TypedAutomatableModel<int>
{
	Q_OBJECT
	MODEL_IS_VISITABLE
public:
	IntModel( int val = 0, int min = 0, int max = 0,
				Model* parent = nullptr,
				const QString& displayName = QString(),
				bool defaultConstructed = false ) :
		TypedAutomatableModel( val, min, max, 1, parent, displayName, defaultConstructed )
	{
	}
	QString displayValue( const float val ) const override;
} ;


class LMMS_EXPORT BoolModel : public TypedAutomatableModel<bool>
{
	Q_OBJECT
	MODEL_IS_VISITABLE
public:
	BoolModel( const bool val = false,
				Model* parent = nullptr,
				const QString& displayName = QString(),
				bool defaultConstructed = false ) :
		TypedAutomatableModel( val, false, true, 1, parent, displayName, defaultConstructed )
	{
	}
	QString displayValue( const float val ) const override;
} ;

/**
 * The values automations give models at a time. Flat, so that the audio thread, which fills one each tick, allocates
 * nothing once it has grown to the project's automations (clear() keeps the room; see Song::processAutomations()).
 */
class AutomatedValueMap
{
public:
	using Entry = std::pair<AutomatableModel*, float>;

	class Iterator
	{
	public:
		explicit Iterator(std::vector<Entry>::iterator it) : m_it{it} {}
		auto key() const -> AutomatableModel* { return m_it->first; }
		auto value() const -> float { return m_it->second; }
		auto operator*() const -> Entry& { return *m_it; }
		auto operator++() -> Iterator& { ++m_it; return *this; }
		auto operator++(int) -> Iterator { auto copy = *this; ++m_it; return copy; }
		auto operator==(const Iterator& other) const -> bool = default;
	private:
		std::vector<Entry>::iterator m_it;
	};

	auto operator[](AutomatableModel* model) -> float&
	{
		for (auto& entry : m_entries) { if (entry.first == model) { return entry.second; } }
		return m_entries.emplace_back(model, 0.f).second;
	}
	auto contains(const AutomatableModel* model) const -> bool
	{
		return std::any_of(m_entries.begin(), m_entries.end(), [model](const Entry& entry) { return entry.first == model; });
	}
	void remove(const AutomatableModel* model)
	{
		std::erase_if(m_entries, [model](const Entry& entry) { return entry.first == model; });
	}
	void clear() { m_entries.clear(); }
	void reserve(std::size_t size) { m_entries.reserve(size); }
	auto size() const -> std::size_t { return m_entries.size(); }
	auto isEmpty() const -> bool { return m_entries.empty(); }
	auto begin() -> Iterator { return Iterator{m_entries.begin()}; }
	auto end() -> Iterator { return Iterator{m_entries.end()}; }
	void swap(AutomatedValueMap& other) noexcept { m_entries.swap(other.m_entries); }

private:
	std::vector<Entry> m_entries;
};

} // namespace lmms

#endif // LMMS_AUTOMATABLE_MODEL_H
