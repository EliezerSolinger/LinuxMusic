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
	m_controls(this)
{
}


AmpAndIrEffect::~AmpAndIrEffect()
{
	delete m_pending.exchange(nullptr);
	delete m_retired.exchange(nullptr);
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
	auto loaded = std::make_unique<Loaded>();
	for (int side = 0; side < 2; ++side)
	{
		for (auto& x : sides[side]) { x *= scale; }
		loaded->sides[side] = std::make_unique<Convolver>(sides[side]);
	}

	// To the audio thread (what it gave back before is freed here, never there)
	delete m_retired.exchange(nullptr);
	delete m_pending.exchange(loaded.release());
	m_hasIr = true;
	return {};
}


auto AmpAndIrEffect::process(ProcessContext& context) -> ProcessStatus
{
	// A new IR, if one is waiting and the one before has been freed
	if (m_retired.load() == nullptr)
	{
		if (auto* next = m_pending.exchange(nullptr))
		{
			m_retired.store(m_current.release());
			m_current.reset(next);
		}
	}

	const auto& in = context.in();
	auto& out = context.out();
	const auto inGain = dbfsToAmp(m_controls.m_inputModel.value());
	const auto outGain = dbfsToAmp(m_controls.m_outputModel.value());
	for (ch_cnt_t ch = 0; ch < 2; ++ch)
	{
		const auto src = in.buffer(ch);
		auto dst = out.buffer(ch);
		for (std::size_t f = 0; f < dst.size(); ++f) { dst[f] = src[f] * inGain; }
		if (m_current) { m_current->sides[ch]->process(dst.data(), static_cast<int>(dst.size())); }
		for (auto& x : dst) { x *= outGain; }
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
