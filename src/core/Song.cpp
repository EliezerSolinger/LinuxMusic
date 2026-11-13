/*
 * Song.cpp - root of the model tree
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

#include "Song.h"
#include <QTextStream>
#include <QTimer>
#include <QCoreApplication>
#include <QDebug>
#include <QFile>
#include <QFileInfo>
#include <QRegularExpression>
#include <QMessageBox>

#include <algorithm>
#include <cmath>
#include <map>

#include "AutomationTrack.h"
#include "AutomationEditor.h"
#include "ConfigManager.h"
#include "ControllerRackView.h"
#include "ControllerConnection.h"
#include "EnvelopeAndLfoParameters.h"
#include "Mixer.h"
#include "MixerView.h"
#include "GuiApplication.h"
#include "ExportFilter.h"
#include "InstrumentTrack.h"
#include "Keymap.h"
#include "NotePlayHandle.h"
#include "MidiClip.h"
#include "PatternEditor.h"
#include "PatternStore.h"
#include "PatternTrack.h"
#include "PianoRoll.h"
#include "ProjectJournal.h"
#include "ProjectNotes.h"
#include "Scale.h"
#include "SongEditor.h"
#include "SampleClip.h"
#include "SampleTrack.h"
#include "PeakController.h"


namespace lmms
{

tick_t TimePos::s_ticksPerBar = DefaultTicksPerBar;



Song::Song() :
	TrackContainer(),
	m_globalAutomationTrack( dynamic_cast<AutomationTrack *>(
				Track::create( Track::Type::HiddenAutomation,
								this ) ) ),
	m_tempoModel( DefaultTempo, MinTempo, MaxTempo, this, tr( "Tempo" ) ),
	m_timeSigModel( this ),
	m_oldTicksPerBar( DefaultTicksPerBar ),
	m_masterVolumeModel( 100, 0, 200, this, tr( "Master volume" ) ),
	m_masterPitchModel( 0, -12, 12, this, tr( "Master pitch" ) ),
	m_nLoadingTrack( 0 ),
	m_fileName(),
	m_oldFileName(),
	m_modified( false ),
	m_loadOnLaunch( true ),
	m_recording( false ),
	m_exporting( false ),
	m_exportLoop( false ),
	m_renderBetweenMarkers( false ),
	m_playing( false ),
	m_paused( false ),
	m_savingProject( false ),
	m_loadingProject( false ),
	m_isCancelled( false ),
	m_playMode( PlayMode::None ),
	m_length( 0 ),
	m_midiClipToPlay( nullptr ),
	m_loopMidiClip( false ),
	m_loopRenderCount(1),
	m_loopRenderRemaining(1),
	m_oldAutomatedValues()
{
	connect( &m_tempoModel, SIGNAL(dataChanged()),
			this, SLOT(setTempo()), Qt::DirectConnection );
	connect( &m_tempoModel, SIGNAL(dataUnchanged()),
			this, SLOT(setTempo()), Qt::DirectConnection );
	connect( &m_timeSigModel, SIGNAL(dataChanged()),
			this, SLOT(setTimeSignature()), Qt::DirectConnection );


	connect( Engine::audioEngine(), SIGNAL(sampleRateChanged()), this,
						SLOT(updateFramesPerTick()));

	connect( &m_masterVolumeModel, SIGNAL(dataChanged()),
			this, SLOT(masterVolumeChanged()), Qt::DirectConnection );
/*	connect( &m_masterPitchModel, SIGNAL(dataChanged()),
			this, SLOT(masterPitchChanged()));*/

	qRegisterMetaType<lmms::Note>( "lmms::Note" );
	setType( Type::Song );

	for (auto& scale : m_scales) {scale = std::make_shared<Scale>();}
	for (auto& keymap : m_keymaps) {keymap = std::make_shared<Keymap>();}

	// Aggregate the `positionJumped` signals from all the timelines into a single `playbackPositionJumped` signal for
	// other objects (sample tracks, LFOs, etc) to use.
	for (auto i = std::size_t{0}; i < static_cast<std::size_t>(PlayMode::Count); ++i)
	{
		const auto onPositionJumped = [this, playMode = static_cast<PlayMode>(i)] {
			// Only emit the signal when the song is actually playing and the active timeline jumps
			// This prevents LFOs from changing phase when the user drags the timeline while paused
			if (m_playing && m_playMode == playMode) { emit playbackPositionJumped(); }
		};

		connect(&m_timelines[i], &Timeline::positionJumped, this, onPositionJumped);
	}

	// A removed track may have been the one that needed the audio input
	connect(this, &TrackContainer::trackRemoved, this, &Song::scheduleCaptureUpdate);

	// Inform VST plugins if the user moved the play head
	connect(this, &Song::playbackPositionJumped, this, [this](){ m_vstSyncController.setPlaybackJumped(); }, Qt::DirectConnection);
}




Song::~Song()
{
	// The tracks are removed by ~TrackContainer(), when this Song is gone already
	disconnect(this, &TrackContainer::trackRemoved, this, &Song::scheduleCaptureUpdate);
	m_playing = false;
	delete m_globalAutomationTrack;
}




void Song::masterVolumeChanged()
{
	Engine::audioEngine()->setMasterGain( m_masterVolumeModel.value() / 100.0f );
}




void Song::setTempo()
{
	Engine::audioEngine()->requestChangeInModel();
	const auto tempo = (bpm_t)m_tempoModel.value();
	PlayHandleList & playHandles = Engine::audioEngine()->playHandles();
	for (const auto& playHandle : playHandles)
	{
		auto nph = dynamic_cast<NotePlayHandle*>(playHandle);
		if( nph && !nph->isReleased() )
		{
			nph->lock();
			nph->resize( tempo );
			nph->unlock();
		}
	}
	Engine::audioEngine()->doneChangeInModel();

	Engine::updateFramesPerTick();

	m_vstSyncController.setTempo( tempo );

	emit tempoChanged( tempo );
}




void Song::setTimeSignature()
{
	TimePos::setTicksPerBar( ticksPerBar() );
	emit timeSignatureChanged( m_oldTicksPerBar, ticksPerBar() );
	emit dataChanged();
	m_oldTicksPerBar = ticksPerBar();

	m_vstSyncController.setTimeSignature(
		getTimeSigModel().getNumerator(), getTimeSigModel().getDenominator() );
}




void Song::savePlayStartPosition()
{
	getTimeline().setPlayStartPosition(getPlayPos());
}




