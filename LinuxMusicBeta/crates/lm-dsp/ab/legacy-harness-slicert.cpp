// The legacy program's SlicerT for the A/B (see legacy-harness.cpp): its sample (ab/data, found by LM_AB_DIR) loaded
// by the plugin itself, its slice points the case's, its notes played by its playNote() at the song's tempo
#include <cstdint>
#include <cstdlib>
#include <memory>
#include <span>
#include <vector>

#include <QJsonArray>
#include <QJsonObject>

#include "AutomatableModel.h"
#include "Engine.h"
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "Note.h"
#include "NotePlayHandle.h"
#include "SampleFrame.h"
#include "Song.h"
#include "Track.h"
#include "SlicerT.h"
#include "legacy-harness-note.h"

using namespace lmms;

auto runSlicerT(const QJsonObject& c, std::int64_t frames, std::int64_t block) -> std::vector<SampleFrame>
{
	auto out = std::vector<SampleFrame>(static_cast<std::size_t>(frames));
	auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
	auto instrument = std::make_unique<SlicerT>(track);
	const auto s = c["settings"].toObject();
	instrument->loadFile(QString{std::getenv("LM_AB_DIR")} + "/" + s["sample"].toString());
	for (auto* model : instrument->findChildren<AutomatableModel*>())
	{
		if (s.contains(model->displayName())) { model->setValue(static_cast<float>(s[model->displayName()].toDouble())); }
	}
	auto points = std::vector<float>{};
	for (const auto& p : s["points"].toArray()) { points.push_back(static_cast<float>(p.toDouble())); }
	instrument->setSlicePoints(points);
	const auto key = track->baseNote() + s["index"].toInt();
	auto* note = NotePlayHandleManager::acquire(track, 0, static_cast<f_cnt_t>(frames * 2),
		Note{TimePos{0}, TimePos{0}, key});
	for (std::int64_t start = 0; start < frames; start += block)
	{
		const auto n = std::min(block, frames - start);
		static_cast<Instrument*>(instrument.get())->playNote(note,
			std::span<SampleFrame>{out.data() + start, static_cast<std::size_t>(n)});
		(note->*get(harness::NoteFramesPlayed{})) += static_cast<f_cnt_t>(n);
	}
	instrument->deleteNotePluginData(note);
	NotePlayHandleManager::release(note);
	return out;
}

auto runSlicerTAnalysis(const QJsonObject& c, std::int64_t frames) -> std::vector<SampleFrame>
{
	// The SlicerT's analysis of its sample: loading it finds its tempo and slices; with knobs, the slices again.
	// Out: its tempo and the count of slices, then the slice points
	auto out = std::vector<SampleFrame>(static_cast<std::size_t>(frames));
	auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
	auto instrument = std::make_unique<SlicerT>(track);
	const auto s = c["settings"].toObject();
	instrument->loadFile(QString{std::getenv("LM_AB_DIR")} + "/" + s["sample"].toString());
	auto bpm = 0.f;
	auto reslice = false;
	for (auto* model : instrument->findChildren<AutomatableModel*>())
	{
		if (model->displayName() == "Original bpm") { bpm = model->value<float>(); }
		if (s.contains(model->displayName()))
		{
			model->setValue(static_cast<float>(s[model->displayName()].toDouble()));
			reslice = true;
		}
	}
	if (reslice) { instrument->findSlices(); }
	const auto& points = instrument->slicePoints();
	out[0] = SampleFrame{bpm, static_cast<float>(points.size())};
	for (std::size_t i = 0; i < points.size() && i + 1 < out.size(); ++i) { out[i + 1] = SampleFrame{points[i], points[i]}; }
	return out;
}
