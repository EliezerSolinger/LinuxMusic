/*
 * Song.h - class song - the root of the model-tree
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

#ifndef LMMS_SONG_H
#define LMMS_SONG_H

#include <array>
#include <atomic>
#include <memory>
#include <span>

#include <QString>
#include <QHash>  // IWYU pragma: keep
#include <QLockFile>
#include <QSet>

#include "AudioEngine.h"
#include "AudioRecorder.h"
#include "MidiRecorder.h"
#include "SongMarkers.h"
#include "DataFile.h"
#include "TempoMap.h"
#include "Transport.h"
#include "SampleClip.h"
#include "Controller.h"
#include "Metronome.h"
#include "lmms_constants.h"
#include "MeterModel.h"
#include "Timeline.h"
#include "TrackContainer.h"
#include "VstSyncController.h"

#include <filesystem> // QTBUG-73263

namespace lmms
{

class AudioTrack;
class AudioItem;

class AutomationTrack;
class Keymap;
class MidiClip;
class Scale;

namespace gui
{
class SongEditor;
class ControllerRackView;
}


const bpm_t MinTempo = 10;
const bpm_t DefaultTempo = 120; //!< (LMMS's was 140: see LegacyDefaultTempo)
//! The tempo of a project that does not say it, made when the default was 140
const bpm_t LegacyDefaultTempo = 140;
const bpm_t MaxTempo = 999;
const tick_t MaxSongLength = 9999 * DefaultTicksPerBar;


class LMMS_EXPORT Song : public TrackContainer
{
	Q_OBJECT
	mapPropertyFromModel( int,getTempo,setTempo,m_tempoModel );
	mapPropertyFromModel( int,masterPitch,setMasterPitch,m_masterPitchModel );
	mapPropertyFromModel( int,masterVolume,setMasterVolume, m_masterVolumeModel );
public:
	enum class PlayMode
	{
		None,
		Song,
		Pattern,
		MidiClip,
		AutomationClip,
		Count
	} ;
	constexpr static auto PlayModeCount = static_cast<std::size_t>(PlayMode::Count);

	struct SaveOptions {
		/**
		 * Should we discard MIDI ControllerConnections from project files?
		 */
		BoolModel discardMIDIConnections{false};
		/**
		 * Should we save the project as a project bundle? (with resources)
		 */
		BoolModel saveAsProjectBundle{false};

		void setDefaultOptions() {
			discardMIDIConnections.setValue(false);
			saveAsProjectBundle.setValue(false);
		}
	};

	void clearErrors();
	void collectError( const QString error );
	bool hasErrors();
	QString errorSummary();

	void processNextBuffer();

	inline int getLoadingTrackCount() const
	{
		return m_nLoadingTrack;
	}

	inline int getMilliseconds() const
	{
		return getMilliseconds(m_playMode);
	}

	/**
	 * The milliseconds of the song up to the play head. In song mode, along the tempo map: the transport's frames
	 * while the song plays, counted from the play head's tick when it does not
	 */
	auto getMilliseconds(PlayMode playMode) const -> int;

	//! Where the play position is in bars: its bar (from 0) and beat (from 0), the ticks into the beat, where the
	//! bar starts and its time signature (the song's bars when it plays the song, otherwise the song's time
	//! signature's from the start)
	struct MusicalPosition
	{
		int bar = 0;
		int beat = 0;
		int tick = 0;
		tick_t barStart = 0;
		TempoMap::Meter meter;
	};
	auto musicalPosition() const -> MusicalPosition;

	inline int getBars() const
	{
		return musicalPosition().bar;
	}

	inline int ticksPerBar() const
	{
		return TimePos::ticksPerBar(m_timeSigModel);
	}

	// Returns the beat position inside the bar, 0-based
	inline int getBeat() const
	{
		return musicalPosition().beat;
	}
	// the remainder after bar and beat are removed
	inline int getBeatTicks() const
	{
		return musicalPosition().tick;
	}
	inline int getTicks() const
	{
		return currentTick();
	}
	inline f_cnt_t getFrames() const
	{
		return currentFrame();
	}
	inline bool isPaused() const
	{
		return m_paused;
	}

	inline bool isPlaying() const
	{
		return m_playing == true && m_exporting == false;
	}

	inline bool isStopped() const
	{
		return m_playing == false && m_paused == false;
	}

	inline bool isExporting() const
	{
		return m_exporting;
	}

	inline void setExportLoop( bool exportLoop )
	{
		m_exportLoop = exportLoop;
	}

	inline bool isRecording() const
	{
		return m_recording;
	}
	
	inline void setLoopRenderCount(int count)
	{
		if (count < 1)
			m_loopRenderCount = 1;
		else
			m_loopRenderCount = count;
		m_loopRenderRemaining = m_loopRenderCount;
	}

	inline int getLoopRenderCount() const
	{
		return m_loopRenderCount;
	}

	bool isExportDone() const;
	int getExportProgress() const;

	inline void setRenderBetweenMarkers( bool renderBetweenMarkers )
	{
		m_renderBetweenMarkers = renderBetweenMarkers;
	}

	inline PlayMode playMode() const
	{
		return m_playMode;
	}

	TimePos getPlayPos(PlayMode pm) const
	{
		return getTimeline(pm).pos();
	}
	TimePos getPlayPos() const
	{
		return getPlayPos(m_playMode);
	}

	void setPlayPos(tick_t ticks, PlayMode playMode)
	{
		getTimeline(playMode).setTicks(ticks);
		m_transport.positionSet();
	}
	void setPlayPos(tick_t ticks)
	{
		setPlayPos(ticks, m_playMode);
	}

	auto getTimeline(PlayMode mode) -> Timeline& { return m_timelines[static_cast<std::size_t>(mode)]; }
	auto getTimeline(PlayMode mode) const -> const Timeline& { return m_timelines[static_cast<std::size_t>(mode)]; }
	auto getTimeline() -> Timeline& { return getTimeline(m_playMode); }
	auto getTimeline() const -> const Timeline& { return getTimeline(m_playMode); }

	void updateLength();
	bar_t length() const
	{
		return m_length;
	}


	bpm_t getTempo();

	AutomationTrack * globalAutomationTrack()
	{
		return m_globalAutomationTrack;
	}

	//TODO: Add Q_DECL_OVERRIDE when Qt4 is dropped
	void automatedValuesAt(TimePos time, int clipNum, AutomatedValueMap& values) const override;
	using TrackContainer::automatedValuesAt;

	// file management
	void createNewProject();
	//! The default template (or an empty project without one), before the settings for new projects apply
	void createNewProjectFromDefault();
	void createNewProjectFromTemplate( const QString & templ );
	void loadProject( const QString & filename );
	bool guiSaveProject();
	bool guiSaveProjectAs(const QString & filename);
	/**
	 * Saves the project to @p filename. With @p gatherMedia (a save of the project itself, not the autosave), the
	 * files it uses are gathered into its folder first (see ProjectMedia)
	 */
	bool saveProjectFile(const QString & filename, bool withResources = false, bool gatherMedia = false);
	/**
	 * The project as it would be saved, without writing it (for the autosave, which writes it on another thread).
	 * While recording, the takes being written are in it too (see recoverInterruptedTakes()).
	 */
	auto serializeProject() -> DataFile;

	const QString & projectFileName() const
	{
		return m_fileName;
	}

	bool isLoadingProject() const
	{
		return m_loadingProject;
	}

	void loadingCancelled()
	{
		m_isCancelled = true;
		Engine::audioEngine()->clearNewPlayHandles();
	}

	bool isCancelled()
	{
		return m_isCancelled;
	}

	bool isModified() const
	{
		return m_modified;
	}

	QString nodeName() const override
	{
		return "song";
	}

	virtual bool fixedClips() const
	{
		return false;
	}

	void addController( Controller * c );
	void removeController( Controller * c );


	const ControllerVector & controllers() const
	{
		return m_controllers;
	}


	MeterModel & getTimeSigModel()
	{
		return m_timeSigModel;
	}

	IntModel& tempoModel()
	{
		return m_tempoModel;
	}

	IntModel& masterVolumeModel() { return m_masterVolumeModel; }
	IntModel& masterPitchModel() { return m_masterPitchModel; }

	void exportProjectMidi(const std::filesystem::path& filePath) const;

	inline void setLoadOnLaunch(bool value) { m_loadOnLaunch = value; }
	SaveOptions &getSaveOptions() {
		return m_saveOptions;
	}

	bool isSavingProject() const;

	std::shared_ptr<const Scale> getScale(unsigned int index) const;
	std::shared_ptr<const Keymap> getKeymap(unsigned int index) const;
	void setScale(unsigned int index, std::shared_ptr<Scale> newScale);
	void setKeymap(unsigned int index, std::shared_ptr<Keymap> newMap);

	const std::string& syncKey() const noexcept { return m_vstSyncController.sharedMemoryKey(); }

	Metronome& metronome() { return m_metronome; }

	void removeOldAutomatedValue(AutomatableModel* am);

	/**
	 * Recording (docs/plano-gravacao.md, "Gravação"): starts playing the song and recording the armed tracks: the
	 * audio input of the sample tracks, the notes played into the instrument tracks. While the song plays, recording
	 * starts there and then (punch in). On stop, each take becomes a clip on its track.
	 * @returns false, with recordingProblem() emitted, if nothing could be recorded (e.g. no armed track)
	 */
	auto record() -> bool;

	//! @returns whether audio is being recorded
	auto isRecordingAudio() const -> bool { return m_audioRecorder.isRecording(); }

	/**
	 * Parts of the project that others keep (the GUI's editors and notes), saved in the project and loaded back
	 * by their node names, in the order they were added. Elements of the project nobody here reads (such parts,
	 * when there is no GUI, or a newer program's) are kept as they were and saved back unchanged.
	 */
	void addProjectState(SerializingObject* state);
	void removeProjectState(SerializingObject* state);

	//! Whether the Piano Roll records into its clip what is played (set by it, read by the audio thread)
	void setRecordingInPianoRoll(bool recording) { m_recordingInPianoRoll = recording; }
	auto isRecordingInPianoRoll() const -> bool { return m_recordingInPianoRoll; }

	//! Main thread: the takes being recorded, as their waveform grows (refreshLiveTakes() brings them up to date)
	auto liveTakes() const -> const std::vector<AudioRecorder::LiveTake>& { return m_liveTakes; }
	void refreshLiveTakes();

	//! Ends the recording (punch out) while the song plays on; the takes become clips
	void stopRecording();

	auto midiRecorder() -> MidiRecorder& { return m_midiRecorder; }

	auto tempoMap() -> TempoMap& { return m_tempoMap; }
	auto tempoMap() const -> const TempoMap& { return m_tempoMap; }
	//! What the song is doing and where, and the global play, stop and record (see Transport)
	auto transport() -> Transport& { return m_transport; }

	//! The song's named markers ("Chorus"...), see SongMarkers
	auto markers() -> SongMarkers& { return m_songMarkers; }

	//! @returns the frames the song plays from tick @p from to tick @p to at its tempo now (and tempo map)
	auto framesBetween(tick_t from, tick_t to) const -> double;

	/**
	 * Where a recording must be held to the grid besides each bar (docs/plano-gravacao.md, "Time-stretch
	 * renderizado"): the ticks strictly between @p from and @p to where the tempo changes, and each beat of a ramp.
	 * A clip crossing any number of changes then lines up with the grid everywhere.
	 */
	auto tempoKeyTicks(tick_t from, tick_t to) const -> std::vector<tick_t>;
	//! @p timing (a recording's, from @p start) with a point at each of tempoKeyTicks() it lacks, its frame
	//! interpolated between the points around it
	auto withTempoKeys(const std::vector<SampleClip::TimingPoint>& timing, tick_t start) const
		-> std::vector<SampleClip::TimingPoint>;

	//! @returns the timing of @p frames recorded from tick @p startTick at the song's tempo now (see SampleClip)
	auto timingOf(tick_t startTick, double frames) const -> std::vector<SampleClip::TimingPoint>;

	//! A recording to render for the song's tempo now (see SampleClip::isTempoBound())
	struct StretchJob
	{
		SampleClip* clip = nullptr; //!< the clip to render, or
		AudioItem* item = nullptr; //!< the audio track's item to render
		QString source; //!< the original file
		std::vector<SampleClip::TimingPoint> from; //!< its timing
		std::vector<SampleClip::TimingPoint> to; //!< the timing at the song's tempo now
		QString target; //!< the file to render to
		double seconds = 0; //!< how long the original is
	};

	/**
	 * @returns the recordings whose timing at the song's tempo now differs from the one they play at. Those back at
	 *          their original timing go back to their original file here and are no job. Clips playing the same
	 *          recording at the same place (the parts of a split clip) share one target: render it once.
	 */
	auto stretchJobs() -> std::vector<StretchJob>;
	//! Makes the clip or item of @p job play its rendering (once rendered)
	static void applyStretch(const StretchJob& job);

	//! Something may have changed the timing a recording must play at (a clip moved, a take chosen, a clip restored
	//! by undo): the GUI checks stretchJobs() again
	void requestStretchCheck() { emit stretchCheckRequested(); }

	//! Any thread: the tick being heard now (the period being rendered, less the output's latency and the period
	//! the engine hands the device ahead)
	auto heardTick() const -> tick_t;

	//! @returns the folder where recordings of this project go: "<project>_media" next to the project file, or a
	//! folder of its own in the working directory's "recordings" while the project has no file
	auto mediaDirectory() const -> QString;

	/**
	 * The files the program made for this project that it no longer uses (docs/plano-gravacao.md, "Mídia do
	 * projeto"): takes undone or discarded, renderings for a tempo left behind. When the project is closed, the
	 * ones its saved file does not refer to go to the trash (ProjectMedia::cleanOrphans()); an unsaved project
	 * keeps none. Done when the project closes, so the undo history never refers to a file that went.
	 */
	void cleanGeneratedMedia() { cleanGeneratedMediaOf(m_fileName); }
	//! At start: the folders of unsaved projects left by a run that ended without closing them (a crash), that no
	//! running instance holds and the project open now does not use
	void cleanAbandonedMedia();
	//! The files the clips of the project open now use (their files, takes and originals; "local:" relative to
	//! @p projectFile)
	auto filesInUse(const QString& projectFile) const -> QSet<QString>;

	//! @returns the frames between a sound played and its recording: the device's latencies and the calibration
	auto recordingLatency() const -> f_cnt_t;

	/**
	 * Soon (on the event loop, once for many calls): opens the audio device's input if a sample track needs it (it
	 * has an input and is armed or always monitored, or audio is being recorded), or closes it
	 */
	void scheduleCaptureUpdate();

