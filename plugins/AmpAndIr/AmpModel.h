/*
 * AmpModel.h - a Neural Amp Modeler model, at the engine's rate
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

#ifndef LMMS_AMP_MODEL_H
#define LMMS_AMP_MODEL_H

#include <memory>
#include <vector>

#include <QString>

typedef struct SRC_STATE_tag SRC_STATE;

namespace nam
{
class DSP;
}

namespace lmms
{

/**
 * An amplifier (or pedal) captured by Neural Amp Modeler (a .nam file), mono. A model runs at the rate it was trained
 * at: at another engine rate the audio is resampled to it and back (libsamplerate, streaming), through buffers made
 * when it loads, with a fixed latency (measured then, by an impulse through the same converters). A model that
 * gives its loudness is brought to -18 dB, as NAM's own plugin does, so that models can be swapped at about the
 * same level.
 *
 * Loaded off the audio thread; process() neither allocates nor locks.
 */
class AmpModel
{
public:
	static constexpr int MaxFrames = 4096; //!< the most process() is given at once

	//! Loads @p path for the engine's @p rate; @p error says why if it returns nothing
	static auto load(const QString& path, double rate, QString& error) -> std::unique_ptr<AmpModel>;
	~AmpModel();
	AmpModel(const AmpModel&) = delete;
	AmpModel& operator=(const AmpModel&) = delete;

	//! @p frames (up to MaxFrames) of mono audio, in place
	void process(float* io, int frames);

	//! Frames the output comes after the input
	auto latency() const -> int { return m_latency; }
	auto modelRate() const -> double { return m_modelRate; }

private:
	AmpModel() = default;
	void runModel(float* in, float* out, int frames);

	std::unique_ptr<nam::DSP> m_dsp;
	double m_engineRate = 0.0;
	double m_modelRate = 0.0;
	bool m_resampling = false;
	float m_gain = 1.f; //!< the loudness normalization
	int m_latency = 0;

	// Resampling: up to the model's rate, the model, back down, into a FIFO of the engine's rate
	SRC_STATE* m_up = nullptr;
	SRC_STATE* m_down = nullptr;
	std::vector<float> m_atModelRate;
	std::vector<float> m_modelOutput;
	std::vector<float> m_downOutput;
	std::vector<float> m_fifo;
	std::size_t m_fifoRead = 0;
	std::size_t m_fifoFill = 0;
};

} // namespace lmms

#endif // LMMS_AMP_MODEL_H
