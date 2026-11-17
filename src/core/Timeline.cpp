/*
 * Timeline.cpp
 *
 * Copyright (c) 2023 Dominic Clark
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
 */

#include "Timeline.h"

#include <algorithm>

#include <QDomElement>

namespace lmms {

void Timeline::setLoop(TimePos begin, TimePos end)
{
	const auto [first, last] = std::minmax(begin.getTicks(), end.getTicks());
	m_loop.store(packLoop(first, last), std::memory_order_relaxed);
}

void Timeline::setLoopBegin(TimePos begin)
{
	setLoop(begin, loopEnd());
}

void Timeline::setLoopEnd(TimePos end)
{
	setLoop(loopBegin(), end);
}

void Timeline::setLoopPoints(TimePos begin, TimePos end)
{
	setLoop(begin, end);
}

void Timeline::setLoopEnabled(bool enabled)
{
	if (m_loopEnabled.exchange(enabled, std::memory_order_relaxed) != enabled) {
		emit loopEnabledChanged(enabled);
	}
}

void Timeline::setStopBehaviour(StopBehaviour behaviour)
{
	if (m_stopBehaviour.exchange(behaviour, std::memory_order_relaxed) != behaviour) {
		emit stopBehaviourChanged(behaviour);
	}
}

void Timeline::saveSettings(QDomDocument& doc, QDomElement& element)
{
	element.setAttribute("lp0pos", static_cast<int>(loopBegin()));
	element.setAttribute("lp1pos", static_cast<int>(loopEnd()));
	element.setAttribute("lpstate", static_cast<int>(loopEnabled()));
	element.setAttribute("stopbehaviour", static_cast<int>(stopBehaviour()));
}

void Timeline::loadSettings(const QDomElement& element)
{
	setLoopPoints(
		static_cast<TimePos>(element.attribute("lp0pos").toInt()),
		static_cast<TimePos>(element.attribute("lp1pos").toInt())
	);
	setLoopEnabled(static_cast<bool>(element.attribute("lpstate").toInt()));
	setStopBehaviour(static_cast<StopBehaviour>(element.attribute("stopbehaviour", "1").toInt()));
}

} // namespace lmms
