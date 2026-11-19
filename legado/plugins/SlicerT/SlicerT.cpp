/*
 * SlicerT.cpp - simple slicer plugin
 *
 * Copyright (c) 2023 Daniel Kauss Serna <daniel.kauss.serna@gmail.com>
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

#include "SlicerT.h"

#include "AudioEngine.h"

#include <QDomElement>
#include <algorithm>
#include <cmath>
#include <complex>
#include <numbers>
#include <vector>
#include <fftw3.h>

#include "Engine.h"
#include "InstrumentTrack.h"
#include "PathUtil.h"
#include "SampleCache.h"
#include "SlicerTView.h"
#include "Song.h"
#include "embed.h"
#include "interpolation.h"
#include "plugin_export.h"

namespace lmms {

extern "C" {
Plugin::Descriptor PLUGIN_EXPORT slicert_plugin_descriptor = {
	LMMS_STRINGIFY(PLUGIN_NAME),
	"SlicerT",
	QT_TRANSLATE_NOOP("PluginBrowser", "Basic Slicer"),
	"Daniel Kauss Serna <daniel.kauss.serna@gmail.com>",
	0x0100,
	Plugin::Type::Instrument,
	new PluginPixmapLoader("logo"),
	nullptr,
	nullptr,
};
} // end extern

// ################################# SlicerT ####################################

SlicerT::SlicerT(InstrumentTrack* instrumentTrack)
	: LegacyInstrument(&slicert_plugin_descriptor, instrumentTrack)
	, m_noteThreshold(0.6f, 0.0f, 2.0f, 0.01f, this, tr("Note threshold"))
	, m_fadeOutFrames(10.0f, 0.0f, 100.0f, 0.1f, this, tr("FadeOut"))
	, m_originalBPM(1, 1, 999, this, tr("Original bpm"))
	, m_sliceSnap(this, tr("Slice snap"))
	, m_enableSync(false, this, tr("BPM sync"))
	, m_loopMode(this, tr("Loop"))
	, m_originalSample()
	, m_parentTrack(instrumentTrack)
{
	m_sliceSnap.addItem("Off");
	m_sliceSnap.addItem("1/1");
	m_sliceSnap.addItem("1/2");
	m_sliceSnap.addItem("1/4");
	m_sliceSnap.addItem("1/8");
	m_sliceSnap.addItem("1/16");
	m_sliceSnap.addItem("1/32");
	m_sliceSnap.setValue(0);

	// (in Sample::Loop's order)
	m_loopMode.addItem(tr("Off"));
	m_loopMode.addItem(tr("Loop"));
	m_loopMode.addItem(tr("Ping-pong"));
	m_loopMode.setValue(0);
}

namespace {
//! A note's: where it plays in the sample, and how far into its slice it went (in the sample's frames)
struct SliceNote
{
	SliceNote(int startFrame) : state(AudioResampler::Mode::Linear, startFrame) {}
	Sample::PlaybackState state;
	double played = 0;
};
} // namespace

void SlicerT::playNoteImpl(NotePlayHandle* handle, std::span<SampleFrame> out)
{
	if (m_originalSample.sampleSize() <= 1) { return; }

	int noteIndex = handle->key() - m_parentTrack->baseNote();
	const f_cnt_t frames = handle->framesLeftForCurrentPeriod();
	const f_cnt_t offset = handle->noteOffset();
	const auto bpm = Engine::getSong()->getTempo();
	const float pitchRatio = 1 / std::exp2(m_parentTrack->pitchModel()->value() / 1200);

	float speedRatio = static_cast<float>(m_originalBPM.value()) / bpm;
	if (!m_enableSync.value()) { speedRatio = 1; }
	speedRatio *= pitchRatio;

	float sliceStart, sliceEnd;
	if (noteIndex == 0) // full sample at base note
	{
		sliceStart = 0;
		sliceEnd = 1;
	}
	else if (noteIndex > 0 && static_cast<std::size_t>(noteIndex) + 1 < m_slicePoints.size())
	{
		noteIndex -= 1;
		sliceStart = m_slicePoints[noteIndex];
		sliceEnd = m_slicePoints[noteIndex + 1];
	}
	else
	{
		emit isPlaying(-1, 0, 0);
		return;
	}

	const auto size = static_cast<int>(m_originalSample.sampleSize());
	const auto startFrame = std::clamp(static_cast<int>(sliceStart * size), 0, size);
	const auto endFrame = std::clamp(static_cast<int>(sliceEnd * size), startFrame, size);
	if (!handle->m_pluginData) { handle->makePluginData<SliceNote>(startFrame); }
	auto* note = static_cast<SliceNote*>(handle->m_pluginData);

	const auto loop = static_cast<Sample::Loop>(m_loopMode.value());
	// Output frames per frame of the sample (the sample's rate counts too)
	const auto outputRatio = speedRatio * static_cast<double>(Engine::audioEngine()->outputSampleRate())
		/ m_originalSample.sampleRate();
	const auto bounds = Sample::Bounds{startFrame, endFrame, startFrame, endFrame};

	if (endFrame > startFrame
		&& (loop != Sample::Loop::Off || note->played < endFrame - startFrame)
		&& m_originalSample.play(out.data() + offset, &note->state, frames, bounds, loop, speedRatio))
	{
		const auto fadeOutFrames = m_fadeOutFrames.value() / 1000.0f * Engine::audioEngine()->outputSampleRate();
		if (loop == Sample::Loop::Off)
		{
			// Faded out at the slice's end (exponential); applyRelease() is not used since it extends the note
			const auto framesLeft = (endFrame - startFrame - note->played) * outputRatio;
			for (auto i = f_cnt_t{0}; i < frames; i++)
			{
				const auto left = static_cast<float>(framesLeft) - static_cast<float>(i);
				auto fadeValue = fadeOutFrames > 0 ? std::clamp(left / fadeOutFrames, 0.0f, 1.0f) : (left > 0 ? 1.f : 0.f);
				fadeValue = cosinusInterpolate(0, 1, fadeValue);
				out[i + offset][0] *= fadeValue;
				out[i + offset][1] *= fadeValue;
			}
		}
		else if (handle->isReleased())
		{
			// Looped, it goes on while held: faded out after the note ends (the fade out is its release)
			applyRelease(out.data(), handle);
		}
		note->played += frames / outputRatio;

		const auto currentNote = static_cast<float>(note->state.frameIndex()) / size;
		emit isPlaying(currentNote, sliceStart, sliceEnd);
	}
	else { emit isPlaying(-1, 0, 0); }
}

float SlicerT::desiredReleaseTimeMs() const
{
	// Looped slices go on while the note is held, and fade out after it
	return m_loopMode.value() != static_cast<int>(Sample::Loop::Off) ? m_fadeOutFrames.value() : 0.f;
}

void SlicerT::deleteNotePluginData(NotePlayHandle* handle)
{
	handle->destroyPluginData<SliceNote>();
	emit isPlaying(-1, 0, 0);
}

// uses the spectral flux to determine the change in magnitude
// resources:
// http://www.iro.umontreal.ca/~pift6080/H09/documents/papers/bello_onset_tutorial.pdf
void SlicerT::findSlices()
{
	if (m_originalSample.sampleSize() <= 1) { return; }
	auto points = std::vector<float>{};

	const int windowSize = 512;
	const float minBeatLength = 0.05f; // in seconds, ~ 1/4 length at 220 bpm

	int sampleRate = m_originalSample.sampleRate();
	int minDist = sampleRate * minBeatLength;

	std::vector<float> singleChannel(m_originalSample.sampleSize(), 0);
	for (auto i = std::size_t{0}; i < m_originalSample.sampleSize(); i++)
	{
		singleChannel[i] = (m_originalSample.data()[i][0] + m_originalSample.data()[i][1]) / 2;
	}

	// find 0 crossings (the flux below is a ratio: the level does not matter)
	std::vector<int> zeroCrossings;
	float lastValue = 1;
	for (auto i = std::size_t{0}; i < singleChannel.size(); i++)
	{
		if ((lastValue >= 0) != (singleChannel[i] >= 0))
		{
			zeroCrossings.push_back(i);
			lastValue = singleChannel[i];
		}
	}

	std::vector<float> prevMags(windowSize / 2, 0);
	std::vector<float> fftIn(windowSize, 0);
	std::array<fftwf_complex, windowSize> fftOut;

	fftwf_plan fftPlan = fftwf_plan_dft_r2c_1d(windowSize, fftIn.data(), fftOut.data(), FFTW_MEASURE);

	int lastPoint = -minDist - 1; // to always store 0 first
	float spectralFlux = 0;
	float prevFlux = 1E-10f; // small value, no divison by zero

	for (int i = 0; i < static_cast<int>(singleChannel.size()) - windowSize; i += windowSize)
	{
		// fft
		std::copy_n(singleChannel.data() + i, windowSize, fftIn.data());
		fftwf_execute(fftPlan);

		// calculate spectral flux in regard to last window
		for (int j = 0; j < windowSize / 2; j++) // only use niquistic frequencies
		{
			float real = fftOut[j][0];
			float imag = fftOut[j][1];
			float magnitude = std::sqrt(real * real + imag * imag);

			// using L2-norm (euclidean distance)
			float diff = std::abs(magnitude - prevMags[j]);
			spectralFlux += diff;

			prevMags[j] = magnitude;
		}

		if (spectralFlux / prevFlux > 1.0f + m_noteThreshold.value() && i - lastPoint > minDist)
		{
			points.push_back(i);
			lastPoint = i;
			if (points.size() > 128) { break; } // no more keys on the keyboard
		}

		prevFlux = spectralFlux;
		spectralFlux = 1E-10f; // again for no divison by zero
	}
	fftwf_destroy_plan(fftPlan);

	// The first slice starts at the start (a sample that starts quietly kept its first onset as a later slice: it
	// was overwritten by the start)
	if (points.empty() || points.front() != 0) { points.insert(points.begin(), 0); }
	points.push_back(m_originalSample.sampleSize());

	for (float& sliceValue : points)
	{
		auto closestZeroCrossing = std::lower_bound(zeroCrossings.begin(), zeroCrossings.end(), sliceValue);
		if (closestZeroCrossing == zeroCrossings.end()) { continue; }
		if (std::abs(sliceValue - *closestZeroCrossing) < windowSize) { sliceValue = *closestZeroCrossing; }
	}

	float beatsPerMin = m_originalBPM.value() / 60.0f;
	float samplesPerBeat = m_originalSample.sampleRate() / beatsPerMin * 4.0f;
	int noteSnap = m_sliceSnap.value();
	int sliceLock = std::max(1, static_cast<int>(samplesPerBeat / std::exp2(noteSnap + 1)));
	if (noteSnap == 0) { sliceLock = 1; }
	const auto lastFrame = static_cast<float>(m_originalSample.sampleSize());
	for (float& sliceValue : points)
	{
		sliceValue += sliceLock / 2.f;
		sliceValue -= static_cast<int>(sliceValue) % sliceLock;
		sliceValue = std::min(sliceValue, lastFrame);
	}

	points.erase(std::unique(points.begin(), points.end()), points.end());
	if (points.size() < 2) { points = {0, lastFrame}; }

	for (float& sliceIndex : points)
	{
		sliceIndex /= m_originalSample.sampleSize();
	}

	points[0] = 0;
	points[points.size() - 1] = 1;

	emit dataChanged();

	// Put where the audio thread reads them, at once
	setSlicePoints(std::move(points));
}

/*
 * The tempo, from the sample's onsets (upstream's #8102: it was only the sample's length taken as a bar, doubled or
 * halved into 100 to 200 BPM, wrong for anything but a loop of whole bars).
 * - Onset strength: the rise of the log spectrum from one hop to the next, summed over the bins, less its local
 *   mean (an onset stands out of what is around it).
 * - Its autocorrelation; the pulse is the tempo that scores best by the autocorrelation at one to four of its
 *   beats and halfway between them (a comb, interpolated between lags: precise to a fraction of a BPM), weighted
 *   towards 120 BPM, from 60 to 200 BPM; then, of its octaves, the one whose beats alone score best (weighted
 *   alike: doubling and halving score alike, so the likelier one wins).
 * - A loop cut at whole bars of that tempo gets the tempo that fits them exactly.
 * Without a beat to find (a single hit, a pad, a short sample) it is the length's estimate, as before.
 */
