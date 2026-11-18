/*
 * Sf2Player.cpp - a soundfont2 player using fluidSynth
 *
 * Copyright (c) 2008 Paul Giblock <drfaygo/at/gmail/dot/com>
 * Copyright (c) 2009-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "Sf2Player.h"
#include "Mixer.h"

#include <optional>
#include <utility>

#include <fluidsynth.h>
#include <QDebug>
#include <QDomElement>
#include <QLabel>

#include "ArrayVector.h"
#include "AudioEngine.h"
#include "ConfigManager.h"
#include "FileDialog.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "InstrumentPlayHandle.h"
#include "Knob.h"
#include "NotePlayHandle.h"
#include "PathUtil.h"
#include "PixmapButton.h"
#include "Song.h"

#include "PatchesDialog.h"
#include "LcdSpinBox.h"

#include "embed.h"
#include "plugin_export.h"

#define FLUID_REVERB_DEFAULT_ROOMSIZE 0.2f
#define FLUID_REVERB_DEFAULT_DAMP 0.0f
#define FLUID_REVERB_DEFAULT_WIDTH 0.5f
#define FLUID_REVERB_DEFAULT_LEVEL 0.9f

#define FLUID_CHORUS_DEFAULT_N 3
#define FLUID_CHORUS_DEFAULT_LEVEL 2.0f
#define FLUID_CHORUS_DEFAULT_SPEED 0.3f
#define FLUID_CHORUS_DEFAULT_DEPTH 8.0f

namespace lmms
{


extern "C"
{

Plugin::Descriptor PLUGIN_EXPORT sf2player_plugin_descriptor =
{
	LMMS_STRINGIFY( PLUGIN_NAME ),
	"Sf2 Player",
	QT_TRANSLATE_NOOP( "PluginBrowser", "Player for SoundFont files" ),
	"Paul Giblock <drfaygo/at/gmail/dot/com>",
	0x0100,
	Plugin::Type::Instrument,
	new PluginPixmapLoader( "logo" ),
	"sf2,sf3",
	nullptr,
} ;

}

/**
 * A non-owning reference to a single FluidSynth voice. Captures some initial
 * properties of the referenced voice to help manage changes to it over time.
 */
class FluidVoice
{
public:
	//! Create a reference to the voice currently pointed at by `voice`.
	explicit FluidVoice(fluid_voice_t* voice) :
		m_voice{voice},
		m_id{fluid_voice_get_id(voice)},
		m_coarseTune{fluid_voice_gen_get(voice, GEN_COARSETUNE)}
	{ }

	//! Get a pointer to the referenced voice.
	fluid_voice_t* get() const noexcept { return m_voice; }

	//! Get the original coarse tuning of the referenced voice.
	float coarseTune() const noexcept { return m_coarseTune; }

	//! Test whether this object still refers to the original voice.
	bool isValid() const
	{
		return fluid_voice_get_id(m_voice) == m_id && fluid_voice_is_playing(m_voice);
	}

private:
	fluid_voice_t* m_voice;
	unsigned int m_id;
	float m_coarseTune;
};

struct Sf2PluginData
{
	int midiNote;
	int lastPanning;
	float lastVelocity;
	// The soundfonts I checked used at most two voices per note, so space for
	// four should be safe. This may need to be increased if a soundfont with
	// more voices per note is found.
	ArrayVector<FluidVoice, 4> fluidVoices;
	bool isNew;
	f_cnt_t offset;
	bool noteOffSent;
	panning_t panning;
};



