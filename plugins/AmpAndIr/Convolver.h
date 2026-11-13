/*
 * Convolver.h - uniformly partitioned FFT convolution, for impulse responses
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

#ifndef LMMS_CONVOLVER_H
#define LMMS_CONVOLVER_H

#include <complex>
#include <vector>

#include <fftw3.h>

namespace lmms
{

/**
 * Convolves one channel with an impulse response, by uniformly partitioned overlap-save (the IR cut in blocks of
 * BlockSize, each block's spectrum multiplied with the input's spectra of as many blocks before, summed in the
 * frequency domain). Its latency is BlockSize frames. Made (with its FFT plans) off the audio thread; process()
 * neither allocates nor locks.
 */
class Convolver
{
public:
	static constexpr int BlockSize = 256;

	explicit Convolver(const std::vector<float>& impulseResponse);
	~Convolver();
	Convolver(const Convolver&) = delete;
	Convolver& operator=(const Convolver&) = delete;

	//! Convolves @p frames samples of @p io in place (any number: they are gathered in blocks)
	void process(float* io, int frames);

	auto partitions() const -> int { return static_cast<int>(m_irSpectra.size()); }

private:
	void processBlock();

	using Spectrum = std::vector<std::complex<float>>;
	std::vector<Spectrum> m_irSpectra;		//!< each partition's spectrum
	std::vector<Spectrum> m_inputSpectra;	//!< the spectra of the input's last blocks, a ring
	int m_newest = 0;						//!< the ring's newest

	std::vector<float> m_timeIn;			//!< the last two blocks of input (overlap-save)
	std::vector<float> m_timeOut;
	Spectrum m_accumulator;
	std::vector<float> m_inBlock;			//!< the block being gathered
	std::vector<float> m_outBlock;			//!< the last block's output, being handed out
	int m_position = 0;

	fftwf_plan m_forward = nullptr;
	fftwf_plan m_inverse = nullptr;
	float* m_fftTime = nullptr;
	fftwf_complex* m_fftFrequency = nullptr;
};

} // namespace lmms

#endif // LMMS_CONVOLVER_H
