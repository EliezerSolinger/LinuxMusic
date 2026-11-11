/*
 * Mixer.h - effect-mixer for LMMS
 *
 * Copyright (c) 2008-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_MIXER_H
#define LMMS_MIXER_H

#include "AudioBuffer.h"
#include "ChannelMeter.h"
#include "EffectChain.h"
#include "JournallingObject.h"
#include "Model.h"
#include "ThreadableJob.h"

#include <atomic>
#include <optional>
#include <vector>
#include <QColor>

#include "SampleFrame.h"

namespace lmms
{


class MixerRoute;
using MixerRouteVector = std::vector<MixerRoute*>;

//! Where a mixer channel takes audio from, besides the tracks and channels that send to it.
//! Used by recording (stage 3 of the LinuxMusic roadmap).
struct ChannelInput
{
	enum class Source
	{
		None,		//!< only tracks and other channels
		Hardware	//!< inputs of the audio interface
	};

	Source source = Source::None;
	int firstChannel = 0; //!< zero-based input channel of the audio interface
	int channelCount = 2; //!< 1 = mono, 2 = stereo pair starting at firstChannel

	bool operator==(const ChannelInput&) const = default;
};

class MixerChannel : public ThreadableJob
{
public:
	MixerChannel(int idx, Model* _parent);
	virtual ~MixerChannel();

	EffectChain m_fxChain;

	// set to true if any effect in the channel is enabled and running
	bool m_stillRunning;

	//! Output levels after the fader, for the GUI
	ChannelMeter m_meter;
	AudioBuffer m_buffer;
	bool m_muteBeforeSolo;
	BoolModel m_muteModel;
	BoolModel m_soloModel;
	FloatModel m_volumeModel;

	// Channel strip, in signal order: input trim, effects, (pre-fader sends), swap L/R, phase invert,
	// stereo width, pan, volume fader, (post-fader sends)
	FloatModel m_trimModel; //!< input gain in dB, applied before the effects
	BoolModel m_swapChannelsModel;
	BoolModel m_phaseInvertModel;
	FloatModel m_widthModel; //!< stereo width in percent: 0 = mono, 100 = unchanged, 200 = twice as wide
	FloatModel m_panModel; //!< -100 (left) to 100 (right)

	// Recording (stage 3): the channel's input, whether it records, and when it plays its input back
	enum class MonitorMode
	{
		Off,	//!< never play the input
		On,		//!< always play the input
		Auto	//!< play the input while armed for recording
	};
	ChannelInput m_input;
	BoolModel m_recordArmModel;
	IntModel m_monitorModel; //!< a MonitorMode

	//! The signal after the effects and before the stereo section, pan and fader, for pre-fader sends
	std::vector<SampleFrame> m_preFaderBuffer;
	bool m_preFaderHasSignal = false;

	QString m_name;
	QMutex m_lock;
	bool m_queued; // are we queued up for rendering yet?
	bool m_muted; // are we muted? updated per period so we don't have to call m_muteModel.value() twice

	// pointers to other channels that this one sends to
	MixerRouteVector m_sends;

	// pointers to other channels that send to this one
	MixerRouteVector m_receives;

	int index() const { return m_channelIndex; }
	void setIndex(int index) { m_channelIndex = index; }

	bool isMaster() { return m_channelIndex == 0; }

	bool requiresProcessing() const override { return true; }

	//! Called instead of processing for a muted channel: keeps its parameters and meter up to date
	void processMuted();
	void unmuteForSolo();
	void unmuteSenderForSolo();
	void unmuteReceiverForSolo();

	auto color() const -> const std::optional<QColor>& { return m_color; }
	void setColor(const std::optional<QColor>& color) { m_color = color; }

	std::atomic_size_t m_dependenciesMet;
	void incrementDeps();
	void processed();

	int useCount() const { return m_useCount; }

	void incrementUseCount() { ++m_useCount; }
	void decrementUseCount() { --m_useCount; }

private:
	void doProcessing() override;
	int m_channelIndex;
	int m_useCount;
	std::optional<QColor> m_color;
};

class MixerRoute : public QObject
{
	Q_OBJECT
public:
	MixerRoute( MixerChannel * from, MixerChannel * to, float amount );
	~MixerRoute() override = default;

	mix_ch_t senderIndex() const
	{
		return m_from->index();
	}

	mix_ch_t receiverIndex() const
	{
		return m_to->index();
	}

	FloatModel * amount()
	{
		return &m_amount;
	}

	//! Pre-fader sends take the sender's signal after its effects, before its stereo section, pan and fader
	BoolModel* preFader() { return &m_preFader; }
	bool isPreFader() const { return m_preFader.value(); }

	MixerChannel * sender() const
	{
		return m_from;
	}

	MixerChannel * receiver() const
	{
		return m_to;
	}

	void updateName();

	private:
		MixerChannel * m_from;
		MixerChannel * m_to;
		FloatModel m_amount;
		BoolModel m_preFader;
};


class LMMS_EXPORT Mixer : public Model, public JournallingObject
{
	Q_OBJECT
public:
	Mixer();
	~Mixer() override;

	void mixToChannel(const AudioBuffer& buffer, mix_ch_t dest);

	void prepareMasterMix();
	void masterMix( SampleFrame* _buf );

	void saveSettings( QDomDocument & _doc, QDomElement & _parent ) override;
	void loadSettings( const QDomElement & _this ) override;

	QString nodeName() const override
	{
		return "mixer";
	}

	MixerChannel * mixerChannel( int _ch )
	{
		return m_mixerChannels[_ch];
	}

	// make the output of channel fromChannel go to the input of channel toChannel
	// it is safe to call even if the send already exists
	MixerRoute * createChannelSend(mix_ch_t fromChannel, mix_ch_t toChannel,
						   float amount = 1.0f);
	MixerRoute * createRoute( MixerChannel * from, MixerChannel * to, float amount );

	// delete the connection made by createChannelSend
	void deleteChannelSend(mix_ch_t fromChannel, mix_ch_t toChannel);
	void deleteChannelSend( MixerRoute * route );

	// determine if adding a send from sendFrom to
	// sendTo would result in an infinite mixer loop.
	bool isInfiniteLoop(mix_ch_t fromChannel, mix_ch_t toChannel);
	bool checkInfiniteLoop( MixerChannel * from, MixerChannel * to );

	// return the FloatModel of fromChannel sending its output to the input of
	// toChannel. NULL if there is no send.
	FloatModel * channelSendModel(mix_ch_t fromChannel, mix_ch_t toChannel);

	// add a new channel to the mixer.
	// returns the index of the channel that was just added
	int createChannel();

	//! Sets where channel @p index takes audio from (see ChannelInput)
	void setChannelInput(mix_ch_t index, const ChannelInput& input);

	// delete a channel from the mixer.
	void deleteChannel(int index);

	// delete all the mixer channels except master and remove all effects
	void clear();

	// re-arrange channels
	void moveChannelLeft(int index);
	void moveChannelRight(int index);

	// reset a channel's name, fx, sends, etc
	void clearChannel(mix_ch_t channelIndex);

	// rename channels when moving etc. if they still have their original name
	void validateChannelName( int index, int oldIndex );

	// check if the index channel receives audio from any other channel
	// or from any instrument or sample track
	bool isChannelInUse(int index);

	void toggledSolo();
	void activateSolo();
	void deactivateSolo();

	inline mix_ch_t numChannels() const
	{
		return m_mixerChannels.size();
	}

	MixerRouteVector m_mixerRoutes;

signals:
	void channelCreated(int index);
	void channelInputChanged(int index);
	void channelDeleted(int index);
	void channelsSwapped(int fromIndex, int toIndex);

private:
	// the mixer channels in the mixer. index 0 is always master.
	std::vector<MixerChannel*> m_mixerChannels;

	// make sure we have at least num channels
	void allocateChannelsTo(int num);

	int m_lastSoloed;
} ;


} // namespace lmms

#endif // LMMS_MIXER_H
