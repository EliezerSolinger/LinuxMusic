/*
 * TimePos.cpp - Class that encapsulates the position of a note/event in terms of
 *   its bar, beat and tick.
 *
 * Copyright (c) 2004-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net
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

#include "TimePos.h"

#include <algorithm>

#include "MeterModel.h"

namespace lmms
{

TimeSig::TimeSig( int num, int denom ) :
	m_num(num),
	m_denom(denom)
{
}

TimeSig::TimeSig( const MeterModel &model ) :
	m_num(model.getNumerator()),
	m_denom(model.getDenominator())
{
}

TimePos::TimePos( const bar_t bar, const tick_t ticks ) :
	m_ticks( bar * s_ticksPerBar + ticks )
{
}

TimePos::TimePos( const tick_t ticks ) :
	m_ticks( ticks )
{
}

TimePos TimePos::quantize(float bars, bool forceRoundDown) const
{
	return quantize(m_ticks, static_cast<tick_t>(s_ticksPerBar * bars), forceRoundDown);
}




auto TimePos::quantize(tick_t ticks, tick_t interval, bool forceRoundDown) -> tick_t
{
	// At least a tick (a snap shorter than a tick, in a short bar, divided by zero)
	interval = std::max(interval, tick_t{1});
	//The lower position we could snap to
	int lowPos = ticks / interval;
	//Offset from the lower position
	int offset = ticks % interval;
	//1 if we should snap up, 0 if we shouldn't
	// Ternary expression is making sure that the snap happens in the direction to
	// the right even if ticks is negative and the offset is exactly half-way
	// More details on issue #5840 and PR #5847
	int snapUp = forceRoundDown || ((2 * offset) == -interval)
		? 0
		: (2 * offset) / interval;

	return (lowPos + snapUp) * interval;
}

} // namespace lmms
