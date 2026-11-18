/*
 * AmpLiteDsp.cpp - the amp of Amplifikation Lite, as measured (black box), at 48 kHz
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

#include "AmpLiteDsp.h"

#include <algorithm>
#include <cmath>

#include "../AmpAndIr/Convolver.h"
#include "AmpLiteData.h"

namespace lmms::amplite
{

namespace
{

constexpr double K = 2.0 * ModelRate; //!< the bilinear transform's constant (no prewarping, as the original)

//! H(s) = (n1 s + n0) / (d1 s + d0) -> a first-order section
auto firstOrder(double n1, double n0, double d1, double d0) -> Biquad
{
	auto q = Biquad{};
	const double norm = d1 * K + d0;
	q.b0 = (n1 * K + n0) / norm;
	q.b1 = (n0 - n1 * K) / norm;
	q.a1 = (d0 - d1 * K) / norm;
	return q;
}

auto highpass(double hz) -> Biquad { return firstOrder(1, 0, 1, 2 * M_PI * hz); }
auto lowpass(double hz) -> Biquad { return firstOrder(0, 1, 1 / (2 * M_PI * hz), 1); }
//! (1 + s/zero) / (1 + s/pole)
auto shelf(double zeroHz, double poleHz) -> Biquad
{
	return firstOrder(1 / (2 * M_PI * zeroHz), 1, 1 / (2 * M_PI * poleHz), 1);
}

//! Polynomial (in z^-1) product, degrees up to 3
void polyMul(const double* p, int np, const double* q, int nq, double* out)
{
	for (int i = 0; i < np + nq - 1; ++i) { out[i] = 0; }
	for (int i = 0; i < np; ++i)
	{
		for (int j = 0; j < nq; ++j) { out[i + j] += p[i] * q[j]; }
	}
}

//! The 3rd-order analog section b(s)/a(s) through the bilinear transform
void bilinear3(const double b[4], const double a[4], Cubic& c)
{
	// s^k (1+q)^3 -> K^k (1-q)^k (1+q)^(3-k), q = z^-1
	double basis[4][4];
	const double minus[2] = {1, -1}, plus[2] = {1, 1};
	for (int k = 0; k < 4; ++k)
	{
		double acc[4] = {1, 0, 0, 0};
		int n = 1;
		for (int i = 0; i < 3; ++i)
		{
			double next[4];
			polyMul(acc, n, i < k ? minus : plus, 2, next);
			++n;
			for (int j = 0; j < n; ++j) { acc[j] = next[j]; }
		}
		const double scale = std::pow(K, k);
		for (int j = 0; j < 4; ++j) { basis[k][j] = acc[j] * scale; }
	}
	double B[4] = {0, 0, 0, 0}, A[4] = {0, 0, 0, 0};
	for (int k = 0; k < 4; ++k)
	{
		for (int j = 0; j < 4; ++j)
		{
			B[j] += b[k] * basis[k][j];
			A[j] += a[k] * basis[k][j];
		}
	}
	for (int j = 0; j < 4; ++j)
	{
		c.b[j] = B[j] / A[0];
		c.a[j] = A[j] / A[0];
	}
}

//! The stage's curve: its table, linearly interpolated, held beyond its range
auto shape(const float* table, double u) -> double
{
#ifdef AMPLITE_LINEAR_TEST
	(void)table;
	return u;
#endif
	// the table is dense near zero (where the curve bends most): uniform in asinh(u / U0)
	const double x = (std::asinh(u / NlLutU0) + NlLutW) / (2 * NlLutW) * (NlLutSize - 1);
	if (x <= 0) { return table[0]; }
	if (x >= NlLutSize - 1) { return table[NlLutSize - 1]; }
	const int i = static_cast<int>(x);
	const double frac = x - i;
	return table[i] + (table[i + 1] - table[i]) * frac;
}

struct ModeData
{
	double inputRef, preHp;
	const double* preX;
	double k1, mid, eHp, eLp;
	const float* nl1;
	const float* nl2;
	const float* midFir;
	int midFirSize;
	const float* postFir;
	int postFirSize;
	const double* gainTable;
	const double* transformer; //!< flux lowpass (Hz), saturation, amount
};

#define AMPLITE_MODE(n) ModeData{Mode##n##InputRef, Mode##n##PreHp, Mode##n##PreX, Mode##n##K1, Mode##n##Mid, \
	Mode##n##EHp, Mode##n##ELp, Mode##n##Nl1, Mode##n##Nl2, Mode##n##MidFir, \
	static_cast<int>(std::size(Mode##n##MidFir)), Mode##n##PostFir, static_cast<int>(std::size(Mode##n##PostFir)), \
	GainTable##n, Mode##n##Transformer}

const ModeData Modes[3] = {AMPLITE_MODE(0), AMPLITE_MODE(1), AMPLITE_MODE(2)};

#undef AMPLITE_MODE

constexpr int OversampleTaps = static_cast<int>(std::size(OversampleFir));

} // namespace


Oversampler::Oversampler() :
	m_upHistory(2 * (OversampleTaps / Oversample), 0.0),
	m_downHistory(2 * OversampleTaps, 0.0),
	m_taps(OversampleTaps)
{
}


void Oversampler::up(double x, double* out)
{
	// newest first: shift (the history is short: taps / Oversample)
	const int n = m_taps / Oversample;
	std::move_backward(m_upHistory.begin(), m_upHistory.begin() + n - 1, m_upHistory.begin() + n);
	m_upHistory[0] = x;
	for (int phase = 0; phase < Oversample; ++phase)
	{
		double acc = 0;
		for (int j = 0; j < n; ++j) { acc += OversampleFir[j * Oversample + phase] * m_upHistory[j]; }
		out[phase] = acc;
	}
}


auto Oversampler::down(const double* in) -> double
{
	std::move_backward(m_downHistory.begin(), m_downHistory.begin() + m_taps - Oversample,
		m_downHistory.begin() + m_taps);
	for (int i = 0; i < Oversample; ++i) { m_downHistory[Oversample - 1 - i] = in[i]; }
	double acc = 0;
	for (int i = 0; i < m_taps; ++i) { acc += OversampleFir[i] * m_downHistory[i]; }
	return acc / Oversample;
}


void Oversampler::reset()
{
	std::fill(m_upHistory.begin(), m_upHistory.end(), 0.0);
	std::fill(m_downHistory.begin(), m_downHistory.end(), 0.0);
}


AmpLiteDsp::AmpLiteDsp() :
	m_midHistory(512, 0.0),
	m_block(8192)
{
	for (int mode = 0; mode < 3; ++mode)
	{
		const auto& data = Modes[mode];
		m_post[mode] = std::make_unique<Convolver>(std::vector<float>(data.postFir, data.postFir + data.postFirSize));
	}
	set(m_settings);
}


AmpLiteDsp::~AmpLiteDsp() = default;


auto AmpLiteDsp::latency() -> int
{
	// the response FIR's own lead, its partitions, the two oversamplers' (linear-phase) filters
	return PostLatency + MidLatency + Convolver::BlockSize + OversampleTaps / Oversample;
}


void AmpLiteDsp::setMode(int channel)
{
	m_mode = std::clamp(channel, 0, 2);
	const auto& data = Modes[m_mode];
	m_k1 = data.k1;
	m_mid = data.mid;
	m_nl1 = data.nl1;
	m_nl2 = data.nl2;
	m_midFir = data.midFir;
	m_midFirSize = data.midFirSize;
	// the output transformer: the flux (a leaky integral of stage 2's input) saturating, at the oversampled rate
	m_fluxAlpha = 1.0 - std::exp(-2 * M_PI * data.transformer[0] / (ModelRate * Oversample));
	m_fluxSaturation = data.transformer[1];
	m_fluxAmount = data.transformer[2];
	auto keep = [](Biquad& target, const Biquad& next) {
		const double z1 = target.z1, z2 = target.z2;
		target = next;
		target.z1 = z1;
		target.z2 = z2;
	};
	keep(m_preHp, highpass(data.preHp));
	keep(m_preShelf, shelf(data.preX[0], data.preX[1]));
	keep(m_preLp, lowpass(data.preX[2]));
	keep(m_eHp, highpass(data.eHp));
	keep(m_eLp, lowpass(data.eLp));
}


void AmpLiteDsp::set(const Settings& s)
{
	if (s.channel != m_mode) { setMode(s.channel); }
	const auto& data = Modes[m_mode];
	m_inputScale = s.input / data.inputRef;

	// the gain knob's filter: the table of its biquads (relative to gain 1), interpolated
	{
		const double position = std::clamp<double>(s.gain, 0, 1) * (GainSteps - 1);
		const int i = std::min(static_cast<int>(position), GainSteps - 2);
		const double frac = position - i;
		const double* lo = data.gainTable + i * 5;
		const double* hi = lo + 5;
		auto mix = [frac](double a, double b) { return a + (b - a) * frac; };
		m_gainFilter.b0 = mix(lo[0], hi[0]);
		m_gainFilter.b1 = mix(lo[1], hi[1]);
		m_gainFilter.b2 = mix(lo[2], hi[2]);
		m_gainFilter.a1 = mix(lo[3], hi[3]);
		m_gainFilter.a2 = mix(lo[4], hi[4]);
	}

	// the tone stack (bass: its pot's resistance goes with the square of the knob)
	double b[4], a[4];
	const double bass = std::clamp<double>(s.bass, 0, 1);
	toneStackAnalog(std::clamp<double>(s.treble, 0, 1), std::clamp<double>(s.middle, 0, 1), bass * bass, b, a);
	if (a[0] == 0 && a[1] == 0) { a[0] = 1; }
	bilinear3(b, a, m_toneStack);

	// presence: the table of P(v)/P(1), interpolated
	const double position = std::clamp<double>(s.presence, 0, 1) * (PresenceSteps - 1);
	const int i = std::min(static_cast<int>(position), PresenceSteps - 2);
	const double frac = position - i;
	double row[6];
	for (int k = 0; k < 6; ++k)
	{
		row[k] = PresenceTable[i * 6 + k] * (1 - frac) + PresenceTable[(i + 1) * 6 + k] * frac;
	}
	m_presence.b0 = row[0] / row[3];
	m_presence.b1 = row[1] / row[3];
	m_presence.b2 = row[2] / row[3];
	m_presence.a1 = row[4] / row[3];
	m_presence.a2 = row[5] / row[3];

	// the model was measured at volume 0.5
	m_volume = s.volume / 0.5;
	m_settings = s;
}


void AmpLiteDsp::reset()
{
	for (auto* q : {&m_gainFilter, &m_preHp, &m_preShelf, &m_preLp, &m_eHp, &m_eLp, &m_presence}) { q->reset(); }
	m_toneStack.reset();
	m_os1.reset();
	m_os2.reset();
	std::fill(m_midHistory.begin(), m_midHistory.end(), 0.0);
	m_flux = 0;
	for (auto& post : m_post) { post->reset(); }
}


void AmpLiteDsp::process(float* io, int frames)
{
	double up[Oversample];
	const int midSize = m_midFirSize;
	for (int f = 0; f < frames; ++f)
	{
		// the input, the gain knob's filter, the input shaping
		double x = io[f] * m_inputScale;
		x = m_gainFilter.process(x);
		x = m_preLp.process(m_preShelf.process(m_preHp.process(x)));
		x *= m_k1;

		// stage 1
		m_os1.up(x, up);
		for (double& v : up) { v = shape(m_nl1, v); }
		double y = m_os1.down(up);

		// the drive into the tone stack, the tone stack, coupling and Miller filters, the decimation filter
		y = m_toneStack.process(y * m_mid);
		y = m_eLp.process(m_eHp.process(y));
		m_midPos = (m_midPos + m_midHistory.size() / 2 - 1) % (m_midHistory.size() / 2);
		m_midHistory[m_midPos] = m_midHistory[m_midPos + m_midHistory.size() / 2] = y;
		double acc = 0;
		for (int k = 0; k < midSize; ++k) { acc += m_midFir[k] * m_midHistory[m_midPos + k]; }
		y = acc;

		// stage 2, with the output transformer's flux
		m_os2.up(y, up);
		for (double& v : up)
		{
			m_flux += m_fluxAlpha * (v - m_flux);
			const double saturated = m_flux - m_fluxSaturation * std::tanh(m_flux / m_fluxSaturation);
			v = shape(m_nl2, v) - m_fluxAmount * saturated;
		}
		io[f] = static_cast<float>(m_os2.down(up));
	}

	// the power amp's response (FIR, partitioned), presence, volume
#if !defined(AMPLITE_LINEAR_TEST) || defined(AMPLITE_WITH_POST)
	m_post[m_mode]->process(io, frames);
#endif
	for (int f = 0; f < frames; ++f)
	{
		io[f] = static_cast<float>(m_presence.process(io[f]) * m_volume);
	}
}

} // namespace lmms::amplite
