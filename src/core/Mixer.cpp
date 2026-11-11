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

#include "AudioEngine.h"
#include "AudioEngineWorkerThread.h"
#include "Mixer.h"
#include "DataFile.h"
#include "Song.h"

#include "InstrumentTrack.h"
#include "MixHelpers.h"
#include "lmms_math.h"
#include "panning.h"
#include "PatternStore.h"
#include "ProjectJournal.h"
#include "SampleTrack.h"
#include "TrackContainer.h" // For TrackContainer::TrackList typedef

namespace lmms
{


MixerRoute::MixerRoute( MixerChannel * from, MixerChannel * to, float amount ) :
	m_from( from ),
	m_to( to ),
	m_amount(amount, 0, 1, 0.001f, nullptr,
			tr("Amount to send from channel %1 to channel %2").arg(m_from->index()).arg(m_to->index())),
	m_preFader(false, nullptr,
			tr("Pre-fader send from channel %1 to channel %2").arg(m_from->index()).arg(m_to->index()))
{
	//qDebug( "created: %d to %d", m_from->m_channelIndex, m_to->m_channelIndex );
	// create send amount model
}


void MixerRoute::updateName()
{
	m_amount.setDisplayName(
			tr("Amount to send from channel %1 to channel %2").arg(m_from->index()).arg(m_to->index()));
}


MixerChannel::MixerChannel( int idx, Model * _parent ) :
	m_fxChain( nullptr ),
	m_stillRunning( false ),
	m_buffer(Engine::audioEngine()->framesPerPeriod()),
	m_muteModel( false, _parent ),
	m_soloModel( false, _parent ),
	m_volumeModel(1.f, 0.f, 2.f, 0.001f, _parent),
	m_trimModel(0.f, -24.f, 24.f, 0.1f, _parent),
	m_swapChannelsModel(false, _parent),
	m_phaseInvertModel(false, _parent),
	m_widthModel(100.f, 0.f, 200.f, 1.f, _parent),
	m_panModel(PanningCenter, PanningLeft, PanningRight, 1.f, _parent),
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
	m_buffer.allocateInterleavedBuffer();
}




