/*
 * AudioItem.h
 *
 * This file is part of LinuxMusic, a fork of LMMS - https://lmms.io
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

#ifndef LMMS_AUDIO_ITEM_H
#define LMMS_AUDIO_ITEM_H

#include <cstdint>
#include <memory>
#include <vector>

#include "Clip.h"
#include "Sample.h"

namespace lmms
{

class AudioTrack;
class SampleThumbnailCache;

namespace gui
{
class AudioItemView;
}

/**
 * A piece of audio on an audio track (docs/plano-gravacao.md, section 11): a stretch of a file, placed on the song
 * in ticks, since it is musical. It has:
 * - its file, played from memory or streamed from disk (Storage, as Ableton's RAM switch);
 * - where it starts in the file (the clip's start offset: slip editing moves the file under the item);
 * - a gain in dB, and fades in and out (5 ms unless changed; where items of a lane overlap, the fades cross over the
 *   overlap);
 * - its lane: the first one is the one that plays (see AudioTrack);
 * - for a recording, the timing it was played at: the tempo it follows (see SampleClip, whose recordings work the
 *   same way).
 * Its own settings change by commands (UndoCommand); its place and length as every clip's.
 */
class LMMS_EXPORT AudioItem : public Clip
{
	Q_OBJECT
public:
	enum class Storage { Automatic, Memory, Disk };
	static constexpr double StreamAboveSeconds = 20.0; //!< Automatic streams files this long or longer
	static constexpr double DefaultFadeMs = 5.0;
	static constexpr double MinGainDb = -60.0; //!< and below: silent
	static constexpr double MaxGainDb = 24.0;

	//! Which frame of the recording sounds at which tick from its start (see SampleClip::TimingPoint)
	struct TimingPoint
	{
		tick_t tick = 0;
		double frame = 0;
		auto operator==(const TimingPoint&) const -> bool = default;
	};
	using Timing = std::vector<TimingPoint>;

	explicit AudioItem(Track* track);
	AudioItem(const AudioItem& other);
	~AudioItem() override;

	auto id() const -> std::uint64_t { return m_id; }

	// The file
	auto file() const -> const QString& { return m_file; }
	//! Plays @p file from its start (an empty one: nothing); its length becomes the file's
	void setFile(const QString& file);
	auto sample() const -> const Sample& { return m_sample; }
	auto isStreamed() const -> bool { return m_sample.isStreamed(); }
	auto storage() const -> Storage { return m_storage; }
	auto peaks() const -> const SampleThumbnailCache* { return m_peaks.get(); }
	//! The file's length on the song from where the item starts in it, at the tempo now
	auto fileLength() const -> TimePos;

	// The item's own settings (changed by commands: see AudioTrack); its lane is the clip's
	auto gainDb() const -> double { return m_gainDb; }
	auto fadeInMs() const -> double { return m_fadeInMs; }
	auto fadeOutMs() const -> double { return m_fadeOutMs; }
	//! The gain as a factor (0 at MinGainDb and below)
	auto gain() const -> float;

	// The tempo it follows, for a recording
	auto isTempoBound() const -> bool { return !m_originalTiming.empty(); }
	auto originalFile() const -> const QString& { return m_originalFile; }
	auto originalTiming() const -> const Timing& { return m_originalTiming; }
	auto currentTiming() const -> const Timing& { return m_renderedTiming.empty() ? m_originalTiming : m_renderedTiming; }
	void setOriginalTiming(Timing timing, const QString& file);
	void setRendering(const QString& file, Timing timing);

	//! Where the audio device dropped out while it was recorded (frames of its file), marked on it
	auto xruns() const -> const std::vector<std::int64_t>& { return m_xruns; }
	void setXruns(std::vector<std::int64_t> xruns);

	//! The frame of the file that sounds @p tick ticks after the start of the recording (the item's start less
	//! its start offset): by its timing for a recording, at the tempo now otherwise (see SampleClip::fileFrameAt)
	auto fileFrameAt(tick_t tick) const -> f_cnt_t;

	void saveSettings(QDomDocument& doc, QDomElement& element) override;
	void loadSettings(const QDomElement& element) override;
	auto nodeName() const -> QString override { return "audioitem"; }
	gui::ClipView* createView(gui::TrackView* view) override;
	auto clone() -> Clip* override { return new AudioItem{*this}; }
	bool isCrossfadeable() const override { return false; } // (its fades are its own)

signals:
	//! What it plays, or how, changed (its file, gain, fades, lane, storage)
	void itemChanged();

private:
	//! The setters, for the commands (AudioTrack's)
	void setGainDb(double gainDb);
	void setFades(double inMs, double outMs);
	void setStorage(Storage storage);
	//! Loads @p file keeping the item's place, length and start in it
	void playFile(const QString& file);
	void load(const QString& file);
	void peaksReady(const QString& file);

	std::uint64_t m_id;
	QString m_file;
	Sample m_sample;
	std::shared_ptr<const SampleThumbnailCache> m_peaks;
	Storage m_storage = Storage::Automatic;
	double m_gainDb = 0.0;
	double m_fadeInMs = DefaultFadeMs;
	double m_fadeOutMs = DefaultFadeMs;
	Timing m_originalTiming;
	QString m_originalFile;
	Timing m_renderedTiming;
	std::vector<std::int64_t> m_xruns;

	friend class AudioTrack;
	friend class gui::AudioItemView;
};

} // namespace lmms

#endif // LMMS_AUDIO_ITEM_H