void SlicerT::findBPM()
{
	if (m_originalSample.sampleSize() <= 1) { return; }

	const auto sampleRate = static_cast<double>(m_originalSample.sampleRate());
	const auto totalFrames = static_cast<double>(m_originalSample.sampleSize());
	const auto sampleLength = totalFrames / sampleRate;

	const auto fromLength = [&] {
		auto estimate = 240.0 / sampleLength;
		while (estimate < 100) { estimate *= 2; }
		while (estimate > 200) { estimate /= 2; }
		return estimate;
	};

	auto bpm = estimateTempo();
	if (bpm <= 0) { bpm = fromLength(); }
	else
	{
		// A loop of whole bars (within half a percent): the tempo that fits them exactly
		const auto bars = sampleLength * bpm / 240.0;
		const auto wholeBars = std::round(bars);
		if (wholeBars >= 1 && std::abs(bars - wholeBars) < 0.005 * wholeBars) { bpm = 240.0 * wholeBars / sampleLength; }
	}

	const auto rounded = std::clamp(static_cast<int>(std::lround(bpm)), m_originalBPM.minValue(), m_originalBPM.maxValue());
	m_originalBPM.setValue(rounded);
	m_originalBPM.setInitValue(rounded);
}

auto SlicerT::estimateTempo() const -> double
{
	constexpr auto WindowSize = 1024;
	constexpr auto Hop = 256;
	constexpr auto LowestBpm = 60.0;
	constexpr auto HighestBpm = 200.0;
	constexpr auto MostLikelyBpm = 120.0;
	constexpr auto Beats = 4; //!< the comb's teeth

	const auto sampleRate = static_cast<double>(m_originalSample.sampleRate());
	// (the first minute: enough to find a beat)
	const auto frames = std::min<std::size_t>(m_originalSample.sampleSize(), static_cast<std::size_t>(60 * sampleRate));
	const auto hopRate = sampleRate / Hop;
	const auto longestLag = Beats * 60.0 / LowestBpm * hopRate;
	if (frames < WindowSize * 4) { return 0; }
	const auto hops = (frames - WindowSize) / Hop + 1;
	// Two beats of the slowest tempo at least
	if (hops < 2 * 60.0 / LowestBpm * hopRate) { return 0; }

	// Onset strength
	auto window = std::vector<float>(WindowSize);
	for (auto i = 0; i < WindowSize; ++i)
	{
		window[i] = 0.5f - 0.5f * std::cos(2 * std::numbers::pi_v<float> * i / WindowSize);
	}
	auto in = std::vector<float>(WindowSize);
	// (std::complex<float> is laid out as fftwf_complex: FFTW's manual, "Complex numbers")
	auto out = std::vector<std::complex<float>>(WindowSize / 2 + 1);
	const auto plan = fftwf_plan_dft_r2c_1d(WindowSize, in.data(), reinterpret_cast<fftwf_complex*>(out.data()),
		FFTW_ESTIMATE);
	auto previous = std::vector<float>(WindowSize / 2 + 1, 0.f);
	auto onsets = std::vector<double>(hops, 0.0);
	const auto* data = m_originalSample.data();
	for (auto hop = std::size_t{0}; hop < hops; ++hop)
	{
		for (auto i = 0; i < WindowSize; ++i)
		{
			const auto& frame = data[hop * Hop + i];
			in[i] = (frame[0] + frame[1]) * 0.5f * window[i];
		}
		fftwf_execute(plan);
		auto rise = 0.0;
		for (auto bin = 0; bin < WindowSize / 2 + 1; ++bin)
		{
			const auto magnitude = std::log1p(100.f * std::abs(out[bin]));
			if (hop > 0) { rise += std::max(0.f, magnitude - previous[bin]); }
			previous[bin] = magnitude;
		}
		onsets[hop] = rise;
	}
	fftwf_destroy_plan(plan);

	// Less the local mean (over about a third of a second each way), kept above zero
	const auto reach = static_cast<std::size_t>(hopRate / 3);
	auto sums = std::vector<double>(hops + 1, 0.0);
	for (auto i = std::size_t{0}; i < hops; ++i) { sums[i + 1] = sums[i] + onsets[i]; }
	auto strength = std::vector<double>(hops);
	for (auto i = std::size_t{0}; i < hops; ++i)
	{
		const auto from = i > reach ? i - reach : 0;
		const auto to = std::min(hops, i + reach + 1);
		strength[i] = std::max(0.0, onsets[i] - (sums[to] - sums[from]) / (to - from));
	}

	// Onsets that stand out (a held tone's strength is a ripple, under its mean rise; drums are tens of times it)
	{
		auto sorted = strength;
		const auto top = sorted.begin() + static_cast<std::ptrdiff_t>(hops * 99 / 100);
		std::nth_element(sorted.begin(), top, sorted.end());
		const auto meanRise = sums[hops] / static_cast<double>(hops);
		if (*top < 4 * meanRise) { return 0; }
	}

	// Autocorrelation, normalized by the overlap
	const auto lags = std::min(hops - 1, static_cast<std::size_t>(std::ceil(longestLag)) + 2);
	auto correlation = std::vector<double>(lags + 1, 0.0);
	for (auto lag = std::size_t{0}; lag <= lags; ++lag)
	{
		auto sum = 0.0;
		for (auto i = std::size_t{0}; i + lag < hops; ++i) { sum += strength[i] * strength[i + lag]; }
		correlation[lag] = sum / static_cast<double>(hops - lag);
	}
	const auto energy = correlation[0];
	if (energy <= 0) { return 0; }
	for (auto& value : correlation) { value /= energy; }
	const auto at = [&](double lag) {
		const auto below = static_cast<std::size_t>(lag);
		if (below + 1 > lags) { return 0.0; }
		const auto t = lag - static_cast<double>(below);
		return correlation[below] * (1 - t) + correlation[below + 1] * t;
	};

	const auto prior = [&](double bpm) {
		const auto octaves = std::log2(bpm / MostLikelyBpm);
		return std::exp(-0.5 * octaves * octaves);
	};
	//! The mean autocorrelation at one to four beats of @p bpm (and halfway between them, @p between)
	const auto comb = [&](double bpm, bool between) {
		const auto beat = 60.0 / bpm * hopRate;
		auto sum = 0.0;
		auto teeth = 0;
		for (auto k = 1; k <= Beats && k * beat <= lags - 1; ++k)
		{
			sum += at(k * beat);
			++teeth;
			if (between) { sum += at((k - 0.5) * beat); ++teeth; }
		}
		return teeth > 0 ? sum / teeth : 0.0;
	};

	// The pulse: on the beats and between them (a tempo at two thirds of the right one finds eighths at its
	// beats, but nothing between them)
	auto pulse = 0.0;
	auto pulseScore = 0.0;
	for (auto bpm = LowestBpm; bpm <= HighestBpm; bpm += 0.05)
	{
		const auto score = comb(bpm, true) * prior(bpm);
		if (score > pulseScore) { pulseScore = score; pulse = bpm; }
	}
	if (pulse <= 0) { return 0; }

	// Its octave: by the beats alone (a kick on every beat and nothing between scores at half the tempo above)
	while (pulse / 2 >= LowestBpm) { pulse /= 2; }
	auto best = 0.0;
	auto bestScore = 0.0;
	auto bestComb = 0.0;
	for (auto bpm = pulse; bpm <= HighestBpm; bpm *= 2)
	{
		const auto beats = comb(bpm, false);
		if (beats * prior(bpm) > bestScore) { bestScore = beats * prior(bpm); best = bpm; bestComb = beats; }
	}
	// A beat stands out of the rest (noise correlates little at any lag; drums, at the beats, most of their energy)
	return bestComb > 0.3 ? best : 0;
}

