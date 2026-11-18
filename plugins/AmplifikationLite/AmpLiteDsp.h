/*
 * AmpLiteDsp.h - the amp of Amplifikation Lite, as measured (black box), at 48 kHz
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

#ifndef LMMS_AMP_LITE_DSP_H
#define LMMS_AMP_LITE_DSP_H

#include <array>
#include <memory>
#include <vector>

namespace lmms
{

class Convolver;

namespace amplite
{

//! A biquad, transposed direct form II, in double
struct Biquad
{
	double b0 = 1, b1 = 0, b2 = 0, a1 = 0, a2 = 0;
	double z1 = 0, z2 = 0;
	auto process(double x) -> double
	{
		const double y = b0 * x + z1;
		z1 = b1 * x - a1 * y + z2;
		z2 = b2 * x - a2 * y;
		return y;
	}
	void reset() { z1 = z2 = 0; }
};

//! The tone stack's 3rd-order section (bilinear transform of the analog circuit)
struct Cubic
{
	double b[4] = {1, 0, 0, 0}, a[4] = {1, 0, 0, 0};
	double z[3] = {0, 0, 0};
	auto process(double x) -> double
	{
		const double y = b[0] * x + z[0];
		z[0] = b[1] * x - a[1] * y + z[1];
		z[1] = b[2] * x - a[2] * y + z[2];
		z[2] = b[3] * x - a[3] * y;
		return y;
	}
	void reset() { z[0] = z[1] = z[2] = 0; }
};

//! Polyphase oversampler: up, then down, by the same prototype
class Oversampler
{
public:
	Oversampler();
	//! One input sample to Oversample samples
	void up(double x, double* out);
	//! Oversample samples to one output sample
	auto down(const double* in) -> double;
	void reset();
private:
	std::vector<double> m_upHistory;	//!< input samples, newest first
	std::vector<double> m_downHistory;	//!< oversampled samples, newest first
	int m_taps;
};

/**
 * The measured model (see AmpLiteData.h): input gain -> gain filter (the knob's) -> the input highpass and
 * shaping -> stage 1 (oversampled table) -> drive -> tone stack -> coupling, Miller and decimation filters ->
 * stage 2 (oversampled table) -> the power amp's response (FIR, partitioned) -> presence -> volume.
 * Mono, at 48 kHz; set() off the audio thread is not needed: it only computes coefficients.
 */
class AmpLiteDsp
{
public:
	AmpLiteDsp();
	~AmpLiteDsp();

	struct Settings
	{
		int channel = 0;	//!< 0 clean, 1 rhythm, 2 lead
		float input = 0.2f, gain = 0.5f, bass = 0.5f, middle = 0.5f, treble = 0.5f, presence = 0.5f, volume = 0.5f;
	};

	//! [audio thread] New knob values (cheap: coefficients only)
	void set(const Settings& settings);
	//! [audio thread] @p frames samples of mono audio at 48 kHz, in place
	void process(float* io, int frames);
	void reset();

	//! Samples the output comes after the input (the response's FIR and its partitions)
	static auto latency() -> int;

private:
	void setMode(int channel);

	Settings m_settings;
	int m_mode = -1;

	// coefficients of the current mode
	double m_inputScale = 1, m_k1 = 1, m_mid = 1;
	const float* m_nl1 = nullptr;
	const float* m_nl2 = nullptr;
	const float* m_midFir = nullptr;
	int m_midFirSize = 0;
	double m_fluxAlpha = 0, m_fluxSaturation = 1, m_fluxAmount = 0, m_flux = 0;

	Biquad m_gainFilter;
	Biquad m_preHp, m_preShelf, m_preLp;
	Cubic m_toneStack;
	Biquad m_eHp, m_eLp;
	Biquad m_presence;
	double m_volume = 1;

	Oversampler m_os1, m_os2;
	std::vector<double> m_midHistory; //!< (twice the between-stages FIR's length: a mirrored ring)
	int m_midPos = 0;

	std::array<std::unique_ptr<Convolver>, 3> m_post; //!< the power amp's response, per mode
	std::vector<float> m_block;
};

} // namespace amplite

} // namespace lmms

#endif // LMMS_AMP_LITE_DSP_H
