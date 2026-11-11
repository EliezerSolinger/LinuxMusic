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
class Track;
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

/**
 * A channel of the mixer.
 *
 * Its buffer holds its track channels in pairs: the main pair (1/2), which goes through the trim, the effects,
 * the stereo section, pan and fader to the channel's output, followed by auxiliary pairs (3/4, 5/6...), like the
 * track channels of REAPER. The auxiliary pairs only pass through the effects (as their pins say) and never reach
 * the channel's output by themselves: an effect's output pins or a send must take them somewhere. A sidechain send
 * is a send to an auxiliary pair of the channel with the compressor, whose sidechain input is connected to it.
 */
class MixerChannel : public ThreadableJob
{
public:
	MixerChannel(int idx, Model* _parent);
	virtual ~MixerChannel();

	//! Number of track channels: 2 (the main pair) plus 2 per auxiliary pair
	auto channelCount() const -> ch_cnt_t { return m_buffer.channels(); }
	auto pairCount() const -> ch_cnt_t { return static_cast<ch_cnt_t>(channelCount() / 2); }

	//! Changes the number of track channels (see Mixer::setChannelCount()) [audio engine paused]
	void setChannelCount(ch_cnt_t channels);

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

	//! Whether the channel was made for a track (see Mixer::setUpTrackChannel()): it is named and colored
	//! after the track, and goes away with it
	bool m_trackChannel = false;

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

/**
 * A send: takes a channel pair of the sender to a channel pair of the receiver.
 *
 * The sender's main pair (0) is taken after its fader, or before it for pre-fader sends; an auxiliary pair is
 * taken after the sender's effects. A send to an auxiliary pair of the receiver (e.g. its channels 3/4) is a
 * sidechain send.
 */
class MixerRoute : public QObject
{
	Q_OBJECT
public:
	MixerRoute(MixerChannel* from, MixerChannel* to, float amount, ch_cnt_t sourcePair = 0, ch_cnt_t destinationPair = 0);
	~MixerRoute() override = default;

	//! The sender's channel pair that is sent: 0 is the main pair (channels 1/2), 1 the channels 3/4...
	auto sourcePair() const -> ch_cnt_t { return m_sourcePair; }

	//! The receiver's channel pair the send arrives at
	auto destinationPair() const -> ch_cnt_t { return m_destinationPair; }

	//! Whether the send goes to an auxiliary pair, e.g. to a sidechain input
	auto isSidechain() const -> bool { return m_destinationPair > 0; }

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
		ch_cnt_t m_sourcePair;
		ch_cnt_t m_destinationPair;
};

//! Most track channels a mixer channel can have
inline constexpr ch_cnt_t MaxMixerChannelTrackChannels = 64;


class LMMS_EXPORT Mixer : public Model, public JournallingObject
{
	Q_OBJECT
public:
	Mixer();
	~Mixer() override;

	void mixToChannel(const AudioBuffer& buffer, mix_ch_t dest);

	//! Adds the auxiliary pairs of @p buffer (its channels from 3 on) to the same channels of channel @p dest,
	//! as far as it has them. Used for instruments with more than two outputs.
	void mixAuxiliaryToChannel(const AudioBuffer& buffer, mix_ch_t dest);

	void prepareMasterMix();
	void masterMix( SampleFrame* _buf );

	void saveSettings( QDomDocument & _doc, QDomElement & _parent ) override;
	void loadSettings( const QDomElement & _this ) override;

	//! For undo/redo, the saved state also records which channel each track sends to: changing the
	//! channel layout (e.g. deleting a channel) renumbers the tracks' channels, and undoing must restore them
	QDomElement saveState(QDomDocument& doc, QDomElement& parent) override;
	void restoreState(const QDomElement& element) override;

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

	/**
	 * Sends the channel pair @p sourcePair of @p fromChannel to the pair @p destinationPair of @p toChannel,
	 * or adjusts the amount of that send if it exists. A destination pair above 0 makes a sidechain send.
	 *
	 * @returns the send, or nullptr if a channel does not have the pair or the send would make a loop
	 */
	MixerRoute* createChannelSend(mix_ch_t fromChannel, mix_ch_t toChannel, float amount,
		ch_cnt_t sourcePair, ch_cnt_t destinationPair);

	MixerRoute* createRoute(MixerChannel* from, MixerChannel* to, float amount,
		ch_cnt_t sourcePair = 0, ch_cnt_t destinationPair = 0);

	// delete the connection made by createChannelSend (the main pair to main pair send)
	void deleteChannelSend(mix_ch_t fromChannel, mix_ch_t toChannel);
	void deleteChannelSend( MixerRoute * route );

	//! @returns the send from pair @p sourcePair of @p fromChannel to pair @p destinationPair of @p toChannel
	auto findChannelSend(mix_ch_t fromChannel, mix_ch_t toChannel, ch_cnt_t sourcePair = 0,
		ch_cnt_t destinationPair = 0) const -> MixerRoute*;

	/**
	 * Sets how many track channels channel @p index has (2 for just the main pair, 4 for the main pair and the
	 * channels 3/4...); rounded up to an even count. Sends to or from pairs that no longer exist are deleted.
	 */
	void setChannelCount(mix_ch_t index, ch_cnt_t channels);

	/**
	 * Makes the mixer channel the only strip of @p track (an instrument or sample track), so that effects,
	 * volume and panning live in the mixer:
	 * - if the track sends to the master, to a channel that other tracks send to as well, or to a bus (a channel
	 *   other channels send to), it gets a channel of its own, named and colored after it, which sends where the
	 *   track sent; a channel only the track uses becomes the track's channel;
	 * - the track's own effects move to the front of its channel's effects, and its volume and panning into the
	 *   channel's input trim and pan (unless automated or controlled: then they stay on the track).
	 *
	 * @returns the track's channel, or -1 if @p track has no mixer channel
	 */
	int setUpTrackChannel(Track* track);

	//! setUpTrackChannel() for every instrument and sample track of the song and the pattern editor
	void setUpTrackChannels();

	//! Deletes the channel made for @p track (see setUpTrackChannel()) unless something else uses it
	void releaseTrackChannel(Track* track);

	//! The channel made for @p track (see setUpTrackChannel()), or nullptr if it sends to another channel
	auto trackChannel(const Track* track) const -> MixerChannel*;

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
	void channelCountChanged(int index);
	void channelDeleted(int index);
	void channelsSwapped(int fromIndex, int toIndex);
	//! A track got or lost a channel of its own (see setUpTrackChannel())
	void trackChannelChanged(lmms::Track* track);

private:
	// the mixer channels in the mixer. index 0 is always master.
	std::vector<MixerChannel*> m_mixerChannels;

	// make sure we have at least num channels
	void allocateChannelsTo(int num);

	int m_lastSoloed;

	//! Makes a structural change one undo step: only the outermost change adds a journal checkpoint
	class StructureChange
	{
	public:
		explicit StructureChange(Mixer& mixer);
		~StructureChange();
	private:
		Mixer& m_mixer;
		bool m_wasJournalling = false;
	};
	int m_structureChangeDepth = 0;
} ;


} // namespace lmms

#endif // LMMS_MIXER_H
