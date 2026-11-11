/*
 * Effect.h - base class for effects
 *
 * Copyright (c) 2006-2007 Danny McRae <khjklujn/at/users.sourceforge.net>
 * Copyright (c) 2006-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
 *
 * This file is part of LMMS - https://lmms.io
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

#ifndef LMMS_EFFECT_H
#define LMMS_EFFECT_H

#include "AudioBufferView.h"
#include "AudioEngine.h"
#include "AudioProcessor.h"
#include "AutomatableModel.h"
#include "Engine.h"
#include "Plugin.h"
#include "TempoSyncKnobModel.h"

namespace lmms
{

class AudioBuffer;
class AudioPortsModel;
class EffectChain;
class EffectControls;
class PluginInstance;

namespace gui
{

class EffectView;

} // namespace gui


class LMMS_EXPORT Effect
	: public Plugin
	, public AudioProcessor
{
	Q_OBJECT
public:
	Effect( const Plugin::Descriptor * _desc,
			Model * _parent,
			const Descriptor::SubPluginFeatures::Key * _key );

	void saveSettings( QDomDocument & _doc, QDomElement & _parent ) override;
	void loadSettings( const QDomElement & _this ) override;

	inline QString nodeName() const override
	{
		return "effect";
	}

	inline bool isOkay() const
	{
		return m_okay;
	}

	inline void setOkay( bool _state )
	{
		m_okay = _state;
	}

	//! @returns true if the processing outputted corrupted audio (infs/nans).
	bool isCorrupted() const { return m_corrupted.load(std::memory_order_relaxed); }

	//! "Awake" means the effect has not been put to sleep by auto-quit
	bool isAwake() const
	{
		return m_awake;
	}

	inline bool isEnabled() const
	{
		return m_enabledModel.value();
	}

	inline f_cnt_t timeout() const
	{
		const float samples = Engine::audioEngine()->outputSampleRate() * m_autoQuitModel.value() / 1000.0f;
		return 1 + ( static_cast<int>( samples ) / Engine::audioEngine()->framesPerPeriod() );
	}

	inline float wetLevel() const
	{
		return m_wetDryModel.value();
	}

	inline float dryLevel() const
	{
		return 1.0f - m_wetDryModel.value();
	}

	inline bool dontRun() const
	{
		return m_noRun;
	}

	inline void setDontRun( bool _state )
	{
		m_noRun = _state;
	}

	bool isProcessingAudio() const
	{
		return isEnabled() && isAwake() && isOkay() && !dontRun();
	}

	inline TempoSyncKnobModel* autoQuitModel()
	{
		return &m_autoQuitModel;
	}

	bool autoQuitEnabled() const
	{
		return m_autoQuitEnabled;
	}

	EffectChain * effectChain() const
	{
		return m_parent;
	}

	virtual EffectControls * controls() = 0;

	static Effect * instantiate( const QString & _plugin_name,
				Model * _parent,
				const Descriptor::SubPluginFeatures::Key * _key );

	// AudioProcessor lifecycle: most effects need nothing beyond their constructor, so these default to
	// doing nothing. Effects that allocate per sample rate or period size can override them.
	auto activate(double sampleRate, f_cnt_t maxFrameCount) -> bool override
	{
		(void)sampleRate;
		(void)maxFrameCount;
		return true;
	}
	void deactivate() override {}
	auto startProcessing() -> bool override { return true; }
	void stopProcessing() override {}
	void reset() override {}


protected:
	gui::PluginView* instantiateView( QWidget * ) override;

	void goToSleep()
	{
		m_quietBufferCount = 0;
		m_awake = false;
	}

	void wakeUp()
	{
		m_quietBufferCount = 0;
		m_awake = true;
	}

	virtual void onEnabledChanged() {}

	//! Runs instead of process() while the effect is awake but disabled or not running
	virtual void processBypassedImpl() {}

	/**
	 * If auto-quit is enabled ("Keep effects running even without input" setting is disabled),
	 * after "decay" ms of the output buffer remaining below the silence threshold, the effect is
	 * put to sleep and won't be processed again until it receives new audio input.
	 */
	void handleAutoQuit(bool silentOutput);

private:
	//! Not used by effects and will be removed in the future
	void playNote(ProcessContext&) final {}

	EffectChain * m_parent;

	bool m_okay;
	bool m_noRun;
	bool m_awake;
	std::atomic<bool> m_corrupted = false;

	//! The number of consecutive periods where output buffers remain below the silence threshold
	f_cnt_t m_quietBufferCount = 0;

	BoolModel m_enabledModel;
	FloatModel m_wetDryModel;
	TempoSyncKnobModel m_autoQuitModel;

	bool m_autoQuitEnabled = false;

	friend class gui::EffectView;
	friend class EffectChain;
	friend class PluginInstance;
};

/**
 * An Effect with interleaved buffers, in-place processing, stereo, etc.
 *
 * Inherit from this to reduce boilerplate.
 * In the future, once all plugins use planar buffers, this can be removed.
 */
class LMMS_EXPORT LegacyEffect : public Effect
{
public:
	using Effect::Effect;

protected:
	auto channelCount(bool isInput) const -> ch_cnt_t final
	{
		// Two inputs and outputs
		(void)isInput;
		return 2;
	}

	auto channelGroupCount(bool isInput) const -> group_cnt_t final
	{
		// One group on the input and output sides (the main stereo channels)
		(void)isInput;
		return 1;
	}

	auto getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup& group) const
		-> ch_cnt_t final
	{
		// Only one group and it has two channels (stereo)
		// TODO: Set channel names and other metadata
		(void)index;
		(void)isInput;
		(void)group;
		return 2;
	}

	auto getInplacePair(ch_cnt_t inputIndex) const -> std::optional<ch_cnt_t> final
	{
		// L input <--> L output
		// R input <--> R output
		return inputIndex;
	}

	auto usesInterleavedBuffers() const -> bool final { return true; }
};

using EffectKey = Effect::Descriptor::SubPluginFeatures::Key;
using EffectKeyList = Effect::Descriptor::SubPluginFeatures::KeyList;

} // namespace lmms

#endif // LMMS_EFFECT_H
