// The legacy program's PatMan for the A/B (see legacy-harness.cpp): its patches (ab/data, found by LM_AB_DIR) loaded
// by the plugin itself, its notes played by its playNote()
#include <cstdint>
#include <cstdlib>
#include <memory>
#include <span>
#include <vector>

#include <QJsonObject>

#include "AutomatableModel.h"
#include "Engine.h"
#include "Instrument.h"
#include "InstrumentTrack.h"
#include "NotePlayHandle.h"
#include "SampleFrame.h"
#include "Song.h"
#include "Track.h"
#include "Patman.h"
#include "legacy-harness-note.h"

using namespace lmms;

auto runPatman(const QJsonObject& c, std::int64_t frames, std::int64_t block) -> std::vector<SampleFrame>
{
	auto out = std::vector<SampleFrame>(static_cast<std::size_t>(frames));
	auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
	auto instrument = std::make_unique<PatmanInstrument>(track);
	const auto s = c["settings"].toObject();
	// (its two switches have no names: looped, then tuned)
	const auto switches = instrument->findChildren<BoolModel*>();
	switches[0]->setValue(s["looped"].toBool());
	switches[1]->setValue(s["tuned"].toBool());
	instrument->loadFile(QString{std::getenv("LM_AB_DIR")} + "/" + s["patch"].toString());
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