Sf2Instrument::Sf2Instrument( InstrumentTrack * _instrument_track ) :
	LegacyInstrument(&sf2player_plugin_descriptor, _instrument_track, nullptr, Flag::IsSingleStreamed),
	m_resampler(AudioResampler::Mode::Linear),
	m_synth(nullptr),
	m_font( nullptr ),
	m_fontId( 0 ),
	m_filename( "" ),
	m_lastMidiPitch( -1 ),
	m_lastMidiPitchRange( -1 ),
	m_channel( 1 ),
	m_bankNum( 0, 0, 999, this, tr("Bank") ),
	m_patchNum( 0, 0, 127, this, tr("Patch") ),
	m_gain( 1.0f, 0.0f, 5.0f, 0.01f, this, tr( "Gain" ) ),
	m_reverbOn( false, this, tr( "Reverb" ) ),
	m_reverbRoomSize( FLUID_REVERB_DEFAULT_ROOMSIZE, 0, 1.0, 0.01f, this, tr( "Reverb room size" ) ),
	m_reverbDamping(FLUID_REVERB_DEFAULT_DAMP, 0, 1.f, 0.01f, this, tr("Reverb damping")),
	m_reverbWidth( FLUID_REVERB_DEFAULT_WIDTH, 0, 1.0, 0.01f, this, tr( "Reverb width" ) ),
	m_reverbLevel( FLUID_REVERB_DEFAULT_LEVEL, 0, 1.0, 0.01f, this, tr( "Reverb level" ) ),
	m_chorusOn( false, this, tr( "Chorus" ) ),
	m_chorusNum( FLUID_CHORUS_DEFAULT_N, 0, 10.0, 1.0, this, tr( "Chorus voices" ) ),
	m_chorusLevel(FLUID_CHORUS_DEFAULT_LEVEL, 0, 10.f, 0.01f, this, tr("Chorus level")),
	m_chorusSpeed(FLUID_CHORUS_DEFAULT_SPEED, 0.29f, 5.f, 0.01f, this, tr("Chorus speed")),
	m_chorusDepth(FLUID_CHORUS_DEFAULT_DEPTH, 0, 46.f, 0.05f, this, tr("Chorus depth"))
{
	// Room for the notes of a period (it would grow on the audio thread)
	m_playingNotes.reserve(1024);

	// Deactivate all audio drivers in fluidsynth
	const char* none[] = { nullptr };
	fluid_audio_driver_register(none);

	m_settings = new_fluid_settings();

	//fluid_settings_setint( m_settings, (char *) "audio.period-size", engine::audioEngine()->framesPerPeriod() );

	// This sets up m_synth and updates reverb/chorus/gain
	reloadSynth();

	// Get the default values from the setting
	double settingVal = 0;

	fluid_settings_getnum_default(m_settings, "synth.reverb.room-size", &settingVal);
	m_reverbRoomSize.setInitValue(settingVal);
	fluid_settings_getnum_default(m_settings, "synth.reverb.damping", &settingVal);
	m_reverbDamping.setInitValue(settingVal);
	fluid_settings_getnum_default(m_settings, "synth.reverb.width", &settingVal);
	m_reverbWidth.setInitValue(settingVal);
	fluid_settings_getnum_default(m_settings, "synth.reverb.level", &settingVal);
	m_reverbLevel.setInitValue(settingVal);

	fluid_settings_getnum_default(m_settings, "synth.chorus.nr", &settingVal);
	m_chorusNum.setInitValue(settingVal);
	fluid_settings_getnum_default(m_settings, "synth.chorus.level", &settingVal);
	m_chorusLevel.setInitValue(settingVal);
	fluid_settings_getnum_default(m_settings, "synth.chorus.speed", &settingVal);
	m_chorusSpeed.setInitValue(settingVal);
	fluid_settings_getnum_default(m_settings, "synth.chorus.depth", &settingVal);
	m_chorusDepth.setInitValue(settingVal);

	// FIXME: there's no good way to tell if we're loading a preset or an empty instrument
	// We rely on instantiate() to load the default soundfont for new instruments,
	// but we don't need that when loading a project/preset/preview
	if (!Engine::getSong()->isLoadingProject() && !instrumentTrack()->isPreviewMode())
	{
		loadFile(ConfigManager::inst()->defaultSoundFont());
	}

	connect( &m_bankNum, SIGNAL( dataChanged() ), this, SLOT( updatePatch() ) );
	connect( &m_patchNum, SIGNAL( dataChanged() ), this, SLOT( updatePatch() ) );

	connect(Engine::audioEngine(), SIGNAL(sampleRateChanged()), this, SLOT(reloadSynth()));
	// An export (or a new project) starts from rest: a new synth, the same each time (FluidSynth's voices keep some
	// of what they played: a reset synth is not quite a new one)
	connect(Engine::mixer(), &Mixer::processingStateDiscarding, this, &Sf2Instrument::reloadSynth);

	// Gain
	connect( &m_gain, SIGNAL( dataChanged() ), this, SLOT( updateGain() ) );

	// Reverb
	connect( &m_reverbOn, SIGNAL( dataChanged() ), this, SLOT( updateReverbOn() ) );
	connect( &m_reverbRoomSize, SIGNAL( dataChanged() ), this, SLOT( updateReverb() ) );
	connect( &m_reverbDamping, SIGNAL( dataChanged() ), this, SLOT( updateReverb() ) );
	connect( &m_reverbWidth, SIGNAL( dataChanged() ), this, SLOT( updateReverb() ) );
	connect( &m_reverbLevel, SIGNAL( dataChanged() ), this, SLOT( updateReverb() ) );

	// Chorus
	connect( &m_chorusOn, SIGNAL( dataChanged() ), this, SLOT( updateChorusOn() ) );
	connect( &m_chorusNum, SIGNAL( dataChanged() ), this, SLOT( updateChorus() ) );
	connect( &m_chorusLevel, SIGNAL( dataChanged() ), this, SLOT( updateChorus() ) );
	connect( &m_chorusSpeed, SIGNAL( dataChanged() ), this, SLOT( updateChorus() ) );
	connect( &m_chorusDepth, SIGNAL( dataChanged() ), this, SLOT( updateChorus() ) );
	
	// Microtuning
	connect(Engine::getSong(), &Song::scaleListChanged, this, &Sf2Instrument::updateTuning);
	connect(Engine::getSong(), &Song::keymapListChanged, this, &Sf2Instrument::updateTuning);
	connect(instrumentTrack()->microtuner()->enabledModel(), &Model::dataChanged, this, &Sf2Instrument::updateTuning, Qt::DirectConnection);
	connect(instrumentTrack()->microtuner()->scaleModel(), &Model::dataChanged, this, &Sf2Instrument::updateTuning, Qt::DirectConnection);
	connect(instrumentTrack()->microtuner()->keymapModel(), &Model::dataChanged, this, &Sf2Instrument::updateTuning, Qt::DirectConnection);
	connect(instrumentTrack()->microtuner()->keyRangeImportModel(), &Model::dataChanged, this, &Sf2Instrument::updateTuning, Qt::DirectConnection);
	connect(instrumentTrack()->baseNoteModel(), &Model::dataChanged, this, &Sf2Instrument::updateTuning, Qt::DirectConnection);

	auto iph = new InstrumentPlayHandle(this, _instrument_track);
	Engine::audioEngine()->addPlayHandle( iph );
}



Sf2Instrument::~Sf2Instrument()
{
	// Its own play handle only: the track's notes are the track's, and the instrument taking its place may be
	// there already (see AudioEngine::removePlayHandlesOf())
	Engine::audioEngine()->removePlayHandlesOf(this);
	freeFont();
	delete_fluid_synth( m_synth );
	delete_fluid_settings( m_settings );
}



void Sf2Instrument::saveSettings( QDomDocument & _doc, QDomElement & _this )
{
	_this.setAttribute( "src", m_filename );
	m_patchNum.saveSettings( _doc, _this, "patch" );
	m_bankNum.saveSettings( _doc, _this, "bank" );

	m_gain.saveSettings( _doc, _this, "gain" );

	m_reverbOn.saveSettings( _doc, _this, "reverbOn" );
	m_reverbRoomSize.saveSettings( _doc, _this, "reverbRoomSize" );
	m_reverbDamping.saveSettings( _doc, _this, "reverbDamping" );
	m_reverbWidth.saveSettings( _doc, _this, "reverbWidth" );
	m_reverbLevel.saveSettings( _doc, _this, "reverbLevel" );

	m_chorusOn.saveSettings( _doc, _this, "chorusOn" );
	m_chorusNum.saveSettings( _doc, _this, "chorusNum" );
	m_chorusLevel.saveSettings( _doc, _this, "chorusLevel" );
	m_chorusSpeed.saveSettings( _doc, _this, "chorusSpeed" );
	m_chorusDepth.saveSettings( _doc, _this, "chorusDepth" );
}




