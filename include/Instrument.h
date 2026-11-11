/*
 * Instrument.h - declaration of class Instrument, which provides a
 *                standard interface for all instrument plugins
 *
 * Copyright (c) 2005-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_INSTRUMENT_H
#define LMMS_INSTRUMENT_H

#include <QString>
#include <cmath>
#include <span>

#include "AudioProcessor.h"
#include "Flags.h"
#include "SampleFrame.h"
#include "lmms_export.h"
#include "LmmsTypes.h"
#include "MidiPatch.h"
#include "Plugin.h"
#include "TimePos.h"

#include <cmath>
#include <optional>
#include <string_view>

namespace lmms
{

// forward-declarations
class AudioPortsModel;
class InstrumentTrack;
class MidiEvent;
class NotePlayHandle;
class Track;

/**
 * Base class of instrument plugins.
 *
 * There are two kinds of instruments:
 *
 * - Single-streamed instruments (Flag::IsSingleStreamed) render all of their notes into one stream.
 *   They are processed once per period through their PluginInstance like effects are, so their audio
 *   ports can have any number of channels, which the pin connector routes to the track channels.
 *   They implement AudioProcessor::process().
 *
 * - NotePlayHandle-based instruments render each note into the note's own stereo buffer in playNote(),
 *   possibly in parallel for different notes. The track then applies the note's envelopes, volume and
 *   panning, and the pin connector's output routing.
 *
 * Instruments that still render into interleaved stereo buffers inherit from LegacyInstrument.
 */
class LMMS_EXPORT Instrument
	: public Plugin
	, public AudioProcessor
{
public:
	enum class Flag
	{
		NoFlags = 0x00,
		IsSingleStreamed = 0x01,	/*! Instrument provides a single audio stream for all notes */
		IsMidiBased = 0x02,			/*! Instrument is controlled by MIDI events rather than NotePlayHandles */
		IsNotBendable = 0x04,		/*! Instrument can't react to pitch bend changes */
	};

	using Flags = lmms::Flags<Flag>;

	Instrument(const Descriptor* _descriptor,
			InstrumentTrack* _instrument_track,
			const Descriptor::SubPluginFeatures::Key* key = nullptr,
			Flags flags = Flag::NoFlags);
	~Instrument() override = default;

	//! Renders a note into its own buffer (NotePlayHandle-based instruments). [audio thread]
	void playNote(NotePlayHandle* notesToPlay, std::span<SampleFrame> out)
	{
		playNoteImpl(notesToPlay, out);
	}

	// AudioProcessor lifecycle: most instruments need nothing beyond their constructor, so these default to
	// doing nothing. Instruments that allocate per sample rate or period size can override them.
	auto activate(double sampleRate, f_cnt_t maxFrameCount) -> bool override
	{
		(void)sampleRate;
		(void)maxFrameCount;
		return true;
	}
	void deactivate() override {}
	auto startProcessing() -> bool override { return true; }
	void stopProcessing() override {}
	void reset() override {}

	// --------------------------------------------------------------------
	// functions that can/should be re-implemented:
	// --------------------------------------------------------------------

	//! Receives all incoming MIDI events; Return true if event was handled.
	virtual bool handleMidiEvent(const MidiEvent&, const TimePos& = TimePos(), f_cnt_t offset = 0)
	{
		return true;
	}

	/**
	 * Needed for deleting plugin-specific-data of a note - plugin has to
	 * cast void-ptr so that the plugin-data is deleted properly
	 * (call of dtor if it's a class etc.)
	 */
	virtual void deleteNotePluginData(NotePlayHandle* noteToPlay) {}

	//! For applicable instruments: Provides host with current MIDI bank and program
	virtual auto midiPatch() const -> std::optional<MidiPatch>
	{
		return std::nullopt;
	}

	//! Called if external source needs to change something but we cannot
	//! reference the class header. Should return nullptr if the model is not found.
	virtual auto childModel(std::string_view modelName) -> AutomatableModel*
	{
		return nullptr;
	}

	/**
	 * Get number of sample-frames that should be used when playing beat
	 * (note with unspecified length)
	 * Per default this function returns 0. In this case, channel is using
	 * the length of the longest envelope (if one active).
	 */
	virtual auto beatLen(NotePlayHandle* nph) const -> f_cnt_t { return 0; }

	virtual bool hasNoteInput() const { return true; }

	// This method can be overridden by instruments that need a certain
	// release time even if no envelope is active. It returns the time
	// in milliseconds that these instruments would like to have for
	// their release stage.
	virtual float desiredReleaseTimeMs() const
	{
		return 0.f;
	}

	// Converts the desired release time in milliseconds to the corresponding
	// number of frames depending on the sample rate.
	f_cnt_t desiredReleaseFrames() const
	{
		const sample_rate_t sampleRate = getSampleRate();

		return static_cast<f_cnt_t>(std::ceil(desiredReleaseTimeMs() * sampleRate / 1000.f));
	}

	sample_rate_t getSampleRate() const;

	bool isSingleStreamed() const
	{
		return m_flags.testFlag(Instrument::Flag::IsSingleStreamed);
	}

	//! Returns whether the instrument is MIDI-based or NotePlayHandle-based
	bool isMidiBased() const
	{
		return m_flags.testFlag(Instrument::Flag::IsMidiBased);
	}

	bool isBendable() const
	{
		return !m_flags.testFlag(Instrument::Flag::IsNotBendable);
	}

	QString fullDisplayName() const override;

	// --------------------------------------------------------------------
	// provided functions:
	// --------------------------------------------------------------------

	//! instantiate instrument-plugin with given name or return NULL
	//! on failure
	static Instrument * instantiate(const QString & _plugin_name,
		InstrumentTrack * _instrument_track,
		const Plugin::Descriptor::SubPluginFeatures::Key* key,
		bool keyFromDnd = false);

	virtual bool isFromTrack( const Track * _track ) const;

	inline InstrumentTrack * instrumentTrack() const
	{
		return m_instrumentTrack;
	}


protected:
	//! Renders a note; to be implemented by NotePlayHandle-based instruments
	virtual void playNoteImpl(NotePlayHandle* notesToPlay, std::span<SampleFrame> out)
	{
		(void)notesToPlay;
		(void)out;
	}

	// fade in to prevent clicks
	void applyFadeIn(SampleFrame* buf, NotePlayHandle * n);

	// instruments may use this to apply a soft fade out at the end of
	// notes - method does this only if really less or equal
	// desiredReleaseFrames() frames are left
	void applyRelease( SampleFrame* buf, const NotePlayHandle * _n );

	float computeReleaseTimeMsByFrameCount(f_cnt_t frames) const;


private:
	//! Not used yet; NotePlayHandle-based instruments use playNote(NotePlayHandle*, std::span<SampleFrame>)
	void playNote(ProcessContext&) final {}

	InstrumentTrack * m_instrumentTrack;
	Flags m_flags;
};


