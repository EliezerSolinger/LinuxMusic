/*
 * DrawnWave.cpp - a wave drawn as points, played at any pitch without aliasing
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

#include "DrawnWave.h"

#include <algorithm>
#include <cmath>
#include <complex>
#include <numbers>

namespace lmms
{

namespace
{

using Complex = std::complex<double>;

//! In place, @p data a power of two long: data[m] becomes the sum of data[k] e^(2 pi i k m / size) (not scaled)
void inverseFft(std::vector<Complex>& data)
{
	const auto size = data.size();
	for (std::size_t i = 1, j = 0; i < size; ++i)
	{
		auto bit = size >> 1;
		for (; j & bit; bit >>= 1) { j ^= bit; }
		j ^= bit;
		if (i < j) { std::swap(data[i], data[j]); }
	}
	for (std::size_t length = 2; length <= size; length <<= 1)
	{
		const auto half = length / 2;
		for (std::size_t k = 0; k < half; ++k)
		{
			const auto w = std::polar(1.0, 2 * std::numbers::pi * static_cast<double>(k) / static_cast<double>(length));
			for (std::size_t start = 0; start < size; start += length)
			{
				const auto even = data[start + k];
				const auto odd = data[start + k + half] * w;
				data[start + k] = even + odd;
				data[start + k + half] = even - odd;
			}
		}
	}
}

auto powerOfTwoAtLeast(std::size_t value) -> std::size_t
{
	auto power = std::size_t{1};
	while (power < value) { power <<= 1; }
	return power;
}

} // namespace


DrawnWave::DrawnWave(std::span<const float> points, Hold hold, std::size_t mostHarmonics)
{
	const auto count = points.size();
	if (count == 0) { return; }

	// The points' discrete Fourier transform, X[k] (it repeats every count harmonics): only the residues needed
	const auto residues = std::min(count, mostHarmonics + 1);
	auto twiddles = std::vector<Complex>(count);
	for (std::size_t m = 0; m < count; ++m)
	{
		twiddles[m] = std::polar(1.0, -2 * std::numbers::pi * static_cast<double>(m) / static_cast<double>(count));
	}
	auto transform = std::vector<Complex>(residues);
	for (std::size_t k = 0; k < residues; ++k)
	{
		auto sum = Complex{};
		auto index = std::size_t{0}; // (j k) mod count
		for (std::size_t j = 0; j < count; ++j)
		{
			sum += static_cast<double>(points[j]) * twiddles[index];
			index += k;
			if (index >= count) { index -= count; }
		}
		transform[k] = sum;
	}

	// The harmonics of the wave as it plays: the points held (a staircase: each point's step contributes
	// e^(-i pi k/n) sinc(k/n)) or joined by lines (each point's triangle: sinc^2(k/n)), k the harmonic, n the points
	auto coefficients = std::vector<Complex>(mostHarmonics + 1);
	auto largest = 0.0;
	for (std::size_t k = 0; k <= mostHarmonics; ++k)
	{
		const auto x = static_cast<double>(k) / static_cast<double>(count);
		const auto sinc = k == 0 ? 1.0 : std::sin(std::numbers::pi * x) / (std::numbers::pi * x);
		const auto shape = hold == Hold::Step ? sinc * std::polar(1.0, -std::numbers::pi * x) : Complex{sinc * sinc};
		coefficients[k] = transform[k % count] / static_cast<double>(count) * shape;
		if (k > 0) { largest = std::max(largest, std::abs(coefficients[k])); }
	}
	// Down to the last harmonic that counts (a smooth wave has few)
	auto harmonics = mostHarmonics;
	while (harmonics > 1 && std::abs(coefficients[harmonics]) <= largest * 1e-6) { --harmonics; }

	// A table for each count of harmonics, two semitones apart, down to one
	const auto ratio = std::exp2(-2.0 / 12.0);
	while (true)
	{
		const auto length = std::max<std::size_t>(256, powerOfTwoAtLeast(16 * harmonics));
		auto spectrum = std::vector<Complex>(length);
		spectrum[0] = Complex{coefficients[0].real()};
		for (std::size_t k = 1; k <= harmonics; ++k)
		{
			spectrum[k] = coefficients[k];
			spectrum[length - k] = std::conj(coefficients[k]);
		}
		inverseFft(spectrum);
		auto table = Table{harmonics, length, std::vector<float>(length + 3)};
		for (std::size_t m = 0; m < length; ++m) { table.samples[m + 1] = static_cast<float>(spectrum[m].real()); }
		table.samples[0] = table.samples[length];
		table.samples[length + 1] = table.samples[1];
		table.samples[length + 2] = table.samples[2];
		m_tables.push_back(std::move(table));

		if (harmonics == 1) { break; }
		const auto fewer = static_cast<std::size_t>(std::floor(static_cast<double>(harmonics) * ratio));
		harmonics = std::clamp<std::size_t>(fewer, 1, harmonics - 1);
	}
}


auto DrawnWave::cutoff(float sampleRate) -> float
{
	return std::min(20000.f, 0.475f * sampleRate);
}


auto DrawnWave::tableFor(float frequency, float sampleRate) const -> const Table*
{
	if (m_tables.empty()) { return nullptr; }
	if (!(frequency > 0)) { return &m_tables.front(); }
	const auto allowed = std::floor(cutoff(sampleRate) / frequency);
	if (allowed < 1) { return nullptr; }
	// The first (the most harmonics) within it
	const auto found = std::find_if(m_tables.begin(), m_tables.end(),
		[allowed](const Table& table) { return static_cast<float>(table.harmonics) <= allowed; });
	return found != m_tables.end() ? &*found : &m_tables.back();
}

} // namespace lmms