void Song::processNextBuffer()
{
	// If nothing is playing, there is nothing to do
	if (!m_playing) { return; }

	// At the beginning of the song, we have to reset the LFOs
	if (m_playMode == PlayMode::Song && getPlayPos() == 0)
	{
		EnvelopeAndLfoParameters::instances()->reset();
	}

	TrackList trackList;
	int clipNum = -1; // The number of the clip that will be played

	// Determine the list of tracks to play and the clip number
	switch (m_playMode)
	{
		case PlayMode::Song:
			trackList = tracks();
			break;

		case PlayMode::Pattern:
			if (Engine::patternStore()->numOfPatterns() > 0)
			{
				clipNum = Engine::patternStore()->currentPattern();
				trackList.push_back(PatternTrack::findPatternTrack(clipNum));
			}
			break;

		case PlayMode::MidiClip:
			if (m_midiClipToPlay)
			{
				clipNum = m_midiClipToPlay->getTrack()->getClipNum(m_midiClipToPlay);
				trackList.push_back(m_midiClipToPlay->getTrack());
			}
			break;

		default:
			return;
	}

	auto& timeline = getTimeline();

	// If the playback position is outside of the range [begin, end), move it to
	// begin and inform interested parties.
	// Returns true if the playback position was moved, else false.
	const auto enforceLoop = [this](const TimePos& begin, const TimePos& end)
	{
		if (getPlayPos() < begin || getPlayPos() >= end)
		{
			getTimeline().setTicks(begin.getTicks());
			return true;
		}
		return false;
	};

	const auto loopEnabled = !m_exporting && timeline.loopEnabled();

	// Ensure playback begins within the loop if it is enabled
	if (loopEnabled) { enforceLoop(timeline.loopBegin(), timeline.loopEnd()); }

	// Where the period starts, for the recording (see AudioRecorder)
	m_periodStartTick.store(static_cast<double>(timeline.ticks()) + timeline.frameOffset() / Engine::framesPerTick(),
		std::memory_order_relaxed);

	// The tempo map gives the tempo at each tick (in song mode): here for where the period starts, below at each
	// tick it reaches
	const auto followTempoMap = [this] {
		if (m_playMode != PlayMode::Song || m_tempoMap.isEmpty()) { return; }
		const auto bpm = static_cast<int>(std::lround(m_tempoMap.tempoAt(getPlayPos().getTicks())));
		if (bpm != m_tempoModel.value()) { m_tempoModel.setValue(static_cast<float>(bpm), true); }
	};
	followTempoMap();

	auto framesPerTick = Engine::framesPerTick();
	const auto framesPerPeriod = Engine::audioEngine()->framesPerPeriod();

	f_cnt_t frameOffsetInPeriod = 0;

	while (frameOffsetInPeriod < framesPerPeriod)
	{
		auto frameOffsetInTick = timeline.frameOffset();

		// If a whole tick has elapsed, update the frame and tick count, and check any loops
		if (frameOffsetInTick >= framesPerTick)
		{
			// Transfer any whole ticks from the frame count to the tick count
			const auto elapsedTicks = static_cast<int>(frameOffsetInTick / framesPerTick);
			frameOffsetInTick -= elapsedTicks * framesPerTick;
			timeline.incrementTicks(elapsedTicks);
			timeline.setFrameOffset(frameOffsetInTick);

			// If we are playing a pattern track, or a MIDI clip with no loop enabled,
			// loop back to the beginning when we reach the end
			if (m_playMode == PlayMode::Pattern)
			{
				enforceLoop(TimePos{0}, TimePos{Engine::patternStore()->lengthOfCurrentPattern(), 0});
			}
			else if (m_playMode == PlayMode::MidiClip && m_loopMidiClip && !loopEnabled)
			{
				enforceLoop(-m_midiClipToPlay->startTimeOffset(), m_midiClipToPlay->length() - m_midiClipToPlay->startTimeOffset());
			}

			// Handle loop points, and inform VST plugins of the loop status
			if (loopEnabled || (m_loopRenderRemaining > 1 && getPlayPos() >= timeline.loopBegin()))
			{
				m_vstSyncController.startCycle(
					timeline.loopBegin().getTicks(), timeline.loopEnd().getTicks());

				// Loop if necessary, and decrement the remaining loops if we did
				if (enforceLoop(timeline.loopBegin(), timeline.loopEnd())
					&& m_loopRenderRemaining > 1)
				{
					m_loopRenderRemaining--;
				}
			}
			else
			{
				m_vstSyncController.stopCycle();
			}
		}

		// A tick starts: the tempo map's tempo there
		if (static_cast<f_cnt_t>(frameOffsetInTick) == 0)
		{
			followTempoMap();
			framesPerTick = Engine::framesPerTick();
		}

		const f_cnt_t framesUntilNextPeriod = framesPerPeriod - frameOffsetInPeriod;
		const auto framesUntilNextTick = static_cast<f_cnt_t>(std::ceil(framesPerTick - frameOffsetInTick));

		// We want to proceed to the next buffer or tick, whichever is closer
		const auto framesToPlay = std::min(framesUntilNextPeriod, framesUntilNextTick);

		if (frameOffsetInPeriod == 0)
		{
			// First frame of buffer: update VST sync position.
			// This must be done after we've corrected the frame/tick count,
			// but before actually playing any frames.
			m_vstSyncController.setAbsolutePosition(getPlayPos().getTicks()
				+ timeline.frameOffset() / static_cast<double>(framesPerTick));
			m_vstSyncController.update();
		}

		if (static_cast<f_cnt_t>(frameOffsetInTick) == 0)
		{
			// First frame of tick: process automation and play tracks
			processAutomations(trackList, getPlayPos(), framesToPlay);
			processMetronome(frameOffsetInPeriod);

			for (const auto track : trackList)
			{
				track->play(getPlayPos(), framesToPlay, frameOffsetInPeriod, clipNum);
			}
		}

		// Update frame counters
		frameOffsetInPeriod += framesToPlay;
		frameOffsetInTick += framesToPlay;
		timeline.setFrameOffset(frameOffsetInTick);
	}

	// The armed tracks' input of this period goes to their takes
	if (m_playMode == PlayMode::Song)
	{
		m_audioRecorder.process(Engine::audioEngine()->capture(), m_periodStartTick.load(std::memory_order_relaxed),
			Engine::framesPerTick());
	}

}

void Song::removeOldAutomatedValue(AutomatableModel* am)
{
	// Get rid of object inside Song.m_oldAutomatedValues #8571
	m_oldAutomatedValues.remove(am);
}

