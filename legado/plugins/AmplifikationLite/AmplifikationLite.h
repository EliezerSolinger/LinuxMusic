/*
 * AmplifikationLite.h - an amp modeled on Kuassa's Amplifikation Lite (measured as a black box)
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

#ifndef LMMS_AMPLIFIKATION_LITE_H
#define LMMS_AMPLIFIKATION_LITE_H

#include <array>
#include <atomic>
#include <memory>
#include <vector>

#include "AmpLiteDsp.h"
#include "AmplifikationLiteControls.h"
#include "Effect.h"
#include "Handoff.h"

typedef struct SRC_STATE_tag SRC_STATE;

namespace lmms
{

class Convolver;

/**
 * The amp of Amplifikation Lite as measured from outside (docs: ~/Desktop/AmplifikationLite-RE/NOTAS.md): three
 * channels (clean, rhythm, lead), gain, a Marshall-type tone stack, presence, volume, and a cabinet switch: INT
 * (the built-in cabinet, measured from the original), OFF, EXT (an impulse response loaded by its button).
 * The model runs at 48 kHz, the rate it was measured at: at another engine rate the mono signal (the sides' mid)
 * is resampled to it and back, with a fixed latency.
 */
class AmplifikationLiteEffect : public Effect
{
public:
	static constexpr int MaxFrames = 4096;

	AmplifikationLiteEffect(Model* parent, const Descriptor::SubPluginFeatures::Key* key);
	~AmplifikationLiteEffect() override;

	auto controls() -> EffectControls* override { return &m_controls; }
	auto latency() const -> f_cnt_t override;
	//! [audio thread] From rest again (a new project, an export): the model's filters, the resamplers, the cabinet
	void reset() override;
	//! [main thread] At the engine's rate (it may have changed since the effect was made)
	auto activate(double sampleRate, f_cnt_t maxFrameCount) -> bool override;
	//! [audio thread] The processing (re)starts from rest
	auto startProcessing() -> bool override;

	auto channelCount(bool) const -> ch_cnt_t override { return 2; }
	auto channelGroupCount(bool) const -> group_cnt_t override { return 1; }
	auto getChannelGroup(group_cnt_t, bool, AudioBuffer::ChannelGroup&) const -> ch_cnt_t override { return 2; }
	auto getInplacePair(ch_cnt_t) const -> std::optional<ch_cnt_t> override { return std::nullopt; }

	//! Loads the external cabinet's impulse response (any thread but the audio one); @returns an error, or nothing
	auto loadImpulseResponse(const QString& path) -> QString;
	void clearImpulseResponse();

private:
	auto process(ProcessContext& context) -> ProcessStatus override;
	void runModel(float* io, int frames);
	void configureRate(double rate);
	void resetModel(); //!< the model and the resamplers from rest (not the cabinet's IR: that is the audio thread's)

	struct ImpulseResponse
	{
		std::unique_ptr<Convolver> convolver;
	};

	AmplifikationLiteControls m_controls;
	amplite::AmpLiteDsp m_dsp;
	Handoff<ImpulseResponse> m_impulseResponse;
	std::atomic<f_cnt_t> m_irLatency = 0;
	std::vector<float> m_mono;

	// resampling to the model's rate and back (when the engine runs at another)
	bool m_resampling = false;
	double m_ratio = 1.0; //!< model rate / engine rate
	SRC_STATE* m_up = nullptr;
	SRC_STATE* m_down = nullptr;
	std::vector<float> m_atModel;
	std::vector<float> m_back;
	std::vector<float> m_fifo;
	std::size_t m_fifoRead = 0;
	std::size_t m_fifoFill = 0;
	int m_resampleLatency = 0;

	friend class AmplifikationLiteControls;
};

} // namespace lmms

#endif // LMMS_AMPLIFIKATION_LITE_H