std::vector<Note> SlicerT::getMidi()
{
	std::vector<Note> outputNotes;

	float speedRatio = static_cast<float>(m_originalBPM.value()) / Engine::getSong()->getTempo();
	float outFrames = m_originalSample.sampleSize() * speedRatio;

	float framesPerTick = Engine::framesPerTick();
	float totalTicks = outFrames / framesPerTick;
	float lastEnd = 0;

	for (auto i = std::size_t{0}; i < m_slicePoints.size() - 1; i++)
	{
		float sliceStart = lastEnd;
		float sliceEnd = totalTicks * m_slicePoints[i + 1];

		Note sliceNote = Note();
		sliceNote.setKey(i + m_parentTrack->baseNote() + 1);
		sliceNote.setPos(sliceStart);
		sliceNote.setLength(sliceEnd - sliceStart + 1); // + 1 so that the notes allign
		outputNotes.push_back(sliceNote);

		lastEnd = sliceEnd;
	}

	return outputNotes;
}

void SlicerT::updateFile(QString file)
{
	const auto path = PathUtil::stringToPath(file);

	// TODO: Return an error value back to the caller (via std::expected or our own polyfill)
	auto buffer = SampleCache::instance().fetchBuffer(path);
	if (!buffer) { return; }

	// TODO: Return an error value back to the caller (via std::expected or our own polyfill)
	auto thumbnailCache = SampleCache::instance().fetchThumbnailCache(path);
	if (!thumbnailCache) { return; }

	setSample(Sample{std::move(buffer)});
	m_sampleThumbnailCache = std::move(thumbnailCache);

	findBPM();
	findSlices();

	emit dataChanged();
}