void Song::processAutomations(const TrackList &tracklist, TimePos timeStart, f_cnt_t)
{
	AutomatedValueMap values;

	QSet<const AutomatableModel*> recordedModels;

	TrackContainer* container = this;
	int clipNum = -1;

	switch (m_playMode)
	{
	case PlayMode::Song:
		break;
	case PlayMode::Pattern:
	{
		if (tracklist.empty()) { return; }
		Q_ASSERT(tracklist.at(0)->type() == Track::Type::Pattern);
		auto patternTrack = dynamic_cast<PatternTrack*>(tracklist.at(0));
		container = Engine::patternStore();
		clipNum = patternTrack->patternIndex();
	}
		break;
	default:
		return;
	}

	values = container->automatedValuesAt(timeStart, clipNum);
	const TrackList& tracks = container->tracks();

	Track::clipVector clips;
	for (Track* track : tracks)
	{
		if (track->type() == Track::Type::Automation) {
			track->getClipsInRange(clips, 0, timeStart);
		}
	}

	// Process recording
	for (Clip* clip : clips)
	{
		auto p = dynamic_cast<AutomationClip *>(clip);
		TimePos relTime = timeStart - p->startPosition();
		if (p->isRecording() && relTime >= 0 && relTime < p->length())
		{
			const AutomatableModel* recordedModel = p->firstObject();
			// The automation system really needs to be reworked.
			// For whatever reason, the values in an automation clip are stored in un-un-scaled format, so if you
			// are automating a log knob, when you draw an curve, the values being stored are not the actual values the
			// knob will take, but instead the unscaled version of the unscaled numbers. The tooltip shows the number you expect, but if you double-click,
			// you can see that the true values are stored by their inverse scaled value....which is wrong, since they weren't scaled in the first place...?
			// Anyhow, in the meantime before we redo the automation system, when recording automations, we have to get the inverseScaledValue
			// and store that so that when playing it back, it scales the value correctly.
			p->recordValue(relTime, recordedModel->inverseScaledValue(recordedModel->value<float>()));

			recordedModels << recordedModel;
		}
	}

	// Checks if an automated model stopped being automated by automation clip
	// so we can move the control back to any connected controller again
	for (auto it = m_oldAutomatedValues.begin(); it != m_oldAutomatedValues.end(); it++)
	{
		AutomatableModel * am = it.key();
		if (!values.contains(am))
		{
			am->setUseControllerValue(true);
		}
	}
	m_oldAutomatedValues = values;

	// Apply values
	for (auto it = values.begin(); it != values.end(); it++)
	{
		AutomatableModel* model = it.key();
		bool isRecording = recordedModels.contains(model);
		model->setUseControllerValue(isRecording);

		if (!isRecording)
		{
			/* TODO
			 * Remove scaleValue() from here when automation editor's
			 * Y axis can be set to logarithmic, and automation clips store
			 * the actual values, and not the invertedScaledValue.
			 */
			model->setValue(model->scaledValue(it.value()), true);
		}
	}
}

void Song::processMetronome(size_t bufferOffset)
{
	const auto currentPlayMode = playMode();
	const auto supported = currentPlayMode == PlayMode::MidiClip
		|| currentPlayMode == PlayMode::Song
		|| currentPlayMode == PlayMode::Pattern;

	if (!supported || m_exporting) { return; } 
	// The count-in clicks during the pre-roll of a recording, metronome or not
	const bool countIn = currentPlayMode == PlayMode::Song && currentTick() < m_countInUntilTick;
	m_metronome.processTick(currentTick(), ticksPerBar(), m_timeSigModel.getNumerator(), bufferOffset, countIn);
}

void Song::setModified(bool value)
{
	if( !m_loadingProject && m_modified != value)
	{
		m_modified = value;
		emit modified();
	}
}

bool Song::isExportDone() const
{
	return !isExporting() || getPlayPos() >= m_exportSongEnd;
}

int Song::getExportProgress() const
{
	TimePos pos = getPlayPos();

	if (pos >= m_exportSongEnd)
	{
		return 100;
	}
	else if (pos <= m_exportSongBegin)
	{
		return 0;
	}
	else if (pos >= m_exportLoopEnd)
	{
		pos = (m_exportLoopBegin-m_exportSongBegin) + (m_exportLoopEnd - m_exportLoopBegin) *
			m_loopRenderCount + (pos - m_exportLoopEnd);
	}
	else if ( pos >= m_exportLoopBegin )
	{
		pos = (m_exportLoopBegin-m_exportSongBegin) + ((m_exportLoopEnd - m_exportLoopBegin) *
			(m_loopRenderCount - m_loopRenderRemaining)) + (pos - m_exportLoopBegin);
	}
	else
	{
		pos = (pos - m_exportSongBegin);
	}

	return (float)pos/(float)m_exportEffectiveLength*100.0f;
}

void Song::playSong()
{
	m_recording = false;

	if( isStopped() == false )
	{
		stop();
	}

	m_playMode = PlayMode::Song;
	m_playing = true;
	m_paused = false;

	m_vstSyncController.setPlaybackState( true );

	savePlayStartPosition();

	emit playbackStateChanged();
}





void Song::playAndRecord()
{
	playSong();
	m_recording = true;
}




void Song::playPattern()
{
	if( isStopped() == false )
	{
		stop();
	}

	m_playMode = PlayMode::Pattern;
	m_playing = true;
	m_paused = false;

	m_vstSyncController.setPlaybackState( true );

	savePlayStartPosition();

	emit playbackStateChanged();
}




void Song::playMidiClip( const MidiClip* midiClipToPlay, bool loop )
{
	if( isStopped() == false )
	{
		stop();
	}

	m_midiClipToPlay = midiClipToPlay;
	m_loopMidiClip = loop;

	if( m_midiClipToPlay != nullptr )
	{
		m_playMode = PlayMode::MidiClip;
		m_playing = true;
		m_paused = false;
	}

	savePlayStartPosition();

	emit playbackStateChanged();
}




void Song::updateLength()
{
	if (m_loadingProject) { return; }

	m_length = 0;
	m_tracksMutex.lockForRead();
	for (auto track : tracks())
	{
		if (m_exporting && track->isMutedInMix())
		{
			continue;
		}

		const bar_t cur = track->length();
		if( cur > m_length )
		{
			m_length = cur;
		}
	}
	m_tracksMutex.unlock();

	emit lengthChanged( m_length );
}




void Song::togglePause()
{
	// Pause/unpause only works when something is actually playing
	if (m_playMode == PlayMode::None)
	{
		return;
	}

	if( m_paused == true )
	{
		m_playing = true;
		m_paused = false;
	}
	else
	{
		m_playing = false;
		m_paused = true;
		Engine::audioEngine()->clear();
	}

	m_vstSyncController.setPlaybackState( m_playing );

	emit playbackStateChanged();
}




