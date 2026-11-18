/*
 * Engine.cpp - implementation of LMMS' engine-system
 *
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


#include "Engine.h"
#include "ErrorLog.h"
#include "NotePlayHandle.h"
#include "AudioEngine.h"
#include "ConfigManager.h"
#include "Mixer.h"
#include "Ladspa2LMMS.h"
#include "Lv2Manager.h"
#include "PatternStore.h"
#include "Plugin.h"
#include "PresetPreviewPlayHandle.h"
#include "ProjectJournal.h"
#include "Song.h"
#include "ThreadedExportManager.h"
#include "BandLimitedWave.h"
#include "Oscillator.h"

namespace lmms
{

std::atomic<float> Engine::s_framesPerTick = 0.f;
AudioEngine* Engine::s_audioEngine = nullptr;
Mixer * Engine::s_mixer = nullptr;
PatternStore * Engine::s_patternStore = nullptr;
Song * Engine::s_song = nullptr;
ProjectJournal * Engine::s_projectJournal = nullptr;
ThreadedExportManager* Engine::s_exportManager = nullptr;
#ifdef LMMS_HAVE_LV2
Lv2Manager * Engine::s_lv2Manager = nullptr;
#endif
Ladspa2LMMS * Engine::s_ladspaManager = nullptr;
void* Engine::s_dndPluginKey = nullptr;




void Engine::init( bool renderOnly )
{
	Engine *engine = inst();

	// The error log, into the working directory's logs/ (only the standard error without one)
	ErrorLog::start(ConfigManager::inst()->hasWorkingDir()
		? ErrorLog::fileIn(ConfigManager::inst()->workingDir()) : QString{});

	// The notes' places, before anything can play a note
	NotePlayHandleManager::init();

	emit engine->initProgress(tr("Generating wavetables"));
	// generate (load from file) bandlimited wavetables
	BandLimitedWave::generateWaves();
	//initialize oscillators
	Oscillator::waveTableInit();

	emit engine->initProgress(tr("Initializing data structures"));
	s_projectJournal = new ProjectJournal;
	s_audioEngine = new AudioEngine( renderOnly );
	s_song = new Song;
	s_mixer = new Mixer;
	s_patternStore = new PatternStore;
	s_exportManager = new ThreadedExportManager;

#ifdef LMMS_HAVE_LV2
	s_lv2Manager = new Lv2Manager;
	s_lv2Manager->initPlugins();
#endif
	s_ladspaManager = new Ladspa2LMMS;

	s_projectJournal->setJournalling( true );

	emit engine->initProgress(tr("Opening audio and midi devices"));
	s_audioEngine->initDevices();

	PresetPreviewPlayHandle::init();

	emit engine->initProgress(tr("Launching audio engine threads"));
	s_audioEngine->startProcessing();
}




void Engine::destroy()
{
	// Taken down in three steps, so that nothing deleted is used afterwards by a thread that still runs (phase 8 of
	// docs/plano-refatoracao.md; EngineLifecycleTest takes it down while a project plays, twice, under ASan too)

	// 1. Nothing calls into the program anymore: the audio device stops asking for periods, the MIDI input stops (its
	//    ports get a client that does nothing), no change is journalled, no export runs
	s_projectJournal->stopAllJournalling();
	s_audioEngine->stopProcessing();
	s_audioEngine->closeMidiClient();
	deleteHelper(&s_exportManager);

	// 2. The project: the previews, the tracks (and their plugins, notes and clips), the metronome's bus
	PresetPreviewPlayHandle::cleanup();
	s_song->clearProject();
	s_song->metronome().release();

	// 3. What is left, each before what it uses: the pattern store and the mixer (their tracks and effects), the song,
	//    the audio engine (its threads end), the plugin managers (no plugin is left), the journal (every journalled
	//    object is gone), the configuration
	deleteHelper(&s_patternStore);
	deleteHelper(&s_mixer);
	deleteHelper(&s_song);
	deleteHelper(&s_audioEngine);
#ifdef LMMS_HAVE_LV2
	deleteHelper(&s_lv2Manager);
#endif
	deleteHelper(&s_ladspaManager);
	deleteHelper(&s_projectJournal);
	delete ConfigManager::inst();

	// What the program keeps besides the engine: the notes' places (every note went back above), the error log
	// (what was reported until now, written out), the oscillators' FFT plans (made again by init())
	NotePlayHandleManager::free();
	ErrorLog::stop();
	Oscillator::destroyFFTPlans();
}




float Engine::framesPerTick(sample_rate_t sampleRate)
{
	return sampleRate * 60.0f * 4 /
			DefaultTicksPerBar / s_song->getTempo();
}




void Engine::updateFramesPerTick()
{
	s_framesPerTick.store(s_audioEngine->outputSampleRate() * 60.0f * 4 / DefaultTicksPerBar / s_song->getTempo(), std::memory_order_relaxed);
}




void Engine::setDndPluginKey(void *newKey)
{
	Q_ASSERT(static_cast<Plugin::Descriptor::SubPluginFeatures::Key*>(newKey));
	s_dndPluginKey = newKey;
}




void *Engine::pickDndPluginKey()
{
	return s_dndPluginKey;
}




Engine * Engine::s_instanceOfMe = nullptr;

} // namespace lmms
