/*
 * Convolver.cpp - uniformly partitioned FFT convolution, for impulse responses
 *
 * This file is part of LinuxMusic, a fork of LMMS - https://lmms.io
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

#include "Convolver.h"

#include <algorithm>
#include <mutex>

namespace lmms
{

namespace
{

constexpr int FftSize = 2 * Convolver::BlockSize;
constexpr int Bins = FftSize / 2 + 1;

//! FFTW's planner is not thread safe
std::mutex& planMutex()
{
	static auto mutex = std::mutex{};
	return mutex;
}

} // namespace


Convolver::Convolver(const std::vector<float>& impulseResponse) :
	m_timeIn(FftSize, 0.f),
	m_timeOut(FftSize, 0.f),
	m_accumulator(Bins),
	m_inBlock(BlockSize, 0.f),
	m_outBlock(BlockSize, 0.f)
{
	m_fftTime = fftwf_alloc_real(FftSize);
	m_fftFrequency = fftwf_alloc_complex(Bins);
	{
		const auto lock = std::scoped_lock{planMutex()};
		m_forward = fftwf_plan_dft_r2c_1d(FftSize, m_fftTime, m_fftFrequency, FFTW_ESTIMATE);
		m_inverse = fftwf_plan_dft_c2r_1d(FftSize, m_fftFrequency, m_fftTime, FFTW_ESTIMATE);
	}

	// Each partition of the IR, zero-padded to the FFT's size, in the frequency domain
	const auto length = std::max<std::size_t>(impulseResponse.size(), 1);
	const auto count = (length + BlockSize - 1) / BlockSize;
	for (std::size_t p = 0; p < count; ++p)
	{
		std::fill_n(m_fftTime, FftSize, 0.f);
		for (int i = 0; i < BlockSize && p * BlockSize + i < impulseResponse.size(); ++i)
		{
			m_fftTime[i] = impulseResponse[p * BlockSize + i];
		}
		fftwf_execute(m_forward);
		auto spectrum = Spectrum(Bins);
		for (int k = 0; k < Bins; ++k) { spectrum[k] = {m_fftFrequency[k][0], m_fftFrequency[k][1]}; }
		m_irSpectra.push_back(std::move(spectrum));
	}
	m_inputSpectra.assign(m_irSpectra.size(), Spectrum(Bins));
}


Convolver::~Convolver()
{
	const auto lock = std::scoped_lock{planMutex()};
	fftwf_destroy_plan(m_forward);
	fftwf_destroy_plan(m_inverse);
	fftwf_free(m_fftTime);
	fftwf_free(m_fftFrequency);
}


void Convolver::process(float* io, int frames)
{
	for (int f = 0; f < frames; ++f)
	{
		// In, and out what the last block made (a block's latency)
		m_inBlock[m_position] = io[f];
		io[f] = m_outBlock[m_position];
		if (++m_position == BlockSize)
		{
			processBlock();
			m_position = 0;
		}
	}
}


void Convolver::reset()
{
	for (auto& spectrum : m_inputSpectra) { std::fill(spectrum.begin(), spectrum.end(), std::complex<float>{}); }
	std::fill(m_timeIn.begin(), m_timeIn.end(), 0.f);
	std::fill(m_timeOut.begin(), m_timeOut.end(), 0.f);
	std::fill(m_inBlock.begin(), m_inBlock.end(), 0.f);
	std::fill(m_outBlock.begin(), m_outBlock.end(), 0.f);
	m_position = 0;
}


void Convolver::processBlock()
{
	// Overlap-save: the last two blocks of input, transformed
	std::copy(m_timeIn.begin() + BlockSize, m_timeIn.end(), m_timeIn.begin());
	std::copy(m_inBlock.begin(), m_inBlock.end(), m_timeIn.begin() + BlockSize);
	std::copy(m_timeIn.begin(), m_timeIn.end(), m_fftTime);
	fftwf_execute(m_forward);
	m_newest = (m_newest + 1) % static_cast<int>(m_inputSpectra.size());
	auto& newest = m_inputSpectra[m_newest];
	for (int k = 0; k < Bins; ++k) { newest[k] = {m_fftFrequency[k][0], m_fftFrequency[k][1]}; }

	// Each partition with the input of as many blocks before
	std::fill(m_accumulator.begin(), m_accumulator.end(), std::complex<float>{});
	const auto count = static_cast<int>(m_irSpectra.size());
	for (int p = 0; p < count; ++p)
	{
		const auto& input = m_inputSpectra[(m_newest - p + count) % count];
		const auto& ir = m_irSpectra[p];
		for (int k = 0; k < Bins; ++k) { m_accumulator[k] += input[k] * ir[k]; }
	}

	// Back in time: the second half is this block's output (the first is the circular wrap)
	for (int k = 0; k < Bins; ++k)
	{
		m_fftFrequency[k][0] = m_accumulator[k].real();
		m_fftFrequency[k][1] = m_accumulator[k].imag();
	}
	fftwf_execute(m_inverse);
	const auto scale = 1.f / FftSize;
	for (int i = 0; i < BlockSize; ++i) { m_outBlock[i] = m_fftTime[BlockSize + i] * scale; }
}

} // namespace lmms