void Song::stop()
{
	// do not stop/reset things again if we're stopped already
	if( m_playMode == PlayMode::None )
	{
		return;
	}

	using gui::TimeLineWidget;

	// Where the recording of notes ends: what was heard last, before the play head moves
	const auto recordingEnd = heardTick();

	// To avoid race conditions with the processing threads
	Engine::audioEngine()->requestChangeInModel();

	auto& timeline = getTimeline();
	m_paused = false;
	m_recording = false;
	m_playing = false;

	switch (timeline.stopBehaviour())
	{
		case Timeline::StopBehaviour::BackToZero:
			if (m_playMode == PlayMode::MidiClip)
			{
				timeline.setTicks(std::max(0, -m_midiClipToPlay->startTimeOffset()));
			}
			else
			{
				timeline.setTicks(0);
			}
			break;

		case Timeline::StopBehaviour::BackToStart:
			if (timeline.playStartPosition() >= 0)
			{
				timeline.setTicks(timeline.playStartPosition().getTicks());

				timeline.setPlayStartPosition(-1);
			}
			break;

		case Timeline::StopBehaviour::KeepPosition:
			break;
	}

	getTimeline(PlayMode::None).setTicks(getPlayPos().getTicks());

	m_vstSyncController.setPlaybackState( m_exporting );
	m_vstSyncController.setAbsolutePosition(
		getPlayPos().getTicks()
		+ timeline.frameOffset()
		/ (double) Engine::framesPerTick() );

	// remove all note-play-handles that are active
	Engine::audioEngine()->clear();

	// Moves the control of the models that were processed on the last frame
	// back to their controllers.
	for (auto it = m_oldAutomatedValues.begin(); it != m_oldAutomatedValues.end(); it++)
	{
		AutomatableModel * am = it.key();
		am->setUseControllerValue(true);
	}
	m_oldAutomatedValues.clear();

	m_playMode = PlayMode::None;

	// The audio thread is held here (requestChangeInModel() above): the recording ends with this period
	auto takes = m_audioRecorder.stop();

	Engine::audioEngine()->doneChangeInModel();

	finishMidiRecording(recordingEnd);
	if (!takes.empty()) { finishAudioRecording(takes); }

	emit stopped();
	emit playbackStateChanged();
}




auto Song::mediaDirectory() const -> QString
{
	if (m_fileName.isEmpty()) { return ConfigManager::inst()->workingDir() + "recordings"; }
	const auto project = QFileInfo{m_fileName};
	return project.absolutePath() + '/' + project.completeBaseName() + "_media";
}




void Song::scheduleCaptureUpdate()
{
	if (m_captureUpdatePending) { return; }
	m_captureUpdatePending = true;
	QTimer::singleShot(0, this, [this] {
		m_captureUpdatePending = false;
		bool needed = isRecordingAudio();
		for (const auto* track : tracks())
		{
			const auto* sampleTrack = dynamic_cast<const SampleTrack*>(track);
			needed = needed || (sampleTrack && sampleTrack->input().source == AudioInput::Source::Hardware
				&& (sampleTrack->isRecordArmed()
					|| sampleTrack->monitorMode() == static_cast<int>(SampleTrack::MonitorMode::On)));
		}
		Engine::audioEngine()->setCaptureEnabled(needed);
	});
}




auto Song::recordingLatency() const -> f_cnt_t
{
	const auto* device = Engine::audioEngine()->audioDev();
	const auto calibration = ConfigManager::inst()->value("recording", "latencyoffset", "0").toInt();
	const auto latency = device
		? static_cast<long long>(device->inputLatency()) + static_cast<long long>(device->outputLatency())
		: 0LL;
	return static_cast<f_cnt_t>(std::max(0LL, latency + calibration));
}




auto Song::heardTick() const -> tick_t
{
	const auto* engine = Engine::audioEngine();
	const auto* device = engine->audioDev();
	const auto ahead = static_cast<double>(engine->framesPerPeriod()) + (device ? device->outputLatency() : 0);
	const auto tick = m_periodStartTick.load(std::memory_order_relaxed) - ahead / Engine::framesPerTick();
	return std::max<tick_t>(0, static_cast<tick_t>(std::floor(tick)));
}




auto Song::record() -> bool
{
	if (m_exporting || m_recording) { return false; }

	const auto* device = Engine::audioEngine()->audioDev();
	const auto deviceInputs = device ? static_cast<int>(device->inputChannels()) : 0;
	const auto directory = mediaDirectory();

	auto requests = std::vector<AudioRecorder::Request>{};
	auto problems = QStringList{};
	auto usedPaths = QStringList{};
	for (auto* track : tracks())
	{
		auto* sampleTrack = dynamic_cast<SampleTrack*>(track);
		if (!sampleTrack || !sampleTrack->recordArmModel()->value()) { continue; }

		const auto& input = sampleTrack->input();
		if (input.source != AudioInput::Source::Hardware)
		{
			problems << tr("%1 is armed but has no input.").arg(sampleTrack->name());
			continue;
		}
		if (input.firstChannel + input.channelCount > deviceInputs)
		{
			problems << tr("%1 records from input %2, which the audio device does not have.")
				.arg(sampleTrack->name()).arg(input.firstChannel + input.channelCount);
			continue;
		}

		// "<track>_<n>.wav", n the first take number free
		auto base = sampleTrack->name();
		base.replace(QRegularExpression{"[^\\w\\-]+"}, "_");
		if (base.isEmpty()) { base = "take"; }
		auto path = QString{};
		for (int take = 1; path.isEmpty() || QFileInfo::exists(path) || usedPaths.contains(path); ++take)
		{
			path = QString{"%1/%2_%3.wav"}.arg(directory, base).arg(take);
		}
		usedPaths << path;
		requests.push_back({sampleTrack, input, path});
	}

	// The armed instrument tracks record the notes played into them
	auto midiTracks = std::vector<InstrumentTrack*>{};
	for (auto* track : tracks())
	{
		auto* instrumentTrack = dynamic_cast<InstrumentTrack*>(track);
		if (instrumentTrack && instrumentTrack->isRecordArmed()) { midiTracks.push_back(instrumentTrack); }
	}

	if (requests.empty() && midiTracks.empty())
	{
		if (problems.isEmpty()) { problems << tr("No track is armed for recording."); }
		emit recordingProblem(problems.join('\n'));
		return false;
	}

	// One undo step takes the recording back: the armed tracks' state before it
	{
		const auto group = ProjectJournal::Group{Engine::projectJournal()};
		for (const auto& request : requests) { request.track->addJournalCheckPoint(); }
		for (auto* track : midiTracks) { track->addJournalCheckPoint(); }
	}

	// The input must be open before the first period is recorded (an armed track has opened it already)
	if (!requests.empty()) { Engine::audioEngine()->setCaptureEnabled(true); }

	// Playing the song, recording starts here and now (punch in); otherwise the song starts from the cursor, after
	// the pre-roll (with the count-in's clicks, if asked for)
	const bool punchIn = m_playing && !m_paused && m_playMode == PlayMode::Song;
	auto fromTick = tick_t{0};
	if (!punchIn)
	{
		if (!isStopped()) { stop(); }
		const auto cursor = getPlayPos(PlayMode::Song).getTicks();
		const auto preRoll = std::clamp(ConfigManager::inst()->value("recording", "prerollbars", "0").toInt(), 0, 16);
		const auto start = std::max(0, cursor - preRoll * ticksPerBar());
		const bool countIn = ConfigManager::inst()->value("recording", "countin", "0").toInt() != 0;
		m_countInUntilTick = countIn && preRoll > 0 ? cursor : -1;
		if (countIn) { m_metronome.prepare(); }
		fromTick = cursor;
		setPlayPos(start, PlayMode::Song);
		playSong();
		getTimeline(PlayMode::Song).setPlayStartPosition(TimePos{cursor}); // stopping goes back to the cursor
	}
	if (!requests.empty())
	{
		const auto error = m_audioRecorder.start(requests, recordingLatency(),
			Engine::audioEngine()->framesPerPeriod(), Engine::audioEngine()->outputSampleRate(), fromTick);
		if (!error.isEmpty())
		{
			if (!punchIn) { stop(); }
			emit recordingProblem(error);
			return false;
		}
	}
	if (!midiTracks.empty())
	{
		m_midiRecorder.start(midiTracks, punchIn ? heardTick() : fromTick);
		for (auto* track : midiTracks) { track->setRecordingMidi(true); }
	}
	m_recording = true;
	if (!problems.isEmpty()) { emit recordingProblem(problems.join('\n')); }
	emit playbackStateChanged();
	return true;
}




