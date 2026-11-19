/*
 * MonoMode.cpp - an instrument track played one note at a time, legato and with glide
 *
 * This file is part of LinuxMusic, based on LMMS - https://lmms.io
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

#include "MonoMode.h"

#include <QDomElement>
#include <limits>

#include "AudioEngine.h"
#include "Engine.h"
#include "InstrumentTrack.h"

namespace lmms
{

MonoMode::MonoMode(InstrumentTrack* track)
	: Model(track, tr("Mono"))
	, m_track(track)
	, m_enabledModel(false, this, tr("Mono"))
	, m_legatoModel(true, this, tr("Legato"))
	, m_glideModel(0.f, 0.f, 2000.f, 1.f, this, tr("Glide"))
{
	// Turned off, the voice ends (it has no end of its own: the notes held gave it one). On this thread: automated,
	// it comes by a queued call
	connect(&m_enabledModel, &BoolModel::dataChanged, this, [this] {
		if (enabled()) { return; }
		const auto guard = AudioEngine::changesGuard();
		auto lock = Lock{m_lock};
		if (m_voice && !m_voice->isReleased()) { m_voice->noteOff(0); }
		m_voice = nullptr;
		m_voiceKey = -1;
		m_heldCount = 0;
	});
}


auto MonoMode::now() -> std::int64_t
{
	const auto* engine = Engine::audioEngine();
	return engine->periodsRendered() * static_cast<std::int64_t>(engine->framesPerPeriod());
}


auto MonoMode::glideFrames() const -> f_cnt_t
{
	return static_cast<f_cnt_t>(m_glideModel.value() / 1000.f * Engine::audioEngine()->outputSampleRate());
}


void MonoMode::noteOn(const Note& note, f_cnt_t offset, f_cnt_t frames, NotePlayHandle::Origin origin, int channel)
{
	auto held = Held{};
	held.key = note.key();
	held.volume = note.getVolume();
	held.panning = note.getPanning();
	held.live = frames == 0;
	held.end = held.live ? Never : now() + offset + frames;
	held.origin = origin;
	held.channel = channel;

	auto fromAnother = false;
	{
		auto lock = Lock{m_lock};
		// (more than can be held at once: the oldest goes)
		if (m_heldCount == MostHeld) { remove(0); }
		m_held[m_heldCount++] = held;
		fromAnother = m_voice != nullptr && !m_voice->isReleased();
	}
	play(held, offset, fromAnother);
}


void MonoMode::noteOff(int key, f_cnt_t offset)
{
	auto next = Held{};
	{
		auto lock = Lock{m_lock};
		// The last time it was pressed
		auto index = m_heldCount;
		for (auto i = m_heldCount; i-- > 0;)
		{
			if (m_held[i].live && m_held[i].key == key) { index = i; break; }
		}
		if (index == m_heldCount) { return; }
		const auto wasPlaying = index + 1 == m_heldCount;
		remove(index);
		// Another one plays: nothing changes
		if (!wasPlaying) { return; }
		if (m_heldCount == 0)
		{
			// (a key let go before its voice sounded still sounds: see NotePlayHandle::noteOffOnceStarted())
			if (m_voice && !m_voice->isReleased()) { m_voice->noteOffOnceStarted(offset); }
			m_voice = nullptr;
			m_voiceKey = -1;
			return;
		}
		// Back to the one held before
		next = m_held[m_heldCount - 1];
	}
	play(next, offset, true);
}


void MonoMode::advance(f_cnt_t offset)
{
	// (the notes end where a tick starts: half a tick on either side is the same tick, frames rounded either way)
	const auto until = now() + offset + static_cast<std::int64_t>(Engine::framesPerTick() / 2);
	while (true)
	{
		auto next = Held{};
		{
			auto lock = Lock{m_lock};
			auto index = m_heldCount;
			for (std::size_t i = 0; i < m_heldCount; ++i)
			{
				if (!m_held[i].live && m_held[i].end <= until) { index = i; break; }
			}
			if (index == m_heldCount) { return; }
			const auto wasPlaying = index + 1 == m_heldCount;
			remove(index);
			if (!wasPlaying) { continue; }
			if (m_heldCount == 0)
			{
				if (m_voice && !m_voice->isReleased()) { m_voice->noteOff(offset); }
				m_voice = nullptr;
				m_voiceKey = -1;
				return;
			}
			next = m_held[m_heldCount - 1];
		}
		play(next, offset, true);
	}
}


void MonoMode::play(const Held& held, f_cnt_t offset, bool fromAnother)
{
	auto glideFrom = 0.f;
	auto glide = false;
	{
		auto lock = Lock{m_lock};
		fromAnother = fromAnother && m_voice != nullptr && !m_voice->isReleased();
		if (fromAnother && m_legatoModel.value())
		{
			// The voice moves to the key, without starting again
			m_voice->slideTo(held.key, glideFrames(), offset);
			m_voiceKey = held.key;
			return;
		}
		if (fromAnother)
		{
			// The voice ends; the new one glides from where it was
			glide = glideFrames() > 0;
			glideFrom = m_voice->currentPitch() - static_cast<float>(held.key);
			m_voice->noteOff(offset);
		}
		m_voice = nullptr;
		m_voiceKey = -1;
	}

	// (outside the lock: a note that cannot be played is destroyed at once, and its destructor takes it)
	auto* voice = NotePlayHandleManager::acquire(m_track, offset, std::numeric_limits<f_cnt_t>::max() / 2,
		Note{TimePos{}, TimePos{}, held.key, held.volume, held.panning}, nullptr, held.channel, held.origin);
	if (glide) { voice->glideFrom(glideFrom, glideFrames()); }
	if (!Engine::audioEngine()->addPlayHandle(voice)) { return; }

	auto lock = Lock{m_lock};
	m_voice = voice;
	m_voiceKey = held.key;
}


void MonoMode::remove(std::size_t index)
{
	for (auto i = index; i + 1 < m_heldCount; ++i) { m_held[i] = m_held[i + 1]; }
	--m_heldCount;
}


void MonoMode::reset()
{
	auto lock = Lock{m_lock};
	m_heldCount = 0;
	m_voice = nullptr;
	m_voiceKey = -1;
}


void MonoMode::forget(const NotePlayHandle* note)
{
	auto lock = Lock{m_lock};
	if (m_voice == note)
	{
		m_voice = nullptr;
		m_voiceKey = -1;
	}
}


auto MonoMode::isHeld(int key) const -> bool
{
	auto lock = Lock{m_lock};
	for (std::size_t i = 0; i < m_heldCount; ++i)
	{
		if (m_held[i].live && m_held[i].key == key) { return true; }
	}
	return false;
}


void MonoMode::saveSettings(QDomDocument& doc, QDomElement& element)
{
	m_enabledModel.saveSettings(doc, element, "enabled");
	m_legatoModel.saveSettings(doc, element, "legato");
	m_glideModel.saveSettings(doc, element, "glide");
}


void MonoMode::loadDefaults()
{
	// (as a new track's: loading settings makes what they had the models' first values, which reset() goes back to)
	m_enabledModel.setInitValue(false);
	m_legatoModel.setInitValue(true);
	m_glideModel.setInitValue(0.f);
}


void MonoMode::loadSettings(const QDomElement& element)
{
	m_enabledModel.loadSettings(element, "enabled");
	m_legatoModel.loadSettings(element, "legato");
	m_glideModel.loadSettings(element, "glide");
}

} // namespace lmms