void SlicerT::setSample(Sample sample)
{
	{
		// The audio thread plays it (playNoteImpl())
		const auto guard = AudioEngine::changesGuard();
		std::swap(m_originalSample, sample);
	}
	// (the old one goes here, without the lock)
}




void SlicerT::setSlicePoints(std::vector<float> points)
{
	{
		// The audio thread reads them (playNoteImpl()): they change at once, never one by one under it
		const auto guard = AudioEngine::changesGuard();
		m_slicePoints.swap(points);
	}
	emit dataChanged();
}




void SlicerT::loadFile(const QString& file)
{
	updateFile(file);
}

void SlicerT::updateSlices()
{
	findSlices();
}

void SlicerT::saveSettings(QDomDocument& document, QDomElement& element)
{
	element.setAttribute("version", "1");
	element.setAttribute("src", m_originalSample.sampleFile());
	if (m_originalSample.sampleFile().isEmpty())
	{
		element.setAttribute("sampledata", m_originalSample.toBase64());
	}

	element.setAttribute("totalSlices", static_cast<int>(m_slicePoints.size()));
	for (auto i = std::size_t{0}; i < m_slicePoints.size(); i++)
	{
		element.setAttribute(QString("slice_%1").arg(i), m_slicePoints[i]);
	}

	m_fadeOutFrames.saveSettings(document, element, "fadeOut");
	m_noteThreshold.saveSettings(document, element, "threshold");
	m_originalBPM.saveSettings(document, element, "origBPM");
	m_enableSync.saveSettings(document, element, "syncEnable");
	m_sliceSnap.saveSettings(document, element, "snap");
	m_loopMode.saveSettings(document, element, "loopMode");
}