void Song::stopRecording()
{
	if (!m_recording) { return; }
	auto takes = std::vector<AudioRecorder::Take>{};
	{
		// The recording ends with the period being rendered; the song plays on
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		takes = m_audioRecorder.stop();
	}
	finishMidiRecording(heardTick());
	finishAudioRecording(takes);
	emit playbackStateChanged();
}




void Song::finishMidiRecording(tick_t atTick)
{
	if (!m_midiRecorder.isRecording()) { return; }
	for (auto* track : tracks())
	{
		if (auto* instrumentTrack = dynamic_cast<InstrumentTrack*>(track)) { instrumentTrack->setRecordingMidi(false); }
	}
	m_midiRecorder.stop(atTick);
	setModified();
}




void Song::finishAudioRecording(const std::vector<AudioRecorder::Take>& takes)
{
	m_recording = false;
	m_countInUntilTick = -1;
	auto problems = QStringList{};
	for (const auto& take : takes)
	{
		// Segments nothing went into are not kept
		auto segments = std::vector<AudioRecorder::Segment>{};
		for (const auto& segment : take.segments)
		{
			if (segment.frames > 0) { segments.push_back(segment); }
			else { QFile::remove(segment.path); }
		}
		if (segments.empty()) { continue; }

		// The track may have been removed while recording: its takes stay on disk
		const auto& all = tracks();
		if (std::find(all.begin(), all.end(), take.track) == all.end())
		{
			problems << tr("The track of %1 was removed while recording; the files were kept.")
				.arg(segments.front().path);
			continue;
		}
		auto* track = static_cast<SampleTrack*>(take.track);

		// The recording's undo step is the tracks' state before it (see record()): nothing is journalled here
		const auto pause = ProjectJournal::Pause{Engine::projectJournal()};

		// The passes of a loop begin on the same tick: they become the takes of one clip, the last pass the active
		// take. A pass shorter than 90% of the longest (stopped before the loop's end) is dropped.
		auto passes = std::map<std::int64_t, std::vector<AudioRecorder::Segment>>{};
		for (const auto& segment : segments) { passes[segment.startTick].push_back(segment); }
		for (auto& [startTick, group] : passes)
		{
			if (group.size() > 1)
			{
				const auto longest = std::max_element(group.begin(), group.end(),
					[](const auto& a, const auto& b) { return a.frames < b.frames; })->frames;
				std::erase_if(group, [longest](const AudioRecorder::Segment& segment) {
					const bool partial = segment.frames < longest * 9 / 10;
					if (partial) { QFile::remove(segment.path); }
					return partial;
				});
			}
			auto* clip = new SampleClip{track};
			clip->setSampleFile(group.front().path);
			clip->movePosition(TimePos{static_cast<tick_t>(startTick)});
			for (std::size_t pass = 1; pass < group.size(); ++pass) { clip->addTake(group[pass].path); }
		}

		if (take.lostFrames > 0)
		{
			problems << tr("%1: the disk could not keep up; %2 ms were recorded as silence.").arg(track->name())
				.arg(take.lostFrames * 1000 / Engine::audioEngine()->outputSampleRate());
		}
		if (take.diskFailed)
		{
			problems << tr("%1: writing a take failed; it holds what was written before.").arg(track->name());
		}
	}
	setModified();
	scheduleCaptureUpdate();
	if (!problems.isEmpty()) { emit recordingProblem(problems.join('\n')); }
}




void Song::startExport()
{
	stop();

	m_exporting = true;
	updateLength();

	auto& timeline = getTimeline(PlayMode::Song);

	if (m_renderBetweenMarkers)
	{
		m_exportSongBegin = m_exportLoopBegin = timeline.loopBegin();
		m_exportSongEnd = m_exportLoopEnd = timeline.loopEnd();

		timeline.setTicks(timeline.loopBegin().getTicks());
	}
	else
	{
		m_exportSongEnd = TimePos(m_length, 0);
		// Handle potentially ridiculous loop points gracefully.
		if (m_loopRenderCount > 1 && timeline.loopEnd() > m_exportSongEnd) 
		{
			m_exportSongEnd = timeline.loopEnd();
		}

		if (!m_exportLoop) 
			m_exportSongEnd += TimePos(1,0);

		m_exportSongBegin = TimePos(0,0);
		m_exportLoopBegin = timeline.loopBegin() < m_exportSongEnd && timeline.loopEnd() <= m_exportSongEnd
			? timeline.loopBegin()
			: TimePos{0};
		m_exportLoopEnd = timeline.loopBegin() < m_exportSongEnd && timeline.loopEnd() <= m_exportSongEnd
			? timeline.loopEnd()
			: TimePos{0};

		getTimeline(PlayMode::Song).setTicks(0);
	}

	m_exportEffectiveLength = (m_exportLoopBegin - m_exportSongBegin) + (m_exportLoopEnd - m_exportLoopBegin) 
		* m_loopRenderCount + (m_exportSongEnd - m_exportLoopEnd);
	m_loopRenderRemaining = m_loopRenderCount;

	playSong();

	m_vstSyncController.setPlaybackState( true );
}




void Song::stopExport()
{
	stop();
	m_exporting = false;

	m_vstSyncController.setPlaybackState( m_playing );
}




void Song::insertBar()
{
	m_tracksMutex.lockForRead();
	for (Track* track: tracks())
	{
		// FIXME journal batch of tracks instead of each track individually
		if (track->numOfClips() > 0) { track->addJournalCheckPoint(); }
		track->insertBar(getPlayPos(PlayMode::Song));
	}
	m_tracksMutex.unlock();
}




void Song::removeBar()
{
	m_tracksMutex.lockForRead();
	for (Track* track: tracks())
	{
		// FIXME journal batch of tracks instead of each track individually
		if (track->numOfClips() > 0) { track->addJournalCheckPoint(); }
		track->removeBar(getPlayPos(PlayMode::Song));
	}
	m_tracksMutex.unlock();
}




void Song::addPatternTrack()
{
	Track * t = Track::create(Track::Type::Pattern, this);
	Engine::patternStore()->setCurrentPattern(dynamic_cast<PatternTrack*>(t)->patternIndex());
}




