/*
 * AmpModel.cpp - a Neural Amp Modeler model, at the engine's rate
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

#include "AmpModel.h"

#include <algorithm>
#include <cmath>
#include <filesystem>

#include <QCoreApplication>
#include <QFile>
#include <samplerate.h>

#include "NAM/dsp.h"
#include "NAM/get_dsp.h"

namespace lmms
{

namespace
{

constexpr auto Converter = SRC_SINC_MEDIUM_QUALITY;
constexpr double TargetLoudness = -18.0; // dB, as NAM's plugin

auto tr(const char* text) -> QString { return QCoreApplication::translate("AmpModel", text); }

//! Runs @p frames frames through @p state at @p ratio; @returns how many came out (all input consumed)
auto convert(SRC_STATE* state, double ratio, const float* in, long frames, float* out, long room) -> long
{
	auto data = SRC_DATA{};
	data.data_in = in;
	data.input_frames = frames;
	data.data_out = out;
	data.output_frames = room;
	data.src_ratio = ratio;
	data.end_of_input = 0;
	auto produced = 0L;
	while (data.input_frames > 0 || produced == 0)
	{
		if (src_process(state, &data) != 0) { break; }
		produced += data.output_frames_gen;
		data.data_in += data.input_frames_used;
		data.input_frames -= data.input_frames_used;
		data.data_out += data.output_frames_gen;
		data.output_frames -= data.output_frames_gen;
		if (data.input_frames_used == 0 && data.output_frames_gen == 0) { break; }
	}
	return produced;
}

//! The delay (in output frames) of a converter of this kind at @p ratio: where an impulse comes out
auto converterDelay(double ratio) -> double
{
	auto error = 0;
	auto* state = src_new(Converter, 1, &error);
	if (!state) { return 0.0; }
	const auto inFrames = 4096L;
	auto in = std::vector<float>(inFrames, 0.f);
	in[0] = 1.f;
	auto out = std::vector<float>(static_cast<std::size_t>(inFrames * ratio + 256), 0.f);
	const auto produced = convert(state, ratio, in.data(), inFrames, out.data(), static_cast<long>(out.size()));
	src_delete(state);
	// The peak, refined between its neighbours (a parabola)
	const auto peak = std::max_element(out.begin(), out.begin() + produced, [](float a, float b) {
		return std::abs(a) < std::abs(b);
	}) - out.begin();
	if (peak <= 0 || peak + 1 >= produced) { return static_cast<double>(peak); }
	const double a = out[peak - 1], b = out[peak], c = out[peak + 1];
	const double denominator = a - 2 * b + c;
	return peak + (denominator != 0.0 ? 0.5 * (a - c) / denominator : 0.0);
}

} // namespace


auto AmpModel::load(const QString& path, double rate, QString& error) -> std::unique_ptr<AmpModel>
{
	auto model = std::unique_ptr<AmpModel>{new AmpModel{}};
	try
	{
		model->m_dsp = nam::get_dsp(std::filesystem::path{QFile::encodeName(path).toStdString()});
	}
	catch (const std::exception& e)
	{
		error = tr("The amp model %1 could not be loaded: %2").arg(path, QString::fromUtf8(e.what()));
		return nullptr;
	}
	if (!model->m_dsp)
	{
		error = tr("The amp model %1 could not be loaded.").arg(path);
		return nullptr;
	}
	if (model->m_dsp->NumInputChannels() != 1 || model->m_dsp->NumOutputChannels() != 1)
	{
		error = tr("The amp model %1 is not mono (one input, one output).").arg(path);
		return nullptr;
	}

	model->m_engineRate = rate;
	const auto trained = model->m_dsp->GetExpectedSampleRate();
	model->m_modelRate = trained > 0.0 && !model->m_dsp->SupportsArbitrarySampleRate() ? trained : rate;
	model->m_resampling = std::abs(model->m_modelRate - rate) > 0.5;
	if (model->m_dsp->HasLoudness())
	{
		model->m_gain = static_cast<float>(std::pow(10.0, (TargetLoudness - model->m_dsp->GetLoudness()) / 20.0));
	}

	const auto modelFrames = static_cast<int>(std::ceil(MaxFrames * model->m_modelRate / rate)) + 64;
	model->m_dsp->Reset(model->m_modelRate, modelFrames);

	if (model->m_resampling)
	{
		auto failure = 0;
		model->m_up = src_new(Converter, 1, &failure);
		model->m_down = src_new(Converter, 1, &failure);
		if (!model->m_up || !model->m_down)
		{
			error = tr("The amp model's resampling could not start: %1").arg(QString::fromUtf8(src_strerror(failure)));
			return nullptr;
		}
		model->m_atModelRate.resize(static_cast<std::size_t>(modelFrames));
		model->m_modelOutput.resize(static_cast<std::size_t>(modelFrames));
		model->m_downOutput.resize(static_cast<std::size_t>(MaxFrames + 256));

		// The output waits on a cushion of silence, so that the converters' jitter never starves it; the latency is
		// that cushion and the two converters' delays
		const auto up = model->m_modelRate / rate;
		const auto cushion = 256;
		const auto delay = converterDelay(up) / up + converterDelay(1.0 / up);
		model->m_latency = cushion + static_cast<int>(std::lround(delay));
		model->m_fifo.assign(static_cast<std::size_t>(4 * MaxFrames + cushion), 0.f);
		model->m_fifoFill = cushion;
	}
	return model;
}


AmpModel::~AmpModel()
{
	if (m_up) { src_delete(m_up); }
	if (m_down) { src_delete(m_down); }
}


void AmpModel::runModel(float* in, float* out, int frames)
{
	float* inputs[] = {in};
	float* outputs[] = {out};
	m_dsp->process(inputs, outputs, frames);
}


void AmpModel::process(float* io, int frames)
{
	frames = std::min(frames, MaxFrames);
	if (!m_resampling)
	{
		runModel(io, io, frames);
		for (int f = 0; f < frames; ++f) { io[f] *= m_gain; }
		return;
	}

	// Up to the model's rate, through the model, back down into the FIFO
	const auto up = m_modelRate / m_engineRate;
	const auto atModel = convert(m_up, up, io, frames, m_atModelRate.data(), static_cast<long>(m_atModelRate.size()));
	if (atModel > 0)
	{
		runModel(m_atModelRate.data(), m_modelOutput.data(), static_cast<int>(atModel));
		const auto back = convert(m_down, 1.0 / up, m_modelOutput.data(), atModel, m_downOutput.data(),
			static_cast<long>(m_downOutput.size()));
		for (long f = 0; f < back && m_fifoFill < m_fifo.size(); ++f)
		{
			m_fifo[(m_fifoRead + m_fifoFill) % m_fifo.size()] = m_downOutput[f];
			++m_fifoFill;
		}
	}

	// Out of the FIFO (silence only if it ran dry, which its cushion is there to prevent)
	for (int f = 0; f < frames; ++f)
	{
		if (m_fifoFill == 0) { io[f] = 0.f; continue; }
		io[f] = m_fifo[m_fifoRead] * m_gain;
		m_fifoRead = (m_fifoRead + 1) % m_fifo.size();
		--m_fifoFill;
	}
}

} // namespace lmms
