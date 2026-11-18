/*
 * DrumMachines.cpp - the drum machines of the DrumSynth kits as an instrument
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

#include "DrumMachines.h"

#include <algorithm>

#include <QDomElement>

#include <samplerate.h>
#include <QMouseEvent>
#include <QMutexLocker>
#include <QPainter>
#include <QPainterPath>

#include "AudioEngine.h"
#include "ComboBox.h"
#include "ConfigManager.h"
#include "Engine.h"
#include "FontHelper.h"
#include "InstrumentPlayHandle.h"
#include "InstrumentTrack.h"
#include "MidiEvent.h"
#include "MidiPort.h"
#include "Mixer.h"
#include "SampleBuffer.h"
#include "embed.h"
#include "Song.h"

#include "plugin_export.h"

namespace lmms
{

extern "C"
{

Plugin::Descriptor PLUGIN_EXPORT drummachines_plugin_descriptor =
{
	LMMS_STRINGIFY(PLUGIN_NAME),
	"Drum Machines",
	QT_TRANSLATE_NOOP("PluginBrowser", "Classic drum machines (CR-78, TR-808, TR-909, LinnDrum...) on the General "
		"MIDI drum map"),
	"LinuxMusic; the kits are DrumSynth's (data/samples/drumsynth)",
	0x0100,
	Plugin::Type::Instrument,
	new PixmapLoader("lmms-plugin-logo"),
	nullptr,
	nullptr,
};

PLUGIN_EXPORT Plugin* lmms_plugin_main(Model* parent, void*)
{
	return new DrumMachinesInstrument(static_cast<InstrumentTrack*>(parent));
}

} // extern "C"


namespace
{

// The General MIDI drum map's notes
enum Note : int
{
	BassDrum2 = 35, BassDrum = 36, SideStick = 37, Snare = 38, HandClap = 39, Snare2 = 40, LowFloorTom = 41,
	ClosedHat = 42, HighFloorTom = 43, PedalHat = 44, LowTom = 45, OpenHat = 46, LowMidTom = 47, HighMidTom = 48,
	Crash = 49, HighTom = 50, Ride = 51, Tambourine = 54, Cowbell = 56, HighBongo = 60, LowBongo = 61,
	MuteHighConga = 62, OpenHighConga = 63, LowConga = 64, Cabasa = 69, Maracas = 70, Claves = 75, MuteCuica = 78,
	OpenCuica = 79,
	//! The first note after the map's: the variations it has no note for
	Extra = 82
};

constexpr int HiHats = 1; // (the choke group of the hi-hats)

} // namespace


auto DrumMachinesInstrument::kits() -> const std::vector<Kit>&
{
	using L = Look;
	static const auto s_kits = std::vector<Kit>{
		{"CR-78", "drummachines/cr78", L{{0x6b, 0x42, 0x26}, {0x1d, 0x1d, 0x1d}, {0xe0, 0x6b, 0x2a}, {0xee, 0xe6, 0xd2},
			{QColor{0xe8, 0xdf, 0xc8}, QColor{0xe0, 0x6b, 0x2a}, QColor{0xe8, 0xdf, 0xc8}, QColor{0x9a, 0x9a, 0x9a}}},
			{{"kick.flac", BassDrum, "BD"}, {"snare.flac", Snare, "SD"}, {"rim.flac", SideStick, "RS"},
				{"hihat.flac", ClosedHat, "HH", HiHats}, {"cymbal.flac", Crash, "CY"}, {"bongo_high.flac", HighBongo, "HB"},
				{"bongo_low.flac", LowBongo, "LB"}, {"conga.flac", OpenHighConga, "CG"}, {"clave.flac", Claves, "CL"},
				{"maracas.flac", Maracas, "MA"}, {"tambourine.flac", Tambourine, "TB"}}},
		{"CR-8000", "drumsynth/cr8000", L{{0x3a, 0x3a, 0x3a}, {0x24, 0x24, 0x24}, {0xd9, 0xa2, 0x27}, {0xe6, 0xe6, 0xe6},
			{QColor{0xd9, 0xa2, 0x27}, QColor{0xcf, 0xcf, 0xcf}, QColor{0x8c, 0xb4, 0xd8}, QColor{0xcf, 0xcf, 0xcf}}},
			{{"Kick.ds", BassDrum, "BD"}, {"Snare.ds", Snare, "SD"}, {"Rim.ds", SideStick, "RS"}, {"Clap.ds", HandClap, "CP"},
				{"Hat_c.ds", ClosedHat, "CH", HiHats}, {"Hat_o.ds", OpenHat, "OH", HiHats}, {"Cymbal.ds", Crash, "CY"},
				{"Cowbell.ds", Cowbell, "CB"}, {"Clave.ds", Claves, "CL"}, {"Conga_m.ds", OpenHighConga, "HC"},
				{"Conga_l.ds", LowConga, "LC"}},
			"photo_cr8000.jpg", QT_TRANSLATE_NOOP("lmms::gui::DrumMachinesView", "Photo: Blair Rideout, CC BY 2.0")},
		{"TR-606", "drumsynth/tr606", L{{0xc9, 0xcc, 0xd1}, {0x2b, 0x2b, 0x2b}, {0xc8, 0x37, 0x2d}, {0x20, 0x20, 0x20},
			{QColor{0xc8, 0x37, 0x2d}, QColor{0xd8, 0xd8, 0xd8}, QColor{0xc8, 0x37, 0x2d}, QColor{0xd8, 0xd8, 0xd8}}},
			{{"TR606_Kick.ds", BassDrum, "BD"}, {"TR606_Snare.ds", Snare, "SD"}, {"TR606_Hat_C.ds", ClosedHat, "CH", HiHats},
				{"TR606_Hat_O.ds", OpenHat, "OH", HiHats}, {"TR606_Cymbal.ds", Crash, "CY"}, {"TR606_Tom_L.ds", LowTom, "LT"},
				{"TR606_Tom_H.ds", HighTom, "HT"}},
			"photo_tr606.jpg", QT_TRANSLATE_NOOP("lmms::gui::DrumMachinesView", "Photo: Midas Wouters, CC BY-SA 3.0")},
		{"TR-77", "drumsynth/tr77", L{{0x5a, 0x3b, 0x26}, {0xe8, 0xdc, 0xc0}, {0x3b, 0x6f, 0xb6}, {0x22, 0x22, 0x22},
			{QColor{0x3b, 0x6f, 0xb6}, QColor{0xf4, 0xee, 0xe0}, QColor{0x3b, 0x6f, 0xb6}, QColor{0xf4, 0xee, 0xe0}}},
			{{"Tr77_kick.ds", BassDrum, "BD"}, {"Tr77_snare.ds", Snare, "SD"}, {"Tr77_hat.ds", ClosedHat, "HH", HiHats},
				{"Tr77_cym.ds", Crash, "CY"}, {"Tr77_tom1.ds", HighTom, "HT"}, {"Tr77_tom2.ds", LowTom, "LT"},
				{"Tr77_cui1.ds", MuteCuica, "C1"}, {"Tr77_cui2.ds", OpenCuica, "C2"}},
			"photo_tr77.jpg", QT_TRANSLATE_NOOP("lmms::gui::DrumMachinesView", "Photo: Trouby, CC BY-SA 3.0")},
		{"TR-808", "drumsynth/tr808", L{{0x4a, 0x4a, 0x46}, {0x2f, 0x2f, 0x2c}, {0xec, 0x7a, 0x1c}, {0xe8, 0xe3, 0xd3},
			{QColor{0xd8, 0x32, 0x2b}, QColor{0xec, 0x7a, 0x1c}, QColor{0xe9, 0xc4, 0x3a}, QColor{0xe8, 0xe3, 0xd3}}},
			{{"Kick.ds", BassDrum, "BD"}, {"Kickhard.ds", BassDrum2, "BD2"}, {"Rimshot.ds", SideStick, "RS"},
				{"Snare.ds", Snare, "SD"}, {"Handclap.ds", HandClap, "CP"}, {"Snr_snap.ds", Snare2, "SD2"},
				{"Hat_c.ds", ClosedHat, "CH", HiHats}, {"Hat_o.ds", OpenHat, "OH", HiHats}, {"Tom_lo.ds", LowTom, "LT"},
				{"Tom_mid.ds", LowMidTom, "MT"}, {"Tom_hi.ds", HighTom, "HT"}, {"Ride.ds", Ride, "RD"},
				{"Cowbell.ds", Cowbell, "CB"}, {"Clave.ds", Claves, "CL"}, {"Kicklong.ds", Extra, "BDL"}},
			"photo_tr808.jpg", QT_TRANSLATE_NOOP("lmms::gui::DrumMachinesView", "Photo: Brandon Daniel, Clusternote, CC BY-SA 2.0")},
		{"TR-909", "drumsynth/tr909", L{{0xb9, 0xbc, 0xbe}, {0x3a, 0x3a, 0x3a}, {0xf0, 0x8a, 0x24}, {0x1e, 0x1e, 0x1e},
			{QColor{0xf0, 0x8a, 0x24}, QColor{0xe2, 0xe2, 0xe2}, QColor{0xe2, 0xe2, 0xe2}, QColor{0xe2, 0xe2, 0xe2}}},
			{{"Kick.ds", BassDrum, "BD"}, {"Kick_1.ds", BassDrum2, "BD2"}, {"Snare.ds", Snare, "SD"},
				{"Snare2.ds", Snare2, "SD2"}, {"Clap.ds", HandClap, "CP"}, {"Hat-c.ds", ClosedHat, "CH", HiHats},
				{"Hat-c2.ds", PedalHat, "PH", HiHats}, {"Hat-o.ds", OpenHat, "OH", HiHats},
				{"Hat-o2.ds", Extra, "OH2", HiHats}, {"Kick_2.ds", Extra + 1, "BD3"}, {"Kick_3.ds", Extra + 2, "BD4"},
				{"TR909_Kick.ds", Extra + 3, "BD5"}, {"TR909_Clap.ds", Extra + 4, "CP2"}},
			"photo_tr909.jpg", QT_TRANSLATE_NOOP("lmms::gui::DrumMachinesView", "Photo: Brandon Daniel, Clusternote, CC BY-SA 2.0")},
		{"LinnDrum", "drumsynth/linn", L{{0x1b, 0x1b, 0x1b}, {0x2a, 0x2a, 0x2a}, {0xc9, 0xa2, 0x27}, {0xe6, 0xe1, 0xd5},
			{QColor{0xe6, 0xe1, 0xd5}, QColor{0xc9, 0xa2, 0x27}, QColor{0xe6, 0xe1, 0xd5}, QColor{0x9c, 0x9c, 0x9c}}},
			{{"linn_kik.ds", BassDrum, "BD"}, {"linn_stk.ds", SideStick, "ST"}, {"linn_snr.ds", Snare, "SD"},
				{"linn_clp.ds", HandClap, "CP"}, {"linn_tom5.ds", HighFloorTom, "T5"}, {"linn_tom4.ds", LowTom, "T4"},
				{"linn_tom3.ds", LowMidTom, "T3"}, {"linn_tom2.ds", HighMidTom, "T2"}, {"linn_tom1.ds", HighTom, "T1"},
				{"linn_tamb.ds", Tambourine, "TB"}, {"linn_cow.ds", Cowbell, "CB"}, {"linn_cgam.ds", MuteHighConga, "CM"},
				{"linn_cga1.ds", OpenHighConga, "C1"}, {"linn_cga2.ds", LowConga, "C2"}, {"linn_cab.ds", Cabasa, "CA"},
				{"linn_cga3.ds", Extra, "C3"}},
			"photo_linn.jpg", QT_TRANSLATE_NOOP("lmms::gui::DrumMachinesView", "Photo: Linn Electronics / Forat Electronics, CC BY-SA 3.0")},
		{"Electro", "drumsynth/electro", L{{0x20, 0x24, 0x2a}, {0x12, 0x15, 0x1a}, {0x2f, 0xd1, 0xc5}, {0xd8, 0xf4, 0xf1},
			{QColor{0x2f, 0xd1, 0xc5}, QColor{0xd6, 0x4f, 0xc8}, QColor{0x2f, 0xd1, 0xc5}, QColor{0x6c, 0x7a, 0xe0}}},
			{{"K_Linn.ds", BassDrum, "BD"}, {"K_8bit.ds", BassDrum2, "BD2"}, {"S_Linn.ds", Snare, "SD"},
				{"S_8bit.ds", Snare2, "SD2"}, {"Boom.ds", LowFloorTom, "BM"}, {"H_closed.ds", ClosedHat, "CH", HiHats},
				{"H_open.ds", OpenHat, "OH", HiHats}, {"Syntom_2.ds", LowTom, "LT"}, {"Syntom_1.ds", HighTom, "HT"},
				{"K_reverb.ds", Extra, "BDR"}, {"S_reverb.ds", Extra + 1, "SDR"}, {"s_eq.ds", Extra + 2, "SEQ"},
				{"Hi-q.ds", Extra + 3, "HQ"}, {"Hi-q_2.ds", Extra + 4, "HQ2"}},
			"photo_dmx.jpg", QT_TRANSLATE_NOOP("lmms::gui::DrumMachinesView", "Oberheim DMX. Photo: Alison, CC BY-SA 4.0")},
		{"R&B", "drumsynth/r_n_b", L{{0x2b, 0x21, 0x35}, {0x1a, 0x14, 0x20}, {0xd6, 0x5d, 0xb1}, {0xf0, 0xe0, 0xec},
			{QColor{0xd6, 0x5d, 0xb1}, QColor{0xe8, 0xc5, 0x6a}, QColor{0xd6, 0x5d, 0xb1}, QColor{0xb9, 0xa8, 0xd8}}},
			{{"k_hard.ds", BassDrum, "BD"}, {"k_soft.ds", BassDrum2, "BD2"}, {"popsnare.ds", Snare, "SD"},
				{"popclap.ds", HandClap, "CP"}, {"S_hollow.ds", Snare2, "SD2"}, {"hat.ds", ClosedHat, "CH", HiHats},
				{"Hat_O.ds", OpenHat, "OH", HiHats}, {"k_muffle.ds", Extra, "BDM"}, {"old_kik.ds", Extra + 1, "BDO"},
				{"clapfilt.ds", Extra + 2, "CPF"}, {"s_pitch.ds", Extra + 3, "SDP"}, {"s_spit.ds", Extra + 4, "SDS"},
				{"snare&hat.ds", Extra + 5, "S+H"}, {"x_backwd.ds", Extra + 6, "REV"},
				{"hat_rattle.ds", Extra + 7, "HR", HiHats}},
			"photo_sp1200.jpg", QT_TRANSLATE_NOOP("lmms::gui::DrumMachinesView", "E-mu SP-1200. Photo: 2xUeL, public domain")},
	};
	return s_kits;
}




DrumMachinesInstrument::DrumMachinesInstrument(InstrumentTrack* track) :
	LegacyInstrument(&drummachines_plugin_descriptor, track, nullptr,
		Flag::IsSingleStreamed | Flag::IsMidiBased | Flag::IsNotBendable),
	m_kitModel(this, tr("Drum machine")),
	m_sampleRate(Engine::audioEngine()->outputSampleRate())
{
	for (const auto& kit : kits()) { m_kitModel.addItem(kit.name); }
	m_soundOfNote.fill(-1);
	// (the TR-808 first)
	m_kitModel.setInitValue(4);
	// Synthesizing a kit takes a while, and the model may change from other threads (automation): on the main
	// thread, afterwards
	connect(&m_kitModel, &ComboBoxModel::dataChanged, this, &DrumMachinesInstrument::loadKit, Qt::QueuedConnection);
	loadKit();

	// The track processes the instrument once per period through this play handle
	Engine::audioEngine()->addPlayHandle(new InstrumentPlayHandle(this, track));
}




DrumMachinesInstrument::~DrumMachinesInstrument()
{
	// Its own play handle only (the instrument taking its place may be there already)
	Engine::audioEngine()->removePlayHandlesOf(this);
}




void DrumMachinesInstrument::loadKit()
{
	const auto index = m_kitModel.value();
	if (index < 0 || index >= static_cast<int>(kits().size())) { return; }
	if (index == m_loadedKit.load(std::memory_order_relaxed) && m_loadedRate == m_sampleRate) { return; }

	const auto& kit = kits()[index];
	const auto folder = ConfigManager::inst()->factorySamplesDir() + kit.folder + "/";
	auto sounds = std::vector<Sound>{};
	auto ofNote = std::array<int, 128>{};
	ofNote.fill(-1);
	for (const auto& piece : kit.pieces)
	{
		auto sound = Sound{{}, piece.note, piece.choke};
		// (DrumSynth renders at the engine's rate, a recording is resampled to it; a file that cannot be read leaves
		// the piece silent)
		try
		{
			const auto buffer = SampleBuffer::fromFile(folder + piece.file);
			sound.frames.assign(buffer->begin(), buffer->end());
			// (a recording at another rate: converted to the engine's, so that it plays at its pitch)
			if (buffer->sampleRate() != m_sampleRate && !sound.frames.empty())
			{
				const auto ratio = static_cast<double>(m_sampleRate) / buffer->sampleRate();
				auto converted = std::vector<SampleFrame>(static_cast<std::size_t>(sound.frames.size() * ratio) + 16);
				auto data = SRC_DATA{};
				data.data_in = &sound.frames.front()[0];
				data.input_frames = static_cast<long>(sound.frames.size());
				data.data_out = &converted.front()[0];
				data.output_frames = static_cast<long>(converted.size());
				data.src_ratio = ratio;
				if (src_simple(&data, SRC_SINC_BEST_QUALITY, DEFAULT_CHANNELS) == 0)
				{
					converted.resize(static_cast<std::size_t>(data.output_frames_gen));
					sound.frames = std::move(converted);
				}
			}
		}
		catch (const std::exception&) {}
		ofNote[piece.note] = static_cast<int>(sounds.size());
		sounds.push_back(std::move(sound));
	}

	{
		const auto lock = QMutexLocker{&m_soundsMutex};
		m_sounds = std::move(sounds);
		m_soundOfNote = ofNote;
		for (auto& voice : m_voices) { voice = Voice{}; }
	}
	for (auto& hit : m_hits) { hit.store(0, std::memory_order_relaxed); }
	m_loadedRate = m_sampleRate;
	m_fadeInFrames = std::max<std::size_t>(1, m_sampleRate / 2000);
	m_loadedKit.store(index, std::memory_order_relaxed);
	emit kitChanged();
}




auto DrumMachinesInstrument::activate(double sampleRate, f_cnt_t maxFrameCount) -> bool
{
	(void)maxFrameCount;
	const auto rate = static_cast<sample_rate_t>(sampleRate);
	if (rate != m_sampleRate)
	{
		// (synthesized again at the new rate)
		m_sampleRate = rate;
		loadKit();
	}
	return true;
}




void DrumMachinesInstrument::reset()
{
	const auto lock = QMutexLocker{&m_soundsMutex};
	for (auto& voice : m_voices) { voice = Voice{}; }
}




void DrumMachinesInstrument::noteOn(int note, float velocity)
{
	if (note < 0 || note >= static_cast<int>(m_soundOfNote.size())) { return; }
	const auto sound = m_soundOfNote[note];
	if (sound < 0 || m_sounds[sound].frames.empty()) { return; }

	// A hi-hat chokes the other hi-hats that ring (fading out, not to click)
	if (const auto choke = m_sounds[sound].choke; choke != 0)
	{
		for (auto& voice : m_voices)
		{
			if (voice.sound >= 0 && voice.sound != sound && m_sounds[voice.sound].choke == choke && voice.fade == 0)
			{
				voice.fade = ChokeFrames;
			}
		}
	}

	// A free voice, else the one that has played longest
	auto* chosen = &m_voices.front();
	for (auto& voice : m_voices)
	{
		if (voice.sound < 0) { chosen = &voice; break; }
		if (voice.position > chosen->position) { chosen = &voice; }
	}
	*chosen = Voice{sound, 0, velocity, 0};
	if (sound < MaxPieces) { m_hits[sound].store(static_cast<int>(velocity * 127.f + 0.5f), std::memory_order_relaxed); }
}




void DrumMachinesInstrument::render(std::span<SampleFrame> out)
{
	for (auto& voice : m_voices)
	{
		if (voice.sound < 0) { continue; }
		const auto& frames = m_sounds[voice.sound].frames;
		for (auto& frame : out)
		{
			if (voice.position >= frames.size()) { voice = Voice{}; break; }
			auto gain = voice.gain;
			// (its first half millisecond fades in: DrumSynth's pieces start on a step, which clicks)
			if (voice.position < m_fadeInFrames)
			{
				gain *= static_cast<float>(voice.position + 1) / static_cast<float>(m_fadeInFrames);
			}
			if (voice.fade > 0)
			{
				gain *= static_cast<float>(voice.fade) / ChokeFrames;
				if (--voice.fade == 0) { voice = Voice{}; break; }
			}
			frame += frames[voice.position] * gain;
			++voice.position;
		}
	}
}




void DrumMachinesInstrument::playImpl(std::span<SampleFrame> out, InputEvents events)
{
	// The kit may be being replaced; while exporting, wait for it
	if (!m_soundsMutex.tryLock(Engine::getSong()->isExporting() ? -1 : 0)) { return; }

	const auto frames = static_cast<f_cnt_t>(out.size());
	auto position = f_cnt_t{0};
	for (const auto& event : events)
	{
		const auto at = std::clamp<f_cnt_t>(event.time, position, frames);
		render(out.subspan(position, at - position));
		position = at;
		switch (event.type)
		{
			case PluginEvent::Type::NoteOn:
				noteOn(event.key, static_cast<float>(std::clamp(event.value, 0.0, 1.0)));
				break;
			case PluginEvent::Type::NoteChoke:
				for (auto& voice : m_voices) { voice = Voice{}; }
				break;
			default:
				// (a drum plays to its end: note-offs do not stop it)
				break;
		}
	}
	render(out.subspan(position, frames - position));
	m_soundsMutex.unlock();
}




auto DrumMachinesInstrument::takeHits() -> std::vector<int>
{
	auto hits = std::vector<int>(MaxPieces);
	for (int i = 0; i < MaxPieces; ++i) { hits[i] = m_hits[i].exchange(0, std::memory_order_relaxed); }
	return hits;
}




void DrumMachinesInstrument::saveSettings(QDomDocument& doc, QDomElement& elem)
{
	m_kitModel.saveSettings(doc, elem, "kit");
}




void DrumMachinesInstrument::loadSettings(const QDomElement& elem)
{
	m_kitModel.loadSettings(elem, "kit");
	loadKit();
}




QString DrumMachinesInstrument::nodeName() const
{
	return drummachines_plugin_descriptor.name;
}




gui::PluginView* DrumMachinesInstrument::instantiateView(QWidget* parent)
{
	return new gui::DrumMachinesView(this, parent);
}




namespace gui
{

namespace
{

constexpr int ControlsHeight = 34;
constexpr int PadsPerRow = 8;

} // namespace


DrumMachinesView::DrumMachinesView(DrumMachinesInstrument* instrument, QWidget* parent) :
	InstrumentView(instrument, parent)
{
	setAutoFillBackground(true);
	auto pal = palette();
	pal.setColor(QPalette::Window, QColor{0x16, 0x18, 0x1b});
	setPalette(pal);
	setMinimumSize(420, ControlsHeight + 200);

	m_kitBox = new ComboBox(this, tr("Drum machine"));
	m_kitBox->setGeometry(8, 6, 230, ComboBox::DEFAULT_HEIGHT);
	setToolTip(tr("Click a pad to play it"));

	startTimer(33);
	modelChanged();
}




void DrumMachinesView::modelChanged()
{
	auto* instrument = castModel<DrumMachinesInstrument>();
	m_kitBox->setModel(&instrument->kitModel());
	// (here, not in the constructor: setting the model again disconnects the model's signals from this view)
	connect(instrument, &DrumMachinesInstrument::kitChanged, this, qOverload<>(&QWidget::update),
		Qt::UniqueConnection);
	update();
}




auto DrumMachinesView::machineRect() const -> QRect
{
	return rect().adjusted(8, ControlsHeight + 4, -8, -8);
}




auto DrumMachinesView::hasPhoto() const -> bool
{
	return !m_photo.isNull() && m_photoKit == castModel<DrumMachinesInstrument>()->loadedKit();
}




auto DrumMachinesView::photoRect() const -> QRect
{
	// The whole machine's area: the photo fills it (cropped to its proportions), the pads over its lower part
	return machineRect();
}




auto DrumMachinesView::padsRect(int count) const -> QRect
{
	// Over the photo: a band along its bottom, as high as the rows of pads need (a fifth of the width each, at most)
	const auto machine = machineRect();
	const auto rows = (count + PadsPerRow - 1) / PadsPerRow;
	const auto padHeight = std::min(56, machine.width() / (PadsPerRow + 2));
	const auto height = rows * padHeight + (rows - 1) * 8 + 24;
	return {machine.left(), machine.bottom() - height, machine.width(), height + 1};
}




auto DrumMachinesView::padRect(int index, int count) const -> QRect
{
	// The pads in rows of up to eight, under the machine's name strip (under its photo, when it has one)
	const auto machine = machineRect();
	const auto rows = (count + PadsPerRow - 1) / PadsPerRow;
	const auto columns = std::min(count, PadsPerRow);
	const auto area = hasPhoto() ? padsRect(count).adjusted(14, 12, -14, -12) : machine.adjusted(18, 64, -18, -22);
	const auto gap = 8;
	const auto width = (area.width() - gap * (columns - 1)) / std::max(columns, 1);
	const auto height = std::min(width, (area.height() - gap * (rows - 1)) / std::max(rows, 1));
	const auto row = index / PadsPerRow;
	const auto column = index % PadsPerRow;
	const auto top = area.top() + (area.height() - (height * rows + gap * (rows - 1))) / 2;
	return {area.left() + column * (width + gap), top + row * (height + gap), width, height};
}




auto DrumMachinesView::padAt(const QPoint& pos) const -> int
{
	const auto kit = castModel<DrumMachinesInstrument>()->loadedKit();
	if (kit < 0) { return -1; }
	const auto count = static_cast<int>(DrumMachinesInstrument::kits()[kit].pieces.size());
	for (int i = 0; i < count; ++i)
	{
		if (padRect(i, count).contains(pos)) { return i; }
	}
	return -1;
}




void DrumMachinesView::mousePressEvent(QMouseEvent* event)
{
	if (event->button() != Qt::LeftButton) { return InstrumentView::mousePressEvent(event); }
	const auto pad = padAt(event->position().toPoint());
	const auto kit = castModel<DrumMachinesInstrument>()->loadedKit();
	auto* track = castModel<DrumMachinesInstrument>()->instrumentTrack();
	if (pad < 0 || kit < 0 || !track) { return; }
	m_heldNote = DrumMachinesInstrument::kits()[kit].pieces[pad].note;
	track->processInEvent(MidiEvent(MidiNoteOn, -1, m_heldNote, track->midiPort()->baseVelocity()));
}




void DrumMachinesView::mouseReleaseEvent(QMouseEvent* event)
{
	if (m_heldNote < 0) { return InstrumentView::mouseReleaseEvent(event); }
	if (auto* track = castModel<DrumMachinesInstrument>()->instrumentTrack())
	{
		track->processInEvent(MidiEvent(MidiNoteOff, -1, m_heldNote, 0));
	}
	m_heldNote = -1;
}




void DrumMachinesView::timerEvent(QTimerEvent*)
{
	const auto hits = castModel<DrumMachinesInstrument>()->takeHits();
	m_glow.resize(hits.size(), 0.f);
	auto changed = false;
	for (std::size_t i = 0; i < hits.size(); ++i)
	{
		const auto level = std::max(m_glow[i] * 0.85f, hits[i] / 127.f);
		const auto shown = level < 0.02f ? 0.f : level;
		changed = changed || shown != m_glow[i];
		m_glow[i] = shown;
	}
	if (changed) { update(); }
}




void DrumMachinesView::paintEvent(QPaintEvent*)
{
	auto p = QPainter{this};
	p.setRenderHint(QPainter::Antialiasing);
	const auto kitIndex = castModel<DrumMachinesInstrument>()->loadedKit();
	const auto machine = machineRect();
	if (kitIndex < 0)
	{
		p.setPen(QColor{0xe0, 0x60, 0x50});
		p.drawText(machine, Qt::AlignCenter, tr("The drum machine could not be loaded"));
		return;
	}
	const auto& kit = DrumMachinesInstrument::kits()[kitIndex];
	const auto& look = kit.look;

	// The body, with a light on top and a shadow beneath
	auto body = QPainterPath{};
	body.addRoundedRect(QRectF(machine), 10, 10);
	auto shade = QLinearGradient{QPointF(machine.topLeft()), QPointF(machine.bottomLeft())};
	shade.setColorAt(0, look.body.lighter(118));
	shade.setColorAt(1, look.body.darker(125));
	p.fillPath(body, shade);
	p.setPen(QPen{look.body.darker(160), 1.5});
	p.drawPath(body);

	// The machine's photo, its credit on it (its license asks for it); the pads under it
	if (m_photoKit != kitIndex)
	{
		m_photoKit = kitIndex;
		m_photo = kit.photo ? PLUGIN_NAME::getIconPixmap(kit.photo) : QPixmap{};
	}
	if (hasPhoto())
	{
		// The photo filling the machine's area (its middle, cropped to the area's proportions)
		const auto photo = photoRect();
		auto frame = QPainterPath{};
		frame.addRoundedRect(QRectF(photo), 10, 10);
		const auto source = QSizeF(m_photo.size());
		const auto scale = std::max(photo.width() / source.width(), photo.height() / source.height());
		const auto cropped = QSizeF(photo.width() / scale, photo.height() / scale);
		const auto from = QRectF(QPointF((source.width() - cropped.width()) / 2, (source.height() - cropped.height()) / 2),
			cropped);
		p.save();
		p.setClipPath(frame);
		p.setRenderHint(QPainter::SmoothPixmapTransform);
		p.drawPixmap(QRectF(photo), m_photo, from);
		// The band the pads are on: dark, fading in from above
		const auto band = QRectF(padsRect(static_cast<int>(kit.pieces.size())));
		auto shade = QLinearGradient{band.topLeft() - QPointF(0, 24), band.topLeft() + QPointF(0, 12)};
		shade.setColorAt(0, QColor{0, 0, 0, 0});
		shade.setColorAt(1, QColor{0, 0, 0, 170});
		p.fillRect(band.adjusted(0, -24, 0, 0), shade);
		p.restore();
		p.setPen(QPen{QColor{0, 0, 0, 160}, 1.5});
		p.drawPath(frame);
		// Its credit (its license asks for it), on its upper right corner
		auto credit = adjustedToPixelSize(font(), 9);
		p.setFont(credit);
		const auto text = tr(kit.credit);
		const auto metrics = QFontMetrics{credit};
		const auto width = metrics.horizontalAdvance(text) + 10.0;
		const auto box = QRectF(photo.right() - width - 8, photo.top() + 8, width, metrics.height() + 4);
		auto pill = QPainterPath{};
		pill.addRoundedRect(box, 4, 4);
		p.fillPath(pill, QColor{0, 0, 0, 140});
		p.setPen(QColor{0xee, 0xee, 0xee});
		p.drawText(box, Qt::AlignCenter, text);
	}
	else
	{
	// The panel: the machine's name, and its accent stripe
	const auto panel = QRectF(machine.adjusted(12, 12, -12, -12));
	auto panelPath = QPainterPath{};
	panelPath.addRoundedRect(panel, 6, 6);
	p.fillPath(panelPath, look.panel);
	p.fillRect(QRectF(panel.left() + 6, panel.top() + 40, panel.width() - 12, 3), look.accent);
	auto title = adjustedToPixelSize(font(), 22);
	title.setBold(true);
	title.setLetterSpacing(QFont::AbsoluteSpacing, 1.5);
	p.setFont(title);
	p.setPen(look.text);
	p.drawText(QRectF(panel.left() + 14, panel.top() + 6, panel.width() - 28, 32), Qt::AlignLeft | Qt::AlignVCenter,
		kit.name);
	auto caption = adjustedToPixelSize(font(), 10);
	p.setFont(caption);
	p.setPen(look.text.darker(130));
	p.drawText(QRectF(panel.left() + 14, panel.top() + 6, panel.width() - 28, 32), Qt::AlignRight | Qt::AlignVCenter,
		tr("DRUM MACHINE"));
	}

	// The pads: lit when hit
	const auto count = static_cast<int>(kit.pieces.size());
	const auto label = [&] {
		auto f = adjustedToPixelSize(font(), 13);
		f.setBold(true);
		return f;
	}();
	const auto noteFont = adjustedToPixelSize(font(), 9);
	for (int i = 0; i < count; ++i)
	{
		const auto& piece = kit.pieces[i];
		const auto pad = QRectF(padRect(i, count));
		const auto glow = i < static_cast<int>(m_glow.size()) ? m_glow[i] : 0.f;
		const auto color = look.pads[i % look.pads.size()];
		auto shape = QPainterPath{};
		shape.addRoundedRect(pad, 5, 5);
		if (glow > 0.f)
		{
			// (a halo around it)
			for (int ring = 3; ring >= 1; --ring)
			{
				auto halo = QPainterPath{};
				halo.addRoundedRect(pad.adjusted(-ring * 2, -ring * 2, ring * 2, ring * 2), 7, 7);
				auto c = color.lighter(140);
				c.setAlphaF(0.18f * glow / ring);
				p.fillPath(halo, c);
			}
		}
		auto fill = QLinearGradient{pad.topLeft(), pad.bottomLeft()};
		const auto lit = QColor::fromRgbF(
			std::min(1.f, color.redF() + (1 - color.redF()) * 0.6f * glow),
			std::min(1.f, color.greenF() + (1 - color.greenF()) * 0.6f * glow),
			std::min(1.f, color.blueF() + (1 - color.blueF()) * 0.6f * glow));
		fill.setColorAt(0, (glow > 0 ? lit : color).lighter(112));
		fill.setColorAt(1, (glow > 0 ? lit : color.darker(125)));
		p.fillPath(shape, fill);
		p.setPen(QPen{color.darker(170), 1});
		p.drawPath(shape);

		const auto ink = color.lightnessF() > 0.55 ? QColor{0x1e, 0x1e, 0x1e} : QColor{0xf2, 0xf2, 0xf2};
		p.setPen(ink);
		p.setFont(label);
		p.drawText(pad.adjusted(0, 0, 0, -pad.height() * 0.25), Qt::AlignCenter, piece.label);
		p.setFont(noteFont);
		p.drawText(pad.adjusted(0, pad.height() * 0.5, 0, -4), Qt::AlignHCenter | Qt::AlignBottom,
			QString::number(piece.note));
	}
}

} // namespace gui

} // namespace lmms