void Song::addSampleTrack()
{
	( void )Track::create( Track::Type::Sample, this );
}




void Song::addAutomationTrack()
{
	( void )Track::create( Track::Type::Automation, this );
}




void Song::addFolderTrack()
{
	(void)Track::create(Track::Type::Folder, this);
}




bpm_t Song::getTempo()
{
	return ( bpm_t )m_tempoModel.value();
}


AutomatedValueMap Song::automatedValuesAt(TimePos time, int clipNum) const
{
	auto trackList = TrackList{m_globalAutomationTrack};
	trackList.insert(trackList.end(), tracks().begin(), tracks().end());
	return TrackContainer::automatedValuesFromTracks(trackList, time, clipNum);
}




void Song::clearProject()
{
	using gui::getGUI;

	Engine::projectJournal()->setJournalling( false );

	if( m_playing )
	{
		stop();
	}

	for (auto i = std::size_t{0}; i < PlayModeCount; i++)
	{
		setPlayPos( 0, ( PlayMode )i );
	}


	Engine::audioEngine()->requestChangeInModel();

	m_tempoMap.clear();

	if( getGUI() != nullptr && getGUI()->patternEditor() )
	{
		getGUI()->patternEditor()->m_editor->clearAllTracks();
	}
	if( getGUI() != nullptr && getGUI()->songEditor() )
	{
		getGUI()->songEditor()->m_editor->clearAllTracks();
	}
	if( getGUI() != nullptr && getGUI()->mixerView() )
	{
		getGUI()->mixerView()->clear();
	}
	QCoreApplication::sendPostedEvents();
	Engine::patternStore()->clearAllTracks();
	clearAllTracks();

	Engine::mixer()->clear();

	if( getGUI() != nullptr && getGUI()->automationEditor() )
	{
		getGUI()->automationEditor()->setCurrentClip( nullptr );
	}

	if( getGUI() != nullptr && getGUI()->pianoRoll() )
	{
		getGUI()->pianoRoll()->reset();
	}

	m_tempoModel.reset();
	m_masterVolumeModel.reset();
	m_masterPitchModel.reset();
	m_timeSigModel.reset();

	// Clear the m_oldAutomatedValues AutomatedValueMap
	m_oldAutomatedValues.clear();

	AutomationClip::globalAutomationClip( &m_tempoModel )->clear();
	AutomationClip::globalAutomationClip( &m_masterVolumeModel )->
									clear();
	AutomationClip::globalAutomationClip( &m_masterPitchModel )->
									clear();

	Engine::audioEngine()->doneChangeInModel();

	if( getGUI() != nullptr && getGUI()->getProjectNotes() )
	{
		getGUI()->getProjectNotes()->clear();
	}

	removeAllControllers();

	emit dataChanged();

	Engine::projectJournal()->clearJournal();

	Engine::projectJournal()->setJournalling( true );
}




// create new file
void Song::createNewProject()
{

	QString defaultTemplate = ConfigManager::inst()->userTemplateDir()
						+ "default.mpt";


	if( QFile::exists( defaultTemplate ) )
	{
		createNewProjectFromTemplate( defaultTemplate );
		return;
	}

	defaultTemplate = ConfigManager::inst()->factoryProjectsDir()
						+ "templates/default.mpt";
	if( QFile::exists( defaultTemplate ) )
	{
		createNewProjectFromTemplate( defaultTemplate );
		return;
	}

	m_loadingProject = true;

	clearProject();

	Engine::projectJournal()->setJournalling( false );

	m_oldFileName = "";
	setProjectFileName("");

	auto tripleOscTrack = Track::create(Track::Type::Instrument, this);
	dynamic_cast<InstrumentTrack*>(tripleOscTrack)->loadInstrument("tripleoscillator");

	auto kickerTrack = Track::create(Track::Type::Instrument, Engine::patternStore());
	dynamic_cast<InstrumentTrack*>(kickerTrack)->loadInstrument("kicker");

	Track::create( Track::Type::Sample, this );
	Track::create( Track::Type::Pattern, this );
	Track::create( Track::Type::Automation, this );

	m_tempoModel.setInitValue( DefaultTempo );
	m_timeSigModel.reset();
	m_masterVolumeModel.setInitValue( 100 );
	m_masterPitchModel.setInitValue( 0 );

	QCoreApplication::instance()->processEvents();

	m_loadingProject = false;
	Engine::mixer()->toleranceModel()->setValue(true);
	Engine::mixer()->setUpTrackChannels();
	Engine::mixer()->ensureFxReturns();
	updateLength();
	Engine::patternStore()->updateAfterTrackAdd();

	Engine::projectJournal()->setJournalling( true );

	QCoreApplication::sendPostedEvents();

	setModified(false);
	m_loadOnLaunch = false;
}




void Song::createNewProjectFromTemplate( const QString & templ )
{
	loadProject( templ );
	// A new project has the desk's tolerances, unless its template chose otherwise
	if (Engine::mixer()->loadedWithoutTolerance()) { Engine::mixer()->toleranceModel()->setValue(true); }
	// clear file-name so that user doesn't overwrite template when
	// saving...
	m_oldFileName = "";
	setProjectFileName("");
	// update window title
	m_loadOnLaunch = false;
}




