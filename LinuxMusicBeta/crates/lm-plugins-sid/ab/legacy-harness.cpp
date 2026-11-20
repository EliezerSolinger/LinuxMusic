/*
 * legacy-harness.cpp - plays the A/B cases of the chip instruments (Sid, FreeBoy, OpulenZ: an ab/cases.json each) on
 * the legacy plugins themselves and writes the references
 *
 * Built by build-references.sh against the legacy build (build/, configured from legado/): the legacy program's core
 * linked as its MixerTest is, the plugins loaded from the build's plugin directory as the program loads them.
 *
 * A case: {"name", "plugin", "frames", "settings": {saved attribute: value}, "notes": [{"key", "start", "length",
 * "volume"}], "then": {"at", "settings"}}. The instrument is loaded into an instrument track and its settings set
 * through its own saveState()/restoreState() (the keys are the attributes the plugin saves). A per-note instrument plays each
 * note through a NotePlayHandle as the engine plays it (NotePlayHandle::play: the plugin's playNoteImpl, then the
 * track's sound shaping and the note's volume), each note into a buffer of its own, summed. A single-streamed one
 * (OpulenZ) gets the notes' MIDI events from the same NotePlayHandles and renders the period through an
 * InstrumentPlayHandle, as the engine does.
 *
 * This file is part of LinuxMusic - GPL-2.0-or-later.
 */

#include <algorithm>
#include <array>
#include <cstdint>
#include <cstdio>
#include <memory>
#include <vector>

#include <QCoreApplication>
#include <QDomDocument>
#include <QDomElement>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>

#include "AudioEngine.h"
#include "AutomatableModel.h"
#include "ConfigManager.h"
#include "Engine.h"
#include "Instrument.h"
#include "InstrumentPlayHandle.h"
#include "InstrumentTrack.h"
#include "MidiEvent.h"
#include "Note.h"
#include "NotePlayHandle.h"
#include "SampleFrame.h"
#include "Song.h"
#include "Track.h"

using namespace lmms;

