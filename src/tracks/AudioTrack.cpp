/*
 * AudioTrack.cpp
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

#include "AudioTrack.h"

#include <algorithm>
#include <cmath>
#include <functional>
#include <numbers>

#include <QDomElement>
#include <QTimer>

#include "AudioEngine.h"
#include "Engine.h"
#include "Handoff.h"
#include "PlayHandle.h"
#include "ProjectJournal.h"
#include "SampleTrackView.h"
#include "Song.h"
#include "UndoCommand.h"

namespace lmms
{

namespace
{

//! What an item plays, as the player needs it (made by the track on the main thread)
struct PlayItem
{
	std::uint64_t id = 0;
	std::uint64_t key = 0; //!< its place and file: a voice playing it starts again when it changes
	Sample sample;
	tick_t begin = 0;
	tick_t end = 0;
	tick_t recordingStart = 0; //!< begin less the start offset: fileFrameAt() counts from there
	AudioItem::Timing timing; //!< for an item following the tempo
	float gain = 1.f;
	double fadeInFrames = 0;
	double fadeOutFrames = 0;
};

struct PlayList
{
	std::vector<PlayItem> items;
};

//! AudioItem::fileFrameAt() on the audio thread
auto fileFrameAt(const PlayItem& item, tick_t tick) -> f_cnt_t
{
	if (item.timing.empty())
	{
		return static_cast<f_cnt_t>(std::max(0.0, std::round(static_cast<double>(Engine::framesPerTick(item.sample.sampleRate())) * tick)));
	}
	auto point = std::upper_bound(item.timing.begin(), item.timing.end(), tick,
		[](tick_t value, const AudioItem::TimingPoint& p) { return value < p.tick; });
	if (point != item.timing.begin()) { --point; }
	const auto engineRate = static_cast<double>(Engine::audioEngine()->outputSampleRate());
	const auto fileRate = static_cast<double>(item.sample.sampleRate() > 0 ? item.sample.sampleRate() : engineRate);
	const auto frame = point->frame + Engine::getSong()->framesBetween(item.recordingStart + point->tick,
		item.recordingStart + tick) * fileRate / engineRate;
	return static_cast<f_cnt_t>(std::max(0.0, std::round(frame)));
}

} // namespace




/**
 * The audio track's player: a play handle that lives as long as the track. Each period it takes the list the track
 * published, and plays the items under the song's position with voices made beforehand: one per item heard, going on
 * from period to period while the song goes straight on, starting again where it jumps (a loop, a click on the
 * ruler) or when the item moved.
 */
class AudioTrackPlayer : public PlayHandle
{
public:
	explicit AudioTrackPlayer(AudioTrack* track)
		: PlayHandle{Type::AudioTrackPlayHandle}
		, m_track{track}
		, m_scratch(ScratchFrames)
	{
		setAudioBusHandle(track->audioBusHandle());
	}

	void offer(std::unique_ptr<PlayList> list) { m_list.offer(std::move(list)); }

	void play(std::span<SampleFrame> buffer) override
	{
		auto* list = m_list.take();
		const auto* song = Engine::getSong();
		if (!list || !song->isPlaying() || song->playMode() != Song::PlayMode::Song || m_track->isSilenced())
		{
			stopAll();
			m_nextTick = -1;
			return;
		}
		const auto period = Engine::audioEngine()->periodsRendered();
		const auto starts = m_track->m_tickStartsPeriod == period ? m_track->m_tickStartCount : std::size_t{0};
		auto from = std::size_t{0};
		for (std::size_t index = 0; index < starts; ++index)
		{
			const auto& start = m_track->m_tickStarts[index];
			const auto at = std::min(static_cast<std::size_t>(start.offset), buffer.size());
			render(buffer, from, at);
			tickStarts(*list, start.tick);
			from = at;
		}
		render(buffer, from, buffer.size());
	}

	bool isFinished() const override { return false; }
	bool isFromTrack(const Track* track) const override { return track == m_track; }

private:
	static constexpr std::size_t ScratchFrames = 4096;

	struct Voice
	{
		bool active = false;
		std::uint64_t id = 0;
		std::uint64_t key = 0;
		const PlayItem* item = nullptr;
		Sample::PlaybackState state{AudioResampler::Mode::SincMedium};
		double framesDone = 0; //!< since the item's start, on the song
		double framesLeft = 0; //!< until its end
	};

