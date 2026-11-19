/*
 * DrawnWave.h - a wave drawn as points, played at any pitch without aliasing
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

#ifndef LMMS_DRAWN_WAVE_H
#define LMMS_DRAWN_WAVE_H

#include <cstddef>
#include <span>
#include <vector>

#include "lmms_export.h"

namespace lmms
{

/**
 * One period of a wave drawn as points (BitInvader's, Watsyn's), made into tables that play it at any pitch with
 * none of its harmonics above the cutoff: one above half the sample rate folds back down, between the note's own,
 * as a tone that has nothing to do with the note (aliasing, upstream's #6098).
 *
 * Its harmonics are those of the wave as it plays, worked out from the points: each held until the next (a
 * staircase), or joined to it by a straight line. There is a table for each count of harmonics, two semitones
 * apart; a note plays the one with the most harmonics that stay below the cutoff (20 kHz, or a little under half
 * the rate). Each has sixteen samples a harmonic at least (and 256 in all), read with a cubic: what that adds is
 * under -88 dB.
 *
 * Made on any thread but the audio one (it allocates; it makes no FFTW plan, whose planner is not thread safe); the
 * tables are only read after.
 */
class LMMS_EXPORT DrawnWave
{
public:
	//! How the points play between them
	enum class Hold
	{
		Step, //!< each held until the next one
		Linear //!< joined by straight lines (the last one to the first)
	};

	//! A period of the wave with @p harmonics harmonics, @p length samples, kept with the last sample before the
	//! first and the first two after the last (for interpolation)
	struct Table
	{
		std::size_t harmonics = 0;
		std::size_t length = 0;
		std::vector<float> samples; //!< length + 3
	};

	//! The most harmonics kept: all of them up to 20 kHz down to notes of about 20 Hz
	static constexpr std::size_t MostHarmonics = 1024;

	DrawnWave(std::span<const float> points, Hold hold, std::size_t mostHarmonics = MostHarmonics);

	//! The table for a note at @p frequency played at @p sampleRate; none when even its first harmonic is above the
	//! cutoff (the note is silent)
	auto tableFor(float frequency, float sampleRate) const -> const Table*;

	//! The wave in @p table at @p phase (a fraction of the period, from 0 to 1): between its samples, a cubic
	//! (Hermite, four of them), whose error is far under linear interpolation's (its images of the harmonics, folded)
	static auto at(const Table& table, float phase) -> float
	{
		const auto position = phase * static_cast<float>(table.length);
		auto index = position > 0 ? static_cast<std::size_t>(position) : std::size_t{0};
		if (index >= table.length) { index = table.length - 1; }
		const auto t = position - static_cast<float>(index);
		const auto* p = table.samples.data() + index; // p[1] is the sample at index
		const auto c1 = 0.5f * (p[2] - p[0]);
		const auto c2 = p[0] - 2.5f * p[1] + 2.f * p[2] - 0.5f * p[3];
		const auto c3 = 0.5f * (p[3] - p[0]) + 1.5f * (p[1] - p[2]);
		return ((c3 * t + c2) * t + c1) * t + p[1];
	}

	//! The highest frequency a note's harmonics reach, played at @p sampleRate
	static auto cutoff(float sampleRate) -> float;

	auto tables() const -> const std::vector<Table>& { return m_tables; }

private:
	std::vector<Table> m_tables; //!< from the most harmonics to the fewest
};

} // namespace lmms

#endif // LMMS_DRAWN_WAVE_H
