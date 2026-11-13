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


#include <atomic>
#include <cstdlib>
#include <dlfcn.h>
#include <execinfo.h>
#include <new>
#include <vector>

#include <QObject>
#include <QTemporaryDir>
#include <QtTest>
#include <sndfile.h>

#include "AudioEngine.h"
#include "AudioStream.h"
#include "AudioTrack.h"
#include "Engine.h"
#include "InstrumentTrack.h"
#include "MidiClip.h"
#include "Mixer.h"
#include "Note.h"
#include "NotePlayHandle.h"
#include "PluginChecks.h"
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
constexpr int MaxTraces = 48;
constexpr int TraceDepth = 28;
void* s_traces[MaxTraces][TraceDepth];
int s_depths[MaxTraces];
std::atomic<int> s_traceCount = 0;
thread_local bool s_inHook = false;

void note()
{
	if (!s_counting.load(std::memory_order_relaxed) || s_inHook || !lmms::ThreadCheck::isAudioThread()) { return; }
	s_inHook = true;
	s_events.fetch_add(1, std::memory_order_relaxed);
	if (const auto index = s_traceCount.fetch_add(1); index < MaxTraces)
	{
		s_depths[index] = backtrace(s_traces[index], TraceDepth);
	}
	s_inHook = false;
}

auto allocate(std::size_t size) -> void*
{
	note();
	if (auto* memory = std::malloc(size ? size : 1)) { return memory; }
	throw std::bad_alloc{};
}

auto allocateAligned(std::size_t size, std::align_val_t alignment) -> void*
{
	note();
	void* memory = nullptr;
	if (posix_memalign(&memory, std::max(sizeof(void*), static_cast<std::size_t>(alignment)), size ? size : 1) == 0)
	{
		return memory;
	}
	throw std::bad_alloc{};
}

void release(void* memory) noexcept
{
	if (memory) { note(); }
	std::free(memory);
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
void operator delete(void* memory, std::align_val_t) noexcept { release(memory); }
void operator delete[](void* memory, std::align_val_t) noexcept { release(memory); }
void operator delete(void* memory, std::size_t, std::align_val_t) noexcept { release(memory); }
void operator delete[](void* memory, std::size_t, std::align_val_t) noexcept { release(memory); }

using namespace lmms;

class AudioThreadAllocationTest : public QObject
{
	Q_OBJECT
private:
	QTemporaryDir m_directory;

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
		s_traceCount = 0;
		s_counting = true;
		for (int period = 0; period < periods; ++period) { engine->renderNextPeriod(); }
		s_counting = false;
		song->stop();
		return s_events.load();
	}

	static void report(long events)
	{
		// Raw addresses with their image and its base, for atos (tools: atos -o <image> -l <base> <address>...)
		const auto traces = std::min<int>(s_traceCount.load(), MaxTraces);
		qInfo("%ld allocations or frees on the audio thread; the first %d:", events, traces);
		for (int index = 0; index < traces; ++index)
		{
			auto line = QString{};
			for (int frame = 3; frame < s_depths[index] && frame < 16; ++frame)
			{
				auto info = Dl_info{};
				if (dladdr(s_traces[index][frame], &info) && info.dli_fname)
				{
					line += QString{" %1@%2+%3"}.arg(QFileInfo{info.dli_fname}.fileName())
						.arg(reinterpret_cast<quintptr>(info.dli_fbase), 0, 16)
						.arg(reinterpret_cast<quintptr>(s_traces[index][frame]), 0, 16);
				}
			}
			qInfo("TRACE %d:%s", index, qPrintable(line));
		}
	}

private slots:
	void initTestCase()
	{
		QVERIFY(m_directory.isValid());
		NotePlayHandleManager::init();
		qputenv("LMMS_PLUGIN_DIR", QFile::encodeName(QCoreApplication::applicationDirPath() + "/../plugins"));
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
		NotePlayHandleManager::free();
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
			clip->addNote(Note{TimePos{24}, TimePos{note * 24}, DefaultKey + note % 12}, false);
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
		QEXPECT_FAIL("", "the notes' lifetime and the play handle list still allocate on the audio thread", Continue);
		QVERIFY2(events == 0, qPrintable(QString{"%1 allocations or frees on the audio thread"}.arg(events)));
	}
};

QTEST_GUILESS_MAIN(AudioThreadAllocationTest)
#include "AudioThreadAllocationTest.moc"