	void stopAll()
	{
		for (auto& voice : m_voices) { voice.active = false; voice.item = nullptr; }
	}

	//! A tick starts: the voices follow the items under it
	void tickStarts(const PlayList& list, tick_t tick)
	{
		// Not where the song went on to: everything starts again from there
		if (tick != m_nextTick) { stopAll(); }
		m_nextTick = tick + 1;

		for (auto& voice : m_voices)
		{
			if (!voice.active) { continue; }
			const auto found = std::find_if(list.items.begin(), list.items.end(),
				[&](const PlayItem& item) { return item.id == voice.id; });
			// Gone, or moved or given another file: it starts again below if it is under the tick
			if (found == list.items.end() || found->key != voice.key) { voice.active = false; voice.item = nullptr; }
			else { voice.item = &*found; }
		}
		for (const auto& item : list.items)
		{
			if (tick < item.begin || tick >= item.end) { continue; }
			const auto playing = std::any_of(m_voices.begin(), m_voices.end(),
				[&](const Voice& voice) { return voice.active && voice.id == item.id; });
			if (playing) { continue; }
			const auto free = std::find_if(m_voices.begin(), m_voices.end(), [](const Voice& voice) { return !voice.active; });
			if (free == m_voices.end()) { break; } // (more items at once than voices)
			const auto frame = fileFrameAt(item, tick - item.recordingStart);
			if (frame >= static_cast<f_cnt_t>(item.sample.sampleSize())) { continue; }
			free->active = true;
			free->id = item.id;
			free->key = item.key;
			free->item = &item;
			free->state.restart(static_cast<int>(frame));
			const auto* song = Engine::getSong();
			free->framesDone = song->framesBetween(item.begin, tick);
			free->framesLeft = song->framesBetween(tick, item.end);
		}
	}

	//! Plays the voices into frames [@p from, @p to) of @p buffer
	void render(std::span<SampleFrame> buffer, std::size_t from, std::size_t to)
	{
		const auto engineRate = static_cast<int>(Engine::audioEngine()->outputSampleRate());
		for (auto& voice : m_voices)
		{
			if (!voice.active || !voice.item) { continue; }
			const auto& item = *voice.item;
			auto at = from;
			while (at < to && voice.framesLeft > 0.5)
			{
				const auto count = std::min({to - at, ScratchFrames,
					static_cast<std::size_t>(std::ceil(voice.framesLeft))});
				if (item.sample.sampleRate() == engineRate)
				{
					item.sample.copyFrames(m_scratch.data(), &voice.state, count);
				}
				else if (!item.sample.play(m_scratch.data(), &voice.state, count))
				{
					std::fill_n(m_scratch.data(), count, SampleFrame{});
				}
				for (std::size_t frame = 0; frame < count; ++frame)
				{
					const auto done = voice.framesDone + static_cast<double>(frame);
					const auto left = voice.framesLeft - static_cast<double>(frame);
					auto gain = item.gain;
					// Equal-power fades (so a crossfade over an overlap keeps the level)
					if (done < item.fadeInFrames)
					{
						gain *= static_cast<float>(std::sin(std::numbers::pi / 2 * done / item.fadeInFrames));
					}
					if (left < item.fadeOutFrames)
					{
						gain *= static_cast<float>(std::sin(std::numbers::pi / 2 * std::max(0.0, left) / item.fadeOutFrames));
					}
					buffer[at + frame] += m_scratch[frame] * gain;
				}
				voice.framesDone += static_cast<double>(count);
				voice.framesLeft -= static_cast<double>(count);
				at += count;
			}
			if (voice.framesLeft <= 0.5) { voice.active = false; voice.item = nullptr; }
		}
	}

	AudioTrack* m_track;
	Handoff<PlayList> m_list;
	std::array<Voice, AudioTrack::MaxVoices> m_voices;
	std::vector<SampleFrame> m_scratch;
	tick_t m_nextTick = -1;
};