void Sf2Instrument::loadSettings( const QDomElement & _this )
{
	QString src = _this.attribute("src");
	if (!src.isEmpty())
	{
		openFile(src, false);
		m_patchNum.loadSettings(_this, "patch");
		m_bankNum.loadSettings(_this, "bank");
	}
	
	m_gain.loadSettings( _this, "gain" );

	m_reverbOn.loadSettings( _this, "reverbOn" );
	m_reverbRoomSize.loadSettings( _this, "reverbRoomSize" );
	m_reverbDamping.loadSettings( _this, "reverbDamping" );
	m_reverbWidth.loadSettings( _this, "reverbWidth" );
	m_reverbLevel.loadSettings( _this, "reverbLevel" );

	m_chorusOn.loadSettings( _this, "chorusOn" );
	m_chorusNum.loadSettings( _this, "chorusNum" );
	m_chorusLevel.loadSettings( _this, "chorusLevel" );
	m_chorusSpeed.loadSettings( _this, "chorusSpeed" );
	m_chorusDepth.loadSettings( _this, "chorusDepth" );

}




void Sf2Instrument::loadFile( const QString & _file )
{
	if( !_file.isEmpty() && QFileInfo( _file ).exists() )
	{
		openFile( _file, false );
	}

	// setting the first bank and patch number that is found (the synth read under its mutex; the models set after
	// it, as their change selects the patch, which takes the mutex)
	auto first = std::optional<std::pair<int, int>>{};
	{
	const auto lock = std::lock_guard{m_synthMutex};
	auto sSoundCount = ::fluid_synth_sfcount( m_synth );
	for ( int i = 0; i < sSoundCount; ++i ) {
		int iBank = 0;
		int iProg = 0;
		fluid_sfont_t *pSoundFont = ::fluid_synth_get_sfont( m_synth, i );

		if ( pSoundFont ) {
#ifdef CONFIG_FLUID_BANK_OFFSET
			int iBankOff = ::fluid_synth_get_bank_offset( m_synth, fluid_sfont_get_id( pSoundFont ) );
#endif

			fluid_sfont_iteration_start( pSoundFont );
			fluid_preset_t* pCurPreset = nullptr;

			if ((pCurPreset = fluid_sfont_iteration_next(pSoundFont)))
			{
				iBank = fluid_preset_get_banknum( pCurPreset );
				iProg = fluid_preset_get_num( pCurPreset );

#ifdef CONFIG_FLUID_BANK_OFFSET
				iBank += iBankOff;
#endif

				::fluid_synth_bank_select( m_synth, 1, iBank );
				::fluid_synth_program_change( m_synth, 1, iProg );
				first = std::pair{iBank, iProg};
				break;
			}
		}
	}
	}
	if (first)
	{
		m_bankNum.setValue(first->first);
		m_patchNum.setValue(first->second);
	}
}




auto Sf2Instrument::midiPatch() const -> std::optional<MidiPatch>
{
	return MidiPatch {
		.bank = static_cast<std::uint16_t>(m_bankNum.value()),
		.program = static_cast<std::uint8_t>(m_patchNum.value())
	};
}




AutomatableModel* Sf2Instrument::childModel(std::string_view modelName)
{
	if (modelName == "bank")
	{
		return &m_bankNum;
	}
	else if (modelName == "patch")
	{
		return &m_patchNum;
	}

	qCritical() << "requested unknown model " << QString::fromUtf8(modelName.data(), modelName.size());

	return nullptr;
}



QString Sf2Instrument::nodeName() const
{
	return sf2player_plugin_descriptor.name;
}




void Sf2Instrument::freeFont()
{
	setFont(nullptr);
}




auto Sf2Instrument::readFont(const std::string& path) -> fluid_sfont_t*
{
	// Read by a synth of its own, without any lock (a soundfont may take long to read), then taken out of it to be
	// moved into the instrument's (as reloadSynth() moves it between synths)
	if (!fluid_is_soundfont(path.c_str())) { return nullptr; }
	auto* reader = new_fluid_synth(m_settings);
	auto* font = static_cast<fluid_sfont_t*>(nullptr);
	if (fluid_synth_sfload(reader, path.c_str(), false) >= 0 && fluid_synth_sfcount(reader) > 0)
	{
		font = fluid_synth_get_sfont(reader, 0);
		fluid_synth_remove_sfont(reader, font);
	}
	delete_fluid_synth(reader);
	return font;
}




void Sf2Instrument::setFont(fluid_sfont_t* font)
{
	// A moment under the audio engine's change lock and the synth's mutex: the audio thread plays the synth (it does
	// not wait for the main thread then, which holds them only for this)
	const auto guard = AudioEngine::changesGuard();
	const auto lock = std::lock_guard{m_synthMutex};
	if (m_font != nullptr)
	{
		// (FluidSynth keeps it until its last voice is done). Without resetting the channels' presets here (it walks
		// every channel and logs for those it finds nothing for, under the locks): the patch is selected right
		// after, without the audio engine's lock (see openFile())
		// (with no font in its place, the channels let go of its presets now)
		fluid_synth_sfunload(m_synth, m_fontId, font == nullptr);
		m_font = nullptr;
	}
	if (font != nullptr)
	{
		m_fontId = fluid_synth_add_sfont(m_synth, font);
		m_font = font;
	}
}



void Sf2Instrument::openFile( const QString & _sf2File, bool updateTrackName )
{
	emit fileLoading();

	const auto filePath = PathUtil::toAbsolute(_sf2File).toStdString();
	QString relativePath = PathUtil::toShortestRelative( _sf2File );

	// Read without any lock, then put in place of the one there (if any) at once
	auto* font = readFont(filePath);
	setFont(font);

	if (!font)
	{
		collectErrorForUI(Sf2Instrument::tr("A soundfont %1 could not be loaded.").arg(QFileInfo(_sf2File).baseName()));
	}

	if (font && m_fontId >= 0)
	{
		// Don't reset patch/bank, so that it isn't cleared when
		// someone resolves a missing file
		//m_patchNum.setValue( 0 );
		//m_bankNum.setValue( 0 );
		m_filename = relativePath;

		emit fileChanged();
	}

	if( updateTrackName || instrumentTrack()->displayName() == displayName() )
	{
		instrumentTrack()->setName( PathUtil::cleanName( _sf2File ) );
	}

	updatePatch();
}




