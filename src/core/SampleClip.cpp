/*
 * SampleClip.cpp
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
 
#include "SampleClip.h"

#include <QDomElement>
#include <QFileInfo>
#include <algorithm>
#include <cmath>
#include <numbers>

#include "PatternStore.h"
#include "PeakCache.h"
#include "ThreadedExportManager.h"
#include "PathUtil.h"
#include "SampleCache.h"
#include "SampleClipView.h"
#include "SampleTrack.h"
#include "Song.h"

namespace lmms
{

SampleClip::SampleClip(Track* _track, Sample sample, bool isPlaying):
	Clip(_track),
	m_sample(std::move(sample)),
	m_isPlaying(false),
	m_startFrameOffset(0)
{
	saveJournallingState( false );
	setSampleFile( "" );
	restoreJournallingState();

	// we need to receive bpm-change-events, because then we have to
	// change length of this Clip
	connect(Engine::getSong(), &Song::tempoChanged, this, &SampleClip::tempoChanged, Qt::DirectConnection);
	connect(Engine::getSong(), &Song::timeSignatureChanged, this, &SampleClip::updateLength);

	//playbutton clicked or space key / on Export Song set isPlaying to false
	connect( Engine::getSong(), SIGNAL(playbackStateChanged()),
			this, SLOT(playbackPositionChanged()), Qt::DirectConnection );
	//care about loops and jumps
	connect(Engine::getSong(), &Song::playbackPositionJumped,
			this, &SampleClip::playbackPositionChanged, Qt::DirectConnection);
	//care about mute Clips
	connect( this, SIGNAL(dataChanged()), this, SLOT(playbackPositionChanged()));
	//care about mute track
	connect( getTrack()->getMutedModel(), SIGNAL(dataChanged()),
			this, SLOT(playbackPositionChanged()), Qt::DirectConnection );
	//care about Clip position
	connect( this, SIGNAL(positionChanged()), this, SLOT(updateTrackClips()));
	// a recording moved under a tempo map may need another rendering
	connect(this, &Clip::positionChanged, this, [this] {
		if (isTempoBound()) { Engine::getSong()->requestStretchCheck(); }
	});
	// the waveform of a file streamed from disk comes when its peaks are built
	connect(&PeakCache::instance(), &PeakCache::ready, this, &SampleClip::peaksReady);

	updateTrackClips();
}

SampleClip::SampleClip(Track* track)
	: SampleClip(track, Sample(), false)
{
}

SampleClip::SampleClip(const SampleClip& orig) :
	Clip(orig),
	m_sample(std::move(orig.m_sample)),
	m_storage(orig.m_storage),
	m_takes(orig.m_takes),
	m_activeTake(orig.m_activeTake),
	m_originalTiming(orig.m_originalTiming),
	m_originalFile(orig.m_originalFile),
	m_renderedTiming(orig.m_renderedTiming),
	m_sampleThumbnailCache(orig.m_sampleThumbnailCache),
	m_isPlaying(orig.m_isPlaying),
	m_startFrameOffset(orig.m_startFrameOffset)
{
	saveJournallingState( false );
	setSampleFile( "" );
	restoreJournallingState();

	// we need to receive bpm-change-events, because then we have to
	// change length of this Clip
	connect(Engine::getSong(), &Song::tempoChanged, this, &SampleClip::tempoChanged, Qt::DirectConnection);
	connect( Engine::getSong(), SIGNAL(timeSignatureChanged(int,int)),
					this, SLOT(updateLength()));

	//playbutton clicked or space key / on Export Song set isPlaying to false
	connect( Engine::getSong(), SIGNAL(playbackStateChanged()),
			this, SLOT(playbackPositionChanged()), Qt::DirectConnection );
	//care about loops and jumps
	connect(Engine::getSong(), &Song::playbackPositionJumped,
			this, &SampleClip::playbackPositionChanged, Qt::DirectConnection);
	//care about mute Clips
	connect( this, SIGNAL(dataChanged()), this, SLOT(playbackPositionChanged()));
	//care about mute track
	connect( getTrack()->getMutedModel(), SIGNAL(dataChanged()),
			this, SLOT(playbackPositionChanged()), Qt::DirectConnection );
	//care about Clip position
	connect( this, SIGNAL(positionChanged()), this, SLOT(updateTrackClips()));
	// a recording moved under a tempo map may need another rendering
	connect(this, &Clip::positionChanged, this, [this] {
		if (isTempoBound()) { Engine::getSong()->requestStretchCheck(); }
	});
	// the waveform of a file streamed from disk comes when its peaks are built
	connect(&PeakCache::instance(), &PeakCache::ready, this, &SampleClip::peaksReady);

	updateTrackClips();
}




SampleClip::~SampleClip()
{
	auto sampletrack = dynamic_cast<SampleTrack*>(getTrack());
	if ( sampletrack )
	{
		sampletrack->updateClips();
	}
}




void SampleClip::changeLength( const TimePos & _length )
{
	Clip::changeLength(std::max(static_cast<int>(_length), 1));
}



const QString& SampleClip::sampleFile() const
{
	return m_sample.sampleFile();
}

bool SampleClip::hasSampleFileLoaded(const QString & filename) const
{
	return m_sample.sampleFile() == filename;
}

void SampleClip::setSampleBuffer(std::shared_ptr<const SampleBuffer> sb)
{
	if (!sb) { return; }

	{
		const auto guard = Engine::audioEngine()->requestChangesGuard();
		m_sample = Sample(std::move(sb));
		m_sampleThumbnailCache = std::make_shared<const SampleThumbnailCache>(m_sample.buffer());
	}

	updateLength();
	emit sampleChanged();
	Engine::getSong()->setModified();
}

void SampleClip::addTake(const QString& file)
{
	// A clip that had one sample keeps it as its first take
	if (m_takes.isEmpty() && !sampleFile().isEmpty()) { m_takes << sampleFile(); }
	m_takes << file;
	setActiveTake(static_cast<int>(m_takes.size()) - 1);
}




void SampleClip::setActiveTake(int index)
{
	if (index < 0 || index >= m_takes.size()) { return; }
	addJournalCheckPoint();
	m_activeTake = index;

	// The clip keeps its place, length and start in the recording (a part of a split clip stays that part, so a
	// take can be chosen for each part: comping); only what it plays changes. A take is played as recorded until
	// it is rendered for the tempo too.
	if (isTempoBound())
	{
		m_originalFile = m_takes[index];
		m_renderedTiming.clear();
	}
	playFile(m_takes[index]);
	if (isTempoBound()) { Engine::getSong()->requestStretchCheck(); }
}




void SampleClip::playFile(const QString& file)
{
	const auto length = this->length();
	const auto offset = startTimeOffset();
	const auto autoResize = getAutoResize();
	const bool hadFile = !sampleFile().isEmpty();
	setSampleFile(file);
	if (hadFile)
	{
		setAutoResize(autoResize);
		changeLength(length);
		setStartTimeOffset(offset);
	}
	emit sampleChanged();
}




void SampleClip::setOriginalTiming(Timing timing, const QString& file)
{
	m_originalTiming = std::move(timing);
	m_originalFile = file;
	m_renderedTiming.clear();
}




void SampleClip::setRendering(const QString& file, Timing timing)
{
	m_renderedTiming = std::move(timing);
	playFile(file);
	Engine::getSong()->setModified();
}




void SampleClip::removeTake(int index)
{
	if (m_takes.size() < 2 || index < 0 || index >= m_takes.size()) { return; }
	addJournalCheckPoint();
	m_takes.removeAt(index);
	const auto active = std::min(m_activeTake > index ? m_activeTake - 1 : m_activeTake,
		static_cast<int>(m_takes.size()) - 1);
	m_activeTake = -1;
	setActiveTake(active);
	if (m_takes.size() == 1)
	{
		// One take left: a clip of one sample again
		m_takes.clear();
		m_activeTake = -1;
	}
}




void SampleClip::setSampleFile(const QString& sf)
{
	if (sf.isEmpty())
	{
		// If there is no sample, make the clip a bar long
		float nom = Engine::getSong()->getTimeSigModel().getNumerator();
		float den = Engine::getSong()->getTimeSigModel().getDenominator();
		changeLength(DefaultTicksPerBar * (nom / den));
	}
	else
	{
		if (auto stream = streamFor(sf))
		{
			// From disk: the waveform from the file's peaks (null until they are built, see peaksReady())
			auto peaks = PeakCache::instance().fetch(stream->path());
			setStartTimeOffset(0);
			{
				const auto guard = Engine::audioEngine()->requestChangesGuard();
				m_sample = Sample{std::move(stream)};
			}
			m_sampleThumbnailCache = std::move(peaks);
			updateLength();
			emit sampleChanged();
			emit playbackPositionChanged();
			return;
		}

		const auto path = PathUtil::stringToPath(sf);

		// TODO: Return an error value back to the caller (via std::expected or our own polyfill)
		auto buffer = SampleCache::instance().fetchBuffer(path);
		if (!buffer) { return; }

		// TODO: Return an error value back to the caller (via std::expected or our own polyfill)
		auto thumbnailCache = SampleCache::instance().fetchThumbnailCache(path);
		if (!thumbnailCache) { return; }

		// Remove any prior offset in the clip
		setStartTimeOffset(0);

		{
			// (the audio thread may be playing the sample being replaced)
			const auto guard = Engine::audioEngine()->requestChangesGuard();
			m_sample = Sample{std::move(buffer)};
		}
		m_sampleThumbnailCache = std::move(thumbnailCache);
		updateLength();
	}

	emit sampleChanged();
	emit playbackPositionChanged();
}








auto SampleClip::streamFor(const QString& file) const -> std::shared_ptr<AudioStream>
{
	if (m_storage == Storage::Memory) { return nullptr; }
	// (the pattern editor plays its clips from their start at any time: they stay in memory)
	if (!getTrack() || getTrack()->trackContainer() != Engine::getSong()) { return nullptr; }
	auto stream = AudioStream::open(file);
	if (!stream) { return nullptr; }
	if (m_storage == Storage::Disk) { return stream; }
	const auto seconds = static_cast<double>(stream->frames()) / stream->sampleRate();
	return seconds >= StreamAboveSeconds ? stream : nullptr;
}




void SampleClip::setStorage(Storage storage)
{
	if (storage == m_storage) { return; }
	addJournalCheckPoint();
	m_storage = storage;
	if (sampleFile().isEmpty()) { return; }

	// The same file, from memory or from disk: the clip keeps its place, length, start and settings
	const auto reversed = m_sample.reversed();
	const auto amplification = m_sample.amplification();
	const auto frequency = m_sample.frequency();
	playFile(sampleFile());
	m_sample.setReversed(reversed);
	m_sample.setAmplification(amplification);
	m_sample.setFrequency(frequency);
	emit sampleChanged();
	Engine::getSong()->setModified();
}




void SampleClip::peaksReady(const QString& file)
{
	if (!m_sample.isStreamed() || QFileInfo{m_sample.stream()->path()}.absoluteFilePath() != file) { return; }
	m_sampleThumbnailCache = PeakCache::instance().fetch(file);
	emit sampleChanged();
}




void SampleClip::playbackPositionChanged()
{
	Engine::audioEngine()->removePlayHandlesOfTypes( getTrack(), PlayHandle::Type::SamplePlayHandle );
	auto st = dynamic_cast<SampleTrack*>(getTrack());
	st->setPlayingClips( false );
}




void SampleClip::updateTrackClips()
{
	auto sampletrack = dynamic_cast<SampleTrack*>(getTrack());
	if( sampletrack)
	{
		sampletrack->updateClips();
	}
}




bool SampleClip::isPlaying() const
{
	return m_isPlaying;
}




void SampleClip::setIsPlaying(bool isPlaying)
{
	m_isPlaying = isPlaying;
}




void SampleClip::updateLength()
{
	// If the clip has already been manually resized, don't automatically resize it.
	if (getAutoResize())
	{
		if (getTrack()->trackContainer() == Engine::patternStore())
		{
			changeLength(TimePos::ticksPerBar() * Engine::patternStore()->lengthOfPattern(getTrack()->getClipNum(this)));
			return;
		}
		changeLength(sampleLength());
		setStartTimeOffset(0);
	}

	emit sampleChanged();
}


void SampleClip::tempoChanged()
{
	if (isTempoBound())
	{
		// A recording is musical: its place, length and start in it stay in ticks, and a rendering follows the tempo
		m_startFrameOffset = startTimeOffset() * Engine::framesPerTick();
		return;
	}
	Clip::setStartTimeOffset(std::round(1.0f * m_startFrameOffset / Engine::framesPerTick()));
	updateLength();
	emit sampleChanged();
}

void SampleClip::setStartTimeOffset(const TimePos& startTimeOffset)
{
	m_startFrameOffset = startTimeOffset * Engine::framesPerTick();
	Clip::setStartTimeOffset(startTimeOffset);
}


auto SampleClip::fileFrameAt(tick_t tick) const -> f_cnt_t
{
	if (!isTempoBound())
	{
		return static_cast<f_cnt_t>(Engine::framesPerTick(m_sample.sampleRate()) * tick);
	}

	// From the timing point before it on, the file plays at its own rate while the song plays at its tempo: at the
	// timing the file was rendered to, that lands each later point on its tick
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




TimePos SampleClip::sampleLength() const
{
	return static_cast<int>(m_sample.sampleSize() / Engine::framesPerTick(m_sample.sampleRate()));
}




void SampleClip::setSampleStartFrame(f_cnt_t startFrame)
{
	m_sample.setStartFrame(startFrame);
}




void SampleClip::setSamplePlayLength(f_cnt_t length)
{
	m_sample.setEndFrame(length);
}




void SampleClip::saveSettings( QDomDocument & _doc, QDomElement & _this )
{
	if( _this.parentNode().nodeName() == "clipboard" )
	{
		_this.setAttribute( "pos", -1 );
	}
	else
	{
		_this.setAttribute( "pos", startPosition() );
	}
	_this.setAttribute( "len", length() );
	_this.setAttribute( "muted", isMuted() );
	_this.setAttribute( "src", sampleFile() );
	_this.setAttribute( "off", startTimeOffset() );
	_this.setAttribute("autoresize", QString::number(getAutoResize()));
	_this.setAttribute("startfadelength", QString::number(startCrossfadeLength()));
	_this.setAttribute("endfadelength", QString::number(endCrossfadeLength()));
	_this.setAttribute("startfadetension", QString::number(startCrossfadeTension()));
	_this.setAttribute("endfadetension", QString::number(endCrossfadeTension()));
	if( sampleFile() == "" )
	{
		QString s;
		_this.setAttribute("data", m_sample.toBase64());
	}

	_this.setAttribute( "sample_rate", m_sample.sampleRate());
	if (m_storage != Storage::Automatic) { _this.setAttribute("storage", m_storage == Storage::Disk ? "disk" : "ram"); }

	// The recording's original and its timing, if the clip is bound to its tempo ("src" is the rendering then)
	if (isTempoBound())
	{
		const auto saveTiming = [&](const char* name, const Timing& timing) {
			auto element = _doc.createElement(name);
			for (const auto& point : timing)
			{
				auto pointElement = _doc.createElement("point");
				pointElement.setAttribute("tick", point.tick);
				pointElement.setAttribute("frame", QString::number(point.frame, 'f', 3));
				element.appendChild(pointElement);
			}
			_this.appendChild(element);
			return element;
		};
		saveTiming("original", m_originalTiming).setAttribute("src", PathUtil::toShortestRelative(m_originalFile));
		if (!m_renderedTiming.empty()) { saveTiming("rendered", m_renderedTiming); }
	}

	// The takes; "src" is the active one, which older versions play
	if (m_takes.size() > 1)
	{
		_this.setAttribute("activetake", m_activeTake);
		for (const auto& take : m_takes)
		{
			auto element = _doc.createElement("take");
			element.setAttribute("src", PathUtil::toShortestRelative(take));
			_this.appendChild(element);
		}
	}
	if (const auto& c = color())
	{
		_this.setAttribute("color", c->name());
	}
	if (m_sample.reversed())
	{
		_this.setAttribute("reversed", "true");
	}
	// TODO: start- and end-frame
}




void SampleClip::loadSettings( const QDomElement & _this )
{
	if( _this.attribute( "pos" ).toInt() >= 0 )
	{
		movePosition( _this.attribute( "pos" ).toInt() );
	}

	// (before the file: it says where the file goes)
	const auto storage = _this.attribute("storage");
	m_storage = storage == "disk" ? Storage::Disk : storage == "ram" ? Storage::Memory : Storage::Automatic;

	if (const auto srcFile = _this.attribute("src"); !srcFile.isEmpty())
	{
		if (QFileInfo(PathUtil::toAbsolute(srcFile)).exists())
		{
			setSampleFile(srcFile);
		}
		else { Engine::getSong()->collectError(QString("%1: %2").arg(tr("Sample not found"), srcFile)); }
	}

	if( sampleFile().isEmpty() && _this.hasAttribute( "data" ) )
	{
		auto sampleRate = _this.hasAttribute("sample_rate") ? _this.attribute("sample_rate").toInt() :
			Engine::audioEngine()->outputSampleRate();
		setSampleBuffer(SampleBuffer::fromBase64(_this.attribute("data"), sampleRate));
	}

	// The recording's original and its timing
	const auto loadTiming = [](const QDomElement& element) {
		auto timing = Timing{};
		for (auto point = element.firstChildElement("point"); !point.isNull(); point = point.nextSiblingElement("point"))
		{
			const auto frame = point.attribute("frame").toDouble();
			timing.push_back({std::clamp(point.attribute("tick").toInt(), 0, TimePos::MaxTicks),
				std::isfinite(frame) ? std::max(0.0, frame) : 0.0});
		}
		return timing;
	};
	const auto original = _this.firstChildElement("original");
	m_originalTiming = original.isNull() ? Timing{} : loadTiming(original);
	m_originalFile = original.attribute("src");
	m_renderedTiming = loadTiming(_this.firstChildElement("rendered"));
	if (m_originalTiming.size() < 2) { m_originalTiming.clear(); m_renderedTiming.clear(); }

	// A rendering that is gone: the original plays (and is rendered again when asked)
	if (isTempoBound() && sampleFile().isEmpty() && QFileInfo::exists(PathUtil::toAbsolute(m_originalFile)))
	{
		m_renderedTiming.clear();
		setSampleFile(m_originalFile);
	}

	// The takes, if any: the active one was loaded above as "src"
	m_takes.clear();
	m_activeTake = -1;
	for (auto element = _this.firstChildElement("take"); !element.isNull(); element = element.nextSiblingElement("take"))
	{
		m_takes << element.attribute("src");
	}
	if (!m_takes.isEmpty())
	{
		m_activeTake = std::clamp(_this.attribute("activetake").toInt(), 0, static_cast<int>(m_takes.size()) - 1);
	}

	changeLength( _this.attribute( "len" ).toInt() );
	setMuted( _this.attribute( "muted" ).toInt() );
	setStartTimeOffset( _this.attribute( "off" ).toInt() );
	setAutoResize(_this.attribute("autoresize", "1").toInt());
	setStartCrossfadeLength(_this.attribute("startfadelength").toInt());
	setEndCrossfadeLength(_this.attribute("endfadelength").toInt());
	setStartCrossfadeTension(_this.attribute("startfadetension", QString::number(1.0f - std::numbers::sqrt2 / 2)).toFloat());
	setEndCrossfadeTension(_this.attribute("endfadetension", QString::number(1.0f - std::numbers::sqrt2 / 2)).toFloat());

	if (_this.hasAttribute("color"))
	{
		setColor(QColor{_this.attribute("color")});
	}

	if(_this.hasAttribute("reversed"))
	{
		m_sample.setReversed(true);
		emit wasReversed(); // tell SampleClipView to update the view
	}

	// Loaded or restored by undo: its rendering may not be the one the tempo now asks for
	if (isTempoBound()) { Engine::getSong()->requestStretchCheck(); }
}




gui::ClipView * SampleClip::createView( gui::TrackView * _tv )
{
	return new gui::SampleClipView( this, _tv );
}

void SampleClip::exportSampleBuffer(const QString& fileName)
{
	// (a clip streamed from disk has no buffer: its file is read for it)
	auto buffer = m_sample.isStreamed() ? SampleBuffer::fromFile(sampleFile()) : m_sample.shareBuffer();
	if (buffer) { Engine::exportManager()->startExporting(fileName, std::move(buffer)); }
}


} // namespace lmms