public slots:
	void playSong();
	void playAndRecord();
	void playPattern();
	void playMidiClip( const lmms::MidiClip * midiClipToPlay, bool loop = true );
	void togglePause();
	void stop();

	void startExport();
	void stopExport();


	void setModified();

	void clearProject();

	void addPatternTrack();


private slots:
	void insertBar();
	void removeBar();
	void addSampleTrack();
	void addAudioTrack();

public:
	/**
	 * The sample rate is the project's (docs/gravacao-levantamento.md §6): a project keeps the rate it was made at,
	 * and opening it opens the audio device at that rate (a new project: the settings' rate). A device that does not
	 * have it runs at its own, the files are converted, and the project's errors say so. @p rate 0: the settings'.
	 * Not when only rendering (the command line chooses its rate).
	 */
	void applyProjectSampleRate(sample_rate_t rate);

public slots:
	void addAutomationTrack();
	void addFolderTrack();

	void setTempo();
	void setTimeSignature();

	void masterVolumeChanged();

	void savePlayStartPosition();

	void updateFramesPerTick();



private:
	//! Makes clips of the takes just recorded
	void finishAudioRecording(const std::vector<AudioRecorder::Take>& takes);

	//! Ends the recording of notes at @p atTick
	void finishMidiRecording(tick_t atTick);

	Song();
	Song( const Song & );
	~Song() override;


	inline bar_t currentBar() const
	{
		return getPlayPos(m_playMode).getBar();
	}

	inline tick_t currentTick() const
	{
		return getPlayPos(m_playMode).getTicks();
	}

	//! The timeline frame of the period being played (see Transport::frame())
	inline f_cnt_t currentFrame() const
	{
		return static_cast<f_cnt_t>(std::max<std::int64_t>(0, m_transport.frame()));
	}

	void saveControllerStates( QDomDocument & doc, QDomElement & element );
	void restoreControllerStates( const QDomElement & element );

	void removeAllControllers();

	void saveScaleStates(QDomDocument &doc, QDomElement &element);
	void restoreScaleStates(const QDomElement &element);

	void saveKeymapStates(QDomDocument &doc, QDomElement &element);
	void restoreKeymapStates(const QDomElement &element);

	void processAutomations(std::span<Track* const> tracks, TimePos timeStart, f_cnt_t frames);
	void processMetronome(size_t bufferOffset);

	void setModified(bool value);

	void setProjectFileName(QString const & projectFileName);

	AutomationTrack * m_globalAutomationTrack;

	IntModel m_tempoModel;
	MeterModel m_timeSigModel;
	int m_oldTicksPerBar;
	IntModel m_masterVolumeModel;
	IntModel m_masterPitchModel;

	ControllerVector m_controllers;

	int m_nLoadingTrack;

	QString m_fileName;
	QString m_oldFileName;
	bool m_modified;
	bool m_loadOnLaunch;

	// The transport's state: written by the main thread or the export thread, read by the audio thread and the
	// interface
	std::atomic<bool> m_recording;
	std::atomic<bool> m_exporting;
	std::atomic<bool> m_recordingInPianoRoll = false;
	std::atomic<bool> m_exportLoop;
	std::atomic<bool> m_renderBetweenMarkers;
	std::atomic<bool> m_playing;
	std::atomic<bool> m_paused;

	bool m_savingProject;
	bool m_loadingProject;
	bool m_isCancelled;

	SaveOptions m_saveOptions;

	QHash<QString, int> m_errors;

	std::array<Timeline, PlayModeCount> m_timelines;

	std::atomic<PlayMode> m_playMode;

	AudioRecorder m_audioRecorder;
	bool m_captureUpdatePending = false;

	std::vector<AudioRecorder::LiveTake> m_liveTakes;
	QString m_unsavedMediaId; //!< names the media folder of the project while it has no file
	QSet<QString> m_generatedFolders; //!< the folders the program made files in for this project
	std::unique_ptr<QLockFile> m_unsavedMediaLock; //!< held while this instance uses its unsaved media folder
	//! Notes a file the program made for this project
	void noteGenerated(const QString& path);
	//! cleanGeneratedMedia() for the project open now, whose file is @p projectFile (loading another project changes
	//! the file name before the project open is cleared), and a media folder of its own for the next one
	void cleanGeneratedMediaOf(const QString& projectFile);
	//! A recording on an audio track: its passes as new lanes on top
	void landOnLanes(AudioTrack* track, const std::vector<AudioRecorder::Segment>& segments);
	/**
	 * The takes a recording was writing when the program stopped (a snapshot of the autosave): their files, whose
	 * header may not say how much was written, are completed, and they land on their tracks as a recording would
	 */
	void recoverInterruptedTakes(const QDomElement& element);
	//! The media folder of a project whose file is @p projectFile
	auto mediaDirectoryOf(const QString& projectFile) const -> QString;
	std::atomic<tick_t> m_countInUntilTick = -1; //!< the metronome clicks before this tick (a recording's count-in)
	//! Where the period being rendered starts, in ticks (written by the audio thread)
	std::atomic<double> m_periodStartTick = 0;
	MidiRecorder m_midiRecorder;
	TempoMap m_tempoMap;
	Transport m_transport{*this};
	SongMarkers m_songMarkers;

	//! See addProjectState()
	std::vector<SerializingObject*> m_projectStates;
	//! The project's elements nobody read when it loaded, saved back as they were (see addProjectState())
	QDomDocument m_keptProjectElements;
	bar_t m_length;

	const MidiClip* m_midiClipToPlay;
	bool m_loopMidiClip;

	VstSyncController m_vstSyncController;

	int m_loopRenderCount;
	int m_loopRenderRemaining;
	TimePos m_exportSongBegin;
	TimePos m_exportLoopBegin;
	TimePos m_exportLoopEnd;
	TimePos m_exportSongEnd;
	TimePos m_exportEffectiveLength;

	std::shared_ptr<Scale> m_scales[MaxScaleCount];
	std::shared_ptr<Keymap> m_keymaps[MaxKeymapCount];

	AutomatedValueMap m_oldAutomatedValues;
	AutomatedValueMap m_automatedValues; //!< (the audio thread's, reused each tick)
	std::vector<const AutomatableModel*> m_recordedModels; //!< (the same)
	Track::clipVector m_automationClips; //!< (the same)
	mutable TrackList m_automationTracks; //!< (the same)

	Metronome m_metronome;

	friend class Engine;
	friend class Transport;
	friend class gui::SongEditor;
	friend class gui::ControllerRackView;

signals:
	//! The project is about to be cleared: views of its tracks, clips and channels let go of them (main thread)
	void aboutToClearProject();
	void projectLoaded();
	void playbackStateChanged();
	void playbackPositionJumped();
	void lengthChanged( int bars );
	void tempoChanged( lmms::bpm_t newBPM );
	void timeSignatureChanged( int oldTicksPerBar, int ticksPerBar );
	void controllerAdded( lmms::Controller * );
	void controllerRemoved( lmms::Controller * );
	void stopped();
	void modified();
	//! The project changed (each change, not only the first after saving): for the autosave
	void changed();
	void projectFileNameChanged();
	void recordingProblem(const QString& message);
	void stretchCheckRequested();
	void scaleListChanged(int index);
	void keymapListChanged(int index);
} ;


} // namespace lmms

#endif // LMMS_SONG_H