void Sf2Instrument::updatePatch()
{
	if( m_bankNum.value() >= 0 && m_patchNum.value() >= 0 )
	{
		// (each call to the synth under its mutex: see playNoteImpl())
		const auto lock = std::lock_guard{m_synthMutex};
		fluid_synth_program_select( m_synth, m_channel, m_fontId,
				m_bankNum.value(), m_patchNum.value() );
	}
}




QString Sf2Instrument::getCurrentPatchName()
{
	int iBankSelected = m_bankNum.value();
	int iProgSelected = m_patchNum.value();
	const auto lock = std::lock_guard{m_synthMutex};

	// For all soundfonts (in reversed stack order) fill the available programs...
	int cSoundFonts = ::fluid_synth_sfcount( m_synth );
	for( int i = 0; i < cSoundFonts; i++ )
	{
		fluid_sfont_t *pSoundFont = fluid_synth_get_sfont( m_synth, i );
		if ( pSoundFont )
		{
#ifdef CONFIG_FLUID_BANK_OFFSET
			int iBankOffset =
				fluid_synth_get_bank_offset(
						m_synth, fluid_sfont_get_id(pSoundFont) );
#endif
			fluid_sfont_iteration_start( pSoundFont );
			fluid_preset_t* pCurPreset = nullptr;

			while ((pCurPreset = fluid_sfont_iteration_next(pSoundFont)))
			{
				int iBank = fluid_preset_get_banknum( pCurPreset );
#ifdef CONFIG_FLUID_BANK_OFFSET
				iBank += iBankOffset;
#endif
				int iProg = fluid_preset_get_num( pCurPreset );
				if( iBank == iBankSelected && iProg ==
								iProgSelected )
				{
					return fluid_preset_get_name( pCurPreset );
				}
			}
		}
	}
	return "";
}




void Sf2Instrument::updateGain()
{
	const auto lock = std::lock_guard{m_synthMutex};
	fluid_synth_set_gain( m_synth, m_gain.value() );
}

void Sf2Instrument::updateReverbOn()
{
	const auto lock = std::lock_guard{m_synthMutex};
	fluid_synth_reverb_on(m_synth, -1, m_reverbOn.value() ? 1 : 0);
}

void Sf2Instrument::updateReverb()
{
	const auto lock = std::lock_guard{m_synthMutex};
	fluid_synth_set_reverb_group_roomsize(m_synth, -1, m_reverbRoomSize.value());
	fluid_synth_set_reverb_group_damp(m_synth, -1, m_reverbDamping.value());
	fluid_synth_set_reverb_group_width(m_synth, -1, m_reverbWidth.value());
	fluid_synth_set_reverb_group_level(m_synth, -1, m_reverbLevel.value());
}

void Sf2Instrument::updateChorusOn()
{
	const auto lock = std::lock_guard{m_synthMutex};
	fluid_synth_chorus_on(m_synth, -1, m_chorusOn.value() ? 1 : 0);
}

void Sf2Instrument::updateChorus()
{
	const auto lock = std::lock_guard{m_synthMutex};
	fluid_synth_set_chorus_group_nr(m_synth, -1, static_cast<int>(m_chorusNum.value()));
	fluid_synth_set_chorus_group_level(m_synth, -1, m_chorusLevel.value());
	fluid_synth_set_chorus_group_speed(m_synth, -1, m_chorusSpeed.value());
	fluid_synth_set_chorus_group_depth(m_synth, -1, m_chorusDepth.value());
	fluid_synth_set_chorus_group_type(m_synth, -1, FLUID_CHORUS_MOD_SINE);
}

void Sf2Instrument::updateTuning()
{
	const auto lock = std::lock_guard{m_synthMutex};
	if (instrumentTrack()->microtuner()->enabledModel()->value())
	{
		auto centArray = std::array<double, 128>{};
		double lowestHz = std::exp2(-69. / 12.) * 440.; // Frequency of MIDI note 0, which is approximately 8.175798916 Hz
		for (int i = 0; i < 128; ++i)
		{
			// Get desired Hz of note
			double noteHz = instrumentTrack()->microtuner()->keyToFreq(i, DefaultBaseKey);
			// Convert Hz to cents
			centArray[i] = noteHz == 0. ? 0. : 1200. * log2(noteHz / lowestHz);
		}

		fluid_synth_activate_key_tuning(m_synth, 0, 0, "", centArray.data(), true);
		for (int chan = 0; chan < 16; chan++)
		{
		    fluid_synth_activate_tuning(m_synth, chan, 0, 0, true);
		}
	}
	else
	{
		fluid_synth_activate_key_tuning(m_synth, 0, 0, "", nullptr, true);
		for (int chan = 0; chan < 16; chan++)
		{
		    fluid_synth_activate_tuning(m_synth, chan, 0, 0, true);
		}
	}
}



