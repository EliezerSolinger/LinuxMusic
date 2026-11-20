// The A/B references of lm-plugins-fluidsynth (interfaces.md, U34): the legacy AvlDrums and Sf2 Player plugins,
// loaded from the legacy build as the program loads them (an instrument track's instrument) and driven as the
// program drives them, writing what they render.
//
// Each case of cases.json: the plugin, its settings, the rate, the period, the frames and the events (frame, kind,
// data). Writes reference/<case>.f32: every output channel, interleaved, f32 little-endian.

#include <algorithm>
#include <array>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <vector>

#include <QCoreApplication>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>

#include "AudioBuffer.h"
#include "SampleFrame.h"
#include "AudioEngine.h"
#include "AutomatableModel.h"
#include "Engine.h"
#include "Instrument.h"
#include "Mixer.h"
#include "InstrumentTrack.h"
#include "MidiPort.h"
#include "Note.h"
#include "NotePlayHandle.h"
#include "PluginEvent.h"
#include "Song.h"
#include "Track.h"

using namespace lmms;

namespace {

//! The plugin's models by their display name (the legacy's "Drum kit", "Output per drum piece"...)
void set(Instrument* instrument, const QJsonObject& settings)
{
	for (auto* model : instrument->findChildren<AutomatableModel*>())
	{
		const auto key = model->displayName();
		if (settings.contains(key)) { model->setValue(static_cast<float>(settings[key].toDouble())); }
	}
	// (the plugins load a kit or apply a setting in queued slots)
	QCoreApplication::processEvents();
	QCoreApplication::processEvents();
}

auto run(const QJsonObject& c) -> std::vector<float>
{
	auto* song = Engine::getSong();
	auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
	if (!track) { std::fprintf(stderr, "no track\n"); std::exit(1); }
	auto* instrument = track->loadInstrument(c["plugin"].toString());
	if (!instrument) { std::fprintf(stderr, "cannot load %s\n", qPrintable(c["plugin"].toString())); std::exit(1); }
	// The plugins make their synthesizer anew when the mixer discards what played (a new project, an export): that
	// comes from the song's timers here, at any moment, so it is cut off: each case is one steady performance
	QObject::disconnect(Engine::mixer(), &Mixer::processingStateDiscarding, instrument, nullptr);
	const auto rate = c["rate"].toDouble();
	const auto block = static_cast<f_cnt_t>(c["block"].toInt());
	const auto frames = static_cast<std::int64_t>(c["frames"].toDouble());
	instrument->activate(rate, block);
	QCoreApplication::processEvents();
	if (c.contains("file"))
	{
		// (as a new instrument opening a file: its first preset)
		// (relative to the tree that has legado/, HARNESS_ROOT)
		const auto file = QString::fromLocal8Bit(std::getenv("HARNESS_ROOT")) + "/" + c["file"].toString();
		if (!QFile::exists(file)) { std::fprintf(stderr, "no file %s\n", qPrintable(file)); std::exit(1); }
		instrument->loadFile(file);
		QCoreApplication::processEvents();
	}
	set(instrument, c["settings"].toObject());
	// A new output layout makes the host activate the plugin again at the engine's rate: back to the case's
	instrument->activate(rate, block);
	QCoreApplication::processEvents();
	const auto channels = static_cast<ch_cnt_t>(instrument->channelCount(false));
	const auto groups = instrument->channelGroupCount(false);
	// (a source buffer per channel: with the default dynamic source count, channels may share one)
	auto buffer = AudioBuffer{block, channels, channels};
	buffer.setGroups(groups, [&](group_cnt_t g, AudioBuffer::ChannelGroup& group) {
		return static_cast<ch_cnt_t>(instrument->getChannelGroup(g, false, group));
	});
	if (std::getenv("HARNESS_DEBUG"))
	{
		for (ch_cnt_t ch = 0; ch < channels; ++ch) { std::fprintf(stderr, "channel %d at %p\n", ch, (void*)buffer.buffer(ch).data()); }
	}
	auto events = std::vector<PluginEvent>{};
	for (const auto& e : c["events"].toArray())
	{
		const auto a = e.toArray();
		const auto at = static_cast<f_cnt_t>(a[0].toDouble());
		const auto kind = a[1].toString();
		if (kind == "on") { events.push_back(PluginEvent::noteOn(at, 0, a[2].toInt(), a[3].toDouble())); }
		else if (kind == "off") { events.push_back(PluginEvent::noteOff(at, 0, a[2].toInt())); }
		else if (kind == "cc")
		{
			events.push_back(PluginEvent::midiMessage(at, 0xB0, static_cast<std::uint8_t>(a[2].toInt()),
				static_cast<std::uint8_t>(a[3].toInt())));
		}
	}
	// Whatever the plugins left queued (a kit loaded in a slot) happens now, before the first period
	for (int i = 0; i < 4; ++i) { QCoreApplication::processEvents(); }
	if (std::getenv("HARNESS_DEBUG")) { std::fprintf(stderr, "rendering\n"); }
	auto out = std::vector<float>(static_cast<std::size_t>(frames) * channels);
	if (c["notes"].toBool())
	{
		// A legacy instrument of note play handles (the Sf2 Player): each note a NotePlayHandle of its length, played
		// every period before the instrument renders, as the engine plays them (velocity 1: the base velocity is the
		// highest, so a volume of 100 % is MIDI 127)
		track->midiPort()->setBaseVelocity(127);
		// (these instruments play at the engine's rate, whatever activate() says)
		if (Engine::audioEngine()->outputSampleRate() != static_cast<sample_rate_t>(rate)) { std::fprintf(stderr, "the engine runs at %d\n", (int)Engine::audioEngine()->outputSampleRate()); std::exit(1); }
		if (Engine::audioEngine()->framesPerPeriod() != block) { std::fprintf(stderr, "period %d\n", (int)Engine::audioEngine()->framesPerPeriod()); std::exit(1); }
		struct Pending { std::int64_t at; std::int64_t length; int key; };
		auto pending = std::vector<Pending>{};
		const auto list = c["events"].toArray();
		// (each note-off ends the earliest note of its key still held)
		auto used = std::vector<bool>(static_cast<std::size_t>(list.size()), false);
		for (qsizetype i = 0; i < list.size(); ++i)
		{
			const auto a = list[i].toArray();
			if (a[1].toString() != "on") { continue; }
			const auto at = static_cast<std::int64_t>(a[0].toDouble());
			const auto key = a[2].toInt();
			auto length = frames - at;
			for (qsizetype j = i + 1; j < list.size(); ++j)
			{
				const auto b = list[j].toArray();
				if (!used[static_cast<std::size_t>(j)] && b[1].toString() == "off" && b[2].toInt() == key)
				{
					used[static_cast<std::size_t>(j)] = true;
					length = static_cast<std::int64_t>(b[0].toDouble()) - at;
					break;
				}
			}
			pending.push_back({at, length, key});
		}
		auto playing = std::vector<NotePlayHandle*>{};
		auto interleaved = AudioBuffer{block, 2, 2, true};
		for (std::int64_t start = 0; start < frames; start += block)
		{
			for (const auto& p : pending)
			{
				if (p.at >= start && p.at < start + block)
				{
					auto note = Note{TimePos{0}, TimePos{0}, p.key, DefaultVolume};
					playing.push_back(NotePlayHandleManager::acquire(track, static_cast<f_cnt_t>(p.at - start),
						static_cast<f_cnt_t>(p.length), note));
				}
			}
			auto view = interleaved.interleavedBuffer().asSampleFrames();
			std::fill(view.begin(), view.end(), SampleFrame{});
			auto scratch = std::vector<SampleFrame>(block);
			for (auto* nph : playing) { nph->play(scratch); }
			auto context = ProcessContext{nullptr, &interleaved, nullptr};
			context.setEvents(InputEvents{}, nullptr);
			instrument->process(context);
			for (f_cnt_t f = 0; f < block && start + f < frames; ++f)
			{
				out[static_cast<std::size_t>(start + f) * 2] = view[f][0];
				out[static_cast<std::size_t>(start + f) * 2 + 1] = view[f][1];
			}
			for (auto it = playing.begin(); it != playing.end();)
			{
				if ((*it)->isFinished()) { NotePlayHandleManager::release(*it); it = playing.erase(it); }
				else { ++it; }
			}
		}
		return out;
	}
	auto period = std::vector<PluginEvent>{};
	for (std::int64_t start = 0; start < frames; start += block)
	{
		const auto n = static_cast<f_cnt_t>(std::min<std::int64_t>(block, frames - start));
		period.clear();
		for (auto e : events)
		{
			if (e.time >= start && e.time < start + n) { e.time -= static_cast<f_cnt_t>(start); period.push_back(e); }
		}
		for (ch_cnt_t ch = 0; ch < channels; ++ch) { std::fill_n(buffer.buffer(ch).data(), block, 0.f); }
		auto context = ProcessContext{nullptr, &buffer, nullptr};
		context.setEvents(InputEvents{period}, nullptr);
		instrument->process(context);
		for (f_cnt_t f = 0; f < n; ++f)
		{
			for (ch_cnt_t ch = 0; ch < channels; ++ch)
			{
				out[static_cast<std::size_t>(start + f) * channels + ch] = buffer.buffer(ch)[f];
			}
		}
	}
	// (the track stays in the song: the engine is stopped, nothing plays it)
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
	Engine::init(true);
	Engine::audioEngine()->stopProcessing();
	auto file = QFile{argv[1]};
	if (!file.open(QIODevice::ReadOnly)) { std::fprintf(stderr, "cannot read %s\n", argv[1]); return 1; }
	const auto cases = QJsonDocument::fromJson(file.readAll()).object()["cases"].toArray();
	for (const auto& value : cases)
	{
		const auto c = value.toObject();
		if (c.contains("legacy") && !c["legacy"].toBool()) { continue; }
		const auto out = run(c);
		auto reference = QFile{QString{"%1/%2.f32"}.arg(argv[2], c["name"].toString())};
		if (!reference.open(QIODevice::WriteOnly)) { std::fprintf(stderr, "cannot write\n"); return 1; }
		reference.write(reinterpret_cast<const char*>(out.data()), static_cast<qint64>(out.size() * sizeof(float)));
		std::printf("%s\n", qPrintable(c["name"].toString()));
	}
	Engine::destroy();
	return 0;
}
