/*
 * AudioTrack.h
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

#ifndef LMMS_AUDIO_TRACK_H
#define LMMS_AUDIO_TRACK_H

#include <array>
#include <cstdint>
#include <vector>

#include "AudioInputTrack.h"
#include "AudioItem.h"

namespace lmms
{

class AudioTrackPlayer;

/**
 * The audio track (docs/plano-gravacao.md, section 11): recordings and audio files, as items (AudioItem) on lanes.
 *
 * - **Lanes:** each pass of a recording is a lane; the playing lane (the first, unless another is chosen) is the one
 *   heard, the others are kept to choose from.
 * - **Playing:** a player of its own (a play handle made with the track) sums the playing lane's items each period,
 *   from a list the track publishes after each change; the audio thread makes nothing. Where items overlap, their
 *   fades cross over the overlap.
 * - **Editing:** the items' own settings (gain, fades, lane, storage) and the playing lane change by commands
 *   (UndoCommand), each an undo step; the items' place and length as every clip's.
 */
class LMMS_EXPORT AudioTrack : public AudioInputTrack
{
	Q_OBJECT
public:
	explicit AudioTrack(TrackContainer* container);
	~AudioTrack() override;

	bool play(const TimePos& start, f_cnt_t frames, f_cnt_t offset, int clipNum = -1) override;
	gui::TrackView* createView(gui::TrackContainerView* view) override;
	Clip* createClip(const TimePos& pos) override;
	void saveTrackSpecificSettings(QDomDocument& doc, QDomElement& element, bool presetMode) override;
	void loadTrackSpecificSettings(const QDomElement& element) override;
	QString nodeName() const override { return "audiotrack"; }

	auto items() const -> std::vector<AudioItem*>;
	auto item(std::uint64_t id) const -> AudioItem*;
	//! Adds an item playing @p file from @p position on @p lane
	auto addItem(const QString& file, const TimePos& position, int lane = 0) -> AudioItem*;

	//! The lanes there are (at least one, and one past the last item's)
	auto laneCount() const -> int;
	auto playingLane() const -> int { return m_playingLane; }

	//! An item's own settings
	struct ItemSettings
	{
		int lane = 0;
		double gainDb = 0.0;
		double fadeInMs = AudioItem::DefaultFadeMs;
		double fadeOutMs = AudioItem::DefaultFadeMs;
		AudioItem::Storage storage = AudioItem::Storage::Automatic;
		auto operator==(const ItemSettings&) const -> bool = default;
	};
	static auto settingsOf(const AudioItem& item) -> ItemSettings;
	//! Changes @p item's settings to @p settings: one undo step (nothing if they are the same)
	void changeItem(AudioItem* item, const ItemSettings& settings);
	//! Makes @p lane the one heard: one undo step
	void setPlayingLane(int lane);

	//! Moves every item @p count lanes down, and makes the first lane the one heard: room on top for a recording's
	//! passes. Not an undo step of its own: part of the recording's, which keeps the track's state
	void insertLanesOnTop(int count);

	//! Gives the player what plays now. It happens by itself after each change, once the event loop runs; this does
	//! it at once (e.g. before rendering without an event loop)
	void publish();

	//! The player's voices: items heard at once on a track (more are skipped)
	static constexpr std::size_t MaxVoices = 32;

private:
	void applySettings(AudioItem* item, const ItemSettings& settings);
	void applyPlayingLane(int lane);
	void schedulePublish();
	void watch(AudioItem* item);

	//! Where ticks begin in the period, as the song plays them (audio thread: from play() to the player)
	struct TickStart
	{
		f_cnt_t offset = 0;
		tick_t tick = 0;
	};
	static constexpr std::size_t MaxTickStarts = 256;
	std::array<TickStart, MaxTickStarts> m_tickStarts{};
	std::size_t m_tickStartCount = 0;
	std::int64_t m_tickStartsPeriod = -1;

	int m_playingLane = 0;
	bool m_publishPending = false;
	AudioTrackPlayer* m_player;

	friend class AudioItem;
	friend class AudioTrackPlayer;
	friend class ItemSettingsCommand;
	friend class PlayingLaneCommand;
};

} // namespace lmms

#endif // LMMS_AUDIO_TRACK_H
