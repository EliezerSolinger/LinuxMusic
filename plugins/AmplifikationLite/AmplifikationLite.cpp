/*
 * AmplifikationLite.cpp - an amp modeled on Kuassa's Amplifikation Lite (measured as a black box)
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

#include "AmplifikationLite.h"

#include <cmath>

#include <QFile>
#include <samplerate.h>
#include <sndfile.h>

#include "../AmpAndIr/Convolver.h"
#include "AmpLiteData.h"
#include "AudioEngine.h"
#include "Engine.h"
#include "PathUtil.h"
#include "embed.h"
#include "plugin_export.h"

namespace lmms
{

extern "C"
{

Plugin::Descriptor PLUGIN_EXPORT amplifikationlite_plugin_descriptor =
{
	LMMS_STRINGIFY(PLUGIN_NAME),
	"Amplifikation Lite",
	QT_TRANSLATE_NOOP("PluginBrowser", "Guitar amp: clean, rhythm and lead channels, after Kuassa's Amplifikation Lite"),
	"LinuxMusic (artwork: Kuassa)",
	0x0100,
	Plugin::Type::Effect,
	new PluginPixmapLoader("logo"),
	nullptr,
	nullptr,
};

}

namespace
{

constexpr auto Converter = SRC_SINC_MEDIUM_QUALITY;
constexpr int Cushion = 256; //!< frames of silence the resampled output waits on (the converters' jitter)

//! Runs @p frames frames through @p state at @p ratio; @returns how many came out (all input consumed)
auto convert(SRC_STATE* state, double ratio, const float* in, long frames, float* out, long room) -> long
{
	auto data = SRC_DATA{};
	data.data_in = in;
	data.input_frames = frames;
	data.data_out = out;
	data.output_frames = room;
	data.src_ratio = ratio;
	auto produced = 0L;
	while (data.input_frames > 0)
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

} // namespace


AmplifikationLiteEffect::AmplifikationLiteEffect(Model* parent, const Descriptor::SubPluginFeatures::Key* key) :
	Effect(&amplifikationlite_plugin_descriptor, parent, key),
	m_controls(this),
	m_mono(MaxFrames)
{
	configureRate(Engine::audioEngine()->outputSampleRate());
}


void AmplifikationLiteEffect::configureRate(double rate)
{
	// [main thread, the plugin not processing] the resampling to the model's rate, for the engine's rate now
	if (m_up) { src_delete(m_up); m_up = nullptr; }
	if (m_down) { src_delete(m_down); m_down = nullptr; }
	m_ratio = amplite::ModelRate / rate;
	m_resampling = std::abs(rate - amplite::ModelRate) > 0.5;
	m_resampleLatency = 0;
	if (m_resampling)
	{
		auto error = 0;
		m_up = src_new(Converter, 1, &error);
		m_down = src_new(Converter, 1, &error);
		m_atModel.assign(static_cast<std::size_t>(std::ceil(MaxFrames * m_ratio)) + 256, 0.f);
		m_back.assign(MaxFrames + 256, 0.f);
		m_fifo.assign(4 * MaxFrames + Cushion, 0.f);
		// the cushion and the two converters' delays (about half their filters each)
		m_resampleLatency = Cushion + 2 * 32;
	}
	resetModel();
}


auto AmplifikationLiteEffect::activate(double sampleRate, f_cnt_t maxFrameCount) -> bool
{
	(void)maxFrameCount;
	configureRate(sampleRate);
	return true;
}


auto AmplifikationLiteEffect::startProcessing() -> bool
{
	// the processing (re)starts from rest, whatever it held when it stopped
	reset();
	return true;
}


AmplifikationLiteEffect::~AmplifikationLiteEffect()
{
	if (m_up) { src_delete(m_up); }
	if (m_down) { src_delete(m_down); }
}


auto AmplifikationLiteEffect::latency() const -> f_cnt_t
{
	const double modelLatency = m_dsp.latency() / m_ratio;
	return static_cast<f_cnt_t>(std::lround(modelLatency)) + m_resampleLatency + m_irLatency.load();
}


void AmplifikationLiteEffect::reset()
{
	resetModel();
	if (auto* impulseResponse = m_impulseResponse.take()) { impulseResponse->convolver->reset(); }
}


void AmplifikationLiteEffect::resetModel()
{
	m_dsp.reset();
	if (m_resampling && m_up && m_down)
	{
		src_reset(m_up);
		src_reset(m_down);
		std::fill(m_fifo.begin(), m_fifo.end(), 0.f);
		m_fifoRead = 0;
		m_fifoFill = Cushion;
	}
}


void AmplifikationLiteEffect::clearImpulseResponse()
{
	m_impulseResponse.offer(nullptr);
	m_irLatency = 0;
}


auto AmplifikationLiteEffect::loadImpulseResponse(const QString& path) -> QString
{
	auto info = SF_INFO{};
	auto* file = sf_open(QFile::encodeName(PathUtil::toAbsolute(path)).constData(), SFM_READ, &info);
	if (!file) { return tr("The impulse response %1 could not be read.").arg(path); }
	auto samples = std::vector<float>(static_cast<std::size_t>(info.frames * info.channels));
	sf_readf_float(file, samples.data(), info.frames);
	sf_close(file);
	if (info.frames <= 0 || info.channels <= 0) { return tr("The impulse response %1 is empty.").arg(path); }

	// mono (its first channel), at the engine's rate, normalized to unit energy
	auto ir = std::vector<float>(static_cast<std::size_t>(info.frames));
	for (long f = 0; f < info.frames; ++f) { ir[f] = samples[f * info.channels]; }
	const auto rate = static_cast<double>(Engine::audioEngine()->outputSampleRate());
	if (info.samplerate != static_cast<int>(rate))
	{
		const auto ratio = rate / info.samplerate;
		auto resampled = std::vector<float>(static_cast<std::size_t>(std::ceil(ir.size() * ratio) + 16));
		auto data = SRC_DATA{};
		data.data_in = ir.data();
		data.input_frames = static_cast<long>(ir.size());
		data.data_out = resampled.data();
		data.output_frames = static_cast<long>(resampled.size());
		data.src_ratio = ratio;
		if (src_simple(&data, SRC_SINC_BEST_QUALITY, 1) != 0) { return tr("The impulse response could not be resampled."); }
		resampled.resize(static_cast<std::size_t>(data.output_frames_gen));
		ir = std::move(resampled);
	}
	auto energy = 0.0;
	for (const auto x : ir) { energy += x * x; }
	if (energy <= 0.0) { return tr("The impulse response %1 is silent.").arg(path); }
	const auto scale = static_cast<float>(1.0 / std::sqrt(energy));
	for (auto& x : ir) { x *= scale; }

	auto loaded = std::make_unique<ImpulseResponse>();
	loaded->convolver = std::make_unique<Convolver>(ir);
	m_impulseResponse.offer(std::move(loaded));
	m_irLatency = Convolver::BlockSize;
	return {};
}


void AmplifikationLiteEffect::runModel(float* io, int frames)
{
	if (!m_resampling)
	{
		m_dsp.process(io, frames);
		return;
	}
	// up to 48 kHz, the model, back down into the FIFO, out of it
	const auto atModel = convert(m_up, m_ratio, io, frames, m_atModel.data(), static_cast<long>(m_atModel.size()));
	if (atModel > 0)
	{
		m_dsp.process(m_atModel.data(), static_cast<int>(atModel));
		const auto back = convert(m_down, 1.0 / m_ratio, m_atModel.data(), atModel, m_back.data(),
			static_cast<long>(m_back.size()));
		for (long f = 0; f < back && m_fifoFill < m_fifo.size(); ++f)
		{
			m_fifo[(m_fifoRead + m_fifoFill) % m_fifo.size()] = m_back[f];
			++m_fifoFill;
		}
	}
	for (int f = 0; f < frames; ++f)
	{
		if (m_fifoFill == 0) { io[f] = 0.f; continue; }
		io[f] = m_fifo[m_fifoRead];
		m_fifoRead = (m_fifoRead + 1) % m_fifo.size();
		--m_fifoFill;
	}
}


auto AmplifikationLiteEffect::process(ProcessContext& context) -> ProcessStatus
{
	auto* impulseResponse = m_impulseResponse.take();
	m_dsp.set(m_controls.settings());

	const auto& in = context.in();
	auto& out = context.out();
	const auto frames = static_cast<int>(out.buffer(0).size());
	const auto left = in.buffer(0);
	const auto right = in.buffer(1);
	auto outLeft = out.buffer(0);
	auto outRight = out.buffer(1);

	for (int at = 0; at < frames; at += MaxFrames)
	{
		const auto count = std::min(MaxFrames, frames - at);
		// the amp is mono: the sides' mid
		for (int f = 0; f < count; ++f) { m_mono[f] = (left[at + f] + right[at + f]) * 0.5f; }
		runModel(m_mono.data(), count);
		// the cabinet: INT is the model's (built in), EXT convolves with the loaded impulse response, OFF passes
		if (m_controls.cabinet() == AmplifikationLiteControls::Cabinet::External && impulseResponse)
		{
			impulseResponse->convolver->process(m_mono.data(), count);
		}
		for (int f = 0; f < count; ++f) { outLeft[at + f] = outRight[at + f] = m_mono[f]; }
	}
	mixDrySignal(in, out, 2);
	return ProcessStatus::ContinueIfNotQuiet;
}


extern "C"
{

PLUGIN_EXPORT Plugin* lmms_plugin_main(Model* parent, void* data)
{
	return new AmplifikationLiteEffect(parent, static_cast<const Plugin::Descriptor::SubPluginFeatures::Key*>(data));
}

}

} // namespace lmms
