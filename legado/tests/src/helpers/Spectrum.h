/*
 * Spectrum.h - measuring what a test renders: how much of it is aliasing
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

#ifndef LMMS_TESTS_SPECTRUM_H
#define LMMS_TESTS_SPECTRUM_H

#include <algorithm>
#include <cmath>
#include <complex>
#include <cstddef>
#include <numbers>
#include <vector>

namespace lmms::tests
{

/**
 * How much of @p samples (@p stride apart: 2 for the left channel of interleaved stereo) is not at the harmonics of
 * @p frequency: aliasing, harmonics above half the rate folded back down between them. @returns its power against
 * the harmonics', in dB. 2^15 samples from @p from, a 4 term Blackman-Harris window (its leakage is under -92 dB);
 * six bins round each harmonic count as it, and below 20 Hz as nothing.
 */
inline auto aliasing(const std::vector<float>& samples, std::size_t from, double frequency, double sampleRate,
	std::size_t stride = 1) -> double
{
	constexpr auto Size = std::size_t{1} << 15;
	if (samples.size() < stride * (from + Size)) { return 0; }
	auto data = std::vector<std::complex<double>>(Size);
	for (auto i = std::size_t{0}; i < Size; ++i)
	{
		const auto x = 2 * std::numbers::pi * static_cast<double>(i) / Size;
		const auto window = 0.35875 - 0.48829 * std::cos(x) + 0.14128 * std::cos(2 * x) - 0.01168 * std::cos(3 * x);
		data[i] = samples[stride * (from + i)] * window;
	}
	for (std::size_t i = 1, j = 0; i < Size; ++i)
	{
		auto bit = Size >> 1;
		for (; j & bit; bit >>= 1) { j ^= bit; }
		j ^= bit;
		if (i < j) { std::swap(data[i], data[j]); }
	}
	for (auto length = std::size_t{2}; length <= Size; length <<= 1)
	{
		for (auto k = std::size_t{0}; k < length / 2; ++k)
		{
			const auto w = std::polar(1.0, -2 * std::numbers::pi * static_cast<double>(k) / static_cast<double>(length));
			for (auto start = std::size_t{0}; start < Size; start += length)
			{
				const auto even = data[start + k];
				const auto odd = data[start + k + length / 2] * w;
				data[start + k] = even + odd;
				data[start + k + length / 2] = even - odd;
			}
		}
	}
	auto harmonic = 0.0;
	auto other = 0.0;
	const auto binWidth = sampleRate / Size;
	for (auto bin = std::size_t{1}; bin < Size / 2; ++bin)
	{
		const auto at = static_cast<double>(bin) * binWidth;
		if (at < 20) { continue; }
		const auto power = std::norm(data[bin]);
		const auto nearest = std::round(at / frequency) * frequency;
		(nearest > 0 && std::abs(at - nearest) <= 6 * binWidth ? harmonic : other) += power;
	}
	return 10 * std::log10(std::max(other, 1e-30) / std::max(harmonic, 1e-30));
}

} // namespace lmms::tests

#endif // LMMS_TESTS_SPECTRUM_H