void SlicerT::loadSettings(const QDomElement& element)
{
	if (auto srcFile = element.attribute("src"); !srcFile.isEmpty())
	{
		if (QFileInfo(PathUtil::toAbsolute(srcFile)).exists())
		{
			const auto path = PathUtil::stringToPath(srcFile);
			if (auto buffer = SampleCache::instance().fetchBuffer(path))
			{
				setSample(Sample{std::move(buffer)});
				// The waveform's (the view drew nothing for a sample loaded with a project)
				m_sampleThumbnailCache = SampleCache::instance().fetchThumbnailCache(path);
			}
			else
			{
				Engine::getSong()->collectError(tr("Failed to load sample: %1").arg(srcFile));
			}
		}
		else
		{
			QString message = tr("Sample not found: %1").arg(srcFile);
			Engine::getSong()->collectError(message);
		}
	}
	else if (auto sampleData = element.attribute("sampledata"); !sampleData.isEmpty())
	{
		auto buffer = SampleBuffer::fromBase64(sampleData);
		setSample(Sample{std::move(buffer)});
		m_sampleThumbnailCache = std::make_shared<const SampleThumbnailCache>(m_originalSample.buffer());
	}

	if (!element.attribute("totalSlices").isEmpty())
	{
		int totalSlices = element.attribute("totalSlices").toInt();
		auto points = std::vector<float>{};
		for (int i = 0; i < totalSlices; i++)
		{
			points.push_back(element.attribute(QString("slice_%1").arg(i)).toFloat());
		}
		setSlicePoints(std::move(points));
	}

	m_fadeOutFrames.loadSettings(element, "fadeOut");
	m_noteThreshold.loadSettings(element, "threshold");
	m_originalBPM.loadSettings(element, "origBPM");
	m_enableSync.loadSettings(element, "syncEnable");
	// (older projects have neither: no snap, slices played once, the models' first values)
	m_sliceSnap.loadSettings(element, "snap");
	m_loopMode.loadSettings(element, "loopMode");

	emit dataChanged();
}

QString SlicerT::nodeName() const
{
	return slicert_plugin_descriptor.name;
}

gui::PluginView* SlicerT::instantiateView(QWidget* parent)
{
	return new gui::SlicerTView(this, parent);
}

extern "C" {
PLUGIN_EXPORT Plugin* lmms_plugin_main(Model* m, void*)
{
	return new SlicerT(static_cast<InstrumentTrack*>(m));
}
} // extern
} // namespace lmms
