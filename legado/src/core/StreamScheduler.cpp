/*
 * StreamScheduler.cpp
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

#include "StreamScheduler.h"

#include <algorithm>

#include "AudioEngine.h"
#include "AudioTrack.h"
#include "Engine.h"
#include "AudioResampler.h"
#include "RealtimeAllocator.h"
#include "SampleClip.h"
#include "SampleTrack.h"
#include "Song.h"

namespace lmms
{

StreamScheduler::StreamScheduler(Song* song)
	: QObject{song}
	, m_song{song}
{
	m_timer.setInterval(25);
	connect(&m_timer, &QTimer::timeout, this, &StreamScheduler::update);
	m_timer.start();
}




auto StreamScheduler::demandAt(const Song& song, tick_t tick, double seconds) -> std::vector<DiskReader::Demand>
{
	// The spans of the song about to play, with how far ahead each starts (in ticks)
	struct Span { tick_t begin; tick_t end; tick_t distance; };
	const auto framesPerTick = std::max(Engine::framesPerTick(), 1.f);
	const auto ahead = static_cast<tick_t>(std::min(seconds * Engine::audioEngine()->outputSampleRate() / framesPerTick,
		static_cast<double>(TimePos::MaxTicks)));
	auto spans = std::vector<Span>{{tick, tick + ahead, 0}};
	const auto& timeline = song.getTimeline(Song::PlayMode::Song);
	const auto loopBegin = timeline.loopBegin().getTicks();
	const auto loopEnd = timeline.loopEnd().getTicks();
	if (timeline.loopEnabled() && loopEnd > loopBegin && tick < loopEnd && tick + ahead > loopEnd)
	{
		// Playing on into the cycle's end goes back to its start
		spans.front().end = loopEnd;
		spans.push_back({loopBegin, loopBegin + (tick + ahead - loopEnd), loopEnd - tick});
	}

	struct Ranked { DiskReader::Demand demand; tick_t distance; };
	auto ranked = std::vector<Ranked>{};
	for (const auto* track : song.tracks())
	{
		// The audio tracks' items of the lane heard
		if (const auto* audioTrack = dynamic_cast<const AudioTrack*>(track))
		{
			for (const auto* item : audioTrack->items())
			{
				if (!item->isStreamed() || item->lane() != audioTrack->playingLane()) { continue; }
				const auto& stream = item->sample().stream();
				const auto itemBegin = item->startPosition().getTicks();
				const auto itemEnd = item->endPosition().getTicks();
				const auto from = itemBegin + item->startTimeOffset().getTicks();
				for (const auto& span : spans)
				{
					const auto begin = std::max(span.begin, itemBegin);
					const auto end = std::min(span.end, itemEnd);
					if (begin >= end) { continue; }
					const auto last = static_cast<f_cnt_t>(stream->frames()) - 1;
					const auto first = std::min(item->fileFrameAt(std::max(begin - from, 0)), last);
					const auto final = std::min(item->fileFrameAt(std::max(end - from, 0)) + 1, last);
					ranked.push_back({{stream, first, final}, span.distance + (begin - span.begin)});
				}
			}
			continue;
		}
		if (track->type() != Track::Type::Sample) { continue; }
		for (auto* clip : track->getClips())
		{
			auto* sampleClip = dynamic_cast<SampleClip*>(clip);
			if (!sampleClip || !sampleClip->isStreamed()) { continue; }
			const auto& stream = sampleClip->sample().stream();
			const auto clipBegin = sampleClip->startPosition().getTicks();
			const auto clipEnd = sampleClip->endPosition().getTicks();
			// Ticks from the start of the recording, as SampleTrack::play() counts them
			const auto from = clipBegin + sampleClip->startTimeOffset().getTicks();
			for (const auto& span : spans)
			{
				const auto begin = std::max(span.begin, clipBegin);
				const auto end = std::min(span.end, clipEnd);
				if (begin >= end) { continue; }
				const auto last = static_cast<f_cnt_t>(stream->frames()) - 1;
				auto first = std::min(sampleClip->fileFrameAt(std::max(begin - from, 0)), last);
				auto final = std::min(sampleClip->fileFrameAt(std::max(end - from, 0)) + 1, last);
				if (sampleClip->sample().reversed())
				{
					// (played from its end)
					first = last - first;
					final = last - final;
					std::swap(first, final);
				}
				ranked.push_back({{stream, first, final}, span.distance + (begin - span.begin)});
			}
		}
	}
	std::stable_sort(ranked.begin(), ranked.end(), [](const Ranked& a, const Ranked& b) { return a.distance < b.distance; });
	auto demand = std::vector<DiskReader::Demand>{};
	demand.reserve(ranked.size());
	for (auto& entry : ranked) { demand.push_back(std::move(entry.demand)); }
	return demand;
}




void StreamScheduler::update()
{
	// The audio thread's housekeeping, on this thread: what it took from the pools is made again, and what it let go
	// of the system's memory is freed (see AudioResampler::reserve(), RealtimeAllocator::deferFree())
	AudioResampler::refillPools();
	RealtimeAllocator::drainDeferredIfAny();

	if (m_song->isLoadingProject()) { return; }
	DiskReader::instance().setDemand(demandAt(*m_song, m_song->getPlayPos(Song::PlayMode::Song).getTicks(),
		DiskReader::ReadAheadSeconds));
}

} // namespace lmms