// load given song
void Song::loadProject( const QString & fileName )
{
	using gui::getGUI;

	QDomNode node;

	m_loadingProject = true;

	Engine::projectJournal()->setJournalling( false );

	m_oldFileName = m_fileName;
	setProjectFileName(fileName);

	DataFile dataFile( m_fileName );

	bool cantLoadProject = false;
	// if file could not be opened, head-node is null and we create
	// new project
	if( dataFile.head().isNull() )
	{
		cantLoadProject = true;
	}
	else
	{
		// We check if plugins contain local paths to prevent malicious code being
		// added to project bundles and loaded with "local:" paths
		if (dataFile.hasLocalPlugins())
		{
			cantLoadProject = true;

			if (getGUI() != nullptr)
			{
				QMessageBox::critical(nullptr, tr("Aborting project load"),
					tr("Project file contains local paths to plugins, which could be used to "
						"run malicious code."));
			}
			else
			{
				QTextStream(stderr) << tr("Can't load project: "
					"Project file contains local paths to plugins.")
					<< Qt::endl;
			}
		}
	}

	if (cantLoadProject)
	{
		if( m_loadOnLaunch )
		{
			createNewProject();
		}
		setProjectFileName(m_oldFileName);
		return;
	}

	m_oldFileName = m_fileName;

	clearProject();
	Engine::mixer()->forgetTolerance();

	clearErrors();

	Engine::audioEngine()->requestChangeInModel();

	// get the header information from the DOM
	m_tempoModel.loadSettings( dataFile.head(), "bpm" );
	m_timeSigModel.loadSettings( dataFile.head(), "timesig" );
	m_masterVolumeModel.loadSettings( dataFile.head(), "mastervol" );
	m_masterPitchModel.loadSettings( dataFile.head(), "masterpitch" );

	getTimeline(PlayMode::Song).setLoopEnabled(false);

	//Backward compatibility for LMMS <= 0.4.15
	PeakController::initGetControllerBySetting();

	// Load mixer first to be able to set the correct range for mixer channels
	node = dataFile.content().firstChildElement( Engine::mixer()->nodeName() );
	if( !node.isNull() )
	{
		Engine::mixer()->restoreState( node.toElement() );
		if( getGUI() != nullptr )
		{
			// refresh MixerView
			getGUI()->mixerView()->refreshDisplay();
		}
	}

	node = dataFile.content().firstChild();

	QDomNodeList tclist=dataFile.content().elementsByTagName("trackcontainer");
	m_nLoadingTrack=0;
	for( int i=0,n=tclist.count(); i<n; ++i )
	{
		QDomNode nd=tclist.at(i).firstChild();
		while(!nd.isNull())
		{
			if( nd.isElement() && nd.nodeName() == "track" )
			{
				++m_nLoadingTrack;
				if (static_cast<Track::Type>(nd.toElement().attribute("type").toInt()) == Track::Type::Pattern)
				{
					n += nd.toElement().elementsByTagName("patterntrack").at(0)
						.toElement().firstChildElement().childNodes().count();
				}
				nd=nd.nextSibling();
			}
		}
	}

	while( !node.isNull() && !isCancelled() )
	{
		if( node.isElement() )
		{
			if( node.nodeName() == "trackcontainer" )
			{
				( (JournallingObject *)( this ) )->restoreState( node.toElement() );
			}
			else if( node.nodeName() == "controllers" )
			{
				restoreControllerStates( node.toElement() );
			}
			else if (node.nodeName() == "scales")
			{
				restoreScaleStates(node.toElement());
			}
			else if (node.nodeName() == "keymaps")
			{
				restoreKeymapStates(node.toElement());
			}
			else if (node.nodeName() == m_tempoMap.nodeName())
			{
				m_tempoMap.restoreState(node.toElement());
			}
			else if( getGUI() != nullptr )
			{
				if( node.nodeName() == getGUI()->getControllerRackView()->nodeName() )
				{
					getGUI()->getControllerRackView()->restoreState( node.toElement() );
				}
				else if( node.nodeName() == getGUI()->pianoRoll()->nodeName() )
				{
					getGUI()->pianoRoll()->restoreState( node.toElement() );
				}
				else if( node.nodeName() == getGUI()->automationEditor()->m_editor->nodeName() )
				{
					getGUI()->automationEditor()->m_editor->restoreState( node.toElement() );
				}
				else if( node.nodeName() == getGUI()->getProjectNotes()->nodeName() )
				{
					 getGUI()->getProjectNotes()->SerializingObject::restoreState( node.toElement() );
				}
				else if (node.nodeName() == getTimeline(PlayMode::Song).nodeName())
				{
					getTimeline(PlayMode::Song).restoreState(node.toElement());
				}
			}
		}
		node = node.nextSibling();
	}

	// quirk for fixing projects with broken positions of Clips inside pattern tracks
	Engine::patternStore()->fixIncorrectPositions();

	// Connect controller links to their controllers
	// now that everything is loaded
	ControllerConnection::finalizeConnections();

	// Remove dummy controllers that was added for correct connections
	m_controllers.erase(std::remove_if(m_controllers.begin(), m_controllers.end(),
		[](Controller* c){return c->type() == Controller::ControllerType::Dummy;}),
		m_controllers.end());

	// resolve all IDs so that autoModels are automated
	AutomationClip::resolveAllIDs();
	// An automation of the tempo (an older project's) becomes the tempo map's markers
	if (m_tempoMap.isEmpty()) { m_tempoMap.takeOverAutomation(&m_tempoModel, m_tempoModel.value()); }
	// Knobs whose meaning changed: their automation follows
	Engine::mixer()->convertLegacyAutomation();

	// Every instrument and sample track gets its own mixer channel, which takes over the track's effects,
	// volume, panning, mute and solo (projects from before that, or from LMMS)
	resolveLegacySolo();
	Engine::patternStore()->resolveLegacySolo();
	Engine::mixer()->setUpTrackChannels();
	// After the tracks' channels, which keep the first numbers; also for projects without a mixer
	Engine::mixer()->ensureFxReturns();
	// Folders route their tracks to their group channels
	updateFolders();
	Engine::mixer()->updateFolderRouting();

	Engine::audioEngine()->doneChangeInModel();

	ConfigManager::inst()->addRecentlyOpenedProject( fileName );

	Engine::projectJournal()->setJournalling( true );

	emit projectLoaded();

	if( isCancelled() )
	{
		m_isCancelled = false;
		createNewProject();
		return;
	}

	if ( hasErrors())
	{
		if ( getGUI() != nullptr )
		{
			QMessageBox::warning( nullptr, tr("LinuxMusic Error report"), errorSummary(),
							QMessageBox::Ok );
		}
		else
		{
			QTextStream(stderr) << Engine::getSong()->errorSummary() << Qt::endl;
		}
	}

	m_loadingProject = false;
	updateLength();
	setModified(false);
	m_loadOnLaunch = false;
}


// only save current song as filename and do nothing else
bool Song::saveProjectFile(const QString & filename, bool withResources)
{
	using gui::getGUI;

	// ensure property order in file is consistent across saves
#if QT_VERSION < QT_VERSION_CHECK(6, 6, 0)
	qSetGlobalQHashSeed(0);
#else
	QHashSeed::setDeterministicGlobalSeed();
#endif

	DataFile dataFile( DataFile::Type::SongProject );
	m_savingProject = true;

	m_tempoModel.saveSettings( dataFile, dataFile.head(), "bpm" );
	m_timeSigModel.saveSettings( dataFile, dataFile.head(), "timesig" );
	m_masterVolumeModel.saveSettings( dataFile, dataFile.head(), "mastervol" );
	m_masterPitchModel.saveSettings( dataFile, dataFile.head(), "masterpitch" );

	saveState( dataFile, dataFile.content() );

	Engine::mixer()->saveState( dataFile, dataFile.content() );
	if( getGUI() != nullptr )
	{
		getGUI()->getControllerRackView()->saveState( dataFile, dataFile.content() );
		getGUI()->pianoRoll()->saveState( dataFile, dataFile.content() );
		getGUI()->automationEditor()->m_editor->saveState( dataFile, dataFile.content() );
		getGUI()->getProjectNotes()->SerializingObject::saveState( dataFile, dataFile.content() );
		getTimeline(PlayMode::Song).saveState(dataFile, dataFile.content());
	}

	if (!m_tempoMap.isEmpty()) { m_tempoMap.saveState(dataFile, dataFile.content()); }
	saveControllerStates( dataFile, dataFile.content() );

	saveScaleStates(dataFile, dataFile.content());
	saveKeymapStates(dataFile, dataFile.content());

	m_savingProject = false;

	bool writeStatus = dataFile.writeFile(filename, withResources);

	// revert to a randomized global seed for Qt
#if QT_VERSION < QT_VERSION_CHECK(6, 6, 0)
	qSetGlobalQHashSeed(-1);
#else
	QHashSeed::resetRandomGlobalSeed();
#endif

	return writeStatus;
}



