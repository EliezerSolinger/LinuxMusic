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
#include "Convolver.h"
#include "Effect.h"

namespace lmms
{

/**
 * The IR plugin (docs/plano-anotacoes-13-11.md, 13.3): the input, through its gain, convolved with an impulse
 * response (a guitar cabinet, an acoustic body, a room), then its output gain; the W/D knob mixes the dry signal,
 * delayed as the convolution is (Convolver::BlockSize frames, its latency). A mono IR serves both sides; a stereo
 * one each its own. The IR is resampled to the engine's rate and normalized in energy, so that changing it keeps
 * about the same level.
 *
 * An IR is loaded off the audio thread and handed over to it without locks (the old one goes back to be freed).
 */
class AmpAndIrEffect : public Effect
{
public:
	AmpAndIrEffect(Model* parent, const Descriptor::SubPluginFeatures::Key* key);
	~AmpAndIrEffect() override;

	auto controls() -> EffectControls* override { return &m_controls; }
	auto latency() const -> f_cnt_t override { return m_hasIr.load() ? Convolver::BlockSize : 0; }

	auto channelCount(bool) const -> ch_cnt_t override { return 2; }
	auto channelGroupCount(bool) const -> group_cnt_t override { return 1; }
	auto getChannelGroup(group_cnt_t, bool, AudioBuffer::ChannelGroup&) const -> ch_cnt_t override { return 2; }
	auto getInplacePair(ch_cnt_t) const -> std::optional<ch_cnt_t> override { return std::nullopt; }

	/**
	 * Loads the impulse response in @p path (any thread but the audio one) and hands it to the audio thread
	 * @returns an error, or nothing if it loaded
	 */
	auto loadImpulseResponse(const QString& path) -> QString;

private:
	auto process(ProcessContext& context) -> ProcessStatus override;

	struct Loaded
	{
		std::array<std::unique_ptr<Convolver>, 2> sides;
	};

	AmpAndIrControls m_controls;
	std::unique_ptr<Loaded> m_current;			//!< the audio thread's
	std::atomic<Loaded*> m_pending = nullptr;	//!< loaded, for the audio thread to take
	std::atomic<Loaded*> m_retired = nullptr;	//!< the one it took over from, to free
	std::atomic<bool> m_hasIr = false;

	friend class AmpAndIrControls;
};

} // namespace lmms

#endif // LMMS_AMP_AND_IR_H
