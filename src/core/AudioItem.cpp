/*
 * AudioItem.cpp
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

#include "AudioItem.h"

#include <algorithm>
#include <cmath>

#include <QDomElement>
#include <QFileInfo>
#include <QRandomGenerator>

#include "AudioEngine.h"
#include "AudioItemView.h"
#include "AudioStream.h"
#include "AudioTrack.h"
#include "Engine.h"
#include "PathUtil.h"
#include "PeakCache.h"
#include "SampleCache.h"
#include "Song.h"
#include "Track.h"

namespace lmms
{

namespace
{

auto newId() -> std::uint64_t
{
	auto id = std::uint64_t{0};
	while (id == 0) { id = QRandomGenerator::global()->generate64(); }
	return id;
}

auto timingToElement(QDomDocument& doc, const char* name, const AudioItem::Timing& timing) -> QDomElement
{
	auto element = doc.createElement(name);
	for (const auto& point : timing)
	{
		auto pointElement = doc.createElement("point");
		pointElement.setAttribute("tick", point.tick);
		pointElement.setAttribute("frame", QString::number(point.frame, 'f', 3));
		element.appendChild(pointElement);
	}
	return element;
}

auto timingFromElement(const QDomElement& element) -> AudioItem::Timing
{
	auto timing = AudioItem::Timing{};
	for (auto point = element.firstChildElement("point"); !point.isNull(); point = point.nextSiblingElement("point"))
	{
		const auto frame = point.attribute("frame").toDouble();
		timing.push_back({std::clamp(point.attribute("tick").toInt(), 0, TimePos::MaxTicks),
			std::isfinite(frame) ? std::max(0.0, frame) : 0.0});
	}
	return timing;
}

} // namespace




AudioItem::AudioItem(Track* track)
	: Clip{track}
	, m_id{newId()}
{
	// (the track hears of its items from here: when the clip told it, the item was not made yet)
	if (auto* audioTrack = dynamic_cast<AudioTrack*>(track)) { audioTrack->watch(this); }
	connect(&PeakCache::instance(), &PeakCache::ready, this, &AudioItem::peaksReady);
	// (a change of tempo changes the file's length on the song, unless the item follows it)
	connect(Engine::getSong(), &Song::tempoChanged, this, [this] { if (!isTempoBound()) { emit itemChanged(); } });
}




AudioItem::AudioItem(const AudioItem& other)
	: Clip{other}
	, m_id{newId()}
	, m_file{other.m_file}
	, m_sample{other.m_sample}
	, m_peaks{other.m_peaks}
	, m_storage{other.m_storage}
	, m_lane{other.m_lane}
	, m_gainDb{other.m_gainDb}
	, m_fadeInMs{other.m_fadeInMs}
	, m_fadeOutMs{other.m_fadeOutMs}
	, m_originalTiming{other.m_originalTiming}
	, m_originalFile{other.m_originalFile}
	, m_renderedTiming{other.m_renderedTiming}
{
	if (auto* audioTrack = dynamic_cast<AudioTrack*>(getTrack())) { audioTrack->watch(this); }
	connect(&PeakCache::instance(), &PeakCache::ready, this, &AudioItem::peaksReady);
	connect(Engine::getSong(), &Song::tempoChanged, this, [this] { if (!isTempoBound()) { emit itemChanged(); } });
}




AudioItem::~AudioItem() = default;




auto AudioItem::gain() const -> float
{
	return m_gainDb <= MinGainDb ? 0.f : static_cast<float>(std::pow(10.0, m_gainDb / 20.0));
}




void AudioItem::load(const QString& file)
{
	auto sample = Sample{};
	if (!file.isEmpty())
	{
		// From disk or in memory, as chosen (a file libsndfile cannot stream is in memory)
		auto stream = m_storage == Storage::Memory ? nullptr : AudioStream::open(file);
		if (stream && m_storage == Storage::Automatic
			&& static_cast<double>(stream->frames()) / stream->sampleRate() < StreamAboveSeconds)
		{
			stream.reset();
		}
		if (stream) { sample = Sample{std::move(stream)}; }
		else if (auto buffer = SampleCache::instance().fetchBuffer(PathUtil::stringToPath(file)))
		{
			sample = Sample{std::move(buffer)};
		}
	}
	{
		// (the audio thread may be playing the sample being replaced)
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		m_sample = std::move(sample);
	}
	m_file = m_sample.sampleSize() > 0 ? m_sample.sampleFile() : QString{};
	m_peaks = m_file.isEmpty() ? nullptr : PeakCache::instance().fetch(PathUtil::toAbsolute(m_file));
}




void AudioItem::setFile(const QString& file)
{
	load(file);
	setStartTimeOffset(0);
	changeLength(std::max(fileLength(), TimePos{1}));
	emit itemChanged();
}




void AudioItem::playFile(const QString& file)
{
	const auto length = this->length();
	const auto offset = startTimeOffset();
	load(file);
	changeLength(length);
	setStartTimeOffset(offset);
	emit itemChanged();
}




auto AudioItem::fileLength() const -> TimePos
{
	if (m_sample.sampleSize() == 0) { return TimePos{0}; }
	const auto framesPerTick = Engine::framesPerTick(m_sample.sampleRate());
	return TimePos{static_cast<tick_t>(std::ceil(m_sample.sampleSize() / framesPerTick))};
}




void AudioItem::setLane(int lane)
{
	lane = std::max(lane, 0);
	if (lane == m_lane) { return; }
	m_lane = lane;
	emit itemChanged();
}




void AudioItem::setGainDb(double gainDb)
{
	gainDb = std::clamp(gainDb, MinGainDb, MaxGainDb);
	if (gainDb == m_gainDb) { return; }
	m_gainDb = gainDb;
	emit itemChanged();
}




void AudioItem::setFades(double inMs, double outMs)
{
	inMs = std::max(inMs, 0.0);
	outMs = std::max(outMs, 0.0);
	if (inMs == m_fadeInMs && outMs == m_fadeOutMs) { return; }
	m_fadeInMs = inMs;
	m_fadeOutMs = outMs;
	emit itemChanged();
}




void AudioItem::setStorage(Storage storage)
{
	if (storage == m_storage) { return; }
	m_storage = storage;
	if (!m_file.isEmpty()) { playFile(m_file); }
	else { emit itemChanged(); }
}




void AudioItem::setOriginalTiming(Timing timing, const QString& file)
{
	m_originalTiming = std::move(timing);
	m_originalFile = file;
	m_renderedTiming.clear();
	emit itemChanged();
}




void AudioItem::setRendering(const QString& file, Timing timing)
{
	m_renderedTiming = std::move(timing);
	playFile(file);
}




auto AudioItem::fileFrameAt(tick_t tick) const -> f_cnt_t
{
	if (!isTempoBound())
	{
		return static_cast<f_cnt_t>(std::max(0.0, std::round(static_cast<double>(Engine::framesPerTick(m_sample.sampleRate())) * tick)));
	}
	// From the timing point before it on, the file plays at its own rate while the song plays at its tempo
	const auto& timing = currentTiming();
	auto point = std::upper_bound(timing.begin(), timing.end(), tick,
		[](tick_t value, const TimingPoint& p) { return value < p.tick; });
	if (point != timing.begin()) { --point; }
	const auto* song = Engine::getSong();
	const auto start = startPosition().getTicks() + startTimeOffset().getTicks();
	const auto engineRate = static_cast<double>(Engine::audioEngine()->outputSampleRate());
	const auto fileRate = static_cast<double>(m_sample.sampleRate() > 0 ? m_sample.sampleRate() : engineRate);
	const auto frame = point->frame + song->framesBetween(start + point->tick, start + tick) * fileRate / engineRate;
	return static_cast<f_cnt_t>(std::max(0.0, std::round(frame)));
}




void AudioItem::peaksReady(const QString& file)
{
	if (m_file.isEmpty() || QFileInfo{PathUtil::toAbsolute(m_file)}.absoluteFilePath() != file) { return; }
	m_peaks = PeakCache::instance().fetch(file);
	emit itemChanged();
}




void AudioItem::saveSettings(QDomDocument& doc, QDomElement& element)
{
	element.setAttribute("pos", element.parentNode().nodeName() == "clipboard" ? -1 : startPosition().getTicks());
	element.setAttribute("len", length().getTicks());
	element.setAttribute("off", startTimeOffset().getTicks());
	element.setAttribute("muted", isMuted());
	element.setAttribute("src", PathUtil::toShortestRelative(m_file));
	element.setAttribute("id", QString::number(m_id));
	element.setAttribute("lane", m_lane);
	element.setAttribute("gain", QString::number(m_gainDb, 'g', 10));
	element.setAttribute("fadein", QString::number(m_fadeInMs, 'g', 10));
	element.setAttribute("fadeout", QString::number(m_fadeOutMs, 'g', 10));
	if (m_storage != Storage::Automatic) { element.setAttribute("storage", m_storage == Storage::Disk ? "disk" : "ram"); }
	if (const auto& c = color()) { element.setAttribute("color", c->name()); }
	if (isTempoBound())
	{
		auto original = timingToElement(doc, "original", m_originalTiming);
		original.setAttribute("src", PathUtil::toShortestRelative(m_originalFile));
		element.appendChild(original);
		if (!m_renderedTiming.empty()) { element.appendChild(timingToElement(doc, "rendered", m_renderedTiming)); }
	}
}




void AudioItem::loadSettings(const QDomElement& element)
{
	if (element.attribute("pos").toInt() >= 0) { movePosition(element.attribute("pos").toInt()); }

	// The id stays, unless another item of the track has it (e.g. pasted next to the item it was copied from)
	auto ok = false;
	const auto id = element.attribute("id").toULongLong(&ok);
	m_id = ok && id != 0 ? id : newId();
	if (auto* track = getTrack())
	{
		for (auto* clip : track->getClips())
		{
			if (clip != this && dynamic_cast<AudioItem*>(clip) && static_cast<AudioItem*>(clip)->m_id == m_id)
			{
				m_id = newId();
				break;
			}
		}
	}

	const auto storage = element.attribute("storage");
	m_storage = storage == "disk" ? Storage::Disk : storage == "ram" ? Storage::Memory : Storage::Automatic;
	m_lane = std::max(element.attribute("lane").toInt(), 0);
	const auto gain = element.attribute("gain", "0").toDouble();
	m_gainDb = std::isfinite(gain) ? std::clamp(gain, MinGainDb, MaxGainDb) : 0.0;
	const auto fadeIn = element.attribute("fadein", QString::number(DefaultFadeMs)).toDouble();
	const auto fadeOut = element.attribute("fadeout", QString::number(DefaultFadeMs)).toDouble();
	m_fadeInMs = std::isfinite(fadeIn) ? std::max(fadeIn, 0.0) : DefaultFadeMs;
	m_fadeOutMs = std::isfinite(fadeOut) ? std::max(fadeOut, 0.0) : DefaultFadeMs;

	const auto original = element.firstChildElement("original");
	m_originalTiming = original.isNull() ? Timing{} : timingFromElement(original);
	m_originalFile = original.attribute("src");
	m_renderedTiming = timingFromElement(element.firstChildElement("rendered"));
	if (m_originalTiming.size() < 2) { m_originalTiming.clear(); m_renderedTiming.clear(); }

	// The file (a rendering that is gone: the original plays, and is rendered again when asked)
	auto file = element.attribute("src");
	if (!file.isEmpty() && !QFileInfo::exists(PathUtil::toAbsolute(file)))
	{
		Engine::getSong()->collectError(QString{"%1: %2"}.arg(tr("Audio file not found"), file));
		if (isTempoBound() && QFileInfo::exists(PathUtil::toAbsolute(m_originalFile)))
		{
			m_renderedTiming.clear();
			file = m_originalFile;
		}
		else { file.clear(); }
	}
	load(file);
	// (a missing file keeps the item's place and length: the reference stays for when it is found)
	if (m_file.isEmpty() && !element.attribute("src").isEmpty()) { m_file = element.attribute("src"); }

	changeLength(std::max(element.attribute("len").toInt(), 1));
	setStartTimeOffset(element.attribute("off").toInt());
	setMuted(element.attribute("muted").toInt());
	if (element.hasAttribute("color")) { setColor(QColor{element.attribute("color")}); }
	if (isTempoBound()) { Engine::getSong()->requestStretchCheck(); }
	emit itemChanged();
}




gui::ClipView* AudioItem::createView(gui::TrackView* view)
{
	return new gui::AudioItemView{this, view};
}

} // namespace lmms
