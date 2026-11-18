/*
 * InstrumentTrack.h - declaration of class InstrumentTrack, a track which
 *                     holds an instrument-plugin
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

#ifndef LMMS_INSTRUMENT_TRACK_H
#define LMMS_INSTRUMENT_TRACK_H

#include <atomic>
#include <memory>
#include <utility>

#include <QDomDocument>

#include "AudioBusHandle.h"
#include "InstrumentFunctions.h"
#include "MonoMode.h"
#include "InstrumentSoundShaping.h"
#include "Microtuner.h"
#include "Midi.h"
#include "MidiEventProcessor.h"
#include "MidiPort.h"
#include "MixerChannelLcdModel.h"
#include "NotePlayHandle.h"
#include "Piano.h"
#include "Plugin.h"
#include "PluginInstance.h"
#include "Track.h"

namespace lmms
{


class Instrument;
class DataFile;

namespace gui
{

class InstrumentTrackView;
class InstrumentTrackWindow;
class InstrumentTuningView;
class MidiCCRackView;

} // namespace gui


class LMMS_EXPORT InstrumentTrack : public Track, public MidiEventProcessor
{
	Q_OBJECT
	mapPropertyFromModel(int,getVolume,setVolume,m_volumeModel);
public:
	InstrumentTrack( TrackContainer* tc );
private:
	/**
	 * Puts @p made in place of the instrument, with the track locked by the caller (the audio thread plays the
	 * instrument through the track only when it can lock it, see playInstrument()) and under the audio engine's
	 * change lock (it plays notes through it too). @p made is whole: made, and restored, before.
	 * @returns the old instrument, to be deleted by the caller (without the audio engine's lock)
	 */
	auto swapInstrument(std::unique_ptr<PluginInstance> made) -> std::unique_ptr<PluginInstance>;

protected:
	//! (see Track::destroy())
	~InstrumentTrack() override;

