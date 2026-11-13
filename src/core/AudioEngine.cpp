/*
 * AudioEngine.cpp - device-independent audio engine for LMMS
 *
 * Copyright (c) 2004-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include <algorithm>
#include <cstdio>
#include <cassert> 
#include <vector>

#include "AudioEngine.h"
#include "AudioResampler.h"

#include <QStringList>

#include "AudioStream.h"

#include "MixHelpers.h"

#include "lmmsconfig.h"

#include "AudioEngineWorkerThread.h"
#include "AudioBusHandle.h"
#include "Hardware.h"
#include "PluginChecks.h"
#include "Mixer.h"
#include "Song.h"
#include "EnvelopeAndLfoParameters.h"
#include "NotePlayHandle.h"
#include "ConfigManager.h"

// platform-specific audio-interface-classes
#include "AudioAlsa.h"
#include "AudioJack.h"
#include "AudioSndio.h"
#include "AudioCoreAudio.h"
#include "AudioPortAudio.h"
#include "AudioPulseAudio.h"
#include "AudioSdl.h"
#include "AudioDummy.h"

// platform-specific midi-interface-classes
#include "MidiAlsaRaw.h"
#include "MidiAlsaSeq.h"
#include "MidiJack.h"
#include "MidiSndio.h"
#include "MidiWinMM.h"
#include "MidiApple.h"
#include "MidiDummy.h"

#include "BufferManager.h"



namespace lmms
{

using LocklessListElement = LocklessList<PlayHandle*>::Element;

static thread_local bool s_renderingThread = false;


namespace
{
//! How deep this thread is in requestChangeInModel() (the change lock is recursive)
thread_local int s_changesRequested = 0;
}



AudioEngine::AudioEngine(bool renderOnly)
	: m_renderOnly(renderOnly)
	, m_framesPerAudioBuffer(std::clamp(
		static_cast<f_cnt_t>(ConfigManager::inst()->value("audioengine", "framesperaudiobuffer",
		QString::number(DEFAULT_BUFFER_SIZE)).toUInt()),
		MINIMUM_BUFFER_SIZE, MAXIMUM_BUFFER_SIZE))
	, m_framesPerPeriod(std::min(m_framesPerAudioBuffer, DEFAULT_BUFFER_SIZE))
	, m_baseSampleRate(
		std::max(ConfigManager::inst()->value("audioengine", "samplerate").toInt(), SUPPORTED_SAMPLERATES.front()))
	, m_outputBufferRead(nullptr)
	, m_outputBufferWrite(nullptr)
	, m_outputBufferReadIndex(0)
	, m_workers()
	, m_numWorkers(QThread::idealThreadCount() - 1)
	, m_newPlayHandles(PlayHandle::MaxNumber)
	, m_masterGain(1.0f)
	, m_audioDev(nullptr)
	, m_oldAudioDev(nullptr)
	, m_audioDevStartFailed(false)
	, m_profiler()
	, m_clearSignal(false)
	, m_sanitizationEnabled(ConfigManager::inst()->value("audioengine", "sanitizemix", "1").toInt())
{
	BufferManager::init( m_framesPerPeriod );
	// The lists the audio thread adds to hold as many handles as can be added at once (see m_newPlayHandles), so
	// adding allocates nothing; removing keeps their room
	m_playHandles.reserve(PlayHandle::MaxNumber);
	m_playHandlesToRemove.reserve(PlayHandle::MaxNumber);
	// The resamplers notes and sample clips make on the audio thread take their state from a pool (instruments that
	// use another mode reserve their own, see AudioFileProcessor)
	AudioResampler::reserve(AudioResampler::Mode::Linear, DEFAULT_CHANNELS, 128);
	m_outputBufferRead = std::make_unique<SampleFrame[]>(m_framesPerPeriod);
	m_outputBufferWrite = std::make_unique<SampleFrame[]>(m_framesPerPeriod);
	m_outputMonitor.reset(std::vector<SampleFrame>(m_framesPerPeriod));

	if (m_numWorkers < 0) { m_numWorkers = 0; }

	for (int i = 0; i < m_numWorkers + 1; ++i)
	{
		auto workerThread = new AudioEngineWorkerThread(this);
		if( i < m_numWorkers )
		{
			workerThread->start(QThread::TimeCriticalPriority);
		}
		m_workers.push_back(workerThread);
	}

}




AudioEngine::~AudioEngine()
{
	for( int w = 0; w < m_numWorkers; ++w )
	{
		m_workers[w]->quit();
	}

	AudioEngineWorkerThread::startAndWaitForJobs();

	for( int w = 0; w < m_numWorkers; ++w )
	{
		m_workers[w]->wait( 500 );
	}

	delete m_midiClient;
	delete m_audioDev;
}




void AudioEngine::initDevices()
{
	bool success_ful = false;
	if( m_renderOnly ) {
		m_audioDev = new AudioDummy(success_ful, this, false);
		m_audioDevName = AudioDummy::name();
		m_midiClient = new MidiDummy;
		m_midiClientName = MidiDummy::name();
	} else {
		m_audioDev = tryAudioDevices();
		m_midiClient = tryMidiClients();
	}
	prepareCapture();

	// Loading audio device may have changed the sample rate
	emit sampleRateChanged();
}

bool AudioEngine::criticalXRuns() const
{
	return cpuLoad() >= 99 && Engine::getSong()->isExporting() == false;
}




void AudioEngine::renderStageNoteSetup()
{
	AudioEngineProfiler::Probe profilerProbe(m_profiler, AudioEngineProfiler::DetailType::NoteSetup);

	if( m_clearSignal )
	{
		m_clearSignal = false;
		clearInternal();
	}

	// remove all play-handles that have to be deleted and delete
	// them if they still exist...
	// maybe this algorithm could be optimized...
	ConstPlayHandleList::Iterator it_rem = m_playHandlesToRemove.begin();
	while( it_rem != m_playHandlesToRemove.end() )
	{
		PlayHandleList::Iterator it = std::find( m_playHandles.begin(), m_playHandles.end(), *it_rem );
		bool removedFromPlayHandles = false;
		if( it != m_playHandles.end() )
		{
			if ((*it)->audioBusHandle()) { (*it)->audioBusHandle()->removePlayHandle(*it); }
			(*it)->dispose();
			it = m_playHandles.erase(it);
			removedFromPlayHandles = true;
		}
		
		if (removedFromPlayHandles)
		{
			it_rem = m_playHandlesToRemove.erase(it_rem); 
		}
		else
		{
			++it_rem;
		}
	}

	swapBuffers();

	// prepare master mix (clear internal buffers etc.)
	Mixer * mixer = Engine::mixer();
	mixer->prepareMasterMix();

	// create play-handles for new notes, samples etc.
	Engine::getSong()->processNextBuffer();

	// add all play-handles that have to be added
	adoptNewPlayHandles();
}




void AudioEngine::adoptNewPlayHandles()
{
	for (LocklessListElement* e = m_newPlayHandles.popList(); e;)
	{
		m_playHandles += e->value;
		LocklessListElement* next = e->next;
		m_newPlayHandles.free(e);
		e = next;
	}
}



void AudioEngine::renderStageInstruments()
{
	AudioEngineProfiler::Probe profilerProbe(m_profiler, AudioEngineProfiler::DetailType::Instruments);

	// The notes ending in this period are released before any note is rendered, so that the release of sub-notes
	// does not depend on the order the threads render them in (see NotePlayHandle::releaseIfEnding())
	for (const auto handle : m_playHandles)
	{
		if (handle->type() != PlayHandle::Type::NotePlayHandle) { continue; }
		const auto note = static_cast<NotePlayHandle*>(handle);
		note->lock();
		note->releaseIfEnding();
		note->unlock();
	}

	AudioEngineWorkerThread::fillJobQueue(m_playHandles);
	AudioEngineWorkerThread::startAndWaitForJobs();
}



void AudioEngine::renderStageEffects()
{
	AudioEngineProfiler::Probe profilerProbe(m_profiler, AudioEngineProfiler::DetailType::Effects);

	// STAGE 2: process effects of all instrument- and sampletracks
	AudioEngineWorkerThread::fillJobQueue(m_audioBusHandles);
	AudioEngineWorkerThread::startAndWaitForJobs();

	// removed all play handles which are done
	for( PlayHandleList::Iterator it = m_playHandles.begin();
						it != m_playHandles.end(); )
	{
		if( ( *it )->affinityMatters() &&
			( *it )->affinity() != QThread::currentThread() )
		{
			++it;
			continue;
		}
		if( ( *it )->isFinished() )
		{
			if ((*it)->audioBusHandle()) { (*it)->audioBusHandle()->removePlayHandle(*it); }
			(*it)->dispose();
			it = m_playHandles.erase(it);
		}
		else
		{
			++it;
		}
	}
}



void AudioEngine::renderStageMix()
{
	AudioEngineProfiler::Probe profilerProbe(m_profiler, AudioEngineProfiler::DetailType::Mixing);

	Mixer *mixer = Engine::mixer();
	mixer->masterMix(m_outputBufferWrite.get());

	MixHelpers::multiply(m_outputBufferWrite.get(), m_masterGain, m_framesPerPeriod);

	// Hand the period that goes out now to the displays, without waiting for them
	auto& monitor = m_outputMonitor.writeSlot();
	std::copy_n(m_outputBufferRead.get(), m_framesPerPeriod, monitor.begin());
	m_outputMonitor.publish();

	// and trigger LFOs
	EnvelopeAndLfoParameters::instances()->trigger();
	Controller::triggerFrameCounter();
	AutomatableModel::incrementPeriodCounter();
}



std::span<const SampleFrame> AudioEngine::renderNextPeriod()
{
	// The thread rendering a period also processes jobs (see AudioEngineWorkerThread::startAndWaitForJobs()),
	// and that thread is not always ours: e.g. the export thread or an audio driver's callback thread. Without
	// disabling denormals there, jobs processed by it would compute denormal tails differently (and much more
	// slowly on x86) than jobs processed by the worker threads, so renders would depend on which thread got
	// which job.
	static thread_local const bool s_denormalsDisabled = [] { disableDenormals(); return true; }();
	(void)s_denormalsDisabled;

	const auto lock = std::lock_guard{m_changeMutex};
	const auto audioThread = ThreadCheck::AudioThreadScope{};

	m_profiler.startPeriod();
	s_renderingThread = true;

	renderStageNoteSetup();     // STAGE 0: clear old play handles and buffers, setup new play handles
	renderStageInstruments();   // STAGE 1: run and render all play handles
	renderStageEffects();       // STAGE 2: process effects of all instrument- and sampletracks
	renderStageMix();           // STAGE 3: do master mix in mixer

	s_renderingThread = false;
	m_periodsRendered.fetch_add(1, std::memory_order_relaxed);
	// (the disk thread frees a block once the period that might have read it has ended)
	AudioStream::periodRendered(static_cast<double>(outputSampleRate()) / m_framesPerPeriod);
	m_profiler.finishPeriod(outputSampleRate(), m_framesPerPeriod);
	m_outputBufferReadIndex = 0;

	return {m_outputBufferRead.get(), m_framesPerPeriod};
}

void AudioEngine::swapBuffers()
{
	std::swap(m_outputBufferRead, m_outputBufferWrite);
	zeroSampleFrames(m_outputBufferWrite.get(), m_framesPerPeriod);
}

void AudioEngine::setCaptureEnabled(bool enabled)
{
	if (enabled == m_captureEnabled) { return; }
	m_captureEnabled = enabled;
	applyCaptureEnabled();
}




void AudioEngine::applyCaptureEnabled()
{
	if (!m_audioDev || !m_audioDev->capturesOnDemand()) { return; }

	// Stopping the device waits for its callback to end, which waits for the change lock: a thread holding it (e.g.
	// loading a project, which processes events meanwhile) would wait for itself. Then the device changes later.
	if (s_changesRequested > 0)
	{
		QMetaObject::invokeMethod(this, [this] { applyCaptureEnabled(); }, Qt::QueuedConnection);
		return;
	}

	const bool running = m_audioDev->isRunning();
	if (running) { m_audioDev->stopProcessing(); }
	prepareCapture();
	if (running) { m_audioDev->startProcessing(); }
}




void AudioEngine::prepareCapture()
{
	assert(!m_audioDev || !m_audioDev->isRunning());
	if (m_audioDev && m_audioDev->capturesOnDemand()) { m_audioDev->setCaptureEnabled(m_captureEnabled); }
	m_captureChannels = m_audioDev ? m_audioDev->inputChannels() : 0;
	for (auto half = std::size_t{0}; half < 2; ++half)
	{
		auto& storage = m_captureStorage[half];
		storage.assign(static_cast<std::size_t>(m_captureChannels) * m_framesPerPeriod, 0.f);
		auto& pointers = m_capturePointers[half];
		pointers.resize(m_captureChannels);
		for (ch_cnt_t channel = 0; channel < m_captureChannels; ++channel)
		{
			pointers[channel] = storage.data() + static_cast<std::size_t>(channel) * m_framesPerPeriod;
		}
	}
	m_captureFill = 0;

	// The new device starts on a new period, with its input in step from its first frame
	m_outputBufferReadIndex = 0;
}

void AudioEngine::clear()
{
	m_clearSignal = true;
}




void AudioEngine::clearNewPlayHandles()
{
	requestChangeInModel();
	for( LocklessListElement * e = m_newPlayHandles.popList(); e; )
	{
		// The handles that live as long as their tracks stay (their tracks remove them)
		const auto type = e->value->type();
		if (PlayHandle::livesWithTrack(type))
		{
			m_playHandles += e->value;
		}
		LocklessListElement * next = e->next;
		m_newPlayHandles.free( e );
		e = next;
	}
	doneChangeInModel();
}



// removes all play-handles. this is necessary, when the song is stopped ->
// all remaining notes etc. would be played until their end
void AudioEngine::clearInternal()
{
	// TODO: m_midiClient->noteOffAll();
	for (auto ph : m_playHandles)
	{
		// The instruments' and the input monitors' handles live as long as their tracks
		if (!PlayHandle::livesWithTrack(ph->type()))
		{
			m_playHandlesToRemove.push_back(ph);
		}
	}
}

void AudioEngine::doSetAudioDevice( AudioDevice * _dev )
{
	// TODO: Use shared_ptr here in the future.
	// Currently, this is safe, because this is only called by
	// ProjectRenderer, and after ProjectRenderer calls this function,
	// it does not access the old device anymore.
	if( m_audioDev != m_oldAudioDev ) {delete m_audioDev;}

	if( _dev )
	{
		m_audioDev = _dev;
	}
	else
	{
		printf( "param _dev == NULL in AudioEngine::setAudioDevice(...). "
					"Trying any working audio-device\n" );
		m_audioDev = tryAudioDevices();
	}
	prepareCapture();
}

void AudioEngine::setAudioDevice(AudioDevice* _dev, bool startNow)
{
	stopProcessing();

	doSetAudioDevice( _dev );

	emit qualitySettingsChanged();
	emit sampleRateChanged();

	if (startNow) { startProcessing(); }
}




auto AudioEngine::reopenAudioDevice(std::optional<sample_rate_t> sampleRate) -> QString
{
	auto* song = Engine::getSong();
	if (song && (song->isRecording() || song->isRecordingAudio()))
	{
		return tr("The audio device cannot change while recording: it changes when the recording ends and the "
			"settings are saved again.");
	}
	const auto wanted = ConfigManager::inst()->value("audioengine", "audiodev");
	m_framesPerAudioBuffer = std::clamp(static_cast<f_cnt_t>(ConfigManager::inst()->value("audioengine",
		"framesperaudiobuffer", QString::number(DEFAULT_BUFFER_SIZE)).toUInt()), MINIMUM_BUFFER_SIZE, MAXIMUM_BUFFER_SIZE);
	m_baseSampleRate = sampleRate.value_or(static_cast<sample_rate_t>(std::max(
		ConfigManager::inst()->value("audioengine", "samplerate").toInt(), SUPPORTED_SAMPLERATES.front())));

	// The song stops (what plays goes on from where it was, on the new device), the old device closes
	if (song && song->isPlaying()) { song->stop(); }
	stopProcessing();
	delete m_audioDev;
	m_audioDev = nullptr;
	m_oldAudioDev = nullptr;
	m_audioDev = tryAudioDevices();
	prepareCapture();
	emit qualitySettingsChanged();
	emit sampleRateChanged();
	startProcessing();
	if (song) { song->scheduleCaptureUpdate(); } // (the new device's input, if a track needs it)

	auto told = QStringList{};
	if (m_audioDevName != wanted)
	{
		told << tr("%1 could not be opened; %2 is used.").arg(wanted, m_audioDevName);
	}
	if (outputSampleRate() != m_baseSampleRate)
	{
		told << tr("The audio device runs at %1 Hz, not at the %2 Hz asked: the audio is converted.")
			.arg(outputSampleRate()).arg(m_baseSampleRate);
	}
	return told.join('\n');
}




void AudioEngine::storeAudioDevice()
{
	if( !m_oldAudioDev )
	{
		m_oldAudioDev = m_audioDev;
	}
}




void AudioEngine::restoreAudioDevice()
{
	if( m_oldAudioDev && m_audioDev != m_oldAudioDev )
	{
		stopProcessing();
		delete m_audioDev;

		m_audioDev = m_oldAudioDev;
		prepareCapture();
		emit sampleRateChanged();

		startProcessing();
	}
	m_oldAudioDev = nullptr;
}


bool AudioEngine::captureDeviceAvailable() const
{
	return audioDev()->supportsCapture();
}


void AudioEngine::removeAudioBusHandle(AudioBusHandle* busHandle)
{
	requestChangeInModel();

	auto it = std::find(m_audioBusHandles.begin(), m_audioBusHandles.end(), busHandle);
	if (it != m_audioBusHandles.end())
	{
		m_audioBusHandles.erase(it);
	}
	doneChangeInModel();
}


bool AudioEngine::addPlayHandle( PlayHandle* handle )
{
	// Only add play handles if we have the CPU capacity to process them.
	// Instrument play handles are not added during playback, but when the
	// associated instrument is created, so add those unconditionally.
	if (PlayHandle::livesWithTrack(handle->type()) || !criticalXRuns())
	{
		m_newPlayHandles.push( handle );
		if (handle->audioBusHandle()) { handle->audioBusHandle()->addPlayHandle(handle); }
		return true;
	}

	handle->dispose();

	return false;
}


void AudioEngine::removePlayHandle(PlayHandle * ph)
{
	requestChangeInModel();
	// check thread affinity as we must not delete play-handles
	// which were created in a thread different than the audio engine thread
	if (ph->affinityMatters() && ph->affinity() == QThread::currentThread())
	{
		if (ph->audioBusHandle()) { ph->audioBusHandle()->removePlayHandle(ph); }
		bool removedFromList = false;
		// Check m_newPlayHandles first because doing it the other way around
		// creates a race condition
		for( LocklessListElement * e = m_newPlayHandles.first(),
				* ePrev = nullptr; e; ePrev = e, e = e->next )
		{
			if (e->value == ph)
			{
				if( ePrev )
				{
					ePrev->next = e->next;
				}
				else
				{
					m_newPlayHandles.setFirst( e->next );
				}
				m_newPlayHandles.free( e );
				removedFromList = true;
				break;
			}
		}
		// Now check m_playHandles
		PlayHandleList::Iterator it = std::find(m_playHandles.begin(), m_playHandles.end(), ph);
		if (it != m_playHandles.end())
		{
			m_playHandles.erase(it);
			removedFromList = true;
		}
		// Only deleting PlayHandles that were actually found in the list
		// "fixes crash when previewing a preset under high load"
		// (See tobydox's 2008 commit 4583e48)
		if ( removedFromList )
		{
			ph->dispose();
		}
	}
	else
	{
		m_playHandlesToRemove.push_back(ph);
	}
	doneChangeInModel();
}




void AudioEngine::removePlayHandlesOfTypes(Track * track, PlayHandle::Types types)
{
	requestChangeInModel();

	// A handle added since the last period is still in the list of new ones: it must be found too, or the next
	// period would adopt a handle deleted with its track
	adoptNewPlayHandles();
	PlayHandleList::Iterator it = m_playHandles.begin();
	while( it != m_playHandles.end() )
	{
		if ((*it)->isFromTrack(track) && ((*it)->type() & types))
		{
			if ((*it)->audioBusHandle()) { (*it)->audioBusHandle()->removePlayHandle(*it); }
			(*it)->dispose();
			it = m_playHandles.erase(it);
		}
		else
		{
			++it;
		}
	}
	doneChangeInModel();
}




void AudioEngine::requestChangeInModel()
{
	if (s_renderingThread) { return; }
	m_changeMutex.lock();
	++s_changesRequested;
}

void AudioEngine::doneChangeInModel()
{
	if (s_renderingThread) { return; }
	--s_changesRequested;
	m_changeMutex.unlock();
}

bool AudioEngine::isAudioDevNameValid(QString name)
{
#ifdef LMMS_HAVE_SDL
	if (name == AudioSdl::name())
	{
		return true;
	}
#endif


#ifdef LMMS_HAVE_ALSA
	if (name == AudioAlsa::name())
	{
		return true;
	}
#endif


#ifdef LMMS_HAVE_PULSEAUDIO
	if (name == AudioPulseAudio::name())
	{
		return true;
	}
#endif

#ifdef LMMS_HAVE_SNDIO
	if (name == AudioSndio::name())
	{
		return true;
	}
#endif

#ifdef LMMS_HAVE_JACK
	if (name == AudioJack::name())
	{
		return true;
	}
#endif


#ifdef LMMS_BUILD_APPLE
	if (name == AudioCoreAudio::name())
	{
		return true;
	}
#endif

#ifdef LMMS_HAVE_PORTAUDIO
	if (name == AudioPortAudio::name())
	{
		return true;
	}
#endif


	if (name == AudioDummy::name())
	{
		return true;
	}

	return false;
}

bool AudioEngine::isMidiDevNameValid(QString name)
{
#ifdef LMMS_HAVE_ALSA
	if (name == MidiAlsaSeq::name() || name == MidiAlsaRaw::name())
	{
		return true;
	}
#endif

#ifdef LMMS_HAVE_JACK
	if (name == MidiJack::name())
	{
		return true;
	}
#endif

#ifdef LMMS_HAVE_SNDIO
	if (name == MidiSndio::name())
	{
		return true;
	}
#endif

#ifdef LMMS_HAVE_WINMM
	if (name == MidiWinMM::name())
	{
		return true;
	}
#endif

#ifdef LMMS_BUILD_APPLE
	if (name == MidiApple::name())
	{
		return true;
	}
#endif

	if (name == MidiDummy::name())
	{
		return true;
	}

	return false;
}

AudioDevice * AudioEngine::tryAudioDevices()
{
	bool success_ful = false;
	AudioDevice * dev = nullptr;
	QString dev_name = ConfigManager::inst()->value( "audioengine", "audiodev" );
	if( !isAudioDevNameValid( dev_name ) )
	{
		dev_name = "";
	}

	m_audioDevStartFailed = false;

#ifdef LMMS_BUILD_APPLE
	// The native driver first: duplex, multichannel, with the devices' latencies
	if (dev_name == AudioCoreAudio::name() || dev_name.isEmpty())
	{
		dev = new AudioCoreAudio(success_ful, this);
		if (success_ful)
		{
			m_audioDevName = AudioCoreAudio::name();
			return dev;
		}
		delete dev;
	}
#endif

#ifdef LMMS_HAVE_SDL
	if( dev_name == AudioSdl::name() || dev_name == "" )
	{
		dev = new AudioSdl( success_ful, this );
		if( success_ful )
		{
			m_audioDevName = AudioSdl::name();
			return dev;
		}
		delete dev;
	}
#endif


#ifdef LMMS_HAVE_ALSA
	if( dev_name == AudioAlsa::name() || dev_name == "" )
	{
		dev = new AudioAlsa( success_ful, this );
		if( success_ful )
		{
			m_audioDevName = AudioAlsa::name();
			return dev;
		}
		delete dev;
	}
#endif


#ifdef LMMS_HAVE_PULSEAUDIO
	if( dev_name == AudioPulseAudio::name() || dev_name == "" )
	{
		dev = new AudioPulseAudio( success_ful, this );
		if( success_ful )
		{
			m_audioDevName = AudioPulseAudio::name();
			return dev;
		}
		delete dev;
	}
#endif

#ifdef LMMS_HAVE_SNDIO
	if( dev_name == AudioSndio::name() || dev_name == "" )
	{
		dev = new AudioSndio( success_ful, this );
		if( success_ful )
		{
			m_audioDevName = AudioSndio::name();
			return dev;
		}
		delete dev;
	}
#endif


#ifdef LMMS_HAVE_JACK
	if( dev_name == AudioJack::name() || dev_name == "" )
	{
		dev = new AudioJack( success_ful, this );
		if( success_ful )
		{
			m_audioDevName = AudioJack::name();
			return dev;
		}
		delete dev;
	}
#endif


#ifdef LMMS_HAVE_PORTAUDIO
	if (dev_name == AudioPortAudio::name() || dev_name.isEmpty())
	{
		dev = new AudioPortAudio( success_ful, this );
		if( success_ful )
		{
			m_audioDevName = AudioPortAudio::name();
			return dev;
		}
		delete dev;
	}
#endif


	// add more device-classes here...
	//dev = new audioXXXX( SAMPLE_RATES[m_qualityLevel], success_ful, this );
	//if( sucess_ful )
	//{
	//	return dev;
	//}
	//delete dev

	if( dev_name != AudioDummy::name() )
	{
		printf( "No audio-driver working - falling back to dummy-audio-"
			"driver\nYou can render your songs and listen to the output "
			"files...\n" );

		m_audioDevStartFailed = true;
	}

	m_audioDevName = AudioDummy::name();

	return new AudioDummy( success_ful, this );
}




MidiClient * AudioEngine::tryMidiClients()
{
	QString client_name = ConfigManager::inst()->value( "audioengine", "mididev" );
	if( !isMidiDevNameValid( client_name ) )
	{
		client_name = "";
	}

#ifdef LMMS_HAVE_ALSA
	if( client_name == MidiAlsaSeq::name() || client_name == "" )
	{
		auto malsas = new MidiAlsaSeq;
		if( malsas->isRunning() )
		{
			m_midiClientName = MidiAlsaSeq::name();
			return malsas;
		}
		delete malsas;
	}

	if( client_name == MidiAlsaRaw::name() || client_name == "" )
	{
		auto malsar = new MidiAlsaRaw;
		if( malsar->isRunning() )
		{
			m_midiClientName = MidiAlsaRaw::name();
			return malsar;
		}
		delete malsar;
	}
#endif

#ifdef LMMS_HAVE_JACK
	if( client_name == MidiJack::name() || client_name == "" )
	{
		auto mjack = new MidiJack;
		if( mjack->isRunning() )
		{
			m_midiClientName = MidiJack::name();
			return mjack;
		}
		delete mjack;
	}
#endif

#ifdef LMMS_HAVE_SNDIO
	if( client_name == MidiSndio::name() || client_name == "" )
	{
		MidiSndio * msndio = new MidiSndio;
		if( msndio->isRunning() )
		{
			m_midiClientName = MidiSndio::name();
			return msndio;
		}
		delete msndio;
	}
#endif

#ifdef LMMS_HAVE_WINMM
	if( client_name == MidiWinMM::name() || client_name == "" )
	{
		MidiWinMM * mwmm = new MidiWinMM;
//		if( moss->isRunning() )
		{
			m_midiClientName = MidiWinMM::name();
			return mwmm;
		}
		delete mwmm;
	}
#endif

#ifdef LMMS_BUILD_APPLE
	printf( "trying midi apple...\n" );
	if( client_name == MidiApple::name() || client_name == "" )
	{
		MidiApple * mapple = new MidiApple;
		m_midiClientName = MidiApple::name();
		printf( "Returning midi apple\n" );
		return mapple;
	}
	printf( "midi apple didn't work: client_name=%s\n", client_name.toUtf8().constData());
#endif

	if(client_name != MidiDummy::name())
	{
		if (client_name.isEmpty())
		{
			printf("Unknown MIDI-client. ");
		}
		else
		{
			printf("Couldn't create %s MIDI-client. ", client_name.toUtf8().constData());
		}
		printf("Will use dummy-MIDI-client.\n");
	}

	m_midiClientName = MidiDummy::name();

	return new MidiDummy;
}

} // namespace lmms
