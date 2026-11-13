/*
 * SampleClip.h
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

#ifndef LMMS_SAMPLE_CLIP_H
#define LMMS_SAMPLE_CLIP_H

#include <memory>
#include "Clip.h"
#include <vector>

#include "Sample.h"

namespace lmms
{

class ThreadedExportManager;
class SampleBuffer;
class SampleThumbnailCache;

namespace gui
{

class SampleClipView;

} // namespace gui


class SampleClip : public Clip
{
	Q_OBJECT
public:
	SampleClip(Track* track, Sample sample, bool isPlaying);
	SampleClip(Track* track);
	~SampleClip() override;

	SampleClip& operator=( const SampleClip& that ) = delete;

	void changeLength( const TimePos & _length ) override;
	void updateLength() override;

	const QString& sampleFile() const;
	bool hasSampleFileLoaded(const QString & filename) const;

	void saveSettings( QDomDocument & _doc, QDomElement & _parent ) override;
	void loadSettings( const QDomElement & _this ) override;
	inline QString nodeName() const override
	{
		return "sampleclip";
	}

	Sample& sample()
	{
		return m_sample;
	}

	const SampleThumbnailCache* sampleThumbnailCache() const { return m_sampleThumbnailCache.get(); }

	TimePos sampleLength() const;
	void setSampleStartFrame( f_cnt_t startFrame );
	void setSamplePlayLength( f_cnt_t length );
	void setStartTimeOffset(const TimePos& startTimeOffset) override;
	gui::ClipView * createView( gui::TrackView * _tv ) override;


	bool isPlaying() const;
	void setIsPlaying(bool isPlaying);
	void setSampleBuffer(std::shared_ptr<const SampleBuffer> sb);

	bool isCrossfadeable() const override { return true; }

	/**
	 * Takes (docs/plano-gravacao.md, "Gravação"): a clip may hold several recordings of the same passage, e.g. the
	 * passes of a loop recording. One of them, the active take, is the one the clip plays; the others are kept to
	 * choose from. A clip without takes holds one sample, as before.
	 */
	//! @returns the takes' files (empty for a clip without takes)
	auto takes() const -> const QStringList& { return m_takes; }
	auto activeTake() const -> int { return m_activeTake; }
	//! Adds a take and makes it the active one
	void addTake(const QString& file);
	//! Makes take @p index the one the clip plays
	void setActiveTake(int index);
	//! Removes take @p index (not the last one left)
	void removeTake(int index);

	/**
	 * Tempo (docs/plano-gravacao.md, "Time-stretch renderizado"): a recording is bound to the tempo it was played
	 * at. Its original timing says which frame of the original file was played at which tick (from the clip's
	 * start); when the song's tempo changes, the clip plays a rendering of the original stretched to the new
	 * timing. A clip without an original timing (an imported sample) plays its file as it is.
	 */
	struct TimingPoint
	{
		tick_t tick = 0;
		double frame = 0;
		auto operator==(const TimingPoint&) const -> bool = default;
	};
	using Timing = std::vector<TimingPoint>;

	auto isTempoBound() const -> bool { return !m_originalTiming.empty(); }
	auto originalTiming() const -> const Timing& { return m_originalTiming; }
	auto originalFile() const -> const QString& { return m_originalFile; }
	//! The timing the clip plays its file at now: the original one, or the one it was rendered to
	auto currentTiming() const -> const Timing& { return m_renderedTiming.empty() ? m_originalTiming : m_renderedTiming; }

	//! Binds the clip to the timing its recording @p file was played at
	void setOriginalTiming(Timing timing, const QString& file);
	//! Plays @p file, a rendering of the original to @p timing
	void setRendering(const QString& file, Timing timing);

	SampleClip* clone() override
	{
		return new SampleClip(*this);
	}

public slots:
	void setSampleFile(const QString& sf);
	void playbackPositionChanged();
	void updateTrackClips();
	void tempoChanged();

protected:
	SampleClip( const SampleClip& orig );

private:
	void exportSampleBuffer(const QString& fileName);

	Sample m_sample;
	QStringList m_takes;
	int m_activeTake = -1;
	Timing m_originalTiming;
	QString m_originalFile;
	Timing m_renderedTiming;

	//! Loads @p file keeping the clip's place, length and start in the recording
	void playFile(const QString& file);
	std::shared_ptr<const SampleThumbnailCache> m_sampleThumbnailCache;
	bool m_isPlaying;
	int m_startFrameOffset;

	friend class gui::SampleClipView;


signals:
	void sampleChanged();
	void wasReversed();
} ;


} // namespace lmms

#endif // LMMS_SAMPLE_CLIP_H