//! Changes an item's settings (finds the track by its journal id and the item by its own)
class ItemSettingsCommand : public UndoCommand
{
public:
	ItemSettingsCommand(AudioTrack* track, AudioItem* item, AudioTrack::ItemSettings before,
		AudioTrack::ItemSettings after)
		: m_track{track->id()}
		, m_item{item->id()}
		, m_before{before}
		, m_after{after}
	{
	}

	void apply() override { set(m_after); }
	void revert() override { set(m_before); }
	auto text() const -> QString override { return AudioTrack::tr("Change audio item"); }

private:
	void set(const AudioTrack::ItemSettings& settings)
	{
		auto* track = dynamic_cast<AudioTrack*>(Engine::projectJournal()->journallingObject(m_track));
		if (!track) { return; }
		if (auto* item = track->item(m_item)) { track->applySettings(item, settings); }
	}

	jo_id_t m_track;
	std::uint64_t m_item;
	AudioTrack::ItemSettings m_before;
	AudioTrack::ItemSettings m_after;
};




//! Chooses the lane heard
class PlayingLaneCommand : public UndoCommand
{
public:
	PlayingLaneCommand(AudioTrack* track, int before, int after)
		: m_track{track->id()}
		, m_before{before}
		, m_after{after}
	{
	}

	void apply() override { set(m_after); }
	void revert() override { set(m_before); }
	auto text() const -> QString override { return AudioTrack::tr("Choose the lane heard"); }

private:
	void set(int lane)
	{
		if (auto* track = dynamic_cast<AudioTrack*>(Engine::projectJournal()->journallingObject(m_track)))
		{
			track->applyPlayingLane(lane);
		}
	}

	jo_id_t m_track;
	int m_before;
	int m_after;
};




AudioTrack::AudioTrack(TrackContainer* container)
	: AudioInputTrack{Track::Type::Audio, container, tr("Audio track")}
	, m_player{new AudioTrackPlayer{this}}
{
	setName(tr("Audio track"));
	Engine::audioEngine()->addPlayHandle(m_player);
	// The fades and crossfades are in frames: the tempo changes them
	connect(Engine::getSong(), &Song::tempoChanged, this, &AudioTrack::schedulePublish);
	connect(&Engine::getSong()->tempoMap(), &TempoMap::changed, this, &AudioTrack::schedulePublish);
	connect(getMutedModel(), &BoolModel::dataChanged, this, &AudioTrack::schedulePublish);
	publish();
}




AudioTrack::~AudioTrack()
{
	// The items go after this track part (~Track): they must not call it back
	for (auto* item : items()) { item->disconnect(this); }
	Engine::audioEngine()->removePlayHandlesOfTypes(this, PlayHandle::Type::AudioTrackPlayHandle);
	m_player = nullptr;
}




bool AudioTrack::play(const TimePos& start, const f_cnt_t, const f_cnt_t offset, int)
{
	// A tick starts at @p offset in the period: the player follows the song from there (see AudioTrackPlayer)
	const auto period = Engine::audioEngine()->periodsRendered();
	if (m_tickStartsPeriod != period)
	{
		m_tickStartsPeriod = period;
		m_tickStartCount = 0;
	}
	if (m_tickStartCount < m_tickStarts.size()) { m_tickStarts[m_tickStartCount++] = {offset, start.getTicks()}; }
	return false;
}




gui::TrackView* AudioTrack::createView(gui::TrackContainerView* view)
{
	return new gui::SampleTrackView{this, view};
}




Clip* AudioTrack::createClip(const TimePos& pos)
{
	auto* item = new AudioItem{this};
	item->movePosition(pos);
	return item;
}




void AudioTrack::watch(AudioItem* item)
{
	connect(item, &AudioItem::itemChanged, this, &AudioTrack::schedulePublish);
	connect(item, &Clip::positionChanged, this, &AudioTrack::schedulePublish);
	connect(item, &Clip::lengthChanged, this, &AudioTrack::schedulePublish);
	connect(item, &Model::dataChanged, this, &AudioTrack::schedulePublish); // (muted)
	connect(item, &QObject::destroyed, this, &AudioTrack::schedulePublish);
	// A recording moved under a tempo map may need another rendering
	connect(item, &Clip::positionChanged, this, [item] {
		if (item->isTempoBound()) { Engine::getSong()->requestStretchCheck(); }
	});
	schedulePublish();
}




