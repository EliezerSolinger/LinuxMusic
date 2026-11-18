/*
 * Metronome.cpp
 *
 * Copyright (c) 2024 saker
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


#include "Metronome.h"

#include "AudioBusHandle.h"
#include "AudioEngine.h"
#include "Engine.h"
#include "Sample.h"
#include "SampleBuffer.h"
#include "ConfigManager.h"
#include "PlayHandle.h"

#include <algorithm>
#include <cmath>

namespace lmms {

/**
 * Plays the metronome's clicks: made once, it lives until the metronome is released. The metronome tells it of a
 * click (on the audio thread, before the handles play), and it plays it from there; nothing is made per click.
 */
class MetronomeHandle : public PlayHandle
{
public:
	explicit MetronomeHandle(AudioBusHandle* bus)
		: PlayHandle{Type::MetronomeHandle}
	{
		setAudioBusHandle(bus);
	}

	//! Audio thread: @p click starts at @p offset in this period, at @p gain
	void click(const Sample* click, std::size_t offset, float gain)
	{
		m_pending = click;
		m_pendingOffset = offset;
		m_pendingGain = gain;
	}

	void play(std::span<SampleFrame> buffer) override
	{
		auto from = std::size_t{0};
		if (m_pending)
		{
			// The click before goes on until this one starts
			from = std::min(m_pendingOffset, buffer.size());
			render(buffer.subspan(0, from));
			m_playing = m_pending;
			m_gain = m_pendingGain;
			m_state.restart(0);
			m_pending = nullptr;
		}
		render(buffer.subspan(from));
	}

	bool isFinished() const override { return false; }
	bool isFromTrack(const Track*) const override { return false; }

private:
	void render(std::span<SampleFrame> frames)
	{
		if (!m_playing || frames.empty()) { return; }
		if (!m_playing->play(frames.data(), &m_state, frames.size())) { m_playing = nullptr; }
		for (auto& frame : frames) { frame *= m_gain; }
	}

	const Sample* m_pending = nullptr;
	std::size_t m_pendingOffset = 0;
	float m_pendingGain = 1.f;
	const Sample* m_playing = nullptr;
	float m_gain = 1.f;
	Sample::PlaybackState m_state;
};

Metronome::Metronome(AudioEngine& engine)
	: m_engine{engine}
{
}
Metronome::~Metronome() = default;




void Metronome::setActive(bool active)
{
	if (active) { prepare(); }
	m_active = active;
}




void Metronome::prepare()
{
	if (m_prepared) { return; }
	m_strongClick = std::make_unique<Sample>(SampleBuffer::fromFile("misc/metronome02.ogg"));
	m_weakClick = std::make_unique<Sample>(SampleBuffer::fromFile("misc/metronome01.ogg"));
	m_bus = std::make_unique<AudioBusHandle>("Metronome", false);
	m_handle = new MetronomeHandle{m_bus.get()};
	m_engine.addPlayHandle(m_handle);
	loadSettings();
	m_prepared = true;
}




void Metronome::loadSettings()
{
	const auto* config = ConfigManager::inst();
	m_onlyRecording = config->value("metronome", "onlyrecording", "0").toInt() != 0;
	m_accent = config->value("metronome", "accent", "1").toInt() != 0;
	const auto volume = config->value("metronome", "volume", QString::number(DefaultVolumeDb)).toDouble();
	m_gain = static_cast<float>(std::pow(10.0, std::clamp(std::isfinite(volume) ? volume : 0.0, -60.0, 12.0) / 20.0));
}




void Metronome::release()
{
	m_prepared = false;
	if (m_handle)
	{
		// (the engine deletes it)
		m_engine.removePlayHandle(m_handle);
		m_handle = nullptr;
	}
	m_bus.reset();
	m_strongClick.reset();
	m_weakClick.reset();
}




void Metronome::processTick(int tickInBar, int ticksPerBeat, size_t bufferOffset, bool force, bool recording)
{
	const bool clicking = force || (m_active && (!m_onlyRecording || recording));
	if (ticksPerBeat <= 0 || tickInBar % ticksPerBeat != 0 || !clicking || !m_prepared || !m_handle) { return; }

	// The handle plays the clicks prepared on the main thread, on the metronome's own bus
	const bool firstBeat = tickInBar == 0 && m_accent;
	m_handle->click(firstBeat ? m_strongClick.get() : m_weakClick.get(), bufferOffset, m_gain);
}
} // namespace lmms
