/*
 * AmpAndIr.h - impulse responses (cabinets, rooms, bodies) by convolution; amp models (NAM) to come
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

#ifndef LMMS_AMP_AND_IR_H
#define LMMS_AMP_AND_IR_H

#include <array>
#include <atomic>
#include <memory>

#include "AmpAndIrControls.h"
#include "AmpModel.h"
#include "Convolver.h"
#include "Effect.h"
#include "Handoff.h"

namespace lmms
{

/**
 * The Amp & IR plugin (docs/plano-anotacoes-13-11.md, 13.3): the input, through its gain, into an amp model
 * (Neural Amp Modeler, mono: the sides' mid), then convolved with an impulse response (a guitar cabinet, an
 * acoustic body, a room), then its output gain; either may be absent. The W/D knob mixes the dry signal, delayed
 * as the wet one is (the latency: the amp's resampling and the convolution's block). A mono IR serves both sides;
 * a stereo one each its own. The IR is resampled to the engine's rate and normalized in energy, so that changing
 * it keeps about the same level.
 *
 * Amp and IR are loaded off the audio thread and handed over to it without locks (see Handoff).
 */
class AmpAndIrEffect : public Effect
{
public:
	AmpAndIrEffect(Model* parent, const Descriptor::SubPluginFeatures::Key* key);
	~AmpAndIrEffect() override = default;

	auto controls() -> EffectControls* override { return &m_controls; }
	auto latency() const -> f_cnt_t override { return m_irLatency.load() + m_ampLatency.load(); }

	auto channelCount(bool) const -> ch_cnt_t override { return 2; }
	auto channelGroupCount(bool) const -> group_cnt_t override { return 1; }
	auto getChannelGroup(group_cnt_t, bool, AudioBuffer::ChannelGroup&) const -> ch_cnt_t override { return 2; }
	auto getInplacePair(ch_cnt_t) const -> std::optional<ch_cnt_t> override { return std::nullopt; }

	/**
	 * Loads the impulse response in @p path (any thread but the audio one) and hands it to the audio thread
	 * @returns an error, or nothing if it loaded
	 */
	auto loadImpulseResponse(const QString& path) -> QString;
	//! Loads the amp model (.nam) in @p path, likewise; an empty one removes it
	auto loadAmpModel(const QString& path) -> QString;
	//! Removes the impulse response
	void clearImpulseResponse();

private:
	auto process(ProcessContext& context) -> ProcessStatus override;

	struct ImpulseResponse
	{
		std::array<std::unique_ptr<Convolver>, 2> sides;
	};

	AmpAndIrControls m_controls;
	Handoff<ImpulseResponse> m_impulseResponse;
	Handoff<AmpModel> m_amp;
	std::atomic<f_cnt_t> m_irLatency = 0;
	std::atomic<f_cnt_t> m_ampLatency = 0;
	std::vector<float> m_mid; //!< the amp's mono signal

	friend class AmpAndIrControls;
};

} // namespace lmms

#endif // LMMS_AMP_AND_IR_H