auto AudioTrack::items() const -> std::vector<AudioItem*>
{
	auto result = std::vector<AudioItem*>{};
	for (auto* clip : getClips())
	{
		if (auto* item = dynamic_cast<AudioItem*>(clip)) { result.push_back(item); }
	}
	return result;
}




auto AudioTrack::item(std::uint64_t id) const -> AudioItem*
{
	for (auto* clip : getClips())
	{
		auto* item = dynamic_cast<AudioItem*>(clip);
		if (item && item->id() == id) { return item; }
	}
	return nullptr;
}




auto AudioTrack::addItem(const QString& file, const TimePos& position, int lane) -> AudioItem*
{
	auto* item = static_cast<AudioItem*>(createClip(position));
	item->m_lane = std::max(lane, 0);
	item->setFile(file);
	return item;
}




auto AudioTrack::laneCount() const -> int
{
	auto count = std::max(1, m_playingLane + 1);
	for (const auto* item : items()) { count = std::max(count, item->lane() + 1); }
	return count;
}




auto AudioTrack::settingsOf(const AudioItem& item) -> ItemSettings
{
	return {item.lane(), item.gainDb(), item.fadeInMs(), item.fadeOutMs(), item.storage()};
}




void AudioTrack::changeItem(AudioItem* item, const ItemSettings& settings)
{
	if (!item || item->getTrack() != this) { return; }
	const auto before = settingsOf(*item);
	if (before == settings) { return; }
	Engine::projectJournal()->execute(std::make_unique<ItemSettingsCommand>(this, item, before, settings));
	Engine::getSong()->setModified();
}




void AudioTrack::applySettings(AudioItem* item, const ItemSettings& settings)
{
	item->setLane(settings.lane);
	item->setGainDb(settings.gainDb);
	item->setFades(settings.fadeInMs, settings.fadeOutMs);
	item->setStorage(settings.storage);
}




void AudioTrack::setPlayingLane(int lane)
{
	lane = std::max(lane, 0);
	if (lane == m_playingLane) { return; }
	Engine::projectJournal()->execute(std::make_unique<PlayingLaneCommand>(this, m_playingLane, lane));
	Engine::getSong()->setModified();
}




void AudioTrack::insertLanesOnTop(int count)
{
	if (count <= 0) { return; }
	for (auto* item : items()) { item->setLane(item->lane() + count); }
	applyPlayingLane(0);
}




void AudioTrack::clearSpan(int lane, tick_t begin, tick_t end, const AudioItem* keep)
{
	if (end <= begin) { return; }
	for (auto* item : items())
	{
		if (item == keep || item->lane() != lane) { continue; }
		const auto itemBegin = item->startPosition().getTicks();
		const auto itemEnd = item->endPosition().getTicks();
		if (itemEnd <= begin || itemBegin >= end) { continue; }
		// Where the recording starts on the song (the file stays where it is under each part)
		const auto recordingStart = itemBegin + item->startTimeOffset().getTicks();
		if (itemBegin >= begin && itemEnd <= end)
		{
			delete item; // (all under the new take)
			continue;
		}
		if (itemBegin < begin && itemEnd > end)
		{
			// Split around it: the part after it is a copy
			auto* after = static_cast<AudioItem*>(item->clone());
			after->movePosition(TimePos{end});
			after->changeLength(TimePos{itemEnd - end});
			after->setStartTimeOffset(TimePos{recordingStart - end});
			item->changeLength(TimePos{begin - itemBegin});
			continue;
		}
		if (itemBegin < begin) { item->changeLength(TimePos{begin - itemBegin}); } // (its end under it)
		else
		{
			// Its start under it
			item->movePosition(TimePos{end});
			item->changeLength(TimePos{itemEnd - end});
			item->setStartTimeOffset(TimePos{recordingStart - end});
		}
	}
}




void AudioTrack::compItem(AudioItem* item)
{
	if (!item || item->getTrack() != this || item->lane() == m_playingLane) { return; }
	addJournalCheckPoint();
	const auto pause = ProjectJournal::Pause{Engine::projectJournal()};
	auto* copy = static_cast<AudioItem*>(item->clone());
	copy->setLane(m_playingLane);
	clearSpan(m_playingLane, copy->startPosition().getTicks(), copy->endPosition().getTicks(), copy);
	Engine::getSong()->setModified();
}




