/*
 * Mixer.cpp - effect mixer for LMMS
 *
 * Copyright (c) 2008-2011 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "Mixer.h"

#include <QDomElement>
#include <QVariant>

#include "AudioEngine.h"
#include "AudioEngineWorkerThread.h"
#include "Mixer.h"
#include "DataFile.h"
#include "FolderTrack.h"
#include "Song.h"

#include "InstrumentTrack.h"
#include "MixHelpers.h"
#include "lmms_math.h"
#include "panning.h"
#include "PatternStore.h"
#include "PluginInstance.h"
#include "ProjectJournal.h"
#include "SampleTrack.h"
#include "TrackContainer.h" // For TrackContainer::TrackList typedef
#include "AudioBusHandle.h"
#include "EffectChain.h"

namespace lmms
{


MixerRoute::MixerRoute(MixerChannel* from, MixerChannel* to, float amount, ch_cnt_t sourcePair,
	ch_cnt_t destinationPair) :
	m_from( from ),
	m_to( to ),
	m_amount(amount, 0, 1, 0.001f, nullptr,
			tr("Amount to send from channel %1 to channel %2").arg(m_from->index()).arg(m_to->index())),
	m_preFader(false, nullptr,
			tr("Pre-fader send from channel %1 to channel %2").arg(m_from->index()).arg(m_to->index())),
	m_sourcePair(sourcePair),
	m_destinationPair(destinationPair)
{
	//qDebug( "created: %d to %d", m_from->m_channelIndex, m_to->m_channelIndex );
	// create send amount model
}


void MixerRoute::updateName()
{
	m_amount.setDisplayName(
			tr("Amount to send from channel %1 to channel %2").arg(m_from->index()).arg(m_to->index()));
}


namespace
{

//! A mixer channel's buffer: @p channels track channels in pairs, with an interleaved copy of the main pair
auto channelBuffer(ch_cnt_t channels) -> AudioBuffer
{
	auto buffer = AudioBuffer{Engine::audioEngine()->framesPerPeriod(), channels, channels, true};
	buffer.setGroups(static_cast<group_cnt_t>(channels / 2), [](group_cnt_t, AudioBuffer::ChannelGroup&) {
		return ch_cnt_t{2};
	});
	return buffer;
}

//! The silence flags of channel pair @p pair
auto pairMask(ch_cnt_t pair) -> AudioBuffer::ChannelFlags
{
	auto mask = AudioBuffer::ChannelFlags{};
	mask.set(2 * pair);
	mask.set(2 * pair + 1);
	return mask;
}

} // namespace


MixerChannel::MixerChannel( int idx, Model * _parent ) :
	m_fxChain( nullptr ),
	m_stillRunning( false ),
	m_buffer(channelBuffer(DEFAULT_CHANNELS)),
	m_muteModel( false, _parent ),
	m_soloModel( false, _parent ),
	m_volumeModel(1.f, 0.f, 2.f, 0.001f, _parent),
	m_trimModel(0.f, -24.f, 24.f, 0.001f, _parent),
	m_monoModel(false, _parent),
	m_swapChannelsModel(false, _parent),
	m_phaseInvertModel(false, _parent),
	m_widthModel(100.f, 0.f, 200.f, 1.f, _parent),
	m_panModel(PanningCenter, PanningLeft, PanningRight, 1.f, _parent),
	m_analogStrip(_parent),
	m_recordArmModel(false, _parent),
	m_monitorModel(static_cast<int>(MonitorMode::Auto), static_cast<int>(MonitorMode::Off),
		static_cast<int>(MonitorMode::Auto), _parent),
	m_preFaderBuffer(Engine::audioEngine()->framesPerPeriod()),
	m_name(),
	m_lock(),
	m_queued( false ),
	m_dependenciesMet(0),
	m_channelIndex(idx),
	m_useCount(0)
{
	// Knobs draw their arc from these
	m_trimModel.setCenterValue(0.f);
	m_widthModel.setCenterValue(100.f);
	m_panModel.setCenterValue(PanningCenter);
}




MixerChannel::~MixerChannel()
{
}




void MixerChannel::setChannelCount(ch_cnt_t channels)
{
	m_buffer = channelBuffer(channels);
	m_fxChain.setTrackChannelCount(channels);
}


inline void MixerChannel::processed()
{
	for( const MixerRoute * receiverRoute : m_sends )
	{
		if( receiverRoute->receiver()->m_muted == false )
		{
			receiverRoute->receiver()->incrementDeps();
		}
	}
}

void MixerChannel::incrementDeps()
{
	const auto i = m_dependenciesMet++ + 1;
	if( i >= m_receives.size() && ! m_queued )
	{
		m_queued = true;
		AudioEngineWorkerThread::addJob( this );
	}
}

void MixerChannel::processMuted()
{
	// Same parameters as doProcessing() reads, see there
	m_volumeModel.valueBuffer();
	m_trimModel.valueBuffer();
	m_widthModel.valueBuffer();
	m_panModel.valueBuffer();
	for (MixerRoute* route : m_receives) { route->amount()->valueBuffer(); }

	m_preFaderHasSignal = false;
	m_meter.measureSilence();
	m_analogStrip.reset();

	// What was delayed must not come out when unmuted
	m_inputDelay.clear();
	for (MixerRoute* route : m_receives) { route->latencyDelay().clear(); }
}




void MixerChannel::doProcessing()
{
	const f_cnt_t fpp = Engine::audioEngine()->framesPerPeriod();

	// Read every parameter in every period, even without signal: a parameter change ramps to its new value
	// over the next period that reads it, so skipping silent periods would delay the ramp until sound arrives
	ValueBuffer* volBuf = m_volumeModel.valueBuffer();
	m_trimModel.valueBuffer();
	m_widthModel.valueBuffer();
	m_panModel.valueBuffer();
	auto sendBufs = std::vector<ValueBuffer*>{};
	sendBufs.reserve(m_receives.size());
	for (MixerRoute* route : m_receives) { sendBufs.push_back(route->amount()->valueBuffer()); }

	m_preFaderHasSignal = false;

	auto buffer = m_buffer.interleavedBuffer().asSampleFrames();
	const auto mainPair = pairMask(0);

	// Latency compensation: the tracks' audio waits for the latest input
	if (m_inputDelay.delay() > 0)
	{
		auto allChannels = AudioBuffer::ChannelFlags{};
		for (ch_cnt_t ch = 0; ch < channelCount(); ++ch) { allChannels.set(ch); }
		const bool hadInput = m_buffer.hasSignal(allChannels);
		if (hadInput || m_inputDelay.holdsSignal())
		{
			for (ch_cnt_t ch = 0; ch < channelCount(); ++ch)
			{
				auto channel = m_buffer.buffer(ch);
				m_inputDelay.process(ch, channel.data(), channel.data(), fpp);
				m_buffer.assumeNonSilent(ch);
			}
			toInterleaved(m_buffer.groupBuffers(0), m_buffer.interleavedBuffer());
		}
		m_inputDelay.advance(fpp, hadInput);
	}

	// Inputs from other channels: the sender's main pair before or after its fader, or one of its auxiliary
	// pairs after its effects, into one of this channel's pairs
	bool receivedMain = false;
	for (auto i = std::size_t{0}; i < m_receives.size(); ++i)
	{
		MixerRoute* route = m_receives[i];
		if (!route->isLive())
		{
			// Leads out of what is soloed; what it delayed must not come out afterwards
			route->latencyDelay().clear();
			continue;
		}
		MixerChannel* sender = route->sender();
		const auto sourcePair = route->sourcePair();
		const auto destinationPair = route->destinationPair();
		if (sourcePair >= sender->pairCount() || destinationPair >= pairCount()) { continue; }

		const bool preFader = sourcePair == 0 && route->isPreFader();
		const bool hasSignal = preFader
			? sender->m_preFaderHasSignal
			: sender->m_buffer.hasSignal(pairMask(sourcePair)) || sender->m_stillRunning;
		auto& delay = route->latencyDelay();
		const bool delayed = delay.delay() > 0;
		// A send turned all the way down, e.g. to an effect return, carries nothing
		const bool closed = route->amount()->value() == 0.f && !sendBufs[i];
		if (closed && delayed) { delay.clear(); }
		if (closed || (!hasSignal && !(delayed && delay.holdsSignal())))
		{
			if (delayed) { delay.advance(fpp, false); }
			continue;
		}

		auto left = m_buffer.buffer(2 * destinationPair);
		auto right = m_buffer.buffer(2 * destinationPair + 1);
		const float amount = route->amount()->value();
		const ValueBuffer* amountBuf = sendBufs[i];
		auto gain = [&](f_cnt_t f) { return amountBuf ? amountBuf->values()[f] : amount; };

		if (delayed)
		{
			// Latency compensation: the sender's pair waits for this channel's latest input
			m_delayScratch.resize(2 * fpp);
			float* delayedLeft = m_delayScratch.data();
			float* delayedRight = delayedLeft + fpp;
			if (!hasSignal)
			{
				std::fill_n(delayedLeft, 2 * fpp, 0.f);
			}
			else if (preFader)
			{
				const auto& src = sender->m_preFaderBuffer;
				for (f_cnt_t f = 0; f < fpp; ++f)
				{
					delayedLeft[f] = src[f].left();
					delayedRight[f] = src[f].right();
				}
			}
			else
			{
				std::copy_n(sender->m_buffer.buffer(2 * sourcePair).data(), fpp, delayedLeft);
				std::copy_n(sender->m_buffer.buffer(2 * sourcePair + 1).data(), fpp, delayedRight);
			}
			delay.process(0, delayedLeft, delayedLeft, fpp);
			delay.process(1, delayedRight, delayedRight, fpp);
			delay.advance(fpp, hasSignal);
			for (f_cnt_t f = 0; f < fpp; ++f)
			{
				left[f] += delayedLeft[f] * gain(f);
				right[f] += delayedRight[f] * gain(f);
			}
			m_buffer.assumeNonSilent(2 * destinationPair);
			m_buffer.assumeNonSilent(2 * destinationPair + 1);
			receivedMain = receivedMain || destinationPair == 0;
			continue;
		}

		if (preFader)
		{
			const auto& src = sender->m_preFaderBuffer;
			for (f_cnt_t f = 0; f < fpp; ++f)
			{
				left[f] += src[f].left() * gain(f);
				right[f] += src[f].right() * gain(f);
			}
		}
		else
		{
			const auto srcLeft = sender->m_buffer.buffer(2 * sourcePair);
			const auto srcRight = sender->m_buffer.buffer(2 * sourcePair + 1);
			for (f_cnt_t f = 0; f < fpp; ++f)
			{
				left[f] += srcLeft[f] * gain(f);
				right[f] += srcRight[f] * gain(f);
			}
		}

		// Like the sender's channels: a sender that is still running may be silent
		if (preFader || !sender->m_buffer.silenceFlags()[2 * sourcePair]) { m_buffer.assumeNonSilent(2 * destinationPair); }
		if (preFader || !sender->m_buffer.silenceFlags()[2 * sourcePair + 1])
		{
			m_buffer.assumeNonSilent(2 * destinationPair + 1);
		}
		receivedMain = receivedMain || destinationPair == 0;
	}

	// Keep the interleaved copy of the main pair in sync
	if (receivedMain) { toInterleaved(m_buffer.groupBuffers(0), m_buffer.interleavedBuffer()); }

	// Input trim
	if (m_buffer.hasSignal(mainPair) && m_trimModel.value() != 0.f)
	{
		const float trim = dbfsToAmp(m_trimModel.value());
		for (auto& frame : buffer) { frame *= trim; }
		toPlanar(m_buffer.interleavedBuffer(), m_buffer.groupBuffers(0));
	}

	// Mono: summed after the trim, before the analog strip and the effects, so that they get a mono signal; switching
	// crossfades over the period
	const float monoTarget = m_monoModel.value() && !isMaster() ? 1.f : 0.f;
	if ((monoTarget > 0.f || m_monoMix > 0.f) && m_buffer.hasSignal(mainPair))
	{
		const auto count = static_cast<float>(buffer.size());
		for (auto f = std::size_t{0}; f < buffer.size(); ++f)
		{
			const float mix = m_monoMix + (monoTarget - m_monoMix) * static_cast<float>(f + 1) / count;
			const float sum = (buffer[f].left() + buffer[f].right()) * 0.5f;
			buffer[f].setLeft(buffer[f].left() + (sum - buffer[f].left()) * mix);
			buffer[f].setRight(buffer[f].right() + (sum - buffer[f].right()) * mix);
		}
		toPlanar(m_buffer.interleavedBuffer(), m_buffer.groupBuffers(0));
	}
	m_monoMix = monoTarget;

	// The master's PRE faders, before its effects, and in analog mode the gain stage they drive
	if (isMaster() && m_buffer.hasSignal(mainPair))
	{
		auto& master = Engine::mixer()->masterSection();
		const auto sampleRate = Engine::audioEngine()->outputSampleRate();
		master.processPreFaders(buffer, sampleRate);
		if (Engine::mixer()->isAnalog()) { master.processDrive(buffer, sampleRate); }
		else { master.resetDrive(); }
		toPlanar(m_buffer.interleavedBuffer(), m_buffer.groupBuffers(0));
	}

	// Analog mode: the strip of an analog desk, before the effects
	const bool hadInput = m_buffer.hasSignal(mainPair);
	if (!isMaster() && Engine::mixer()->isAnalog() && (hadInput || m_analogStrip.holdsSignal()))
	{
		const auto tolerance = Engine::mixer()->toleranceModel()->value() ? ChannelTolerance::forChannel(m_channelIndex)
			: ChannelTolerance::none();
		m_analogStrip.process(buffer, Engine::audioEngine()->outputSampleRate(), hadInput, tolerance);
		toPlanar(m_buffer.interleavedBuffer(), m_buffer.groupBuffers(0));
		m_buffer.assumeNonSilent(0);
		m_buffer.assumeNonSilent(1);
	}
	else { m_analogStrip.reset(); }

	m_stillRunning = m_fxChain.processAudioBuffer(m_buffer);

	// The auxiliary pairs do not reach the channel's output
	const bool hasSignal = m_buffer.hasSignal(mainPair) || m_stillRunning;

	// The master's stereo section (with the master's width) and bus compressor, after its effects and before its
	// fader
	if (isMaster() && hasSignal)
	{
		auto& master = Engine::mixer()->masterSection();
		const auto rate = Engine::audioEngine()->outputSampleRate();
		master.processStereo(buffer, rate, m_widthModel.value());
		master.processCompressor(buffer, rate);
		toPlanar(m_buffer.interleavedBuffer(), m_buffer.groupBuffers(0));
	}

	// Tap for pre-fader sends
	const bool feedsPreFader = std::any_of(m_sends.begin(), m_sends.end(),
		[](const MixerRoute* route) { return route->isPreFader(); });
	if (feedsPreFader && hasSignal)
	{
		std::copy(buffer.begin(), buffer.end(), m_preFaderBuffer.begin());
		m_preFaderHasSignal = true;
	}

	// Stereo section, pan and fader
	if (hasSignal)
	{
		const bool swap = m_swapChannelsModel.value();
		const float phase = m_phaseInvertModel.value() ? -1.f : 1.f;
		// The master's width is applied by its stereo section, before the bus compressor
		const float width = isMaster() ? 1.f : m_widthModel.value() / 100.f;
		// The master has no panning
		const auto pan = panningToVolumeVector(static_cast<panning_t>(isMaster() ? PanningCenter : m_panModel.value()));
		const float volume = m_volumeModel.value();

		for (f_cnt_t f = 0; f < fpp; ++f)
		{
			float left = buffer[f].left() * phase;
			float right = buffer[f].right() * phase;
			if (swap) { std::swap(left, right); }
			if (width != 1.f)
			{
				const float mid = (left + right) * 0.5f;
				const float side = (left - right) * 0.5f * width;
				left = mid + side;
				right = mid - side;
			}
			const float gain = volBuf ? volBuf->values()[f] : volume;
			buffer[f].setLeft(left * pan.vol[0] * gain);
			buffer[f].setRight(right * pan.vol[1] * gain);
		}
		toPlanar(m_buffer.interleavedBuffer(), m_buffer.groupBuffers(0));
	}

	// Analog mode: the master's output transformer, after its fader
	if (isMaster() && hasSignal && Engine::mixer()->isAnalog()
		&& Engine::mixer()->masterSection().m_transformerModel.value())
	{
		Engine::mixer()->masterSection().processTransformer(buffer, Engine::audioEngine()->outputSampleRate());
		toPlanar(m_buffer.interleavedBuffer(), m_buffer.groupBuffers(0));
	}
	else if (isMaster()) { Engine::mixer()->masterSection().bypassTransformer(); }

	if (hasSignal) { m_meter.measure(buffer.data(), fpp, Engine::audioEngine()->outputSampleRate()); }
	else { m_meter.measureSilence(); }
	if (isMaster() && hasSignal)
	{
		Engine::mixer()->masterSection().measure(buffer, Engine::audioEngine()->outputSampleRate());
	}
	else if (isMaster()) { Engine::mixer()->masterSection().measureSilence(); }

	// increment dependency counter of all receivers
	processed();
}



Mixer::Mixer() :
	Model( nullptr ),
	JournallingObject(),
	m_mixerChannels(),
	m_analogModel(true, this, tr("Analog mixer")),
	m_toleranceModel(true, this, tr("Channel tolerances")),
	m_masterSection(this)
{
	// create master channel
	createChannel();
}



Mixer::~Mixer()
{
	while (!m_mixerRoutes.empty())
	{
		deleteChannelSend(m_mixerRoutes.front());
	}
	while( m_mixerChannels.size() )
	{
		MixerChannel * f = m_mixerChannels[m_mixerChannels.size() - 1];
		m_mixerChannels.pop_back();
		delete f;
	}
}



namespace
{

//! Tracks that send to a mixer channel, with the model holding the channel number
auto tracksWithMixerChannel() -> std::vector<std::pair<Track*, IntModel*>>
{
	auto result = std::vector<std::pair<Track*, IntModel*>>{};
	for (const auto container : std::initializer_list<TrackContainer*>{Engine::getSong(), Engine::patternStore()})
	{
		if (!container) { continue; }
		for (const auto track : container->tracks())
		{
			if (auto instrumentTrack = dynamic_cast<InstrumentTrack*>(const_cast<Track*>(track)))
			{
				result.emplace_back(track, instrumentTrack->mixerChannelModel());
			}
			else if (auto sampleTrack = dynamic_cast<SampleTrack*>(track))
			{
				result.emplace_back(track, sampleTrack->mixerChannelModel());
			}
			else if (auto folderTrack = dynamic_cast<FolderTrack*>(track))
			{
				result.emplace_back(track, folderTrack->mixerChannelModel());
			}
		}
	}
	return result;
}

} // namespace




QDomElement Mixer::saveState(QDomDocument& doc, QDomElement& parent)
{
	auto element = JournallingObject::saveState(doc, parent);

	// Only the undo history needs the tracks' channels; projects store them with the tracks
	// (DataFile records its type on the root element)
	const bool forUndo = doc.documentElement().attribute("type") == "journaldata";
	if (!element.isNull() && forUndo)
	{
		auto routing = doc.createElement("trackchannels");
		for (const auto& [track, channel] : tracksWithMixerChannel())
		{
			auto entry = doc.createElement("track");
			entry.setAttribute("id", track->id());
			entry.setAttribute("channel", channel->value());
			routing.appendChild(entry);
		}
		element.appendChild(routing);
	}
	return element;
}




void Mixer::restoreState(const QDomElement& element)
{
	JournallingObject::restoreState(element);

	const auto routing = element.firstChildElement("trackchannels");
	if (routing.isNull()) { return; }

	const auto tracks = tracksWithMixerChannel();
	for (auto entry = routing.firstChildElement("track"); !entry.isNull(); entry = entry.nextSiblingElement("track"))
	{
		const auto id = static_cast<jo_id_t>(entry.attribute("id").toInt());
		const auto it = std::find_if(tracks.begin(), tracks.end(), [id](const auto& t) { return t.first->id() == id; });
		if (it != tracks.end()) { it->second->setValue(entry.attribute("channel").toInt()); }
	}

	// The tracks' names and colors are not in the undo history, so their channels keep following them
	followTracks();
}




Mixer::StructureChange::StructureChange(Mixer& mixer)
	: m_mixer(mixer)
{
	if (m_mixer.m_structureChangeDepth++ > 0) { return; }

	// The mixer's checkpoint covers the whole change, including the tracks' channels, so other objects
	// changed along the way (e.g. renumbered tracks) must not add checkpoints of their own
	m_mixer.addJournalCheckPoint();
	const auto journal = Engine::projectJournal();
	m_wasJournalling = journal->isJournalling();
	journal->setJournalling(false);
}




Mixer::StructureChange::~StructureChange()
{
	if (--m_mixer.m_structureChangeDepth > 0) { return; }
	Engine::projectJournal()->setJournalling(m_wasJournalling);
}




void Mixer::setChannelName(mix_ch_t index, const QString& name)
{
	if (index >= numChannels() || m_mixerChannels[index]->m_name == name) { return; }
	if (const auto track = trackOf(index))
	{
		track->setName(name);
		return;
	}
	const auto change = StructureChange{*this};
	m_mixerChannels[index]->m_name = name;
	emit dataChanged();
}




void Mixer::setChannelColor(mix_ch_t index, const std::optional<QColor>& color)
{
	if (index >= numChannels() || m_mixerChannels[index]->color() == color) { return; }
	if (const auto track = trackOf(index))
	{
		track->setColor(color);
		return;
	}
	const auto change = StructureChange{*this};
	m_mixerChannels[index]->setColor(color);
	emit dataChanged();
}




auto Mixer::trackOf(mix_ch_t index) const -> Track*
{
	if (index <= 0 || index >= numChannels() || !m_mixerChannels[index]->m_trackChannel) { return nullptr; }
	for (const auto& [track, channel] : tracksWithMixerChannel())
	{
		if (channel->value() == index && trackChannel(track) == m_mixerChannels[index]) { return track; }
	}
	return nullptr;
}




void Mixer::followTracks()
{
	for (const auto& [track, channel] : tracksWithMixerChannel())
	{
		if (const auto own = trackChannel(track))
		{
			own->m_name = track->name();
			own->setColor(track->color());
		}
	}
	emit dataChanged();
}




void Mixer::setChannelInput(mix_ch_t index, const ChannelInput& input)
{
	auto channel = m_mixerChannels[index];
	if (channel->m_input == input) { return; }
	const auto change = StructureChange{*this};
	channel->m_input = input;
	emit channelInputChanged(index);
}




int Mixer::createChannel()
{
	const auto change = StructureChange{*this};
	const int index = m_mixerChannels.size();
	// create new channel
	m_mixerChannels.push_back( new MixerChannel( index, this ) );

	// reset channel state
	clearChannel( index );

	// Like every channel, it sends to the effect returns, with nothing at first
	for (const auto fxReturn : fxReturns()) { createChannelSend(index, fxReturn->index(), 0.f); }

	emit channelCreated(index);
	return index;
}

void Mixer::deleteChannel( int index )
{
	const auto change = StructureChange{*this};
	// channel deletion is performed between mixer rounds
	Engine::audioEngine()->requestChangeInModel();

	MixerChannel * ch = m_mixerChannels[index];

	// delete all of this channel's sends and receives
	while (!ch->m_sends.empty())
	{
		deleteChannelSend(ch->m_sends.front());
	}
	while (!ch->m_receives.empty())
	{
		deleteChannelSend(ch->m_receives.front());
	}

	// Channels made for it stay as plain channels
	for (const auto channel : m_mixerChannels)
	{
		if (channel->m_owner == ch) { channel->m_owner = nullptr; }
	}

	// actually delete the channel
	m_mixerChannels.erase(m_mixerChannels.begin() + index);
	delete ch;

	for (auto i = static_cast<std::size_t>(index); i < m_mixerChannels.size(); ++i)
	{
		validateChannelName( i, i + 1 );

		// set correct channel index
		m_mixerChannels[i]->setIndex(i);

		// now check all routes and update names of the send models
		for( MixerRoute * r : m_mixerChannels[i]->m_sends )
		{
			r->updateName();
		}
		for( MixerRoute * r : m_mixerChannels[i]->m_receives )
		{
			r->updateName();
		}
	}

	emit channelDeleted(index);

	Engine::audioEngine()->doneChangeInModel();
}



void Mixer::moveChannelLeft( int index )
{
	const auto change = StructureChange{*this};
	// can't move master or first channel
	if (index <= 1 || static_cast<std::size_t>(index) >= m_mixerChannels.size()) { return; }

	// Swap positions in array
	Engine::audioEngine()->requestChangeInModel();
	std::swap(m_mixerChannels[index], m_mixerChannels[index - 1]);

	// Update m_channelIndex of both channels
	m_mixerChannels[index]->setIndex(index);
	m_mixerChannels[index - 1]->setIndex(index - 1);
	Engine::audioEngine()->doneChangeInModel();

	emit channelsSwapped(index, index - 1);
}



void Mixer::moveChannelRight( int index )
{
	moveChannelLeft( index + 1 );
}



MixerRoute * Mixer::createChannelSend( mix_ch_t fromChannel, mix_ch_t toChannel,
								float amount )
{
	return createChannelSend(fromChannel, toChannel, amount, 0, 0);
}


MixerRoute* Mixer::createChannelSend(mix_ch_t fromChannel, mix_ch_t toChannel, float amount,
	ch_cnt_t sourcePair, ch_cnt_t destinationPair)
{
	const auto change = StructureChange{*this};

	// find the existing connection, and simply adjust its amount
	if (auto send = findChannelSend(fromChannel, toChannel, sourcePair, destinationPair))
	{
		send->amount()->setValue(amount);
		return send;
	}

	// connection does not exist. create a new one
	return createRoute(m_mixerChannels[fromChannel], m_mixerChannels[toChannel], amount, sourcePair, destinationPair);
}


auto Mixer::findChannelSend(mix_ch_t fromChannel, mix_ch_t toChannel, ch_cnt_t sourcePair,
	ch_cnt_t destinationPair) const -> MixerRoute*
{
	const MixerChannel* to = m_mixerChannels[toChannel];
	for (MixerRoute* send : m_mixerChannels[fromChannel]->m_sends)
	{
		if (send->receiver() == to && send->sourcePair() == sourcePair && send->destinationPair() == destinationPair)
		{
			return send;
		}
	}
	return nullptr;
}


MixerRoute* Mixer::createRoute(MixerChannel* from, MixerChannel* to, float amount, ch_cnt_t sourcePair,
	ch_cnt_t destinationPair)
{
	if( from == to )
	{
		return nullptr;
	}
	if (sourcePair >= from->pairCount() || destinationPair >= to->pairCount()) { return nullptr; }

	Engine::audioEngine()->requestChangeInModel();
	auto route = new MixerRoute(from, to, amount, sourcePair, destinationPair);

	// add us to from's sends
	from->m_sends.push_back(route);

	// add us to to's receives
	to->m_receives.push_back(route);

	// add us to mixer's list
	Engine::mixer()->m_mixerRoutes.push_back(route);
	Engine::audioEngine()->doneChangeInModel();

	return route;
}


// delete the connection made by createChannelSend
void Mixer::deleteChannelSend( mix_ch_t fromChannel, mix_ch_t toChannel )
{
	const auto change = StructureChange{*this};

	// the main pair to main pair send
	if (auto send = findChannelSend(fromChannel, toChannel)) { deleteChannelSend(send); }
}


void Mixer::deleteChannelSend( MixerRoute * route )
{
	const auto change = StructureChange{*this};
	Engine::audioEngine()->requestChangeInModel();

	auto removeFromMixerRoute = [route](MixerRouteVector& routeVec)
	{
		auto it = std::find(routeVec.begin(), routeVec.end(), route);
		if (it != routeVec.end()) { routeVec.erase(it); }
	};

	// remove us from from's sends
	removeFromMixerRoute(route->sender()->m_sends);

	// remove us from to's receives
	removeFromMixerRoute(route->receiver()->m_receives);

	// remove us from mixer's list
	removeFromMixerRoute(Engine::mixer()->m_mixerRoutes);

	delete route;
	Engine::audioEngine()->doneChangeInModel();
}


bool Mixer::isInfiniteLoop( mix_ch_t sendFrom, mix_ch_t sendTo )
{
	if( sendFrom == sendTo ) return true;
	MixerChannel * from = m_mixerChannels[sendFrom];
	MixerChannel * to = m_mixerChannels[sendTo];
	bool b = checkInfiniteLoop( from, to );
	return b;
}


bool Mixer::checkInfiniteLoop( MixerChannel * from, MixerChannel * to )
{
	// can't send master to anything
	if( from == m_mixerChannels[0] )
	{
		return true;
	}

	// can't send channel to itself
	if( from == to )
	{
		return true;
	}

	// follow sendTo's outputs recursively looking for something that sends
	// to sendFrom
	for (const auto& send : to->m_sends)
	{
		if (checkInfiniteLoop(from, send->receiver()))
		{
			return true;
		}
	}

	return false;
}


// how much does fromChannel send its output to the input of toChannel?
FloatModel * Mixer::channelSendModel( mix_ch_t fromChannel, mix_ch_t toChannel )
{
	if( fromChannel == toChannel )
	{
		return nullptr;
	}
	const auto route = findChannelSend(fromChannel, toChannel);
	return route ? route->amount() : nullptr;
}




void Mixer::setChannelCount(mix_ch_t index, ch_cnt_t channels)
{
	channels = std::clamp<ch_cnt_t>(static_cast<ch_cnt_t>((channels + 1) / 2 * 2), DEFAULT_CHANNELS,
		MaxMixerChannelTrackChannels);
	MixerChannel* channel = m_mixerChannels[index];
	if (channel->channelCount() == channels) { return; }

	const auto change = StructureChange{*this};

	// Sends from or to pairs the channel will not have anymore
	const auto pairs = static_cast<ch_cnt_t>(channels / 2);
	auto obsolete = std::vector<MixerRoute*>{};
	for (MixerRoute* send : channel->m_sends)
	{
		if (send->sourcePair() >= pairs) { obsolete.push_back(send); }
	}
	for (MixerRoute* receive : channel->m_receives)
	{
		if (receive->destinationPair() >= pairs) { obsolete.push_back(receive); }
	}
	for (MixerRoute* route : obsolete) { deleteChannelSend(route); }

	Engine::audioEngine()->requestChangeInModel();
	channel->setChannelCount(channels);
	Engine::audioEngine()->doneChangeInModel();

	emit channelCountChanged(index);
}



void Mixer::mixToChannel(const AudioBuffer& buffer, mix_ch_t dest)
{
	const auto channel = m_mixerChannels[dest];
	if (!channel->m_muteModel.value())
	{
		channel->m_lock.lock();
		MixHelpers::add(channel->m_buffer.groupBuffers(0), buffer.groupBuffers(0));

		// Copy the planar buffer to the temporary interleaved buffer so they stay in sync
		toInterleaved(channel->m_buffer.groupBuffers(0), channel->m_buffer.interleavedBuffer());

		channel->m_buffer.mixSilenceFlags(buffer);
		channel->m_lock.unlock();
	}
}




void Mixer::mixAuxiliaryToChannel(const AudioBuffer& buffer, mix_ch_t dest)
{
	const auto channel = m_mixerChannels[dest];
	if (channel->m_muteModel.value()) { return; }

	const auto channels = std::min(buffer.channels(), channel->channelCount());
	channel->m_lock.lock();
	for (ch_cnt_t ch = DEFAULT_CHANNELS; ch < channels; ++ch)
	{
		if (buffer.silenceFlags()[ch]) { continue; }
		const auto src = buffer.buffer(ch);
		auto dst = channel->m_buffer.buffer(ch);
		std::transform(dst.begin(), dst.end(), src.begin(), dst.begin(), std::plus{});
		channel->m_buffer.assumeNonSilent(ch);
	}
	channel->m_lock.unlock();
}




void Mixer::prepareMasterMix()
{
	m_mixerChannels[0]->m_buffer.silenceAllChannels();
}



void Mixer::addLatencySource(const IntModel* channel, const std::atomic<f_cnt_t>* latency)
{
	Engine::audioEngine()->requestChangeInModel();
	m_latencySources.push_back({channel, latency});
	Engine::audioEngine()->doneChangeInModel();
}




void Mixer::removeLatencySource(const std::atomic<f_cnt_t>* latency)
{
	Engine::audioEngine()->requestChangeInModel();
	std::erase_if(m_latencySources, [latency](const LatencySource& source) { return source.latency == latency; });
	Engine::audioEngine()->doneChangeInModel();
}




void Mixer::applySolo()
{
	bool anySoloed = false;
	for (const auto channel : m_mixerChannels) { anySoloed = anySoloed || channel->m_soloModel.value(); }
	if (!anySoloed)
	{
		for (const auto channel : m_mixerChannels) { channel->m_silencedBySolo = false; }
		for (const auto route : m_mixerRoutes) { route->setLive(true); }
		return;
	}

	constexpr std::uint8_t Heard = 1;
	constexpr std::uint8_t Processed = 2;
	m_soloState.assign(m_mixerChannels.size(), 0);

	// From each soloed channel, along the sends into main inputs: downstream (to the master) or upstream (its
	// sources), marking the channels with @p flag
	auto spread = [this](MixerChannel* from, bool downstream, std::uint8_t flag) {
		m_soloStack.clear();
		m_soloStack.push_back(from);
		while (!m_soloStack.empty())
		{
			const auto channel = m_soloStack.back();
			m_soloStack.pop_back();
			for (const MixerRoute* route : downstream ? channel->m_sends : channel->m_receives)
			{
				if (route->isSidechain() && flag == Heard) { continue; }
				const auto next = downstream ? route->receiver() : route->sender();
				if (m_soloState[next->index()] & flag) { continue; }
				m_soloState[next->index()] |= flag;
				m_soloStack.push_back(next);
			}
		}
	};
	for (const auto channel : m_mixerChannels)
	{
		if (!channel->m_soloModel.value()) { continue; }
		m_soloState[channel->index()] |= Heard;
		spread(channel, true, Heard);
		spread(channel, false, Heard);
	}
	m_soloState[0] |= Heard;

	// Whatever is heard is processed, and so are the sources of the sidechain inputs of what is heard
	for (auto& state : m_soloState)
	{
		if (state & Heard) { state |= Processed; }
	}
	for (const auto route : m_mixerRoutes)
	{
		if (!route->isSidechain() || !(m_soloState[route->receiverIndex()] & Heard)) { continue; }
		const auto source = route->sender();
		if (m_soloState[source->index()] & Processed) { continue; }
		m_soloState[source->index()] |= Processed;
		spread(source, false, Processed);
	}

	for (const auto channel : m_mixerChannels)
	{
		channel->m_silencedBySolo = !(m_soloState[channel->index()] & Processed);
	}
	// Into what is heard, only from what is heard, or into sidechain inputs
	for (const auto route : m_mixerRoutes)
	{
		const auto from = m_soloState[route->senderIndex()];
		const auto to = m_soloState[route->receiverIndex()];
		route->setLive((from & Processed) && (to & Processed) && (!(to & Heard) || (from & Heard) || route->isSidechain()));
	}
}




void Mixer::compensateLatency()
{
	// The latest of the tracks' audio of each channel
	for (const auto channel : m_mixerChannels) { channel->m_trackLatency = 0; }
	for (const auto& source : m_latencySources)
	{
		const int index = source.channel->value();
		if (index < 0 || index >= numChannels()) { continue; }
		auto& latest = m_mixerChannels[index]->m_trackLatency;
		latest = std::max(latest, source.latency->load(std::memory_order_relaxed));
	}

	m_latencyPass.assign(m_mixerChannels.size(), 0);
	for (auto index = std::size_t{0}; index < m_mixerChannels.size(); ++index)
	{
		compensateLatency(static_cast<mix_ch_t>(index));
	}
}




auto Mixer::compensateLatency(mix_ch_t index) -> f_cnt_t
{
	MixerChannel* channel = m_mixerChannels[index];
	if (m_latencyPass[index] == 2) { return channel->m_latency; }
	if (m_latencyPass[index] == 1) { return 0; } // a loop, which the mixer does not allow
	m_latencyPass[index] = 1;

	// The latest input
	f_cnt_t latest = channel->m_trackLatency;
	for (const MixerRoute* route : channel->m_receives)
	{
		latest = std::max(latest, compensateLatency(route->senderIndex()));
	}

	channel->m_inputDelay.setDelay(latest - channel->m_trackLatency, channel->channelCount());
	for (MixerRoute* route : channel->m_receives)
	{
		route->latencyDelay().setDelay(latest - compensateLatency(route->senderIndex()), 2);
	}

	channel->m_latency = latest + channel->m_fxChain.latency();
	m_latencyPass[index] = 2;
	return channel->m_latency;
}




auto Mixer::latency() const -> f_cnt_t
{
	return m_mixerChannels.empty() ? 0 : m_mixerChannels[0]->m_latency;
}




void Mixer::masterMix( SampleFrame* _buf )
{
	const int fpp = Engine::audioEngine()->framesPerPeriod();

	compensateLatency();
	applySolo();

	// add the channels that have no dependencies (no incoming senders, ie.
	// no receives) to the jobqueue. The channels that have receives get
	// added when their senders get processed, which is detected by
	// dependency counting.
	// also instantly add all muted channels as they don't need to care
	// about their senders, and can just increment the deps of their
	// recipients right away.
	AudioEngineWorkerThread::resetJobQueue( AudioEngineWorkerThread::JobQueue::OperationMode::Dynamic );
	for( MixerChannel * ch : m_mixerChannels )
	{
		ch->m_muted = ch->m_muteModel.value() || ch->m_silencedBySolo;
		if( ch->m_muted ) // instantly "process" muted channels
		{
			// Silenced by solo, the channel may hold its tracks' audio, which must not reach anything
			if (ch->m_silencedBySolo) { ch->m_buffer.silenceAllChannels(); }
			ch->processMuted();
			ch->processed();
			ch->done();
		}
		else if( ch->m_receives.size() == 0 )
		{
			ch->m_queued = true;
			AudioEngineWorkerThread::addJob( ch );
		}
	}
	// A channel is queued while the last of its senders is being processed (see MixerChannel::incrementDeps()), and
	// waiting for the jobs lasts until every queued job is done, including those queued meanwhile: afterwards every
	// channel is done, the master too. (Looking at the channels' states one after the other instead, as before,
	// raced with the workers: a channel looked at early could be queued while the others were looked at, the scan
	// found nothing in flight and gave up on the master, and the period came out silent.)
	AudioEngineWorkerThread::startAndWaitForJobs();

	// The master channel already applied its fader when it was processed; the monitoring changes only what is
	// heard, not what is exported
	auto buffer = m_mixerChannels[0]->m_buffer.interleavedBuffer().asSampleFrames();
	if (!Engine::getSong() || !Engine::getSong()->isExporting()) { m_masterSection.processMonitoring(buffer); }
	MixHelpers::add(_buf, buffer.data(), fpp);

	// clear all channel buffers and
	// reset channel process state
	for( int i = 0; i < numChannels(); ++i)
	{
		m_mixerChannels[i]->m_buffer.silenceAllChannels();
		m_mixerChannels[i]->reset();
		m_mixerChannels[i]->m_queued = false;
		m_mixerChannels[i]->m_dependenciesMet = 0;
	}
}




void Mixer::clear()
{
	m_masterSection.setDefaults();

	while( m_mixerChannels.size() > 1 )
	{
		deleteChannel(1);
	}

	clearChannel(0);
}



void Mixer::clearChannel(mix_ch_t index)
{
	MixerChannel * ch = m_mixerChannels[index];
	ch->m_fxChain.clear();
	ch->m_volumeModel.setValue( 1.0f );
	ch->m_muteModel.setValue( false );
	ch->m_soloModel.setValue( false );
	ch->m_trimModel.setValue(0.f);
	ch->m_monoModel.setValue(false);
	ch->m_swapChannelsModel.setValue(false);
	ch->m_phaseInvertModel.setValue(false);
	ch->m_widthModel.setValue(100.f);
	ch->m_panModel.setValue(PanningCenter);
	ch->m_analogStrip.setNeutral();
	ch->m_input = ChannelInput{};
	ch->m_trackChannel = false;
	ch->m_groupChannel = false;
	ch->m_fxReturn = false;
	ch->m_owner = nullptr;
	setChannelCount(index, DEFAULT_CHANNELS);
	ch->m_recordArmModel.setValue(false);
	ch->m_monitorModel.setValue(static_cast<int>(MixerChannel::MonitorMode::Auto));
	ch->m_name = ( index == 0 ) ? tr( "Master" ) : tr( "Channel %1" ).arg( index );
	ch->m_volumeModel.setDisplayName( ch->m_name + ">" + tr( "Volume" ) );
	ch->m_muteModel.setDisplayName( ch->m_name + ">" + tr( "Mute" ) );
	ch->m_soloModel.setDisplayName( ch->m_name + ">" + tr( "Solo" ) );
	ch->m_trimModel.setDisplayName(ch->m_name + ">" + tr("Trim"));
	ch->m_monoModel.setDisplayName(ch->m_name + ">" + tr("Mono"));
	ch->m_swapChannelsModel.setDisplayName(ch->m_name + ">" + tr("Swap left/right"));
	ch->m_phaseInvertModel.setDisplayName(ch->m_name + ">" + tr("Invert phase"));
	ch->m_widthModel.setDisplayName(ch->m_name + ">" + tr("Stereo width"));
	ch->m_panModel.setDisplayName(ch->m_name + ">" + tr("Panning"));
	ch->m_recordArmModel.setDisplayName(ch->m_name + ">" + tr("Record arm"));
	ch->m_monitorModel.setDisplayName(ch->m_name + ">" + tr("Input monitoring"));
	ch->m_analogStrip.setDisplayNames(ch->m_name);
	ch->setColor(std::nullopt);

	// send only to master
	if( index > 0)
	{
		// delete existing sends
		while (!ch->m_sends.empty())
		{
			deleteChannelSend(ch->m_sends.front());
		}

		// add send to master
		createChannelSend( index, 0 );
	}

	// delete receives
	while (!ch->m_receives.empty())
	{
		deleteChannelSend(ch->m_receives.front());
	}
}

void Mixer::saveSettings( QDomDocument & _doc, QDomElement & _this )
{
	m_analogModel.saveSettings(_doc, _this, "analog");
	m_toleranceModel.saveSettings(_doc, _this, "tolerance");
	m_masterSection.saveSettings(_doc, _this);

	// save channels
	for (auto i = std::size_t{0}; i < m_mixerChannels.size(); ++i)
	{
		MixerChannel * ch = m_mixerChannels[i];

		QDomElement mixch = _doc.createElement( QString( "mixerchannel" ) );
		_this.appendChild( mixch );

		ch->m_fxChain.saveState( _doc, mixch );
		ch->m_volumeModel.saveSettings( _doc, mixch, "volume" );
		ch->m_muteModel.saveSettings( _doc, mixch, "muted" );
		ch->m_soloModel.saveSettings( _doc, mixch, "soloed" );
		ch->m_trimModel.saveSettings(_doc, mixch, "trim");
		ch->m_monoModel.saveSettings(_doc, mixch, "mono");
		ch->m_swapChannelsModel.saveSettings(_doc, mixch, "swapchannels");
		ch->m_phaseInvertModel.saveSettings(_doc, mixch, "phaseinvert");
		ch->m_widthModel.saveSettings(_doc, mixch, "width");
		ch->m_panModel.saveSettings(_doc, mixch, "pan");
		ch->m_recordArmModel.saveSettings(_doc, mixch, "recordarm");
		ch->m_monitorModel.saveSettings(_doc, mixch, "monitor");
		if (ch->m_input.source == ChannelInput::Source::Hardware)
		{
			auto input = _doc.createElement("input");
			input.setAttribute("source", "hardware");
			input.setAttribute("channel", ch->m_input.firstChannel);
			input.setAttribute("channels", ch->m_input.channelCount);
			mixch.appendChild(input);
		}
		mixch.setAttribute("num", static_cast<qulonglong>(i));
		if (ch->channelCount() != DEFAULT_CHANNELS) { mixch.setAttribute("channels", ch->channelCount()); }
		if (i > 0) { ch->m_analogStrip.saveSettings(_doc, mixch); }
		if (ch->m_trackChannel) { mixch.setAttribute("trackchannel", 1); }
		if (ch->m_groupChannel) { mixch.setAttribute("groupchannel", 1); }
		if (ch->m_fxReturn) { mixch.setAttribute("fxreturn", 1); }
		if (ch->m_owner) { mixch.setAttribute("owner", ch->m_owner->index()); }
		mixch.setAttribute( "name", ch->m_name );
		if (const auto& color = ch->color()) { mixch.setAttribute("color", color->name()); }

		// add the channel sends
		for (const auto& send : ch->m_sends)
		{
			QDomElement sendsDom = _doc.createElement( QString( "send" ) );
			mixch.appendChild( sendsDom );

			sendsDom.setAttribute("channel", send->receiverIndex());
			if (send->sourcePair() != 0) { sendsDom.setAttribute("sourcepair", send->sourcePair()); }
			if (send->destinationPair() != 0) { sendsDom.setAttribute("destinationpair", send->destinationPair()); }
			send->amount()->saveSettings(_doc, sendsDom, "amount");
			send->preFader()->saveSettings(_doc, sendsDom, "prefader");
		}
	}
}

// make sure we have at least num channels
void Mixer::allocateChannelsTo(int num)
{
	if (num <= 0) { return; }
	while (static_cast<std::size_t>(num) > m_mixerChannels.size() - 1)
	{
		createChannel();

		// delete the default send to master
		deleteChannelSend( m_mixerChannels.size()-1, 0 );
	}
}


void Mixer::loadSettings( const QDomElement & _this )
{
	clear();
	// Projects from before the modes load in analog mode, whose strips start neutral: they sound the same
	m_analogModel.setValue(true);
	if (_this.hasAttribute("analog") || !_this.firstChildElement("analog").isNull())
	{
		m_analogModel.loadSettings(_this, "analog");
	}
	// Projects from before the tolerances load without them
	m_loadedWithoutTolerance = !_this.hasAttribute("tolerance") && _this.firstChildElement("tolerance").isNull();
	if (m_loadedWithoutTolerance) { m_toleranceModel.setValue(false); }
	else { m_toleranceModel.loadSettings(_this, "tolerance"); }
	m_masterSection.loadSettings(_this);
	QDomNode node = _this.firstChild();
	auto sends = std::vector<std::pair<int, QDomElement>>{};
	auto owners = std::vector<std::pair<int, int>>{};

	while( ! node.isNull() )
	{
		QDomElement mixch = node.toElement();
		if (mixch.nodeName() != "mixerchannel")
		{
			node = node.nextSibling();
			continue;
		}

		// index of the channel we are about to load
		int num = mixch.attribute( "num" ).toInt();

		// allocate enough channels
		allocateChannelsTo( num );

		m_mixerChannels[num]->m_volumeModel.loadSettings( mixch, "volume" );
		m_mixerChannels[num]->m_muteModel.loadSettings( mixch, "muted" );
		m_mixerChannels[num]->m_soloModel.loadSettings( mixch, "soloed" );
		m_mixerChannels[num]->m_trimModel.loadSettings(mixch, "trim");
		m_mixerChannels[num]->m_monoModel.loadSettings(mixch, "mono");
		m_mixerChannels[num]->m_swapChannelsModel.loadSettings(mixch, "swapchannels");
		m_mixerChannels[num]->m_phaseInvertModel.loadSettings(mixch, "phaseinvert");
		m_mixerChannels[num]->m_widthModel.loadSettings(mixch, "width");
		m_mixerChannels[num]->m_panModel.loadSettings(mixch, "pan");
		m_mixerChannels[num]->m_recordArmModel.loadSettings(mixch, "recordarm");
		m_mixerChannels[num]->m_monitorModel.loadSettings(mixch, "monitor");
		m_mixerChannels[num]->m_analogStrip.loadSettings(mixch);
		if (const auto input = mixch.firstChildElement("input");
			!input.isNull() && input.attribute("source") == "hardware")
		{
			setChannelInput(num, ChannelInput{ChannelInput::Source::Hardware,
				input.attribute("channel").toInt(), std::clamp(input.attribute("channels", "2").toInt(), 1, 2)});
		}
		m_mixerChannels[num]->m_name = mixch.attribute( "name" );
		m_mixerChannels[num]->m_trackChannel = mixch.attribute("trackchannel", "0").toInt() != 0;
		m_mixerChannels[num]->m_groupChannel = mixch.attribute("groupchannel", "0").toInt() != 0;
		m_mixerChannels[num]->m_fxReturn = mixch.attribute("fxreturn", "0").toInt() != 0;
		if (mixch.hasAttribute("owner")) { owners.emplace_back(num, mixch.attribute("owner").toInt()); }
		if (mixch.hasAttribute("color"))
		{
			m_mixerChannels[num]->setColor(QColor{mixch.attribute("color")});
		}

		// Before the effects, so that their pins to auxiliary channels can be loaded
		setChannelCount(num, static_cast<ch_cnt_t>(mixch.attribute("channels", "2").toInt()));

		m_mixerChannels[num]->m_fxChain.restoreState( mixch.firstChildElement(
			m_mixerChannels[num]->m_fxChain.nodeName() ) );

		// mixer sends, created once all channels have their channel counts (a sidechain send needs the
		// receiver's auxiliary pair)
		QDomNodeList chData = mixch.childNodes();
		for (auto i = 0; i < chData.length(); ++i)
		{
			QDomElement chDataItem = chData.at(i).toElement();
			if (chDataItem.nodeName() == QString("send")) { sends.emplace_back(num, chDataItem); }
		}

		node = node.nextSibling();
	}

	for (const auto& [num, owner] : owners)
	{
		if (owner > 0 && owner < numChannels() && owner != num) { m_mixerChannels[num]->m_owner = m_mixerChannels[owner]; }
	}

	for (const auto& [num, send] : sends)
	{
		const int sendTo = send.attribute("channel").toInt();
		allocateChannelsTo(sendTo);
		MixerRoute* mxr = createChannelSend(num, sendTo, 1.0f,
			static_cast<ch_cnt_t>(send.attribute("sourcepair", "0").toInt()),
			static_cast<ch_cnt_t>(send.attribute("destinationpair", "0").toInt()));
		if (mxr)
		{
			mxr->amount()->loadSettings(send, "amount");
			mxr->preFader()->loadSettings(send, "prefader");
		}
	}

	// Projects from before the effect returns get them
	ensureFxReturns();

	emit dataChanged();
}


void Mixer::validateChannelName( int index, int oldIndex )
{
	if( m_mixerChannels[index]->m_name == tr( "Channel %1" ).arg( oldIndex ) )
	{
		m_mixerChannels[index]->m_name = tr( "Channel %1" ).arg( index );
	}
}

bool Mixer::isChannelInUse(int index)
{
	assert(index >= 0 && index < m_mixerChannels.size());

	// The effect returns are always there
	if (m_mixerChannels[index]->m_fxReturn) { return true; }

	// check if the index mixer channel receives audio from any other channel
	if (!m_mixerChannels[index]->m_receives.empty()) { return true; }

	return m_mixerChannels[index]->useCount() > 0;
}


namespace
{

//! What an instrument, sample or folder track has of a mixer strip of its own (a folder only the channel)
struct TrackStrip
{
	IntModel* channel;
	FloatModel* volume;
	FloatModel* panning;
	AudioBusHandle* bus;
};

auto stripOf(const Track* track) -> std::optional<TrackStrip>
{
	if (auto instrumentTrack = dynamic_cast<InstrumentTrack*>(const_cast<Track*>(track)))
	{
		return TrackStrip{instrumentTrack->mixerChannelModel(), instrumentTrack->volumeModel(),
			instrumentTrack->panningModel(), instrumentTrack->audioBusHandle()};
	}
	if (auto sampleTrack = dynamic_cast<SampleTrack*>(const_cast<Track*>(track)))
	{
		return TrackStrip{sampleTrack->mixerChannelModel(), sampleTrack->volumeModel(), sampleTrack->panningModel(),
			sampleTrack->audioBusHandle()};
	}
	// A folder has only its group channel
	if (auto folderTrack = dynamic_cast<FolderTrack*>(const_cast<Track*>(track)))
	{
		return TrackStrip{folderTrack->mixerChannelModel(), nullptr, nullptr, nullptr};
	}
	return std::nullopt;
}

} // namespace


int Mixer::setUpTrackChannel(Track* track)
{
	const auto strip = stripOf(track);
	if (!strip) { return -1; }

	const auto change = StructureChange{*this};

	// A channel of its own
	int index = strip->channel->value();
	if (index < 0 || index >= numChannels()) { index = 0; }
	const auto tracks = tracksWithMixerChannel();
	const auto users = std::count_if(tracks.begin(), tracks.end(), [index](const auto& entry) {
		return entry.second->value() == index;
	});
	// A bus (a channel other channels send to) is not a track's own channel either, unless made for it
	const bool bus = !m_mixerChannels[index]->m_trackChannel && !m_mixerChannels[index]->m_receives.empty();
	const bool shared = index == 0 || users > 1 || bus;
	if (!shared)
	{
		// The track's alone on it: it is the track's channel (e.g. in projects from before)
		m_mixerChannels[index]->m_trackChannel = true;
		m_mixerChannels[index]->m_groupChannel = track->type() == Track::Type::Folder;
	}
	else
	{
		const int sendTo = index;
		index = createChannel();
		auto channel = m_mixerChannels[index];
		channel->m_name = track->name();
		channel->setColor(track->color());
		channel->m_trackChannel = true;
		channel->m_groupChannel = track->type() == Track::Type::Folder;
		emit dataChanged();

		// Where the track sent before
		if (sendTo != 0)
		{
			deleteChannelSend(index, 0);
			createChannelSend(index, sendTo);
		}
		strip->channel->setValue(index);
	}
	auto channel = m_mixerChannels[index];

	// A channel made for the track follows its name and color
	connect(track, &Track::nameChanged, this, &Mixer::followTracks, Qt::UniqueConnection);
	connect(track, &Track::colorChanged, this, &Mixer::followTracks, Qt::UniqueConnection);

	// Mute and solo live on the channel. A mute automated or controlled stays on the track, where its automation acts.
	if (track->isMuted() && !track->getMutedModel()->isAutomatedOrControlled())
	{
		channel->m_muteModel.setValue(true);
		track->setMuted(false);
	}
	if (track->legacySolo())
	{
		channel->m_soloModel.setValue(true);
		track->clearLegacySolo();
	}

	// The track's effects go in front of the channel's
	if (auto effects = strip->bus ? strip->bus->effects() : nullptr; effects && !effects->effects().empty())
	{
		auto doc = DataFile{DataFile::Type::EffectSettings};
		auto saved = effects->saveState(doc, doc.content());
		channel->m_fxChain.prependEffects(saved);
		effects->clear();
	}

	// The track keeps its own volume and panning, on the source before the channel (the track is the source, the
	// channel the mix)

	emit trackChannelChanged(track);
	return index;
}




void Mixer::setUpTrackChannels()
{
	for (const auto& [track, channel] : tracksWithMixerChannel()) { setUpTrackChannel(track); }
}




void Mixer::releaseTrackChannel(Track* track)
{
	const auto strip = stripOf(track);
	if (!strip) { return; }

	const int index = strip->channel->value();
	if (index <= 0 || index >= numChannels() || !m_mixerChannels[index]->m_trackChannel) { return; }

	// Only if no other track sends to it and no other channel either (a folder's tracks send to its group channel,
	// but they go to the folder above once the folder is gone)
	const auto tracks = tracksWithMixerChannel();
	const auto users = std::count_if(tracks.begin(), tracks.end(), [index](const auto& entry) {
		return entry.second->value() == index;
	});
	if (users > 1 || (!m_mixerChannels[index]->m_groupChannel && !m_mixerChannels[index]->m_receives.empty()))
	{
		return;
	}

	const auto change = StructureChange{*this};
	strip->channel->setValue(0);

	// The channels made for it, e.g. its drum pieces, go with it
	const auto channel = m_mixerChannels[index];
	for (const auto owned : ownedChannels(index)) { deleteChannel(owned->index()); }
	deleteChannel(channel->index());
	emit trackChannelChanged(track);
}




auto Mixer::toggleSidechainSend(mix_ch_t from, mix_ch_t to) -> bool
{
	if (from == to || from >= numChannels() || to >= numChannels()) { return false; }

	if (const auto existing = findChannelSend(from, to, 0, 1))
	{
		deleteChannelSend(existing);
		return false;
	}
	if (isInfiniteLoop(from, to)) { return false; }

	const auto change = StructureChange{*this};
	if (m_mixerChannels[to]->channelCount() < 4) { setChannelCount(to, 4); }
	return createChannelSend(from, to, 1.f, 0, 1) != nullptr;
}




auto Mixer::setUpChannelPerOutput(InstrumentTrack* track) -> int
{
	const auto instance = track->instrumentInstance();
	const auto groups = instance ? instance->outputGroups() : std::vector<PluginInstance::OutputGroup>{};
	if (groups.size() < 2) { return -1; }

	// Done already
	if (const auto channel = trackChannel(track))
	{
		for (const auto owned : ownedChannels(channel->index()))
		{
			if (owned->m_groupChannel) { return owned->index(); }
		}
	}

	const auto change = StructureChange{*this};

	if (!trackChannel(track)) { setUpTrackChannel(track); }
	const auto trackIndex = static_cast<mix_ch_t>(track->mixerChannelModel()->value());
	auto source = m_mixerChannels[trackIndex];

	// Where the track's channel sends its main output
	mix_ch_t destination = 0;
	for (const MixerRoute* route : source->m_sends)
	{
		if (route->sourcePair() == 0 && route->destinationPair() == 0) { destination = route->receiverIndex(); break; }
	}

	// A pair per group on the track's channel, which the pins route the groups to
	setChannelCount(trackIndex, static_cast<ch_cnt_t>(2 * (groups.size() + 1)));
	if (!instance->routeOutputGroupsToPairs()) { return -1; }

	const auto group = createChannel();
	auto groupChannel = m_mixerChannels[group];
	groupChannel->m_name = tr("%1 (pieces)").arg(track->name());
	groupChannel->setColor(track->color());
	groupChannel->m_groupChannel = true;
	groupChannel->m_owner = source;
	deleteChannelSend(group, 0);
	createChannelSend(group, destination);

	for (auto g = std::size_t{0}; g < groups.size(); ++g)
	{
		const auto piece = createChannel();
		auto pieceChannel = m_mixerChannels[piece];
		pieceChannel->m_name = groups[g].name.empty()
			? tr("%1 %2").arg(track->name()).arg(g + 1)
			: QString::fromStdString(groups[g].name);
		pieceChannel->setColor(track->color());
		pieceChannel->m_owner = source;
		createChannelSend(trackIndex, piece, 1.f, static_cast<ch_cnt_t>(g + 1), 0);
		deleteChannelSend(piece, 0);
		createChannelSend(piece, group);
	}

	// Next to the track's channel: the group channel, then the pieces
	const int first = source->index() + 1;
	const int made = static_cast<int>(groups.size()) + 1;
	for (int i = 0; i < made; ++i)
	{
		for (int at = numChannels() - made + i; at > first + i; --at) { moveChannelLeft(at); }
	}

	emit dataChanged();
	return groupChannel->index();
}




auto Mixer::channelOf(const EffectChain* chain) const -> int
{
	for (const auto channel : m_mixerChannels)
	{
		if (&channel->m_fxChain == chain) { return channel->index(); }
	}
	return -1;
}




void Mixer::updateFolderRouting()
{
	const auto song = Engine::getSong();
	if (!song) { return; }

	const auto journal = Engine::projectJournal();
	const bool wasJournalling = journal->isJournalling();
	journal->setJournalling(false);

	// Sends the main outputs of @p channel (sends of the main pair after the fader) to @p target
	auto routeTo = [this](MixerChannel* channel, mix_ch_t target) {
		bool reachesTarget = false;
		bool sentByHand = false;
		auto wrong = std::vector<mix_ch_t>{};
		for (const MixerRoute* route : channel->m_sends)
		{
			if (route->sourcePair() != 0 || route->destinationPair() != 0 || route->isPreFader()) { continue; }
			const auto receiver = route->receiver();
			if (receiver->m_fxReturn) { continue; } // effect sends, not the channel's output
			if (receiver->index() == target) { reachesTarget = true; }
			else if (receiver->isMaster() || (receiver->m_groupChannel && receiver->m_owner != channel))
			{
				wrong.push_back(receiver->index());
			}
			else { sentByHand = true; }
		}
		if (sentByHand && !reachesTarget && wrong.empty()) { return; }

		for (const auto index : wrong) { deleteChannelSend(channel->index(), index); }
		if (!reachesTarget) { createChannelSend(channel->index(), target); }
	};

	for (const auto track : song->tracks())
	{
		const auto channel = trackChannel(track);
		if (!channel) { continue; }

		const auto folder = song->folderOf(track);
		const auto folderChannel = folder ? trackChannel(folder) : nullptr;
		const mix_ch_t target = folderChannel ? folderChannel->index() : 0;

		routeTo(channel, target);

		// The group channels made for it (e.g. of its drum pieces) go where it goes
		for (const auto owned : ownedChannels(channel->index()))
		{
			if (owned->m_groupChannel) { routeTo(owned, target); }
		}
	}

	journal->setJournalling(wasJournalling);
}




auto Mixer::fxReturns() const -> std::vector<MixerChannel*>
{
	auto result = std::vector<MixerChannel*>{};
	for (const auto channel : m_mixerChannels)
	{
		if (channel->m_fxReturn) { result.push_back(channel); }
	}
	return result;
}




void Mixer::ensureFxReturns()
{
	const auto journal = Engine::projectJournal();
	const bool wasJournalling = journal->isJournalling();
	journal->setJournalling(false);

	auto returns = fxReturns();
	while (static_cast<int>(returns.size()) < FxReturnCount)
	{
		const int index = createChannel();
		auto channel = m_mixerChannels[index];
		channel->m_fxReturn = true;
		channel->m_name = tr("FX %1").arg(returns.size() + 1);
		channel->m_analogStrip.setDisplayNames(channel->m_name);
		// An effect return sends only to the master
		while (channel->m_sends.size() > 1)
		{
			const auto other = std::find_if(channel->m_sends.begin(), channel->m_sends.end(),
				[](const MixerRoute* route) { return !route->receiver()->isMaster(); });
			if (other == channel->m_sends.end()) { break; }
			deleteChannelSend(*other);
		}
		returns.push_back(channel);
	}

	// Every channel sends to them
	for (const auto channel : m_mixerChannels)
	{
		if (channel->isMaster() || channel->m_fxReturn) { continue; }
		for (const auto fxReturn : returns)
		{
			if (!findChannelSend(channel->index(), fxReturn->index())) { createChannelSend(channel->index(), fxReturn->index(), 0.f); }
		}
	}

	journal->setJournalling(wasJournalling);
	emit dataChanged();
}




auto Mixer::fxSend(mix_ch_t index, int fxReturn) const -> MixerRoute*
{
	const auto returns = fxReturns();
	if (fxReturn < 0 || fxReturn >= static_cast<int>(returns.size())) { return nullptr; }
	return findChannelSend(index, returns[fxReturn]->index());
}




auto Mixer::ownedChannels(mix_ch_t index) const -> std::vector<MixerChannel*>
{
	auto result = std::vector<MixerChannel*>{};
	if (index <= 0 || index >= numChannels()) { return result; }
	for (const auto channel : m_mixerChannels)
	{
		if (channel->m_owner == m_mixerChannels[index]) { result.push_back(channel); }
	}
	return result;
}




void Mixer::arrangeChannelsByTracks()
{
	auto order = std::vector<MixerChannel*>{m_mixerChannels[0]};
	auto placed = std::vector<bool>(m_mixerChannels.size(), false);
	placed[0] = true;

	auto place = [&](MixerChannel* channel) {
		if (placed[channel->index()]) { return; }
		placed[channel->index()] = true;
		order.push_back(channel);
		// Its group channels first, then the channels sending to them, like the folders' tracks
		for (const auto owned : ownedChannels(channel->index()))
		{
			if (owned->m_groupChannel && !placed[owned->index()]) { placed[owned->index()] = true; order.push_back(owned); }
		}
		for (const auto owned : ownedChannels(channel->index()))
		{
			if (!placed[owned->index()]) { placed[owned->index()] = true; order.push_back(owned); }
		}
	};

	for (const auto container : std::initializer_list<TrackContainer*>{Engine::getSong(), Engine::patternStore()})
	{
		if (!container) { continue; }
		for (const auto track : container->tracks())
		{
			const auto strip = stripOf(track);
			if (!strip) { continue; }
			const int index = strip->channel->value();
			if (index > 0 && index < numChannels() && m_mixerChannels[index]->m_trackChannel)
			{
				place(m_mixerChannels[index]);
			}
		}
	}
	for (const auto channel : m_mixerChannels)
	{
		if (!placed[channel->index()]) { order.push_back(channel); placed[channel->index()] = true; }
	}

	// Moves each channel into place one step at a time, so that every model following a channel (the tracks'
	// channel numbers, the views) keeps up through channelsSwapped
	const auto change = StructureChange{*this};
	for (auto target = std::size_t{1}; target < order.size(); ++target)
	{
		for (int at = order[target]->index(); at > static_cast<int>(target); --at) { moveChannelLeft(at); }
	}
}




auto Mixer::trackChannel(const Track* track) const -> MixerChannel*
{
	const auto strip = stripOf(track);
	if (!strip) { return nullptr; }

	const int index = strip->channel->value();
	if (index <= 0 || index >= numChannels() || !m_mixerChannels[index]->m_trackChannel) { return nullptr; }

	// Not if other tracks send to it too
	const auto tracks = tracksWithMixerChannel();
	const auto users = std::count_if(tracks.begin(), tracks.end(), [index](const auto& entry) {
		return entry.second->value() == index;
	});
	return users == 1 ? m_mixerChannels[index] : nullptr;
}


} // namespace lmms