void Sf2Instrument::reloadSynth()
{
	double tempRate;

	// Set & get, returns the true sample rate
	fluid_settings_setnum( m_settings, (char *) "synth.sample-rate", Engine::audioEngine()->outputSampleRate() );
	fluid_settings_getnum( m_settings, (char *) "synth.sample-rate", &tempRate );

	// A new synth, made without any lock, takes the old one's place (and its soundfont) in a moment under the audio
	// engine's change lock and the synth's mutex; the old one is deleted after, without them
	auto* made = new_fluid_synth( m_settings );
	auto* old = static_cast<fluid_synth_t*>(nullptr);
	{
		const auto guard = AudioEngine::changesGuard();
		const auto lock = std::lock_guard{m_synthMutex};
		if (m_font && m_synth)
		{
			fluid_synth_remove_sfont( m_synth, m_font );
			m_fontId = fluid_synth_add_sfont( made, m_font );
		}
		old = std::exchange(m_synth, made);
		m_framesRendered = 0;
		// (what the render reads, changed with the synth)
		m_internalSampleRate = static_cast<int>( tempRate );
		m_resampler.setRatio(m_internalSampleRate, Engine::audioEngine()->outputSampleRate());
	}
	if (old != nullptr) { delete_fluid_synth(old); }

	// synth program change (set bank and patch)
	if (m_font) { updatePatch(); }

	m_synthMutex.lock();

	if (m_internalSampleRate != Engine::audioEngine()->outputSampleRate())
	{
		// LMMS supports a sample rate of 192 kHZ, while FluidSynth only supports up to 96 kHZ.
		// Because of this, the instrument is resampled using libsamplerate when necessary.
		// This uses linear interpolation, so the instrument's interpolation is set to FLUID_INTERP_LINEAR
		// to match. A better option might be to make the interpolation option modifiable by the user, as well as only
		// supporting only up to 96 kHZ (though that may be a problem if theres a strong need for 192 kHZ).
		fluid_synth_set_interp_method(m_synth, -1, FLUID_INTERP_LINEAR);
	}

	m_synthMutex.unlock();

	updateReverb();
	updateChorus();
	updateReverbOn();
	updateChorusOn();
	updateGain();
	updateTuning();

	// Reset last MIDI pitch properties, which will be set to the correct values
	// upon playing the next note
	m_lastMidiPitch = -1;
	m_lastMidiPitchRange = -1;
}




void Sf2Instrument::reset()
{
	// The synth was made anew on the main thread just before (see Mixer::processingStateDiscarding and
	// reloadSynth()): here, every voice off at once, channel by channel ("all channels" (-1) left a voice that was
	// releasing playing on), and nothing kept of what was rendered before. (No fluid_synth_system_reset(): it logs
	// a warning for each channel whose default preset the soundfont lacks, which allocates on the audio thread.)
	const auto lock = std::lock_guard{m_synthMutex};
	for (int channel = 0; channel < fluid_synth_count_midi_channels(m_synth); ++channel)
	{
		fluid_synth_all_sounds_off(m_synth, channel);
	}
	// (what was rendered at the synth's rate and not handed out yet)
	m_bufferView = {};
	m_resampler.reset();
	// FluidSynth renders in blocks and hands out what is left of the last one first: that rest, from before the
	// reset, is rendered and dropped, so that the next render starts on a block of its own
	const auto block = fluid_synth_get_internal_bufsize(m_synth);
	auto rest = static_cast<int>((block - m_framesRendered % block) % block);
	auto scratch = std::array<float, 2 * 64>{};
	while (rest > 0)
	{
		const auto frames = std::min(rest, 64);
		fluid_synth_write_float(m_synth, frames, scratch.data(), 0, 2, scratch.data(), 1, 2);
		rest -= frames;
	}
	m_framesRendered = 0;
	// (the pitch bend and its range are sent again with the next period)
	m_lastMidiPitch = -1;
	m_lastMidiPitchRange = -1;
}




void Sf2Instrument::playNoteImpl(NotePlayHandle* _n, std::span<SampleFrame>)
{
	if( _n->isMasterNote() || ( _n->hasParent() && _n->isReleased() ) )
	{
		return;
	}

	int masterPitch = instrumentTrack()->useMasterPitchModel()->value() ? Engine::getSong()->masterPitch() : 0;
	int baseNote = instrumentTrack()->baseNoteModel()->value();
	int midiNote = _n->midiKey() - baseNote + DefaultBaseKey + masterPitch;

	// out of range?
	if (midiNote < 0 || midiNote >= 128)
	{
		return;
	}

	if (!_n->m_pluginData)
	{
		const int baseVelocity = instrumentTrack()->midiPort()->baseVelocity();

		// (in the note's own room: nothing allocated on the audio thread)
		auto* pluginData = _n->makePluginData<Sf2PluginData>();
		pluginData->midiNote = midiNote;
		pluginData->lastPanning = 0;
		pluginData->lastVelocity = _n->midiVelocity( baseVelocity );
		pluginData->isNew = true;
		pluginData->offset = _n->offset();
		pluginData->noteOffSent = false;
		pluginData->panning = _n->getPanning();

		// insert the nph to the playing notes vector
		m_playingNotesMutex.lock();
		m_playingNotes.push_back(_n);
		m_playingNotesMutex.unlock();
	}
	else if( _n->isReleased() && ! _n->instrumentTrack()->isSustainPedalPressed() ) // note is released during this period
	{
		auto pluginData = static_cast<Sf2PluginData*>(_n->m_pluginData);
		pluginData->offset = _n->framesBeforeRelease();
		pluginData->isNew = false;

		m_playingNotesMutex.lock();
		m_playingNotes.push_back(_n);
		m_playingNotesMutex.unlock();
	}

	// Update the pitch of all the voices. Under the synth's mutex: FluidSynth's voice functions are not part of its
	// locked API, and they queue events for its render, which takes one writer at a time (notes play on several
	// threads, the render and the controls on others: unguarded, the queue broke and the render called a null pointer)
	if (const auto data = static_cast<Sf2PluginData*>(_n->m_pluginData); data && !data->fluidVoices.empty()) {
		const auto detuning = _n->currentDetuning();
		const auto lock = std::lock_guard{m_synthMutex};
		for (const auto& voice : data->fluidVoices) {
			if (voice.isValid()) {
				fluid_voice_gen_set(voice.get(), GEN_COARSETUNE, voice.coarseTune() + detuning);
				fluid_voice_update_param(voice.get(), GEN_COARSETUNE);
			}
		}
	}
}


