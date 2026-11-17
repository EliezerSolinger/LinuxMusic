/*
 * AudioThreadAllocationTest.cpp
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


#include <algorithm>
#include <atomic>
#include <cstdlib>
#include <dlfcn.h>
#include <execinfo.h>
#include <new>
#include <numeric>
#include <vector>

#include <QDir>
#include <QDomDocument>
#include <QObject>
#include <QTemporaryDir>
#include <QtTest>
#include <sndfile.h>

#include "AudioEngine.h"
#include "AudioStream.h"
#include "AudioTrack.h"
#include "BasicFilters.h"
#include "BufferManager.h"
#include "ConfigManager.h"
#include "Engine.h"
#include "ErrorLog.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Mixer.h"
#include "Note.h"
#include "NotePlayHandle.h"
#include "PluginChecks.h"
#include "PluginFactory.h"
#include "PluginInstance.h"
#include "RealtimeAllocator.h"
#include "SampleClip.h"
#include "SampleTrack.h"
#include "Song.h"
#include "StreamScheduler.h"

// The audio thread must not allocate (docs/plano-gravacao.md, "Thread de áudio limpa"): every allocation and free
// made on it while a song plays is counted here, with where it was made.
namespace
{

std::atomic<bool> s_counting = false;
std::atomic<long> s_events = 0;
//! As the program does (RealtimeNew.cpp): the audio threads' memory from the realtime pool. Only what then still
//! reaches the system is counted; frees of the system's memory the program hands over, counted apart
std::atomic<bool> s_usePool = false;
std::atomic<long> s_handedOver = 0;

//! Every place the audio thread allocated or freed at, with how many times: the frames above the hook, in a table
//! made before counting (the hook must not allocate), shared by the engine's threads under a spin lock
constexpr int TraceDepth = 24;
constexpr int TraceSlots = 1024;
struct Site
{
	void* frames[TraceDepth];
	int depth;
	long count;
};
Site s_sites[TraceSlots];
int s_siteCount = 0;
long s_unrecorded = 0; //!< events at sites past the table's end
std::atomic_flag s_sitesLock = ATOMIC_FLAG_INIT;
thread_local bool s_inHook = false;

void record(void* const* frames, int depth)
{
	while (s_sitesLock.test_and_set(std::memory_order_acquire)) {}
	auto* site = std::find_if(s_sites, s_sites + s_siteCount, [&](const Site& known) {
		return known.depth == depth && std::equal(frames, frames + depth, known.frames);
	});
	if (site != s_sites + s_siteCount) { ++site->count; }
	else if (s_siteCount < TraceSlots)
	{
		site = &s_sites[s_siteCount++];
		std::copy(frames, frames + depth, site->frames);
		site->depth = depth;
		site->count = 1;
	}
	else { ++s_unrecorded; }
	s_sitesLock.clear(std::memory_order_release);
}

void note()
{
	if (!s_counting.load(std::memory_order_relaxed) || s_inHook || !lmms::ThreadCheck::isAudioThread()) { return; }
	s_inHook = true;
	s_events.fetch_add(1, std::memory_order_relaxed);
	// Leaves out this function only: allocate() and release() may be inlined into the operators, so how many frames
	// they take varies (the report's reader leaves them out by name)
	void* frames[TraceDepth + 1];
	const auto depth = backtrace(frames, TraceDepth + 1);
	if (depth > 1) { record(frames + 1, depth - 1); }
	s_inHook = false;
}

//! Set while the operators below call the system, so that the system's hook (see hookSystemAllocator()) does not
//! count the same allocation again
thread_local bool s_insideOperator = false;

auto systemMalloc(std::size_t size) -> void*
{
	s_insideOperator = true;
	auto* memory = std::malloc(size);
	s_insideOperator = false;
	return memory;
}

void systemFree(void* memory)
{
	s_insideOperator = true;
	std::free(memory);
	s_insideOperator = false;
}

//! Qt and C code call malloc() and free() themselves, not operator new: the observer library loaded with the test
//! (tests/src/support/AllocationObserver.c) tells of each call to the system's allocator. Without it, only operator
//! new is seen
void countSystemCall()
{
	if (!s_insideOperator) { note(); }
}

auto observeSystemAllocator() -> bool
{
	auto* observer = reinterpret_cast<void (**)()>(dlsym(RTLD_DEFAULT, "lmmsAllocationObserver"));
	if (!observer) { return false; }
	*observer = countSystemCall;

	// Seen only if the library really stands before the system's allocator (not under the address sanitizer,
	// which replaces malloc itself): checked with an allocation of its own
	s_events = 0;
	s_counting = true;
	{
		const auto audioThread = lmms::ThreadCheck::AudioThreadScope{};
		void* volatile memory = std::malloc(64);
		std::free(memory);
	}
	s_counting = false;
	const bool seen = s_events.load() >= 2;
	s_events = 0;
	return seen;
}

auto allocate(std::size_t size) -> void*
{
	if (s_usePool.load(std::memory_order_relaxed) && lmms::ThreadCheck::isAudioThread())
	{
		if (auto* memory = lmms::RealtimeAllocator::allocate(size)) { return memory; }
	}
	note();
	if (auto* memory = systemMalloc(size ? size : 1)) { return memory; }
	throw std::bad_alloc{};
}

auto allocateAligned(std::size_t size, std::align_val_t alignment) -> void*
{
	if (s_usePool.load(std::memory_order_relaxed) && lmms::ThreadCheck::isAudioThread())
	{
		if (auto* memory = lmms::RealtimeAllocator::allocateAligned(size, static_cast<std::size_t>(alignment)))
		{
			return memory;
		}
	}
	note();
	void* memory = nullptr;
	s_insideOperator = true;
	const auto result = posix_memalign(&memory, std::max(sizeof(void*), static_cast<std::size_t>(alignment)),
		size ? size : 1);
	s_insideOperator = false;
	if (result == 0) { return memory; }
	throw std::bad_alloc{};
}

void release(void* memory, bool aligned = false) noexcept
{
	if (!memory) { return; }
	if (lmms::RealtimeAllocator::owns(memory))
	{
		if (aligned) { lmms::RealtimeAllocator::releaseAligned(memory); }
		else { lmms::RealtimeAllocator::release(memory); }
		return;
	}
	if (s_usePool.load(std::memory_order_relaxed) && lmms::ThreadCheck::isAudioThread())
	{
		if (s_counting.load(std::memory_order_relaxed)) { s_handedOver.fetch_add(1, std::memory_order_relaxed); }
	}
	else { note(); }
	systemFree(memory);
}

} // namespace

void* operator new(std::size_t size) { return allocate(size); }
void* operator new[](std::size_t size) { return allocate(size); }
void* operator new(std::size_t size, const std::nothrow_t&) noexcept
{
	try { return allocate(size); } catch (...) { return nullptr; }
}
void* operator new[](std::size_t size, const std::nothrow_t&) noexcept
{
	try { return allocate(size); } catch (...) { return nullptr; }
}
void* operator new(std::size_t size, std::align_val_t alignment) { return allocateAligned(size, alignment); }
void* operator new[](std::size_t size, std::align_val_t alignment) { return allocateAligned(size, alignment); }
void operator delete(void* memory) noexcept { release(memory); }
void operator delete[](void* memory) noexcept { release(memory); }
void operator delete(void* memory, std::size_t) noexcept { release(memory); }
void operator delete[](void* memory, std::size_t) noexcept { release(memory); }
void operator delete(void* memory, std::align_val_t) noexcept { release(memory, true); }
void operator delete[](void* memory, std::align_val_t) noexcept { release(memory, true); }
void operator delete(void* memory, std::size_t, std::align_val_t) noexcept { release(memory, true); }
void operator delete[](void* memory, std::size_t, std::align_val_t) noexcept { release(memory, true); }

using namespace lmms;

class AudioThreadAllocationTest : public QObject
{
	Q_OBJECT
private:
	QTemporaryDir m_directory;
	bool m_observingSystem = false;

	void writeWav(const QString& path, int frames)
	{
		auto info = SF_INFO{};
		info.samplerate = static_cast<int>(Engine::audioEngine()->outputSampleRate());
		info.channels = 2;
		info.format = SF_FORMAT_WAV | SF_FORMAT_FLOAT;
		auto* file = sf_open(QFile::encodeName(path).constData(), SFM_WRITE, &info);
		QVERIFY(file);
		auto samples = std::vector<float>(static_cast<std::size_t>(frames) * 2);
		for (std::size_t i = 0; i < samples.size(); ++i) { samples[i] = static_cast<float>((i * 7919) % 2000) / 2000.f - 0.5f; }
		sf_writef_float(file, samples.data(), frames);
		sf_close(file);
	}

	//! Plays the song for @p periods periods, counting what the audio thread allocates
	auto playCounting(int warmUp, int periods) -> long
	{
		auto* song = Engine::getSong();
		auto* engine = Engine::audioEngine();
		song->setPlayPos(0, Song::PlayMode::Song);
		song->playSong();
		for (int period = 0; period < warmUp; ++period) { engine->renderNextPeriod(); }
		s_events = 0;
		s_siteCount = 0;
		s_unrecorded = 0;
		s_counting = true;
		for (int period = 0; period < periods; ++period) { engine->renderNextPeriod(); }
		s_counting = false;
		song->stop();
		return s_events.load();
	}

	static void report(long events)
	{
		// Each site with its count, most first, as raw addresses with their image and its base, for atos
		// (atos -o <image> -l <base> <address>...)
		auto order = std::vector<int>(s_siteCount);
		std::iota(order.begin(), order.end(), 0);
		std::sort(order.begin(), order.end(), [](int a, int b) { return s_sites[a].count > s_sites[b].count; });
		qInfo("%ld allocations or frees on the audio thread, at %d places (%ld past the table):", events, s_siteCount,
			s_unrecorded);
		for (const auto index : order)
		{
			const auto& site = s_sites[index];
			auto line = QString{};
			for (int frame = 0; frame < site.depth && frame < 14; ++frame)
			{
				auto info = Dl_info{};
				if (dladdr(site.frames[frame], &info) && info.dli_fname)
				{
					line += QString{" %1@%2+%3"}.arg(QFileInfo{info.dli_fname}.fileName())
						.arg(reinterpret_cast<quintptr>(info.dli_fbase), 0, 16)
						.arg(reinterpret_cast<quintptr>(site.frames[frame]), 0, 16);
				}
			}
			qInfo("TRACE %ld:%s", site.count, qPrintable(line));
		}
	}

private slots:
	void initTestCase()
	{
		QVERIFY(m_directory.isValid());
		RealtimeAllocator::init(64 << 20);
		m_observingSystem = observeSystemAllocator();
		if (!m_observingSystem) { qWarning("the system's allocator is not observed: only operator new is counted"); }
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	//! The counting sees what the audio thread allocates itself, through operator new and through the system (as Qt
	//! and C code do), and nothing on other threads
	void countingSeesEveryAllocation()
	{
		s_events = 0;
		s_counting = true;
		{
			const auto audioThread = ThreadCheck::AudioThreadScope{};
			// Through pointers the compiler cannot see through, so that it keeps the calls
			int* volatile number = new int{1};
			delete number;
			void* volatile memory = std::malloc(100);
			std::free(memory);
			const auto text = QString::number(123456789).repeated(20);
			QVERIFY(!text.isEmpty());
		}
		const auto onAudioThread = s_events.load();
		delete new int{1};
		s_counting = false;
		QCOMPARE(s_events.load(), onAudioThread);
		QVERIFY2(onAudioThread >= (m_observingSystem ? 4 : 2), qPrintable(QString::number(onAudioThread)));
	}

	//! The error log takes reports from the audio thread without allocating (see ErrorLog::report())
	void reportingAnErrorAllocatesNothing()
	{
		s_events = 0;
		s_counting = true;
		{
			const auto audioThread = ThreadCheck::AudioThreadScope{};
			for (int i = 0; i < 100; ++i)
			{
				ErrorLog::report(ErrorLog::Severity::Error, "AudioThreadAllocationTest", "reported", i);
			}
			ErrorLog::report(ErrorLog::Severity::Warning, "AudioThreadAllocationTest", "no value");
		}
		s_counting = false;
		QCOMPARE(s_events.load(), 0);
	}

	void playingASongAllocatesNothingOnTheAudioThread()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		const auto bar = TimePos::ticksPerBar();
		const auto file = m_directory.filePath("audio.wav");
		writeWav(file, 30 * static_cast<int>(Engine::audioEngine()->outputSampleRate()));

		// An audio track with an item in memory and one from disk, crossing
		auto* audio = dynamic_cast<AudioTrack*>(Track::create(Track::Type::Audio, song));
		auto* inMemory = audio->addItem(file, TimePos{0});
		inMemory->changeLength(TimePos{4 * bar});
		auto* fromDisk = audio->addItem(file, TimePos{3 * bar});
		auto settings = AudioTrack::settingsOf(*fromDisk);
		settings.storage = AudioItem::Storage::Disk;
		audio->changeItem(fromDisk, settings);
		settings = AudioTrack::settingsOf(*inMemory);
		settings.storage = AudioItem::Storage::Memory;
		audio->changeItem(inMemory, settings);
		audio->publish();

		// An instrument playing notes all along
		auto* instrumentTrack = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QVERIFY(instrumentTrack->loadInstrument("tripleoscillator"));
		auto* clip = dynamic_cast<MidiClip*>(instrumentTrack->createClip(TimePos{0}));
		for (int note = 0; note < 32; ++note)
		{
			clip->addNote(Note{TimePos{24}, TimePos{note * 24}, DefaultKey + note % 12});
		}
		Engine::mixer()->setUpTrackChannels();

		DiskReader::instance().setDemand(StreamScheduler::demandAt(*song, 0, 30.0));
		QVERIFY(DiskReader::instance().waitUntilLoaded(5000));
		AudioStream::setMissPolicy(AudioStream::MissPolicy::PlaySilence);

		const auto events = playCounting(40, 1200);
		AudioStream::setMissPolicy(AudioStream::MissPolicy::Automatic);
		DiskReader::instance().setDemand({});
		report(events);
		// Left (docs/plano-gravacao.md, "Thread de áudio limpa"): the notes' lifetime (NotePlayHandle, the instruments'
		// note data made with new in playNote), the engine's list of play handles and the master's monitoring. The
		// automations allocate nothing since 13/11 (they did on every tick).
		QVERIFY2(events == 0, qPrintable(QString{"%1 allocations or frees on the audio thread"}.arg(events)));
	}

	//! Chords, an arpeggio, the envelope's double filter, a send between mixer channels with an effect: the notes
	//! made from other notes and what each note uses
	void chordsArpeggioFilterAndSendsAllocateNothing()
	{
		auto* song = Engine::getSong();
		song->createNewProject();

		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QVERIFY(track->loadInstrument("tripleoscillator"));
		// Through the track's settings, as a project sets them
		auto document = QDomDocument{};
		auto saved = document.createElement("saved");
		track->saveState(document, saved);
		auto settings = saved.firstChildElement();
		auto set = [&](const QString& tag, const QString& attribute, const QString& value) {
			auto element = settings.elementsByTagName(tag).item(0).toElement();
			QVERIFY2(!element.isNull(), qPrintable(tag));
			element.setAttribute(attribute, value);
		};
		set("eldata", "fwet", "1");
		set("eldata", "ftype", QString::number(static_cast<int>(BasicFilters<>::FilterType::DoubleMoog)));
		set("chordcreator", "chord-enabled", "1");
		set("arpeggiator", "arp-enabled", "1");
		set("arpeggiator", "arptime", "50");
		set("arpeggiator", "arpmode", "1"); // sort: all the track's notes
		track->restoreState(settings);

		// The track into channel 1, which has an effect and sends to channel 2
		auto* mixer = Engine::mixer();
		const auto first = mixer->createChannel();
		const auto second = mixer->createChannel();
		track->mixerChannelModel()->setValue(first);
		auto key = Plugin::Descriptor::SubPluginFeatures::Key{};
		auto* channel = mixer->mixerChannel(first);
		channel->m_fxChain.appendEffect(
			std::make_unique<PluginInstance>(Plugin::Type::Effect, "amplifier", &channel->m_fxChain, nullptr));
		mixer->createChannelSend(first, second, 0.5f);
		mixer->setUpTrackChannels();

		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		for (int note = 0; note < 16; ++note)
		{
			clip->addNote(Note{TimePos{48}, TimePos{note * 24}, DefaultKey + note % 7});
		}

		const auto events = playCounting(40, 1200);
		report(events);
		QVERIFY2(events == 0, qPrintable(QString{"%1 allocations or frees on the audio thread"}.arg(events)));
		QCOMPARE(BufferManager::overflows(), 0L);
	}

	//! A sample track (the grid's), its clips starting one after the other: each start plays the clip's sample
	void sampleTrackAllocatesNothing()
	{
		auto* song = Engine::getSong();
		song->createNewProject();
		const auto file = m_directory.filePath("hit.wav");
		writeWav(file, static_cast<int>(Engine::audioEngine()->outputSampleRate() / 4));
		auto* track = dynamic_cast<SampleTrack*>(Track::create(Track::Type::Sample, song));
		for (int step = 0; step < 16; ++step)
		{
			auto* clip = dynamic_cast<SampleClip*>(track->createClip(TimePos{step * 48}));
			clip->setSampleFile(file);
			clip->changeLength(TimePos{48});
		}
		Engine::mixer()->setUpTrackChannels();

		const auto events = playCounting(40, 1200);
		report(events);
		QVERIFY2(events == 0, qPrintable(QString{"%1 allocations or frees on the audio thread"}.arg(events)));
	}

	//! Each instrument playing notes: what its notes allocate on the audio thread
	void instrumentsAllocateNothing_data()
	{
		QTest::addColumn<QString>("instrument");
		for (const auto* name : {"tripleoscillator", "bitinvader", "kicker", "lb302", "monstro", "nes", "organic",
			"sfxr", "sid", "vibedstrings", "watsyn", "xpressive", "freeboy", "malletsstk", "opulenz",
			"audiofileprocessor", "slicert", "zynaddsubfx", "sf2player", "avldrums"})
		{
			QTest::newRow(name) << QString{name};
		}
	}

	void instrumentsAllocateNothing()
	{
		QFETCH(QString, instrument);
		auto* song = Engine::getSong();
		song->createNewProject();
		if (instrument == "malletsstk")
		{
			// The STK's wave files: the program's, or a system one (the instrument plays nothing without them)
			for (const auto* dir : {"/opt/homebrew/share/stk/rawwaves", "/usr/share/stk/rawwaves",
				"/usr/local/share/stk/rawwaves"})
			{
				if (QDir{dir}.exists()) { ConfigManager::inst()->setSTKDir(dir); }
			}
			if (!QDir{ConfigManager::inst()->stkDir()}.exists()) { QSKIP("no STK wave files here"); }
		}
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		if (!track->loadInstrument(instrument)) { QSKIP("not built here"); }
		if (instrument == "audiofileprocessor" || instrument == "slicert")
		{
			// A sample to play
			const auto file = m_directory.filePath("note.wav");
			writeWav(file, static_cast<int>(Engine::audioEngine()->outputSampleRate()));
			auto document = QDomDocument{};
			auto saved = document.createElement("saved");
			track->saveState(document, saved);
			auto settings = saved.firstChildElement();
			auto plugin = settings.elementsByTagName("instrument").item(0).firstChildElement();
			QVERIFY(!plugin.isNull());
			plugin.setAttribute("src", file);
			// Slices for the notes to play (one per key from the base note up)
			plugin.setAttribute("totalSlices", 13);
			for (int slice = 0; slice < 13; ++slice) { plugin.setAttribute(QString{"slice_%1"}.arg(slice), slice / 12.0); }
			track->restoreState(settings);
		}
		Engine::mixer()->setUpTrackChannels();
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		for (int note = 0; note < 16; ++note)
		{
			clip->addNote(Note{TimePos{36}, TimePos{note * 24}, DefaultKey + note % 12});
		}

		// These make each note's voice on the audio thread as a library builds it, with new (Xpressive's compiled
		// expressions, ZynAddSubFX's notes, Vibed's strings as long as the note's pitch asks): from the realtime pool,
		// as in the program. Nothing reaches the system's allocator
		const bool library = instrument == "xpressive" || instrument == "zynaddsubfx" || instrument == "vibedstrings";
		s_usePool = library;
		s_handedOver = 0;
		const auto events = playCounting(40, 600);
		s_usePool = false;
		report(events);
		if (library)
		{
			const auto stats = RealtimeAllocator::stats();
			qInfo("realtime pool: %zu KB at most in use, %ld frees of the system's memory handed over",
				stats.peakBytes / 1024, s_handedOver.load());
		}
		QVERIFY2(events == 0, qPrintable(QString{"%1 allocations or frees on the audio thread"}.arg(events)));
	}

	//! Each built-in effect on a mixer channel, a track playing through it: what it allocates on the audio thread
	void effectsAllocateNothing_data()
	{
		QTest::addColumn<QString>("effect");
		auto names = QStringList{};
		for (const auto* descriptor : getPluginFactory()->descriptors(Plugin::Type::Effect))
		{
			const auto name = QString::fromUtf8(descriptor->name);
			// (third-party hosts: their plugins are not ours)
			if (name == "ladspaeffect" || name == "lv2effect" || name == "vsteffect") { continue; }
			names << name;
		}
		names.sort();
		for (const auto& name : names) { QTest::newRow(qPrintable(name)) << name; }
	}

	void effectsAllocateNothing()
	{
		QFETCH(QString, effect);
		auto* song = Engine::getSong();
		song->createNewProject();
		auto* track = dynamic_cast<InstrumentTrack*>(Track::create(Track::Type::Instrument, song));
		QVERIFY(track->loadInstrument("tripleoscillator"));
		auto* mixer = Engine::mixer();
		const auto channelIndex = mixer->createChannel();
		track->mixerChannelModel()->setValue(channelIndex);
		auto* chain = &mixer->mixerChannel(channelIndex)->m_fxChain;
		chain->appendEffect(std::make_unique<PluginInstance>(Plugin::Type::Effect, effect, chain, nullptr));
		mixer->setUpTrackChannels();
		auto* clip = dynamic_cast<MidiClip*>(track->createClip(TimePos{0}));
		for (int note = 0; note < 16; ++note)
		{
			clip->addNote(Note{TimePos{36}, TimePos{note * 24}, DefaultKey + note % 12});
		}

		const auto events = playCounting(40, 600);
		report(events);
		QVERIFY2(events == 0, qPrintable(QString{"%1 allocations or frees on the audio thread"}.arg(events)));
	}
};

QTEST_GUILESS_MAIN(AudioThreadAllocationTest)
#include "AudioThreadAllocationTest.moc"
