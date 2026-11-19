/*
 * AmpAndIr.cpp - impulse responses (cabinets, rooms, bodies) by convolution; amp models (NAM) to come
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

#include "AmpAndIr.h"

#include <cmath>
#include <vector>

#include <QFile>
#include <samplerate.h>
#include <sndfile.h>

#include "AudioEngine.h"
#include "Engine.h"
#include "PathUtil.h"
#include "embed.h"
#include "lmms_math.h"
#include "plugin_export.h"

namespace lmms
{

extern "C"
{

Plugin::Descriptor PLUGIN_EXPORT ampandir_plugin_descriptor =
{
	LMMS_STRINGIFY(PLUGIN_NAME),
	"Amp & IR",
	QT_TRANSLATE_NOOP("PluginBrowser", "Impulse responses: guitar cabinets, acoustic bodies, rooms"),
	"LinuxMusic",
	0x0100,
	Plugin::Type::Effect,
	new PixmapLoader("lmms-plugin-logo"),
	nullptr,
	nullptr,
};

}


AmpAndIrEffect::AmpAndIrEffect(Model* parent, const Descriptor::SubPluginFeatures::Key* key) :
	Effect(&ampandir_plugin_descriptor, parent, key),
	m_controls(this),
	m_mid(AmpModel::MaxFrames)
{
}


auto AmpAndIrEffect::loadAmpModel(const QString& path) -> QString
{
	if (path.isEmpty())
	{
		m_amp.offer(nullptr);
		m_ampLatency = 0;
		return {};
	}
	auto error = QString{};
	auto model = AmpModel::load(PathUtil::toAbsolute(path), Engine::audioEngine()->outputSampleRate(), error);
	if (!model) { return error; }
	m_ampLatency = model->latency();
	m_amp.offer(std::move(model));
	return {};
}


void AmpAndIrEffect::clearImpulseResponse()
{
	m_impulseResponse.offer(nullptr);
	m_irLatency = 0;
}


auto AmpAndIrEffect::loadImpulseResponse(const QString& path) -> QString
{
	auto info = SF_INFO{};
	auto* file = sf_open(QFile::encodeName(PathUtil::toAbsolute(path)).constData(), SFM_READ, &info);
	if (!file) { return tr("The impulse response %1 could not be read.").arg(path); }
	auto samples = std::vector<float>(static_cast<std::size_t>(info.frames * info.channels));
	sf_readf_float(file, samples.data(), info.frames);
	sf_close(file);
	if (info.frames <= 0 || info.channels <= 0) { return tr("The impulse response %1 is empty.").arg(path); }

	// At the engine's rate
	const auto rate = static_cast<double>(Engine::audioEngine()->outputSampleRate());
	auto frames = static_cast<long>(info.frames);
	if (info.samplerate != static_cast<int>(rate))
	{
		const auto ratio = rate / info.samplerate;
		auto resampled = std::vector<float>(static_cast<std::size_t>(std::ceil(frames * ratio) + 16) * info.channels);
		auto data = SRC_DATA{};
		data.data_in = samples.data();
		data.input_frames = frames;
		data.data_out = resampled.data();
		data.output_frames = static_cast<long>(resampled.size() / info.channels);
		data.src_ratio = ratio;
		if (src_simple(&data, SRC_SINC_BEST_QUALITY, info.channels) != 0) { return tr("The impulse response could not be resampled."); }
		frames = data.output_frames_gen;
		resampled.resize(static_cast<std::size_t>(frames * info.channels));
		samples = std::move(resampled);
	}

	// Each side's IR (a mono one for both), normalized together to unit energy per side
	auto sides = std::array<std::vector<float>, 2>{};
	auto energy = 0.0;
	for (int side = 0; side < 2; ++side)
	{
		const int channel = std::min(side, info.channels - 1);
		sides[side].resize(static_cast<std::size_t>(frames));
		for (long f = 0; f < frames; ++f)
		{
			sides[side][f] = samples[f * info.channels + channel];
			energy += sides[side][f] * sides[side][f];
		}
	}
	if (energy <= 0.0) { return tr("The impulse response %1 is silent.").arg(path); }
	const auto scale = static_cast<float>(1.0 / std::sqrt(energy / 2.0));
	auto loaded = std::make_unique<ImpulseResponse>();
	for (int side = 0; side < 2; ++side)
	{
		for (auto& x : sides[side]) { x *= scale; }
		loaded->sides[side] = std::make_unique<Convolver>(sides[side]);
	}

	// To the audio thread
	m_impulseResponse.offer(std::move(loaded));
	m_irLatency = Convolver::BlockSize;
	return {};
}


auto AmpAndIrEffect::process(ProcessContext& context) -> ProcessStatus
{
	auto* impulseResponse = m_impulseResponse.take();
	auto* amp = m_amp.take();

	const auto& in = context.in();
	auto& out = context.out();
	const auto inGain = dbfsToAmp(m_controls.m_inputModel.value());
	const auto outGain = dbfsToAmp(m_controls.m_outputModel.value());
	const auto frames = static_cast<int>(out.buffer(0).size());
	const auto left = in.buffer(0);
	const auto right = in.buffer(1);
	auto outLeft = out.buffer(0);
	auto outRight = out.buffer(1);

	if (amp)
	{
		// The amp is mono: the sides' mid, in pieces it can take, to both sides
		for (int at = 0; at < frames; at += AmpModel::MaxFrames)
		{
			const auto count = std::min(AmpModel::MaxFrames, frames - at);
			for (int f = 0; f < count; ++f) { m_mid[f] = (left[at + f] + right[at + f]) * 0.5f * inGain; }
			amp->process(m_mid.data(), count);
			for (int f = 0; f < count; ++f) { outLeft[at + f] = outRight[at + f] = m_mid[f]; }
		}
	}
	else
	{
		for (int f = 0; f < frames; ++f)
		{
			outLeft[f] = left[f] * inGain;
			outRight[f] = right[f] * inGain;
		}
	}
	if (impulseResponse)
	{
		impulseResponse->sides[0]->process(outLeft.data(), frames);
		impulseResponse->sides[1]->process(outRight.data(), frames);
	}
	for (ch_cnt_t ch = 0; ch < 2; ++ch)
	{
		for (auto& x : out.buffer(ch)) { x *= outGain; }
	}
	// The dry signal, delayed by the latency, under the W/D knob
	mixDrySignal(in, out, 2);
	return ProcessStatus::ContinueIfNotQuiet;
}


extern "C"
{

PLUGIN_EXPORT Plugin* lmms_plugin_main(Model* parent, void* data)
{
	return new AmpAndIrEffect(parent, static_cast<const Plugin::Descriptor::SubPluginFeatures::Key*>(data));
}

}

} // namespace lmms