// Save the current song
bool Song::guiSaveProject()
{
	return guiSaveProjectAs(m_fileName);
}




// Save the current song with the given filename
bool Song::guiSaveProjectAs(const QString & filename)
{
	DataFile dataFile(DataFile::Type::SongProject);
	QString fileNameWithExtension = dataFile.nameWithExtension(filename);

	bool withResources = m_saveOptions.saveAsProjectBundle.value();

	bool const saveResult = saveProjectFile(fileNameWithExtension, withResources);

	// After saving, restore default save options.
	m_saveOptions.setDefaultOptions();

	// If we saved a bundle, we keep the project on the original
	// file and still keep it as modified
	if (saveResult && !withResources)
	{
		setModified(false);
		setProjectFileName(fileNameWithExtension);
	}

	return saveResult;
}




void Song::saveControllerStates( QDomDocument & doc, QDomElement & element )
{
	// save settings of controllers
	QDomElement controllersNode = doc.createElement( "controllers" );
	element.appendChild( controllersNode );
	for (const auto& controller : m_controllers)
	{
		controller->saveState(doc, controllersNode);
	}
}




void Song::restoreControllerStates( const QDomElement & element )
{
	QDomNode node = element.firstChild();
	while( !node.isNull() && !isCancelled() )
	{
		Controller * c = Controller::create( node.toElement(), this );
		if (c) {addController(c);}
		else
		{
			// Fix indices to ensure correct connections
			m_controllers.push_back(Controller::create(Controller::ControllerType::Dummy, this));
		}

		node = node.nextSibling();
	}
}



void Song::removeAllControllers()
{
	while (m_controllers.size() != 0)
	{
		removeController(m_controllers.at(0));
	}

	m_controllers.clear();
}



void Song::saveScaleStates(QDomDocument &doc, QDomElement &element)
{
	QDomElement scalesNode = doc.createElement("scales");
	element.appendChild(scalesNode);

	for (const auto& scale : m_scales)
	{
		scale->saveState(doc, scalesNode);
	}
}


void Song::restoreScaleStates(const QDomElement &element)
{
	QDomNode node = element.firstChild();

	for (auto i = std::size_t{0}; i < MaxScaleCount && !node.isNull() && !isCancelled(); i++)
	{
		m_scales[i]->restoreState(node.toElement());
		node = node.nextSibling();
	}
	emit scaleListChanged(-1);
}


void Song::saveKeymapStates(QDomDocument &doc, QDomElement &element)
{
	QDomElement keymapsNode = doc.createElement("keymaps");
	element.appendChild(keymapsNode);

	for (const auto& keymap : m_keymaps)
	{
		keymap->saveState(doc, keymapsNode);
	}
}


void Song::restoreKeymapStates(const QDomElement &element)
{
	QDomNode node = element.firstChild();

	for (auto i = std::size_t{0}; i < MaxKeymapCount && !node.isNull() && !isCancelled(); i++)
	{
		m_keymaps[i]->restoreState(node.toElement());
		node = node.nextSibling();
	}
	emit keymapListChanged(-1);
}


void Song::exportProjectMidi(const std::filesystem::path& filePath) const
{
	// instantiate midi export plugin
	TrackContainer::TrackList const & tracks = this->tracks();
	TrackContainer::TrackList const & patternStoreTracks = Engine::patternStore()->tracks();

	ExportFilter *exf = dynamic_cast<ExportFilter *> (Plugin::instantiate("midiexport", nullptr, nullptr));
	if (exf)
	{
		exf->tryExport(tracks, patternStoreTracks, getTempo(), m_masterPitchModel.value(), filePath);
	}
	else
	{
		qDebug() << "failed to load midi export filter!";
	}

}



void Song::updateFramesPerTick()
{
	Engine::updateFramesPerTick();
}




void Song::setModified()
{
	setModified(true);
}

void Song::setProjectFileName(QString const & projectFileName)
{
	if (m_fileName != projectFileName)
	{
		m_fileName = projectFileName;
		emit projectFileNameChanged();
	}
}




void Song::addController( Controller * controller )
{
	bool containsController = std::find(m_controllers.begin(), m_controllers.end(), controller) != m_controllers.end();
	if (controller && !containsController)
	{
		m_controllers.push_back(controller);
		emit controllerAdded( controller );

		this->setModified();
	}
}




void Song::removeController( Controller * controller )
{
	auto it = std::find(m_controllers.begin(), m_controllers.end(), controller);
	if (it != m_controllers.end())
	{
		m_controllers.erase(it);

		emit controllerRemoved( controller );
		delete controller;

		this->setModified();
	}
}




void Song::clearErrors()
{
	m_errors.clear();
}



void Song::collectError( const QString error )
{
	if (!m_errors.contains(error)) { m_errors[error] = 1; }
	else { m_errors[ error ]++; }
}



bool Song::hasErrors()
{
	return !(m_errors.isEmpty());
}



QString Song::errorSummary()
{
	QString errors;

	auto i = m_errors.constBegin();
	while (i != m_errors.constEnd())
	{
		errors.append( i.key() );
		if( i.value() > 1 )
		{
			errors.append( tr(" (repeated %1 times)").arg( i.value() ) );
		}
		errors.append("\n");
		++i;
	}

	errors.prepend( "\n\n" );
	errors.prepend( tr( "The following errors occurred while loading: " ) );

	return errors;
}

bool Song::isSavingProject() const {
	return m_savingProject;
}


std::shared_ptr<const Scale> Song::getScale(unsigned int index) const
{
	if (index >= MaxScaleCount) {index = 0;}

	return std::atomic_load(&m_scales[index]);
}


std::shared_ptr<const Keymap> Song::getKeymap(unsigned int index) const
{
	if (index >= MaxKeymapCount) {index = 0;}

	return std::atomic_load(&m_keymaps[index]);
}


void Song::setScale(unsigned int index, std::shared_ptr<Scale> newScale)
{
	if (index >= MaxScaleCount) {index = 0;}

	Engine::audioEngine()->requestChangeInModel();
	std::atomic_store(&m_scales[index], newScale);
	emit scaleListChanged(index);
	Engine::audioEngine()->doneChangeInModel();
}


void Song::setKeymap(unsigned int index, std::shared_ptr<Keymap> newMap)
{
	if (index >= MaxKeymapCount) {index = 0;}

	Engine::audioEngine()->requestChangeInModel();
	std::atomic_store(&m_keymaps[index], newMap);
	emit keymapListChanged(index);
	Engine::audioEngine()->doneChangeInModel();
}
} // namespace lmms
