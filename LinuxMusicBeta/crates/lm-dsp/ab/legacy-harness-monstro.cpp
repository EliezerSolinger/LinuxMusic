// The legacy program's Monstro for the A/B (see legacy-harness.cpp): in a file of its own, as its header's constants
// clash with another plugin's
#include <atomic>
#include <cstdint>
#include <memory>
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
#include "lmms_math.h"
#include "Monstro.h"
#include "legacy-harness-note.h"

using namespace lmms;


auto runMonstro(const QJsonObject& c, std::int64_t frames, std::int64_t block) -> std::vector<SampleFrame>
{
	auto out = std::vector<SampleFrame>(static_cast<std::size_t>(frames));
	// The Monstro's note (MonstroSynth compiled in from the plugin) on a note of a track of the song: its knobs
	// set by their names, its frequency the case's, its frames played and its release moved as its play() would,
	// the legacy fastRand seeded
	auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, Engine::getSong()));
	auto instrument = std::make_unique<MonstroInstrument>(track);
	const auto s = c["settings"].toObject();
	for (auto* model : instrument->findChildren<AutomatableModel*>())
	{
		if (s.contains(model->displayName())) { model->setValue(static_cast<float>(s[model->displayName()].toDouble())); }
	}
	AutomatableModel::incrementPeriodCounter();
	for (auto* model : instrument->findChildren<AutomatableModel*>()) { model->valueBuffer(); }
	static_cast<Instrument*>(instrument.get())->preparePeriod();
	auto* note = NotePlayHandleManager::acquire(track, 0, static_cast<f_cnt_t>(frames), Note{});
	const_cast<float&>(note->frequency()) = static_cast<float>(c["freq"].toDouble());
	fastRandState() = static_cast<unsigned long>(c["seed"].toDouble());
	auto synth = MonstroSynth{instrument.get(), note};
	const auto release = static_cast<std::int64_t>(c["release"].toDouble());
	for (std::int64_t start = 0; start < frames; start += block)
	{
		const auto n = std::min(block, frames - start);
		if (release >= start && release < start + n)
		{
			(note->*get(harness::NoteReleased{})) = true;
			(note->*get(harness::NoteBeforeRelease{})) = static_cast<f_cnt_t>(release - start);
		}
		else if (release < start) { (note->*get(harness::NoteBeforeRelease{})) = 0; }
		synth.renderOutput(static_cast<f_cnt_t>(n), out.data() + start);
		(note->*get(harness::NoteFramesPlayed{})) += static_cast<f_cnt_t>(n);
	}
	NotePlayHandleManager::release(note);
	return out;
}
