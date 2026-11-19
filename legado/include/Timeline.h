/*
 * Timeline.h
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

#ifndef LMMS_TIMELINE_H
#define LMMS_TIMELINE_H

#include <QObject>

#include <atomic>
#include <cstdint>

#include "AudioEngine.h"
#include "Engine.h"
#include "JournallingObject.h"
#include "TimePos.h"

namespace lmms {

class Timeline : public QObject, public JournallingObject
{
	Q_OBJECT

public:
	enum class StopBehaviour
	{
		BackToZero,
		BackToStart,
		KeepPosition
	};

	// The position and the loop are written by the audio thread (as the song plays) or the GUI, and read by both:
	// they are atomic, and the loop's begin and end change together

	auto pos() const -> TimePos { return TimePos{ticks()}; }

	auto ticks() const -> tick_t { return m_ticks.load(std::memory_order_relaxed); }

	//! Forcefully sets the current ticks, resets the frame offset, and sets the elapsed seconds based on the global position (ignoring potential mid-song tempo changes)
	//! Unless `jumped` is passed as false, this function will emit the `positionJumped` signal to allow other widgets to update accordingly.
	void setTicks(tick_t ticks, bool jumped = true)
	{
		m_ticks.store(ticks, std::memory_order_relaxed);
		m_frameOffset.store(0, std::memory_order_relaxed);
		m_elapsedSeconds.store(ticks * Engine::framesPerTick() / Engine::audioEngine()->outputSampleRate(),
			std::memory_order_relaxed);
		if (jumped) { emit positionJumped(); }
		emit positionChanged();
	}

	//! Advances the current timeline position by a certain number of ticks, in addition to updating the elapsed time based on the current tempo.
	void incrementTicks(tick_t increment)
	{
		m_ticks.store(ticks() + increment, std::memory_order_relaxed);
		m_elapsedSeconds.store(m_elapsedSeconds.load(std::memory_order_relaxed)
			+ increment * Engine::framesPerTick() / Engine::audioEngine()->outputSampleRate(), std::memory_order_relaxed);
		emit positionChanged();
	}

	auto frameOffset() const -> float { return m_frameOffset.load(std::memory_order_relaxed); }
	void setFrameOffset(const float frame) { m_frameOffset.store(frame, std::memory_order_relaxed); }

	auto loopBegin() const -> TimePos { return TimePos{loopBeginTicks(m_loop.load(std::memory_order_relaxed))}; }
	auto loopEnd() const -> TimePos { return TimePos{loopEndTicks(m_loop.load(std::memory_order_relaxed))}; }
	auto loopEnabled() const -> bool { return m_loopEnabled.load(std::memory_order_relaxed); }

	void setLoopBegin(TimePos begin);
	void setLoopEnd(TimePos end);
	void setLoopPoints(TimePos begin, TimePos end);
	void setLoopEnabled(bool enabled);

	auto playStartPosition() const -> TimePos { return TimePos{m_playStartPosition.load(std::memory_order_relaxed)}; }
	auto stopBehaviour() const -> StopBehaviour { return m_stopBehaviour.load(std::memory_order_relaxed); }

	void setPlayStartPosition(TimePos position)
	{
		m_playStartPosition.store(position.getTicks(), std::memory_order_relaxed);
	}
	void setStopBehaviour(StopBehaviour behaviour);

	auto getElapsedSeconds() const -> double
	{
		return m_elapsedSeconds.load(std::memory_order_relaxed) + frameOffset() / Engine::audioEngine()->outputSampleRate();
	}

	auto nodeName() const -> QString override { return "timeline"; }

signals:
	void loopEnabledChanged(bool enabled);
	void stopBehaviourChanged(lmms::Timeline::StopBehaviour behaviour);
	void positionChanged();
	void positionJumped();

protected:
	void saveSettings(QDomDocument& doc, QDomElement& element) override;
	void loadSettings(const QDomElement& element) override;

private:
	//! The loop's begin and end in one word (see packLoop()), so that nobody sees one changed without the other
	static constexpr auto packLoop(tick_t begin, tick_t end) -> std::uint64_t
	{
		return (static_cast<std::uint64_t>(static_cast<std::uint32_t>(begin)) << 32) | static_cast<std::uint32_t>(end);
	}
	static constexpr auto loopBeginTicks(std::uint64_t loop) -> tick_t
	{
		return static_cast<tick_t>(static_cast<std::uint32_t>(loop >> 32));
	}
	static constexpr auto loopEndTicks(std::uint64_t loop) -> tick_t
	{
		return static_cast<tick_t>(static_cast<std::uint32_t>(loop));
	}
	void setLoop(TimePos begin, TimePos end);

	std::atomic<std::uint64_t> m_loop = packLoop(0, DefaultTicksPerBar);
	std::atomic<bool> m_loopEnabled = false;
	std::atomic<tick_t> m_ticks = 0;

	std::atomic<float> m_frameOffset = 0;

	std::atomic<double> m_elapsedSeconds = 0;

	std::atomic<StopBehaviour> m_stopBehaviour = StopBehaviour::BackToStart;
	std::atomic<tick_t> m_playStartPosition = -1;
};

} // namespace lmms

#endif // LMMS_TIMELINE_H