void Sf2Instrument::noteOn( Sf2PluginData * n )
{
	m_synthMutex.lock();

	// get list of current voice IDs so we can easily spot the new
	// voice after the fluid_synth_noteon() call
	const int poly = fluid_synth_get_polyphony( m_synth );
#ifndef _MSC_VER
	fluid_voice_t* voices[poly];
#else
	const auto voices = static_cast<fluid_voice_t**>(_alloca(poly * sizeof(fluid_voice_t*)));
#endif

	fluid_synth_noteon( m_synth, m_channel, n->midiNote, n->lastVelocity );

	// Get any new voices and store them in the plugin data
	fluid_synth_get_voicelist(m_synth, voices, poly, -1);
	for (int i = 0; i < poly && voices[i] && !n->fluidVoices.full(); ++i)
	{
		const auto voice = voices[i];
		// FluidSynth stops voices with the same channel and pitch upon note-on,
		// so voices with the current channel and pitch are playing this note.
		if (fluid_voice_get_channel(voice) == m_channel
			&& fluid_voice_get_key(voice) == n->midiNote
			&& fluid_voice_is_on(voice)
		) {
			n->fluidVoices.emplace_back(voices[i]);
		}
	}

	// Smallest balance value that results in full attenuation of one channel.
	// Corresponds to internal FluidSynth macro `FLUID_CB_AMP_SIZE`.
	constexpr static auto maxBalance = 1441.f;
	// Convert panning from linear to exponential for FluidSynth
	const auto panning = n->panning;
	const auto factor = 1.f - std::abs(panning) / static_cast<float>(PanningRight);
	const auto balance = std::copysign(
		factor <= 0 ? maxBalance : std::min(-200.f * std::log10(factor), maxBalance),
		panning
	);
	// Set note panning on all the voices
	for (const auto& voice : n->fluidVoices) {
		if (voice.isValid()) {
			fluid_voice_gen_set(voice.get(), GEN_CUSTOM_BALANCE, balance);
			fluid_voice_update_param(voice.get(), GEN_CUSTOM_BALANCE);
		}
	}

	m_synthMutex.unlock();

	m_notesRunningMutex.lock();
	++m_notesRunning[ n->midiNote ];
	m_notesRunningMutex.unlock();
}


void Sf2Instrument::noteOff( Sf2PluginData * n )
{
	n->noteOffSent = true;
	m_notesRunningMutex.lock();
	const int notes = --m_notesRunning[n->midiNote];
	m_notesRunningMutex.unlock();

	if( notes <= 0 )
	{
		m_synthMutex.lock();
		fluid_synth_noteoff( m_synth, m_channel, n->midiNote );
		m_synthMutex.unlock();
	}
}


void Sf2Instrument::playImpl(std::span<SampleFrame> out)
{
	// set midi pitch for this period
	const int currentMidiPitch = instrumentTrack()->midiPitch();
	if( m_lastMidiPitch != currentMidiPitch )
	{
		m_lastMidiPitch = currentMidiPitch;
		m_synthMutex.lock();
		fluid_synth_pitch_bend( m_synth, m_channel, m_lastMidiPitch );
		m_synthMutex.unlock();
	}

	const int currentMidiPitchRange = instrumentTrack()->midiPitchRange();
	if( m_lastMidiPitchRange != currentMidiPitchRange )
	{
		m_lastMidiPitchRange = currentMidiPitchRange;
		m_synthMutex.lock();
		fluid_synth_pitch_wheel_sens( m_synth, m_channel, m_lastMidiPitchRange );
		m_synthMutex.unlock();
	}
	// if we have no new noteons/noteoffs, just render a period and call it a day
	if (m_playingNotes.empty())
	{
		renderFrames(out.size(), out.data());
		return;
	}

	// processing loop
	// go through noteplayhandles in processing order
	f_cnt_t currentFrame = 0;

	while (!m_playingNotes.empty())
	{
		// find the note with lowest offset
		NotePlayHandle * currentNote = m_playingNotes[0];
		for (std::size_t i = 1; i < m_playingNotes.size(); ++i)
		{
			auto currentData = static_cast<Sf2PluginData*>(currentNote->m_pluginData);
			auto iData = static_cast<Sf2PluginData*>(m_playingNotes[i]->m_pluginData);
			if( currentData->offset > iData->offset )
			{
				currentNote = m_playingNotes[i];
			}
		}

		// process the current note:
		// first see if we're synced in frame count
		auto currentData = static_cast<Sf2PluginData*>(currentNote->m_pluginData);
		if( currentData->offset > currentFrame )
		{
			renderFrames(currentData->offset - currentFrame, out.data() + currentFrame);
			currentFrame = currentData->offset;
		}
		if( currentData->isNew )
		{
			noteOn( currentData );
			if( currentNote->isReleased() ) // if the note is released during the same period, we have to process it again for noteoff
			{
				currentData->isNew = false;
				currentData->offset = currentNote->framesBeforeRelease();
			}
			else // otherwise remove the handle
			{
				m_playingNotesMutex.lock();
				std::erase(m_playingNotes, currentNote);
				m_playingNotesMutex.unlock();
			}
		}
		else
		{
			noteOff( currentData );
			m_playingNotesMutex.lock();
			std::erase(m_playingNotes, currentNote);
			m_playingNotesMutex.unlock();
		}
	}

	if (currentFrame < out.size())
	{
		renderFrames(out.size() - currentFrame, out.data() + currentFrame);
	}
}


void Sf2Instrument::renderFrames( f_cnt_t frames, SampleFrame* buf )
{
	const auto guard = std::lock_guard{m_synthMutex};

	fluid_synth_get_gain(m_synth); // This flushes voice updates as a side effect

	if (m_internalSampleRate == Engine::audioEngine()->outputSampleRate()) {
		fluid_synth_write_float(m_synth, frames, buf, 0, 2, buf, 1, 2);
		m_framesRendered += frames;
		return;
	}

	// TODO: These kind of playback pipelines/graphs are repeated within other parts of the codebase that work with
	// audio samples. We should find a way to unify this but the right abstraction is not so clear yet.
	while (frames > 0)
	{
		if (m_bufferView.empty())
		{
			fluid_synth_write_float(m_synth, m_buffer.size(), m_buffer.data(), 0, 2, m_buffer.data(), 1, 2);
			m_framesRendered += static_cast<std::int64_t>(m_buffer.size());
			m_bufferView = m_buffer;
		}

		const auto [inputFramesUsed, outputFramesGenerated]
			= m_resampler.process({&m_bufferView.data()[0][0], 2, m_bufferView.size()}, {&buf[0][0], 2, frames});

		if (inputFramesUsed == 0 && outputFramesGenerated == 0)
		{
			std::fill_n(buf, frames, SampleFrame{});
			break;
		}

		m_bufferView = m_bufferView.subspan(inputFramesUsed);
		buf += outputFramesGenerated;
		frames -= outputFramesGenerated;
	}
}