/**
 * An Instrument with a stereo output and interleaved buffers and without inputs.
 *
 * Single-streamed legacy instruments render into an interleaved stereo buffer in playImpl().
 * NotePlayHandle-based ones only implement playNoteImpl().
 *
 * Inherit from this to reduce boilerplate.
 * In the future, once all plugins use planar buffers, this can be removed.
 */
class LMMS_EXPORT LegacyInstrument : public Instrument
{
public:
	using Instrument::Instrument;

	auto process(ProcessContext& context) -> ProcessStatus final
	{
		playImpl(context.out().interleavedBuffer().asSampleFrames());
		return ProcessStatus::Continue;
	}

protected:
	//! Renders all notes into @p out, which is silent; to be implemented by single-streamed instruments
	virtual void playImpl(std::span<SampleFrame> out) { (void)out; }

	auto channelCount(bool isInput) const -> ch_cnt_t final
	{
		// No inputs, two outputs
		return isInput ? 0 : 2;
	}

	auto channelGroupCount(bool isInput) const -> group_cnt_t final
	{
		// One group on the output side (the main stereo channels)
		return isInput ? 0 : 1;
	}

	auto getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup& group) const
		-> ch_cnt_t final
	{
		// TODO: Set channel names and other metadata
		(void)index;
		(void)group;
		return isInput ? 0 : 2;
	}

	auto getInplacePair(ch_cnt_t inputIndex) const -> std::optional<ch_cnt_t> final
	{
		(void)inputIndex;
		return std::nullopt;
	}

	auto usesInterleavedBuffers() const -> bool final { return true; }
};


LMMS_DECLARE_OPERATORS_FOR_FLAGS(Instrument::Flag)


} // namespace lmms

#endif // LMMS_INSTRUMENT_H