void AudioTrack::applyPlayingLane(int lane)
{
	m_playingLane = std::max(lane, 0);
	schedulePublish();
	emit dataChanged();
}




void AudioTrack::schedulePublish()
{
	// (whatever changed may change the lanes too: told with the list, see publishLanes())
	if (m_publishPending || !m_player) { return; }
	m_publishPending = true;
	QTimer::singleShot(0, this, [this] {
		if (!m_publishPending) { return; }
		publish();
	});
}




void AudioTrack::publishLanes()
{
	const auto lanes = laneCount();
	if (lanes != m_lastLaneCount || m_playingLane != m_lastPlayingLane)
	{
		m_lastLaneCount = lanes;
		m_lastPlayingLane = m_playingLane;
		emit lanesChanged();
	}
}




void AudioTrack::publish()
{
	m_publishPending = false;
	if (!m_player) { return; }
	const auto* song = Engine::getSong();
	const auto rate = static_cast<double>(Engine::audioEngine()->outputSampleRate());

	auto list = std::make_unique<PlayList>();
	for (const auto* item : items())
	{
		if (item->lane() != m_playingLane || item->isMuted() || item->sample().sampleSize() == 0) { continue; }
		auto entry = PlayItem{};
		entry.id = item->id();
		entry.sample = item->sample();
		entry.begin = item->startPosition().getTicks();
		entry.end = item->endPosition().getTicks();
		entry.recordingStart = entry.begin + item->startTimeOffset().getTicks();
		if (item->isTempoBound()) { entry.timing = item->currentTiming(); }
		entry.gain = item->gain();

		// Its fades, at most half its length each
		const auto length = song->framesBetween(entry.begin, entry.end);
		entry.fadeInFrames = std::min(item->fadeInMs() * rate / 1000.0, length / 2);
		entry.fadeOutFrames = std::min(item->fadeOutMs() * rate / 1000.0, length / 2);

		// What a voice must start again for: its place and its file
		const auto* media = entry.sample.isStreamed() ? static_cast<const void*>(entry.sample.stream().get())
			: static_cast<const void*>(&entry.sample.buffer());
		auto key = std::hash<std::uint64_t>{}(static_cast<std::uint64_t>(entry.begin));
		for (const auto value : {static_cast<std::uint64_t>(entry.end), static_cast<std::uint64_t>(entry.recordingStart),
			static_cast<std::uint64_t>(reinterpret_cast<std::uintptr_t>(media)),
			static_cast<std::uint64_t>(entry.timing.size()),
			static_cast<std::uint64_t>(entry.timing.empty() ? 0.0 : entry.timing.back().frame)})
		{
			key ^= std::hash<std::uint64_t>{}(value) + 0x9e3779b97f4a7c15ull + (key << 6) + (key >> 2);
		}
		entry.key = key;
		list->items.push_back(std::move(entry));
	}

	// Where items overlap, the earlier fades out and the later fades in over the overlap
	std::sort(list->items.begin(), list->items.end(), [](const PlayItem& a, const PlayItem& b) { return a.begin < b.begin; });
	for (std::size_t first = 0; first < list->items.size(); ++first)
	{
		for (auto second = first + 1; second < list->items.size(); ++second)
		{
			auto& earlier = list->items[first];
			auto& later = list->items[second];
			if (later.begin >= earlier.end) { break; }
			if (later.end <= earlier.end) { continue; } // (inside it: no crossfade)
			const auto overlap = song->framesBetween(later.begin, earlier.end);
			earlier.fadeOutFrames = std::max(earlier.fadeOutFrames, overlap);
			later.fadeInFrames = std::max(later.fadeInFrames, overlap);
		}
	}
	m_player->offer(std::move(list));
	publishLanes();
}




void AudioTrack::saveTrackSpecificSettings(QDomDocument& doc, QDomElement& element, bool)
{
	saveAudioSettings(doc, element);
	element.setAttribute("playinglane", m_playingLane);
}




void AudioTrack::loadTrackSpecificSettings(const QDomElement& element)
{
	loadAudioSettings(element);
	m_playingLane = std::max(element.attribute("playinglane").toInt(), 0);
	schedulePublish();
}

} // namespace lmms