void Sf2Instrument::deleteNotePluginData( NotePlayHandle * _n )
{
	auto pluginData = static_cast<Sf2PluginData*>(_n->m_pluginData);
	if( ! pluginData->noteOffSent ) // if we for some reason haven't noteoffed the note before it gets deleted,
									// do it here
	{
		noteOff( pluginData );
		m_playingNotesMutex.lock();
		std::erase(m_playingNotes, _n);
		m_playingNotesMutex.unlock();
	}
	_n->destroyPluginData<Sf2PluginData>();
}




gui::PluginView * Sf2Instrument::instantiateView( QWidget * _parent )
{
	return new gui::Sf2InstrumentView( this, _parent );
}




namespace gui
{


class Sf2Knob : public Knob
{
public:
	Sf2Knob( QWidget * _parent ) :
			Knob( KnobType::Styled, _parent )
	{
		setFixedSize( 31, 38 );
	}
};



Sf2InstrumentView::Sf2InstrumentView( Instrument * _instrument, QWidget * _parent ) :
	InstrumentViewFixedSize( _instrument, _parent )
{
//	QVBoxLayout * vl = new QVBoxLayout( this );
//	QHBoxLayout * hl = new QHBoxLayout();

	auto k = castModel<Sf2Instrument>();

	connect(&k->m_bankNum, SIGNAL(dataChanged()), this, SLOT(updatePatchName()));
	connect(&k->m_patchNum, SIGNAL(dataChanged()), this, SLOT(updatePatchName()));

	// File Button
	m_fileDialogButton = new PixmapButton(this);
	m_fileDialogButton->setCursor(Qt::PointingHandCursor);
	m_fileDialogButton->setActiveGraphic(PLUGIN_NAME::getIconPixmap("fileselect_on"));
	m_fileDialogButton->setInactiveGraphic(PLUGIN_NAME::getIconPixmap("fileselect_off"));
	m_fileDialogButton->move(217, 107);

	connect(m_fileDialogButton, SIGNAL(clicked()), this, SLOT(showFileDialog()));

	m_fileDialogButton->setToolTip(tr("Open SoundFont file"));

	// Patch Button
	m_patchDialogButton = new PixmapButton(this);
	m_patchDialogButton->setCursor(Qt::PointingHandCursor);
	m_patchDialogButton->setActiveGraphic(PLUGIN_NAME::getIconPixmap("patches_on"));
	m_patchDialogButton->setInactiveGraphic(PLUGIN_NAME::getIconPixmap("patches_off"));
	m_patchDialogButton->setEnabled(false);
	m_patchDialogButton->move(217, 125);

	connect(m_patchDialogButton, SIGNAL(clicked()), this, SLOT(showPatchDialog()));

	m_patchDialogButton->setToolTip(tr("Choose patch"));

	// LCDs
	m_bankNumLcd = new LcdSpinBox(3, "21pink", this);
	m_bankNumLcd->move(131, 62);
//	m_bankNumLcd->addTextForValue( -1, "---" );
//	m_bankNumLcd->setEnabled( false );

	m_patchNumLcd = new LcdSpinBox( 3, "21pink", this );
	m_patchNumLcd->move(190, 62);
//	m_patchNumLcd->addTextForValue( -1, "---" );
//	m_patchNumLcd->setEnabled( false );

	/*hl->addWidget( m_fileDialogButton );
	hl->addWidget( m_bankNumLcd );
	hl->addWidget( m_patchNumLcd );
	hl->addWidget( m_patchDialogButton );

	vl->addLayout( hl );*/

	// Next row

	//hl = new QHBoxLayout();

	m_filenameLabel = new QLabel( this );
	m_filenameLabel->setGeometry( 58, 109, 156, 11 );
	m_patchLabel = new QLabel( this );
	m_patchLabel->setGeometry( 58, 127, 156, 11 );

	//hl->addWidget( m_filenameLabel );
//	vl->addLayout( hl );

	// Gain
	m_gainKnob = new Sf2Knob( this );
	m_gainKnob->setHintText( tr("Gain:"), "" );
	m_gainKnob->move( 86, 55 );
//	vl->addWidget( m_gainKnob );

	// Reverb
//	hl = new QHBoxLayout();


	m_reverbButton = new PixmapButton( this );
	m_reverbButton->setCheckable( true );
	m_reverbButton->move( 14, 180 );
	m_reverbButton->setActiveGraphic( PLUGIN_NAME::getIconPixmap( "reverb_on" ) );
	m_reverbButton->setInactiveGraphic( PLUGIN_NAME::getIconPixmap( "reverb_off" ) );
	m_reverbButton->setToolTip(tr("Apply reverb (if supported)"));


	m_reverbRoomSizeKnob = new Sf2Knob( this );
	m_reverbRoomSizeKnob->setHintText( tr("Room size:"), "" );
	m_reverbRoomSizeKnob->move( 93, 160 );

	m_reverbDampingKnob = new Sf2Knob( this );
	m_reverbDampingKnob->setHintText( tr("Damping:"), "" );
	m_reverbDampingKnob->move( 130, 160 );

	m_reverbWidthKnob = new Sf2Knob( this );
	m_reverbWidthKnob->setHintText( tr("Width:"), "" );
	m_reverbWidthKnob->move( 167, 160 );

	m_reverbLevelKnob = new Sf2Knob( this );
	m_reverbLevelKnob->setHintText( tr("Level:"), "" );
	m_reverbLevelKnob->move( 204, 160 );

/*	hl->addWidget( m_reverbOnLed );
	hl->addWidget( m_reverbRoomSizeKnob );
	hl->addWidget( m_reverbDampingKnob );
	hl->addWidget( m_reverbWidthKnob );
	hl->addWidget( m_reverbLevelKnob );

	vl->addLayout( hl );
*/

	// Chorus
//	hl = new QHBoxLayout();

	m_chorusButton = new PixmapButton( this );
	m_chorusButton->setCheckable( true );
	m_chorusButton->move( 14, 226 );
	m_chorusButton->setActiveGraphic( PLUGIN_NAME::getIconPixmap( "chorus_on" ) );
	m_chorusButton->setInactiveGraphic( PLUGIN_NAME::getIconPixmap( "chorus_off" ) );
	m_chorusButton->setToolTip(tr("Apply chorus (if supported)"));

	m_chorusNumKnob = new Sf2Knob( this );
	m_chorusNumKnob->setHintText( tr("Voices:"), "" );
	m_chorusNumKnob->move( 93, 206 );

	m_chorusLevelKnob = new Sf2Knob( this );
	m_chorusLevelKnob->setHintText( tr("Level:"), "" );
	m_chorusLevelKnob->move( 130 , 206 );

	m_chorusSpeedKnob = new Sf2Knob( this );
	m_chorusSpeedKnob->setHintText( tr("Speed:"), "" );
	m_chorusSpeedKnob->move( 167 , 206 );

	m_chorusDepthKnob = new Sf2Knob( this );
	m_chorusDepthKnob->setHintText( tr("Depth:"), "" );
	m_chorusDepthKnob->move( 204 , 206 );
/*
	hl->addWidget( m_chorusOnLed );
	hl->addWidget( m_chorusNumKnob);
	hl->addWidget( m_chorusLevelKnob);
	hl->addWidget( m_chorusSpeedKnob);
	hl->addWidget( m_chorusDepthKnob);

	vl->addLayout( hl );
*/
	setAutoFillBackground( true );
	QPalette pal;
	pal.setBrush( backgroundRole(), PLUGIN_NAME::getIconPixmap( "artwork" ) );
	setPalette( pal );

	updateFilename();
}




void Sf2InstrumentView::modelChanged()
{
	auto k = castModel<Sf2Instrument>();
	m_bankNumLcd->setModel( &k->m_bankNum );
	m_patchNumLcd->setModel( &k->m_patchNum );

	m_gainKnob->setModel( &k->m_gain );

	m_reverbButton->setModel( &k->m_reverbOn );
	m_reverbRoomSizeKnob->setModel( &k->m_reverbRoomSize );
	m_reverbDampingKnob->setModel( &k->m_reverbDamping );
	m_reverbWidthKnob->setModel( &k->m_reverbWidth );
	m_reverbLevelKnob->setModel( &k->m_reverbLevel );

	m_chorusButton->setModel( &k->m_chorusOn );
	m_chorusNumKnob->setModel( &k->m_chorusNum );
	m_chorusLevelKnob->setModel( &k->m_chorusLevel );
	m_chorusSpeedKnob->setModel( &k->m_chorusSpeed );
	m_chorusDepthKnob->setModel( &k->m_chorusDepth );


	connect( k, SIGNAL( fileChanged() ), this, SLOT( updateFilename() ) );

	connect( k, SIGNAL( fileLoading() ), this, SLOT( invalidateFile() ) );

	updateFilename();
}




void Sf2InstrumentView::updateFilename()
{
	auto i = castModel<Sf2Instrument>();
	QFontMetrics fm( m_filenameLabel->font() );
	QString file = i->m_filename.endsWith( ".sf2", Qt::CaseInsensitive ) ?
			i->m_filename.left( i->m_filename.length() - 4 ) :
			i->m_filename;
	m_filenameLabel->setText( fm.elidedText( file, Qt::ElideLeft, m_filenameLabel->width() ) );
			//		i->m_filename + "\nPatch: TODO" );

	m_patchDialogButton->setEnabled( !i->m_filename.isEmpty() );

	updatePatchName();

	update();
}




void Sf2InstrumentView::updatePatchName()
{
	auto i = castModel<Sf2Instrument>();
	QFontMetrics fm( font() );
	QString patch = i->getCurrentPatchName();
	m_patchLabel->setText( fm.elidedText( patch, Qt::ElideLeft, m_patchLabel->width() ) );


	update();
}




void Sf2InstrumentView::invalidateFile()
{
	m_patchDialogButton->setEnabled( false );
}




void Sf2InstrumentView::showFileDialog()
{
	auto k = castModel<Sf2Instrument>();

	FileDialog ofd( nullptr, tr( "Open SoundFont file" ) );
	ofd.setFileMode( FileDialog::ExistingFiles );

	QStringList types;
	types << tr( "SoundFont Files (*.sf2 *.sf3)" );
	ofd.setNameFilters( types );

	if( k->m_filename != "" )
	{
		QString f = PathUtil::toAbsolute( k->m_filename );
		ofd.setDirectory( QFileInfo( f ).absolutePath() );
		ofd.selectFile( QFileInfo( f ).fileName() );
	}
	else
	{
		ofd.setDirectory( ConfigManager::inst()->sf2Dir() );
	}

	m_fileDialogButton->setEnabled( false );

	if( ofd.exec() == QDialog::Accepted && !ofd.selectedFiles().isEmpty() )
	{
		QString f = ofd.selectedFiles()[0];
		if( f != "" )
		{
			k->openFile( f );
			Engine::getSong()->setModified();
		}
	}

	m_fileDialogButton->setEnabled( true );
}




void Sf2InstrumentView::showPatchDialog()
{
	auto k = castModel<Sf2Instrument>();

	PatchesDialog pd( this );

	pd.setup( k->m_synth, 1, k->instrumentTrack()->name(), &k->m_bankNum, &k->m_patchNum, m_patchLabel );

	pd.exec();
}


} // namespace gui

extern "C"
{

// necessary for getting instance out of shared lib
PLUGIN_EXPORT Plugin * lmms_plugin_main( Model *m, void * )
{
	return new Sf2Instrument( static_cast<InstrumentTrack *>( m ) );
}
}


} // namespace lmms
