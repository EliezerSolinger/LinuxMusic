/*
 * AudioFileProcessor.h - declaration of class AudioFileProcessor
 *                          (instrument-plugin for using audio-files)
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

#ifndef LMMS_AUDIO_FILE_PROCESSOR_H
#define LMMS_AUDIO_FILE_PROCESSOR_H

#include <atomic>
#include "AutomatableModel.h"
#include "ComboBoxModel.h"
#include "Instrument.h"
#include "Sample.h"
#include "LmmsTypes.h"


namespace lmms
{

class SampleThumbnailCache;

class AudioFileProcessor : public LegacyInstrument
{
	Q_OBJECT
public:
	AudioFileProcessor( InstrumentTrack * _instrument_track );

	void deleteNotePluginData( NotePlayHandle * _n ) override;

	void saveSettings(QDomDocument& doc, QDomElement& elem) override;
	void loadSettings(const QDomElement& elem) override;

	void loadFile( const QString & _file ) override;

	//! Where the latest note is playing (0 when none) [any thread]
	auto playingFrame() const -> f_cnt_t { return m_playingFrame.load(std::memory_order_relaxed); }

	QString nodeName() const override;

	auto beatLen(NotePlayHandle* note) const -> f_cnt_t override;

	float desiredReleaseTimeMs() const override
	{
		return 3.f;
	}

	gui::PluginView* instantiateView( QWidget * _parent ) override;

	Sample const & sample() const { return m_sample; }
	const SampleThumbnailCache* sampleThumbnailCache() const { return m_sampleThumbnailCache.get(); }

	FloatModel & ampModel() { return m_ampModel; }
	FloatModel & startPointModel() { return m_startPointModel; }
	FloatModel & endPointModel() { return m_endPointModel; }
	FloatModel & loopPointModel() { return m_loopPointModel; }
	BoolModel & reverseModel() { return m_reverseModel; }
	IntModel & loopModel() { return m_loopModel; }
	BoolModel & stutterModel() { return m_stutterModel; }
	BoolModel& declickModel() { return m_declickModel; }
	ComboBoxModel & interpolationModel() { return m_interpolationModel; }


public slots:
	void setAudioFile(const QString& _audio_file, bool _rename = true);

private slots:
	void reverseModelChanged();
	void ampModelChanged();
	void loopPointChanged();
	void startPointChanged();
	void endPointChanged();
	//! The sample's points from the three knobs, in order whatever moves them (see startPointChanged())
	void pointChanged();
	void stutterModelChanged();


signals:

	void sampleUpdated();

private:
	//! The resampler's mode its notes use (the interpolation chosen)
	auto interpolationMode() const -> AudioResampler::Mode;

	void playNoteImpl(NotePlayHandle* nph, std::span<SampleFrame> out) override;

	//! Puts @p sample in place of the sample, at once for the audio thread; the old one goes without the lock
	void setSample(Sample sample);

	Sample m_sample;
	std::shared_ptr<const SampleThumbnailCache> m_sampleThumbnailCache;

	FloatModel m_ampModel;
	FloatModel m_startPointModel;
	FloatModel m_endPointModel;
	FloatModel m_loopPointModel;
	BoolModel m_reverseModel;
	IntModel m_loopModel;
	BoolModel m_stutterModel;
	//! Microfades (see Sample::setDeclick()): on for new instruments, off for projects from before it
	BoolModel m_declickModel;
	ComboBoxModel m_interpolationModel;

	//! Where the next note starts and in which direction, with stutter (set by the notes, on the audio threads)
	std::atomic<f_cnt_t> m_nextPlayStartPoint;
	std::atomic<bool> m_nextPlayBackwards;
	//! Where the latest note is, for the view's cursor (read by the GUI on its timer; 0 when none plays)
	std::atomic<f_cnt_t> m_playingFrame = 0;
	//! (while a knob is moved to keep the points in order: the knobs moved then do not start it again)
	bool m_orderingPoints = false;
} ;

} // namespace lmms

#endif // LMMS_AUDIO_FILE_PROCESSOR_H