namespace
{

//! Sets @p settings (attribute → value) through the plugin's own saveSettings/loadSettings (saveState, restoreState)
void apply(Instrument* instrument, const QJsonObject& settings)
{
	auto doc = QDomDocument{};
	auto root = doc.createElement("harness");
	doc.appendChild(root);
	auto element = instrument->saveState(doc, root);
	for (auto it = settings.begin(); it != settings.end(); ++it)
	{
		if (it.value().isString()) { element.setAttribute(it.key(), it.value().toString()); }
		else { element.setAttribute(it.key(), QString::number(it.value().toDouble(), 'g', 17)); }
	}
	instrument->restoreState(element);
}

struct Playing
{
	NotePlayHandle* handle;
	std::vector<SampleFrame> buffer;
};

auto run(const QJsonObject& c) -> std::vector<SampleFrame>
{
	const auto period = static_cast<std::int64_t>(Engine::audioEngine()->framesPerPeriod());
	const auto frames = static_cast<std::int64_t>(c["frames"].toDouble());
	auto out = std::vector<SampleFrame>(static_cast<std::size_t>(frames));

	auto* song = Engine::getSong();
	auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
	if (!track) { std::fprintf(stderr, "no instrument track\n"); std::exit(1); }
	auto* instrument = track->loadInstrument(c["plugin"].toString());
	if (!instrument) { std::fprintf(stderr, "no plugin %s\n", qPrintable(c["plugin"].toString())); std::exit(1); }
	apply(instrument, c["settings"].toObject());
	const auto then = c["then"].toObject();
	const auto changeAt = then.isEmpty() ? -1 : static_cast<std::int64_t>(then["at"].toDouble());

	auto notes = c["notes"].toArray();
	auto playing = std::vector<Playing>{};
	// A single-streamed instrument renders the period after its notes have sent their events, as the engine has it
	auto stream = instrument->isSingleStreamed() ? std::make_unique<InstrumentPlayHandle>(instrument, track) : nullptr;
	auto streamed = std::vector<SampleFrame>(static_cast<std::size_t>(period));
	for (std::int64_t start = 0; start < frames; start += period)
	{
		// A period starts, as the engine counts them: the models' period (their changes are seen once, in it) and the
		// instrument's preparePeriod() (where OpulenZ takes its knobs' changes to the chip)
		AutomatableModel::incrementPeriodCounter();
		if (start == changeAt) { apply(instrument, then["settings"].toObject()); }
		for (auto* model : instrument->findChildren<AutomatableModel*>()) { model->valueBuffer(); }
		instrument->prepareFor(start / period);
		// The notes starting in this period, at their offset in it
		for (const auto& value : notes)
		{
			const auto n = value.toObject();
			const auto at = static_cast<std::int64_t>(n["start"].toDouble());
			if (at < start || at >= start + period) { continue; }
			const auto length = static_cast<f_cnt_t>(n["length"].toDouble());
			const auto volume = static_cast<volume_t>(n.contains("volume") ? n["volume"].toDouble() : 100.0);
			auto note = Note{TimePos{0}, TimePos{0}, n["key"].toInt(), volume};
			auto* handle = NotePlayHandleManager::acquire(track, static_cast<f_cnt_t>(at - start), length, note);
			playing.push_back(Playing{handle, std::vector<SampleFrame>(static_cast<std::size_t>(period))});
		}
		const auto count = std::min(period, frames - start);
		for (auto& p : playing)
		{
			std::fill(p.buffer.begin(), p.buffer.end(), SampleFrame{});
			p.handle->play(std::span<SampleFrame>{p.buffer.data(), p.buffer.size()});
			for (std::int64_t f = 0; f < count; ++f) { out[start + f] += p.buffer[f]; }
		}
		if (stream)
		{
			std::fill(streamed.begin(), streamed.end(), SampleFrame{});
			stream->play(std::span<SampleFrame>{streamed.data(), streamed.size()});
			for (std::int64_t f = 0; f < count; ++f) { out[start + f] += streamed[f]; }
		}
		std::erase_if(playing, [](Playing& p) {
			if (!p.handle->isFinished()) { return false; }
			NotePlayHandleManager::release(p.handle);
			return true;
		});
	}
	for (auto& p : playing) { NotePlayHandleManager::release(p.handle); }
	// (the track stays in the song: the harness exits after the cases)
	return out;
}

} // namespace

auto main(int argc, char** argv) -> int
{
	if (argc != 3)
	{
		std::fprintf(stderr, "usage: legacy-harness <cases.json> <reference directory>\n");
		return 2;
	}
	auto app = QCoreApplication{argc, argv};
	// (the test environment's configuration, isolated: the rate and the period of the cases)
	ConfigManager::inst()->setValue("audioengine", "samplerate", "48000");
	ConfigManager::inst()->setValue("audioengine", "framesperaudiobuffer", "256");
	Engine::init(true);
	Engine::audioEngine()->stopProcessing();
	if (Engine::audioEngine()->outputSampleRate() != 48000 || Engine::audioEngine()->framesPerPeriod() != 256)
	{
		std::fprintf(stderr, "the engine runs at %d Hz, %d frames a period (the cases want 48000 and 256)\n",
			static_cast<int>(Engine::audioEngine()->outputSampleRate()),
			static_cast<int>(Engine::audioEngine()->framesPerPeriod()));
		return 1;
	}

	auto file = QFile{argv[1]};
	if (!file.open(QIODevice::ReadOnly)) { std::fprintf(stderr, "cannot read %s\n", argv[1]); return 1; }
	const auto cases = QJsonDocument::fromJson(file.readAll()).object()["cases"].toArray();
	for (const auto& value : cases)
	{
		const auto c = value.toObject();
		const auto name = c["name"].toString();
		const auto out = run(c);
		auto reference = QFile{QString{"%1/%2.f32"}.arg(argv[2], name)};
		if (!reference.open(QIODevice::WriteOnly)) { std::fprintf(stderr, "cannot write %s\n", qPrintable(name)); return 1; }
		for (const auto& frame : out)
		{
			const auto pair = std::array{frame[0], frame[1]};
			reference.write(reinterpret_cast<const char*>(pair.data()), sizeof(pair));
		}
		std::printf("%s\n", qPrintable(name));
	}
	Engine::destroy();
	return 0;
}