MixerChannel::~MixerChannel()
{
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

void MixerChannel::unmuteForSolo()
{
	m_muteModel.setValue(false);

	// if channel is not master, unmute also every channel it sends to/receives from
	if (!isMaster())
	{
		for (const MixerRoute* sendsRoute : m_sends)
		{
			sendsRoute->receiver()->unmuteSenderForSolo();
		}

		for (const MixerRoute* receiverRoute : m_receives)
		{
			receiverRoute->sender()->unmuteReceiverForSolo();
		}
	}
}

void MixerChannel::unmuteSenderForSolo()
{
	m_muteModel.setValue(false);

	// if channel is not master, unmute every channel it sends to
	if (!isMaster())
	{
		for (const MixerRoute* sendsRoute : m_sends)
		{
			sendsRoute->receiver()->unmuteSenderForSolo();
		}
	}
}


void MixerChannel::unmuteReceiverForSolo()
{
	m_muteModel.setValue(false);

	// if channel is not master, unmute every channel it receives from, and of those, unmute the channels they send to
	if (!isMaster())
	{
		for (const MixerRoute* receiverRoute : m_receives)
		{
			receiverRoute->sender()->unmuteReceiverForSolo();
		}

		for (const MixerRoute* sendsRoute : m_sends)
		{
			sendsRoute->receiver()->unmuteSenderForSolo();
		}
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

	// Inputs from other channels, taken before or after their fader
	for (auto i = std::size_t{0}; i < m_receives.size(); ++i)
	{
		MixerRoute* route = m_receives[i];
		MixerChannel* sender = route->sender();
		const bool preFader = route->isPreFader();
		const bool hasSignal = preFader
			? sender->m_preFaderHasSignal
			: sender->m_buffer.hasAnySignal() || sender->m_stillRunning;
		if (!hasSignal) { continue; }

		const SampleFrame* src = preFader
			? sender->m_preFaderBuffer.data()
			: sender->m_buffer.interleavedBuffer().asSampleFrames().data();
		if (sendBufs[i]) { MixHelpers::addMultipliedByBuffer(buffer.data(), src, 1.f, sendBufs[i], fpp); }
		else { MixHelpers::addMultiplied(buffer.data(), src, route->amount()->value(), fpp); }

		toPlanar(m_buffer.interleavedBuffer(), m_buffer.groupBuffers(0));
		m_buffer.mixSilenceFlags(sender->m_buffer);
	}

	// Input trim
	if (m_buffer.hasAnySignal() && m_trimModel.value() != 0.f)
	{
		const float trim = dbfsToAmp(m_trimModel.value());
		for (auto& frame : buffer) { frame *= trim; }
		toPlanar(m_buffer.interleavedBuffer(), m_buffer.groupBuffers(0));
	}

	m_stillRunning = m_fxChain.processAudioBuffer(m_buffer);
	const bool hasSignal = m_buffer.hasAnySignal() || m_stillRunning;

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
		const float width = m_widthModel.value() / 100.f;
		const auto pan = panningToVolumeVector(static_cast<panning_t>(m_panModel.value()));
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

	if (hasSignal) { m_meter.measure(buffer.data(), fpp, Engine::audioEngine()->outputSampleRate()); }
	else { m_meter.measureSilence(); }

	// increment dependency counter of all receivers
	processed();
}



Mixer::Mixer() :
	Model( nullptr ),
	JournallingObject(),
	m_mixerChannels(),
	m_lastSoloed(-1)
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
			if (auto instrumentTrack = dynamic_cast<InstrumentTrack*>(track))
			{
				result.emplace_back(track, instrumentTrack->mixerChannelModel());
			}
			else if (auto sampleTrack = dynamic_cast<SampleTrack*>(track))
			{
				result.emplace_back(track, sampleTrack->mixerChannelModel());
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

	// if there is a soloed channel, mute the new track
	if (m_lastSoloed != -1 && m_mixerChannels[m_lastSoloed]->m_soloModel.value())
	{
		m_mixerChannels[index]->m_muteBeforeSolo = m_mixerChannels[index]->m_muteModel.value();
		m_mixerChannels[index]->m_muteModel.setValue(true);
	}

	emit channelCreated(index);
	return index;
}

void Mixer::activateSolo()
{
	for (auto i = std::size_t{1}; i < m_mixerChannels.size(); ++i)
	{
		m_mixerChannels[i]->m_muteBeforeSolo = m_mixerChannels[i]->m_muteModel.value();
		m_mixerChannels[i]->m_muteModel.setValue( true );
	}
}

void Mixer::deactivateSolo()
{
	for (auto i = std::size_t{1}; i < m_mixerChannels.size(); ++i)
	{
		m_mixerChannels[i]->m_muteModel.setValue( m_mixerChannels[i]->m_muteBeforeSolo );
	}
}

void Mixer::toggledSolo()
{
	int soloedChan = -1;
	bool resetSolo = m_lastSoloed != -1;
	//untoggle if lastsoloed is entered
	if (resetSolo)
	{
		m_mixerChannels[m_lastSoloed]->m_soloModel.setValue( false );
	}
	//determine the soloed channel
	for (auto i = std::size_t{0}; i < m_mixerChannels.size(); ++i)
	{
		if (m_mixerChannels[i]->m_soloModel.value() == true)
			soloedChan = i;
	}
	// if no channel is soloed, unmute everything, else mute everything
	if (soloedChan != -1)
	{
		if (resetSolo)
		{
			deactivateSolo();
			activateSolo();
		} else {
			activateSolo();
		}
		// unmute the soloed chan and every channel it sends to/receives from
		m_mixerChannels[soloedChan]->unmuteForSolo();
	} else {
		deactivateSolo();
	}
	m_lastSoloed = soloedChan;
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

	// if m_lastSoloed was our index, reset it
	if (m_lastSoloed == index) { m_lastSoloed = -1; }
	// if m_lastSoloed is > delete index, it will move left
	else if (m_lastSoloed > index) { --m_lastSoloed; }

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

	// channels to swap
	int a = index - 1, b = index;

	// check if m_lastSoloed is one of our swaps
	if (m_lastSoloed == a) { m_lastSoloed = b; }
	else if (m_lastSoloed == b) { m_lastSoloed = a; }

	// Swap positions in array
	std::swap(m_mixerChannels[index], m_mixerChannels[index - 1]);

	// Update m_channelIndex of both channels
	m_mixerChannels[index]->setIndex(index);
	m_mixerChannels[index - 1]->setIndex(index - 1);

	emit channelsSwapped(index, index - 1);
}



void Mixer::moveChannelRight( int index )
{
	moveChannelLeft( index + 1 );
}



MixerRoute * Mixer::createChannelSend( mix_ch_t fromChannel, mix_ch_t toChannel,
								float amount )
{
	const auto change = StructureChange{*this};
//	qDebug( "requested: %d to %d", fromChannel, toChannel );
	// find the existing connection
	MixerChannel * from = m_mixerChannels[fromChannel];
	MixerChannel * to = m_mixerChannels[toChannel];

	for (const auto& send : from->m_sends)
	{
		if (send->receiver() == to)
		{
			// simply adjust the amount
			send->amount()->setValue(amount);
			return send;
		}
	}

	// connection does not exist. create a new one
	return createRoute( from, to, amount );
}


MixerRoute * Mixer::createRoute( MixerChannel * from, MixerChannel * to, float amount )
{
	if( from == to )
	{
		return nullptr;
	}
	Engine::audioEngine()->requestChangeInModel();
	auto route = new MixerRoute(from, to, amount);

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
	// delete the send
	MixerChannel * from = m_mixerChannels[fromChannel];
	MixerChannel * to	 = m_mixerChannels[toChannel];

	// find and delete the send entry
	for (const auto& send : from->m_sends)
	{
		if (send->receiver() == to)
		{
			deleteChannelSend(send);
			break;
		}
	}
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
	const MixerChannel * from = m_mixerChannels[fromChannel];
	const MixerChannel * to = m_mixerChannels[toChannel];

	for( MixerRoute * route : from->m_sends )
	{
		if( route->receiver() == to )
		{
			return route->amount();
		}
	}

	return nullptr;
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




void Mixer::prepareMasterMix()
{
	m_mixerChannels[0]->m_buffer.silenceAllChannels();
}



void Mixer::masterMix( SampleFrame* _buf )
{
	const int fpp = Engine::audioEngine()->framesPerPeriod();

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
		ch->m_muted = ch->m_muteModel.value();
		if( ch->m_muted ) // instantly "process" muted channels
		{
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
	while (m_mixerChannels[0]->state() != ThreadableJob::ProcessingState::Done)
	{
		bool found = false;
		for( MixerChannel * ch : m_mixerChannels )
		{
			const auto s = ch->state();
			if (s == ThreadableJob::ProcessingState::Queued
				|| s == ThreadableJob::ProcessingState::InProgress)
			{
				found = true;
				break;
			}
		}
		if( !found )
		{
			break;
		}
		AudioEngineWorkerThread::startAndWaitForJobs();
	}

	// The master channel already applied its fader when it was processed
	const auto buffer = m_mixerChannels[0]->m_buffer.interleavedBuffer().asSampleFrames();
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
	ch->m_swapChannelsModel.setValue(false);
	ch->m_phaseInvertModel.setValue(false);
	ch->m_widthModel.setValue(100.f);
	ch->m_panModel.setValue(PanningCenter);
	ch->m_input = ChannelInput{};
	ch->m_recordArmModel.setValue(false);
	ch->m_monitorModel.setValue(static_cast<int>(MixerChannel::MonitorMode::Auto));
	ch->m_name = ( index == 0 ) ? tr( "Master" ) : tr( "Channel %1" ).arg( index );
	ch->m_volumeModel.setDisplayName( ch->m_name + ">" + tr( "Volume" ) );
	ch->m_muteModel.setDisplayName( ch->m_name + ">" + tr( "Mute" ) );
	ch->m_soloModel.setDisplayName( ch->m_name + ">" + tr( "Solo" ) );
	ch->m_trimModel.setDisplayName(ch->m_name + ">" + tr("Trim"));
	ch->m_swapChannelsModel.setDisplayName(ch->m_name + ">" + tr("Swap left/right"));
	ch->m_phaseInvertModel.setDisplayName(ch->m_name + ">" + tr("Invert phase"));
	ch->m_widthModel.setDisplayName(ch->m_name + ">" + tr("Stereo width"));
	ch->m_panModel.setDisplayName(ch->m_name + ">" + tr("Panning"));
	ch->m_recordArmModel.setDisplayName(ch->m_name + ">" + tr("Record arm"));
	ch->m_monitorModel.setDisplayName(ch->m_name + ">" + tr("Input monitoring"));
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
		mixch.setAttribute( "name", ch->m_name );
		if (const auto& color = ch->color()) { mixch.setAttribute("color", color->name()); }

		// add the channel sends
		for (const auto& send : ch->m_sends)
		{
			QDomElement sendsDom = _doc.createElement( QString( "send" ) );
			mixch.appendChild( sendsDom );

			sendsDom.setAttribute("channel", send->receiverIndex());
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
	QDomNode node = _this.firstChild();

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
		m_mixerChannels[num]->m_swapChannelsModel.loadSettings(mixch, "swapchannels");
		m_mixerChannels[num]->m_phaseInvertModel.loadSettings(mixch, "phaseinvert");
		m_mixerChannels[num]->m_widthModel.loadSettings(mixch, "width");
		m_mixerChannels[num]->m_panModel.loadSettings(mixch, "pan");
		m_mixerChannels[num]->m_recordArmModel.loadSettings(mixch, "recordarm");
		m_mixerChannels[num]->m_monitorModel.loadSettings(mixch, "monitor");
		if (const auto input = mixch.firstChildElement("input");
			!input.isNull() && input.attribute("source") == "hardware")
		{
			setChannelInput(num, ChannelInput{ChannelInput::Source::Hardware,
				input.attribute("channel").toInt(), std::clamp(input.attribute("channels", "2").toInt(), 1, 2)});
		}
		m_mixerChannels[num]->m_name = mixch.attribute( "name" );
		if (mixch.hasAttribute("color"))
		{
			m_mixerChannels[num]->setColor(QColor{mixch.attribute("color")});
		}

		m_mixerChannels[num]->m_fxChain.restoreState( mixch.firstChildElement(
			m_mixerChannels[num]->m_fxChain.nodeName() ) );

		// mixer sends
		QDomNodeList chData = mixch.childNodes();
		for (auto i = 0; i < chData.length(); ++i)
		{
			QDomElement chDataItem = chData.at(i).toElement();
			if( chDataItem.nodeName() == QString( "send" ) )
			{
				int sendTo = chDataItem.attribute( "channel" ).toInt();
				allocateChannelsTo( sendTo ) ;
				MixerRoute * mxr = createChannelSend( num, sendTo, 1.0f );
				if (mxr)
				{
					mxr->amount()->loadSettings(chDataItem, "amount");
					mxr->preFader()->loadSettings(chDataItem, "prefader");
				}
			}
		}



		node = node.nextSibling();
	}

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

	// check if the index mixer channel receives audio from any other channel
	if (!m_mixerChannels[index]->m_receives.empty()) { return true; }

	return m_mixerChannels[index]->useCount() > 0;
}


} // namespace lmms
