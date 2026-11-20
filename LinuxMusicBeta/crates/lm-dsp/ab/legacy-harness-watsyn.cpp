// The legacy program's Watsyn for the A/B (see legacy-harness.cpp): in a file of its own, as its header's constants
// clash with another plugin's
#include <cstdint>
#include <memory>
#include <span>
#include <vector>

#include <QJsonArray>
#include <QJsonObject>

#include "AutomatableModel.h"
#include "Engine.h"
#include "Graph.h"
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "NotePlayHandle.h"
#include "SampleFrame.h"
#include "Song.h"
#include "Track.h"
#include "Watsyn.h"
#include "legacy-harness-note.h"

using namespace lmms;

auto runWatsyn(const QJsonObject& c, std::int64_t frames, std::int64_t block) -> std::vector<SampleFrame>
{
	// The Watsyn's notes as the instrument plays them (playNote(): the note's waves, then the A-B mix and its
	// envelope), its knobs and graphs set, on a note of a track of the song whose frames played move as its play()
	// would (the note longer than the case: no release)
	auto out = std::vector<SampleFrame>(static_cast<std::size_t>(frames));
	auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
	auto instrument = std::make_unique<WatsynInstrument>(track);
	const auto s = c["settings"].toObject();
	for (auto* model : instrument->findChildren<AutomatableModel*>())
	{
		if (s.contains(model->displayName())) { model->setValue(static_cast<float>(s[model->displayName()].toDouble())); }
	}
	const auto graphs = instrument->findChildren<graphModel*>();
	const auto shapes = c["graphs"].toArray();
	for (qsizetype g = 0; g < std::min(shapes.size(), graphs.size()); ++g)
	{
		auto values = std::vector<float>{};
		for (const auto& v : shapes[g].toArray()) { values.push_back(static_cast<float>(v.toDouble())); }
		graphs[g]->setSamples(std::span<const float>{values});
	}
	AutomatableModel::incrementPeriodCounter();
	for (auto* model : instrument->findChildren<AutomatableModel*>()) { model->valueBuffer(); }
	static_cast<Instrument*>(instrument.get())->preparePeriod();
	auto* note = NotePlayHandleManager::acquire(track, 0, static_cast<f_cnt_t>(frames * 2), Note{});
	const_cast<float&>(note->frequency()) = static_cast<float>(c["freq"].toDouble());
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