public:

	// used by instrument
	void processAudioBuffer( SampleFrame* _buf, const f_cnt_t _frames,
							NotePlayHandle * _n );

	MidiEvent applyMasterKey( const MidiEvent& event );

	void processInEvent( const MidiEvent& event, const TimePos& time = TimePos(), f_cnt_t offset = 0 ) override;
	void processOutEvent( const MidiEvent& event, const TimePos& time = TimePos(), f_cnt_t offset = 0 ) override;
	// silence all running notes played by this track
	void silenceAllNotes( bool removeIPH = false );

	auto monoMode() -> MonoMode& { return m_monoMode; }
	//! The ENV tab: envelopes, LFOs and the filter
	auto soundShaping() const -> const InstrumentSoundShaping& { return m_soundShaping; }

	bool isSustainPedalPressed() const
	{
		return m_sustainPedalPressed;
	}

	f_cnt_t beatLen( NotePlayHandle * _n ) const;


	// for capturing note-play-events -> need that for arpeggio,
	// filter and so on
	void playNote(NotePlayHandle* _n, std::span<SampleFrame> _working_buffer);

	/**
	 * Processes a single-streamed instrument for one period through its PluginInstance and writes the
	 * main track channels into @p out. Does nothing if @p instrument is not this track's current
	 * instrument, e.g. while instruments are being replaced. [audio thread]
	 */
	void playInstrument(const Instrument* instrument, std::span<SampleFrame> out);

	//! The track has as many channels as its mixer channel: the main pair and its auxiliary pairs
	auto trackChannelCount() const -> ch_cnt_t { return m_instrumentChannels.channels(); }

	QString instrumentName() const;
	const Instrument *instrument() const
	{
		return m_instrument ? std::as_const(*m_instrument).instrument() : nullptr;
	}

	Instrument *instrument()
	{
		return m_instrument ? m_instrument->instrument() : nullptr;
	}

	void deleteNotePluginData( NotePlayHandle * _n );

	// name-stuff
	void setName( const QString & _new_name ) override;

	// translate given key of a note-event to absolute key (i.e.
	// add global master-pitch and base-note of this instrument track)
	int masterKey( int _midi_key ) const;

	// translate pitch to midi-pitch [0,16383]
	int midiPitch() const
	{
		return static_cast<int>( ( ( m_pitchModel.value() + m_pitchModel.range()/2 ) * MidiMaxPitchBend ) / m_pitchModel.range() );
	}

	/*! \brief Returns current range for pitch bend in semitones */
	int midiPitchRange() const
	{
		return m_pitchRangeModel.value();
	}

	// play everything in given frame-range - creates note-play-handles
	bool play( const TimePos & _start, const f_cnt_t _frames,
						const f_cnt_t _frame_base, int _clip_num = -1 ) override;
	// create new view for me
	gui::TrackView* createView( gui::TrackContainerView* tcv ) override;

	// create new track-content-object = clip
	Clip* createClip(const TimePos & pos) override;


	// called by track
	void saveTrackSpecificSettings(QDomDocument& doc, QDomElement& parent, bool presetMode) override;
	void loadTrackSpecificSettings( const QDomElement & _this ) override;

	using Track::setJournalling;


	// load instrument whose name matches given one
	//! The host side of the instrument plugin (buffers, pin connector, state)
	PluginInstance* instrumentInstance() { return m_instrument.get(); }

	//! Makes @p instance the track's instrument (e.g. one made outside of the plugin factory, as in tests)
	void setInstrumentInstance(std::unique_ptr<PluginInstance> instance);

	Instrument * loadInstrument(const QString & _instrument_name,
				const Plugin::Descriptor::SubPluginFeatures::Key* key = nullptr,
				bool keyFromDnd = false);

	AudioBusHandle* audioBusHandle()
	{
		return &m_audioBusHandle;
	}

	MidiPort * midiPort()
	{
		return &m_midiPort;
	}

	const IntModel *baseNoteModel() const
	{
		return &m_baseNoteModel;
	}

	IntModel *baseNoteModel()
	{
		return &m_baseNoteModel;
	}

	IntModel *firstKeyModel()
	{
		return &m_firstKeyModel;
	}

	IntModel *lastKeyModel()
	{
		return &m_lastKeyModel;
	}

	bool keyRangeImport() const;

	//! @brief Check if there is a valid mapping for the given key and it is within defined of range.
	bool isKeyMapped(int key) const;

	//! @brief Return first mapped key, based on currently selected keymap or user selection.
	//! @return Number ranging from 0 to NumKeys -1
	int firstKey() const;

	//! @brief Return last mapped key, based on currently selected keymap or user selection.
	//! @return Number ranging from 0 to NumKeys -1
	int lastKey() const;

	//! @brief Return base key number, based on currently selected keymap or user selection.
	//! @return Number ranging from 0 to NumKeys -1
	int baseNote() const;

	//! @brief Return frequency assigned to the base key, based on currently selected keymap.
	//! @return Frequency in Hz
	float baseFreq() const;

	Piano *pianoModel()
	{
		return &m_piano;
	}

	Microtuner *microtuner()
	{
		return &m_microtuner;
	}

	bool isArpeggioEnabled() const
	{
		return m_arpeggio.m_arpEnabledModel.value();
	}

	// simple helper for removing midiport-XML-node when loading presets
	static void removeMidiPortNode( DataFile& dataFile );

	FloatModel * pitchModel()
	{
		return &m_pitchModel;
	}

	FloatModel * volumeModel()
	{
		return &m_volumeModel;
	}

	FloatModel * panningModel()
	{
		return &m_panningModel;
	}

	IntModel* pitchRangeModel()
	{
		return &m_pitchRangeModel;
	}

	IntModel * mixerChannelModel()
	{
		return &m_mixerChannelModel;
	}

	BoolModel* useMasterPitchModel()
	{
		return &m_useMasterPitchModel;
	}

	/**
	 * MIDI recording on the timeline (see MidiRecorder): an armed track records the notes played into it while the
	 * song records, in its record mode (a MidiRecorder::Mode).
	 */
	BoolModel* recordArmModel() { return &m_recordArmModel; }
	auto isRecordArmed() const -> bool { return m_recordArmModel.value(); }
	IntModel* midiRecordModeModel() { return &m_midiRecordModeModel; }
	/**
	 * Quantizing what is recorded (docs/gravacao-levantamento.md §6, off by default): the grid, 0 for off, else the
	 * note value (4 for 1/4, 8, 16, 32 or 64); and how far toward it a note's start moves, in percent
	 */
	IntModel* recordQuantizeModel() { return &m_recordQuantizeModel; }
	IntModel* recordQuantizeStrengthModel() { return &m_recordQuantizeStrengthModel; }
	//! Where a note played at @p tick is recorded (@p tick if the track does not quantize)
	auto quantizedRecordTick(tick_t tick) const -> tick_t;

	//! Main thread: whether the notes played into the track go to the song's MIDI recording
	void setRecordingMidi(bool recording) { m_recordingMidi = recording; }

	void setPreviewMode( const bool );

	bool isPreviewMode() const
	{
		return m_previewMode;
	}
	
	void replaceInstrument(DataFile dataFile);

	//! @brief Automatically assign a midi controller to this track, based on the midiautoassign setting
	//! @param assign Set to true to connect the midi device, set to false to disconnect
	void autoAssignMidiDevice(bool assign);

	//! Returns a non-owning pointer to the model for the knob at the given index in the track's MIDI CC rack
	FloatModel* midiCCModel(int index) const { return m_midiCCModel[index].get(); }

	//! How many notes started and ended so far (see InstrumentTrackView's activity light); wrap around
	auto notesStarted() const -> unsigned { return m_notesStarted.load(std::memory_order_relaxed); }
	auto notesEnded() const -> unsigned { return m_notesEnded.load(std::memory_order_relaxed); }

signals:
	void instrumentChanged();
	void midiNoteOn( const lmms::Note& );
	void midiNoteOff( const lmms::Note& );

protected:
	QString nodeName() const override
	{
		return "instrumenttrack";
	}

	// get the name of the instrument in the saved data
	QString getSavedInstrumentName(const QDomElement & thisElement) const;


protected slots:
	void updateBaseNote();
	void updatePitch();
	void updatePitchRange();
	void updateMixerChannel();

	//! Takes over the channel count of the mixer channel the track sends to
	void updateTrackChannelCount();

	//! Gives a MIDI event to the instrument: through its event queue if it takes events, otherwise directly
	bool deliverToInstrument(const MidiEvent& event, const TimePos& time, f_cnt_t offset);

private:
	void processCCEvent(int controller);

	MidiPort m_midiPort;

	NotePlayHandle* m_notes[NumKeys];
	NotePlayHandleList m_sustainedNotes;

	int m_runningMidiNotes[NumKeys];
	QMutex m_midiNotesMutex;

	bool m_sustainPedalPressed;

	bool m_silentBuffersProcessed;

	bool m_previewMode;

	IntModel m_baseNoteModel;	//!< The "A4" or "440 Hz" key (default 69)
	IntModel m_firstKeyModel;	//!< First key the instrument reacts to
	IntModel m_lastKeyModel;	//!< Last key the instrument reacts to

	bool m_hasAutoMidiDev;
	static InstrumentTrack *s_autoAssignedTrack;

	NotePlayHandle::ProcessHandleList m_processHandles; //!< its notes without a parent (linked, allocating nothing)

	FloatModel m_volumeModel;
	FloatModel m_panningModel;

	AudioBusHandle m_audioBusHandle;

	FloatModel m_pitchModel;
	IntModel m_pitchRangeModel;
	MixerChannelLcdModel m_mixerChannelModel;
	BoolModel m_useMasterPitchModel;
	BoolModel m_recordArmModel;
	IntModel m_midiRecordModeModel;
	//! The saved instrument that could not be loaded (a plugin missing here): kept as it was, and saved so, until
	//! another instrument is chosen
	QDomDocument m_unloadedInstrument;
	IntModel m_recordQuantizeModel;
	IntModel m_recordQuantizeStrengthModel;
	std::atomic<bool> m_recordingMidi = false; //!< read by the MIDI thread

	//! The notes started and ended so far (by the MIDI input's and the song's threads), for the GUI's activity light
	std::atomic<unsigned> m_notesStarted = 0;
	std::atomic<unsigned> m_notesEnded = 0;

	std::unique_ptr<PluginInstance> m_instrument;

	//! The track channels single-streamed instruments are processed into
	//! TODO: Becomes the track's channels once tracks have more than two
	AudioBuffer m_instrumentChannels;

	//! The instrument's latency, for the mixer's delay compensation (see Mixer::addLatencySource())
	std::atomic<f_cnt_t> m_latency = 0;

	InstrumentSoundShaping m_soundShaping;
	InstrumentFunctionArpeggio m_arpeggio;
	//! One note at a time, legato and with glide (#6516): see MonoMode
	MonoMode m_monoMode;
	//! The keys let go (from a device or the piano) while the sustain pedal is down, in mono: let go when it rises
	std::array<bool, NumKeys> m_monoSustained = {};
	InstrumentFunctionNoteStacking m_noteStacking;

	Piano m_piano;

	Microtuner m_microtuner;

	std::unique_ptr<BoolModel> m_midiCCEnable;
	std::unique_ptr<FloatModel> m_midiCCModel[MidiControllerCount];

	friend class gui::InstrumentTrackView;
	friend class gui::InstrumentTrackWindow;
	friend class NotePlayHandle;
	friend class gui::InstrumentTuningView;
	friend class gui::MidiCCRackView;

} ;



} // namespace lmms

#endif // LMMS_INSTRUMENT_TRACK_H
