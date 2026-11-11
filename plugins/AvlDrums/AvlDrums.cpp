/*
 * AvlDrums.cpp - the AVL drum kits as an instrument
 *
 * This file is part of LinuxMusic, a fork of LMMS - https://lmms.io
 *
 * The note map and the output layout follow avldrums.lv2 by Robin Gareus, so that MIDI drum parts written for
 * it play the same. The drum kits are by Glen MacArthur, see data/avldrums/README.
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

#include "AvlDrums.h"

#include <algorithm>

#include <QDomElement>
#include <QFileInfo>
#include <QMutexLocker>
#include <QPainter>

#include <fluidsynth.h>

#include "AudioEngine.h"
#include "ComboBox.h"
#include "ConfigManager.h"
#include "Engine.h"
#include "FontHelper.h"
#include "Host.h"
#include "InstrumentPlayHandle.h"
#include "InstrumentTrack.h"
#include "LedCheckBox.h"
#include "Midi.h"
#include "Song.h"

#include "plugin_export.h"

namespace lmms
{

extern "C"
{

Plugin::Descriptor PLUGIN_EXPORT avldrums_plugin_descriptor =
{
	LMMS_STRINGIFY(PLUGIN_NAME),
	"AVL Drums",
	QT_TRANSLATE_NOOP("PluginBrowser", "Acoustic drum kits by Glen MacArthur, with an output per drum piece"),
	"LinuxMusic; drum kits by Glen MacArthur; note map of avldrums.lv2 by Robin Gareus",
	0x0100,
	Plugin::Type::Instrument,
	new PixmapLoader("lmms-plugin-logo"),
	nullptr,
	nullptr,
};

PLUGIN_EXPORT Plugin* lmms_plugin_main(Model* parent, void*)
{
	return new AvlDrumsInstrument(static_cast<InstrumentTrack*>(parent));
}

} // extern "C"


namespace
{

// The output groups of the multi-output mode: FluidSynth renders MIDI channel N into its audio group N
enum MultiGroup : std::uint8_t { Kick, Snare, HiHat, Tom, FloorTom, Cymbals, Percussion, MultiGroupCount };
constexpr auto MultiOutputChannels = ch_cnt_t{9};
constexpr auto MonoGroups = 5; // Kick to FloorTom

//! Which output group (and MIDI channel) a note plays on, as in avldrums.lv2
auto groupOf(int note) -> std::uint8_t
{
	switch (note)
	{
		case 36: return Kick;
		case 37: case 38: case 40: return Snare;
		case 42: case 44: case 46: case 48: return HiHat;
		case 45: case 47: return Tom;
		case 41: case 43: return FloorTom;
		case 35: case 39: case 54: case 56: case 61: return Percussion;
		default: return Cymbals;
	}
}

constexpr auto PieceVoices = 26;

} // namespace


const std::array<AvlDrumsInstrument::Kit, 5> AvlDrumsInstrument::Kits = {{
	{"Black Pearl", "Black_Pearl_4_LV2.sf2", true},
	{"Red Zeppelin", "Red_Zeppelin_4_LV2.sf2", true},
	{"Blonde Bop", "Blonde_Bop_LV2.sf2", true},
	{"Blonde Bop Hot Rod", "Blonde_Bop_HR_LV2.sf2", true},
	{"Buskman's Holiday (percussion)", "Buskmans_Holiday_LV2.sf2", false},
}};




AvlDrumsInstrument::AvlDrumsInstrument(InstrumentTrack* track) :
	Instrument(&avldrums_plugin_descriptor, track, nullptr,
		Flag::IsSingleStreamed | Flag::IsMidiBased | Flag::IsNotBendable),
	m_kitModel(this, tr("Drum kit")),
	m_multiOutputModel(false, this, tr("Output per drum piece")),
	m_sampleRate(Engine::audioEngine()->outputSampleRate())
{
	for (const auto& kit : Kits) { m_kitModel.addItem(kit.name); }

	// Loading a kit takes a while, and the models may change from other threads (automation), so the kit is
	// loaded in the main thread afterwards
	connect(&m_kitModel, &ComboBoxModel::dataChanged, this, &AvlDrumsInstrument::loadKit, Qt::QueuedConnection);
	connect(&m_multiOutputModel, &BoolModel::dataChanged, this, &AvlDrumsInstrument::loadKit,
		Qt::QueuedConnection);

	loadKit();

	// The track processes the instrument once per period through this play handle
	Engine::audioEngine()->addPlayHandle(new InstrumentPlayHandle(this, track));
}




AvlDrumsInstrument::~AvlDrumsInstrument()
{
	Engine::audioEngine()->removePlayHandlesOfTypes(instrumentTrack(),
		PlayHandle::Type::NotePlayHandle | PlayHandle::Type::InstrumentPlayHandle);
	destroySynth();
}




void AvlDrumsInstrument::destroySynth()
{
	const auto lock = QMutexLocker{&m_synthMutex};
	if (m_synth) { delete_fluid_synth(m_synth); }
	if (m_settings) { delete_fluid_settings(m_settings); }
	m_synth = nullptr;
	m_settings = nullptr;
	m_loaded = false;
}




auto AvlDrumsInstrument::multiOutputActive() const -> bool
{
	return m_synthMultiOutput;
}




void AvlDrumsInstrument::loadKit()
{
	const auto index = std::clamp(m_kitModel.value(), 0, static_cast<int>(Kits.size()) - 1);
	const auto& kit = Kits[index];
	const bool multiOutput = m_multiOutputModel.value() && kit.hasMultiOutput;

	// Nothing to do if this kit plays already as asked
	if (m_loaded && index == m_loadedKit && multiOutput == m_synthMultiOutput && m_sampleRate == m_loadedSampleRate)
	{
		return;
	}

	const auto path = QFileInfo{ConfigManager::inst()->dataDir() + "avldrums/" + kit.file};

	auto settings = new_fluid_settings();
	fluid_settings_setnum(settings, "synth.sample-rate", m_sampleRate);
	fluid_settings_setint(settings, "synth.threadsafe-api", 0); // m_synthMutex guards the synth
	fluid_settings_setint(settings, "synth.reverb.active", 0); // dry kits, reverb belongs in the mixer
	fluid_settings_setint(settings, "synth.chorus.active", 0);
	fluid_settings_setint(settings, "synth.polyphony", PieceVoices * 16);
	fluid_settings_setint(settings, "synth.audio-channels", multiOutput ? MultiGroupCount : 1);
	fluid_settings_setint(settings, "synth.audio-groups", multiOutput ? MultiGroupCount : 1);

	auto synth = new_fluid_synth(settings);
	bool loaded = false;
	if (synth && path.exists())
	{
		fluid_synth_set_gain(synth, 1.f);
		const auto id = fluid_synth_sfload(synth, path.absoluteFilePath().toUtf8().constData(), 1);
		auto font = id != FLUID_FAILED ? fluid_synth_get_sfont_by_id(synth, id) : nullptr;
		if (font)
		{
			// Every MIDI channel plays the kit's preset
			fluid_sfont_iteration_start(font);
			if (auto preset = fluid_sfont_iteration_next(font))
			{
				for (int channel = 0; channel < 16; ++channel)
				{
					fluid_synth_program_select(synth, channel, id,
						fluid_preset_get_banknum(preset), fluid_preset_get_num(preset));
				}
				loaded = true;
			}
		}

		if (multiOutput)
		{
			// The mono pieces are panned hard left, so that the left channel of their group has all of them
			for (int channel = 0; channel < MonoGroups; ++channel)
			{
				fluid_synth_cc(synth, channel, 10, 0);
				fluid_synth_cc(synth, channel, 42, 0);
			}
		}
		for (int channel = 0; channel < Percussion; ++channel)
		{
			fluid_synth_set_channel_type(synth, channel, CHANNEL_TYPE_DRUM);
		}
	}

	if (!loaded)
	{
		if (synth) { delete_fluid_synth(synth); }
		delete_fluid_settings(settings);
		synth = nullptr;
		settings = nullptr;
		Engine::getSong()->collectError(tr("The drum kit \"%1\" could not be loaded from %2.")
			.arg(kit.name, path.absoluteFilePath()));
	}

	const bool layoutChanged = multiOutput != m_synthMultiOutput;
	{
		// Swap in the new synth; the old one is deleted outside of the lock
		const auto lock = QMutexLocker{&m_synthMutex};
		std::swap(synth, m_synth);
		std::swap(settings, m_settings);
		m_synthMultiOutput = multiOutput;
		m_loaded = loaded;
		m_loadedKit = index;
		m_loadedSampleRate = m_sampleRate;
	}
	if (synth) { delete_fluid_synth(synth); }
	if (settings) { delete_fluid_settings(settings); }

	// Another output layout needs other buffers and pins
	if (layoutChanged && host()) { host()->rescanAudioPorts(Host::AudioPortsRescanFlag::ChannelCount); }

	emit kitChanged();
}




auto AvlDrumsInstrument::activate(double sampleRate, f_cnt_t maxFrameCount) -> bool
{
	(void)maxFrameCount;
	const auto rate = static_cast<sample_rate_t>(sampleRate);
	if (rate != m_sampleRate)
	{
		m_sampleRate = rate;
		loadKit();
	}
	return true;
}




auto AvlDrumsInstrument::channelCount(bool isInput) const -> ch_cnt_t
{
	if (isInput) { return 0; }
	return multiOutputActive() ? MultiOutputChannels : DEFAULT_CHANNELS;
}




auto AvlDrumsInstrument::channelGroupCount(bool isInput) const -> group_cnt_t
{
	if (isInput) { return 0; }
	return multiOutputActive() ? MultiGroupCount : 1;
}




auto AvlDrumsInstrument::getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup& group) const
	-> ch_cnt_t
{
	(void)group;
	if (isInput) { return 0; }
	if (!multiOutputActive()) { return DEFAULT_CHANNELS; }

	// TODO: Name the groups (kick, snare...) once channel groups have names
	return index < MonoGroups ? 1 : 2;
}




auto AvlDrumsInstrument::getInplacePair(ch_cnt_t inputIndex) const -> std::optional<ch_cnt_t>
{
	(void)inputIndex;
	return std::nullopt;
}




void AvlDrumsInstrument::applyEvent(const PluginEvent& event)
{
	switch (event.type)
	{
		case PluginEvent::Type::NoteOn:
		{
			const auto key = event.key;
			const auto velocity = std::clamp(static_cast<int>(event.value * 127.0 + 0.5), 1, 127);
			fluid_synth_noteon(m_synth, groupOf(key), key, velocity);
			const auto piece = key - FirstNote;
			if (piece >= 0 && piece < PieceCount) { m_hits[piece].store(velocity, std::memory_order_relaxed); }
			break;
		}
		case PluginEvent::Type::NoteOff:
			if (event.key >= 0) { fluid_synth_noteoff(m_synth, groupOf(event.key), event.key); }
			break;
		case PluginEvent::Type::NoteChoke:
			for (int channel = 0; channel < MultiGroupCount; ++channel) { fluid_synth_all_sounds_off(m_synth, channel); }
			break;
		case PluginEvent::Type::Midi:
		{
			// Only control changes, and not the ones that would change the kit's panning or hold notes, as in
			// avldrums.lv2; no programs, pitch bend or pressure for a drum kit
			if ((event.midi[0] & 0xf0) != MidiControlChange) { break; }
			switch (event.midi[1])
			{
				case 10: case 42: case 64: case 66: case 68: break;
				default:
					for (int channel = 0; channel < MultiGroupCount; ++channel)
					{
						fluid_synth_cc(m_synth, channel, event.midi[1], event.midi[2]);
					}
			}
			break;
		}
		default:
			break;
	}
}




void AvlDrumsInstrument::render(AudioBuffer& out, f_cnt_t offset, f_cnt_t frames)
{
	if (frames <= 0) { return; }

	if (!m_synthMultiOutput)
	{
		fluid_synth_write_float(m_synth, frames, out.buffer(0).data(), offset, 1, out.buffer(1).data(), offset, 1);
		return;
	}

	// FluidSynth adds to the buffers; the right channels of the mono groups are not needed
	auto at = [&](ch_cnt_t ch) { return out.buffer(ch).data() + offset; };
	auto buffers = std::array<float*, 2 * MultiGroupCount>{
		at(0), nullptr, at(1), nullptr, at(2), nullptr, at(3), nullptr, at(4), nullptr,
		at(5), at(6), at(7), at(8)
	};
	for (const auto buffer : buffers)
	{
		if (buffer) { std::fill_n(buffer, frames, 0.f); }
	}
	fluid_synth_process(m_synth, frames, 0, nullptr, static_cast<int>(buffers.size()), buffers.data());
}




auto AvlDrumsInstrument::process(ProcessContext& context) -> ProcessStatus
{
	auto& out = context.out();
	const auto frames = out.frames();

	// The kit may be being replaced; while exporting, wait for it
	if (!m_synthMutex.tryLock(Engine::getSong()->isExporting() ? -1 : 0))
	{
		out.silenceAllChannels();
		return ProcessStatus::Continue;
	}

	// Until the host has set up buffers for a new output layout, there is nothing to render into
	if (!m_synth || out.channels() != channelCount(false))
	{
		m_synthMutex.unlock();
		out.silenceAllChannels();
		return ProcessStatus::Continue;
	}

	// Render up to each event, so that it starts at its frame
	auto position = f_cnt_t{0};
	for (const auto& event : context.inEvents())
	{
		const auto at = std::clamp<f_cnt_t>(event.time, position, frames);
		render(out, position, at - position);
		position = at;
		applyEvent(event);
	}
	render(out, position, frames - position);

	m_synthMutex.unlock();
	return ProcessStatus::Continue;
}




auto AvlDrumsInstrument::takeHits() -> std::array<int, PieceCount>
{
	auto hits = std::array<int, PieceCount>{};
	for (int piece = 0; piece < PieceCount; ++piece)
	{
		hits[piece] = m_hits[piece].exchange(0, std::memory_order_relaxed);
	}
	return hits;
}




void AvlDrumsInstrument::saveSettings(QDomDocument& doc, QDomElement& elem)
{
	m_kitModel.saveSettings(doc, elem, "kit");
	m_multiOutputModel.saveSettings(doc, elem, "multioutput");
}




void AvlDrumsInstrument::loadSettings(const QDomElement& elem)
{
	m_kitModel.loadSettings(elem, "kit");
	m_multiOutputModel.loadSettings(elem, "multioutput");

	// Right away, so that a project renders with its kit; loadKit() is quick to skip a kit already loaded
	loadKit();
}




QString AvlDrumsInstrument::nodeName() const
{
	return avldrums_plugin_descriptor.name;
}




gui::PluginView* AvlDrumsInstrument::instantiateView(QWidget* parent)
{
	return new gui::AvlDrumsView(this, parent);
}




namespace gui
{

namespace
{

//! A drum piece drawn by the view: the notes that light it up
struct Pad
{
	const char* label;
	QRectF rect;
	std::initializer_list<int> notes;
	bool round;
};

const auto Pads = std::array<Pad, 11>{{
	{QT_TRANSLATE_NOOP("lmms::gui::AvlDrumsView", "Crash"), {14, 92, 58, 40}, {49, 50}, true},
	{QT_TRANSLATE_NOOP("lmms::gui::AvlDrumsView", "Splash"), {96, 86, 44, 30}, {55}, true},
	{QT_TRANSLATE_NOOP("lmms::gui::AvlDrumsView", "Crash 2"), {164, 92, 58, 40}, {57, 58}, true},
	{QT_TRANSLATE_NOOP("lmms::gui::AvlDrumsView", "Hi-hat"), {8, 140, 52, 36}, {42, 44, 46, 48}, true},
	{QT_TRANSLATE_NOOP("lmms::gui::AvlDrumsView", "Tom"), {76, 124, 44, 36}, {45, 47}, true},
	{QT_TRANSLATE_NOOP("lmms::gui::AvlDrumsView", "Crash 3"), {130, 122, 46, 30}, {60}, true},
	{QT_TRANSLATE_NOOP("lmms::gui::AvlDrumsView", "Ride"), {184, 140, 58, 40}, {51, 52, 53, 59}, true},
	{QT_TRANSLATE_NOOP("lmms::gui::AvlDrumsView", "Snare"), {44, 182, 52, 40}, {37, 38, 40}, true},
	{QT_TRANSLATE_NOOP("lmms::gui::AvlDrumsView", "Kick"), {100, 170, 58, 58}, {36}, true},
	{QT_TRANSLATE_NOOP("lmms::gui::AvlDrumsView", "Floor tom"), {164, 186, 52, 44}, {41, 43}, true},
	{QT_TRANSLATE_NOOP("lmms::gui::AvlDrumsView", "Percussion"), {8, 226, 234, 18}, {35, 39, 54, 56, 61}, false},
}};

} // namespace


AvlDrumsView::AvlDrumsView(AvlDrumsInstrument* instrument, QWidget* parent) :
	InstrumentViewFixedSize(instrument, parent)
{
	setAutoFillBackground(true);
	auto pal = palette();
	pal.setColor(QPalette::Window, QColor{0x1e, 0x21, 0x24});
	setPalette(pal);

	m_kitBox = new ComboBox(this, tr("Drum kit"));
	m_kitBox->setGeometry(10, 34, 230, ComboBox::DEFAULT_HEIGHT);

	m_multiOutputBox = new LedCheckBox(tr("Output per drum piece (9 channels)"), this, tr("Output per drum piece"),
		LedCheckBox::LedColor::Green);
	m_multiOutputBox->move(10, 62);
	m_multiOutputBox->setToolTip(tr("Kick, snare, hi-hat, tom and floor tom (mono), cymbals and percussion "
		"(stereo) get their own outputs, which the pin connector can route to the track's auxiliary channels"));

	connect(instrument, &AvlDrumsInstrument::kitChanged, this, [this] {
		updateMultiOutputAvailability();
		update();
	});

	startTimer(40);
	modelChanged();
}




void AvlDrumsView::modelChanged()
{
	auto drums = castModel<AvlDrumsInstrument>();
	m_kitBox->setModel(&drums->kitModel());
	m_multiOutputBox->setModel(&drums->multiOutputModel());
	updateMultiOutputAvailability();
}




void AvlDrumsView::updateMultiOutputAvailability()
{
	auto drums = castModel<AvlDrumsInstrument>();
	const auto kit = std::clamp(drums->kitModel().value(), 0, static_cast<int>(AvlDrumsInstrument::Kits.size()) - 1);
	m_multiOutputBox->setEnabled(AvlDrumsInstrument::Kits[kit].hasMultiOutput);
}




void AvlDrumsView::timerEvent(QTimerEvent*)
{
	const auto hits = castModel<AvlDrumsInstrument>()->takeHits();
	bool changed = false;
	for (int piece = 0; piece < AvlDrumsInstrument::PieceCount; ++piece)
	{
		const float level = std::max(m_glow[piece] * 0.82f, hits[piece] / 127.f);
		const float shown = level < 0.02f ? 0.f : level;
		changed = changed || shown != m_glow[piece];
		m_glow[piece] = shown;
	}
	if (changed) { update(); }
}




void AvlDrumsView::paintEvent(QPaintEvent*)
{
	auto p = QPainter{this};
	p.setRenderHint(QPainter::Antialiasing);

	p.setPen(QColor{0xe0, 0xe0, 0xe0});
	p.setFont(adjustedToPixelSize(font(), 16));
	p.drawText(QRect{10, 4, 230, 26}, Qt::AlignLeft | Qt::AlignVCenter, tr("AVL Drums"));

	auto drums = castModel<AvlDrumsInstrument>();
	if (!drums->isLoaded())
	{
		p.setPen(QColor{0xe0, 0x60, 0x50});
		p.setFont(adjustedToPixelSize(font(), 10));
		p.drawText(QRect{10, 120, 230, 40}, Qt::AlignCenter | Qt::TextWordWrap, tr("The drum kit could not be loaded"));
		return;
	}

	p.setFont(adjustedToPixelSize(font(), 9));
	for (const auto& pad : Pads)
	{
		float glow = 0.f;
		for (const int note : pad.notes)
		{
			const auto piece = note - AvlDrumsInstrument::FirstNote;
			if (piece >= 0 && piece < AvlDrumsInstrument::PieceCount) { glow = std::max(glow, m_glow[piece]); }
		}

		const auto idle = QColor{0x3a, 0x3f, 0x44};
		const auto hit = QColor{0x5d, 0xd2, 0x8a};
		const auto fill = QColor::fromRgbF(
			idle.redF() + (hit.redF() - idle.redF()) * glow,
			idle.greenF() + (hit.greenF() - idle.greenF()) * glow,
			idle.blueF() + (hit.blueF() - idle.blueF()) * glow);

		p.setPen(QPen{QColor{0x55, 0x5b, 0x61}, 1.5});
		p.setBrush(fill);
		if (pad.round) { p.drawEllipse(pad.rect); }
		else { p.drawRoundedRect(pad.rect, 4, 4); }

		p.setPen(glow > 0.5f ? QColor{0x10, 0x14, 0x18} : QColor{0xc8, 0xcc, 0xd0});
		p.drawText(pad.rect, Qt::AlignCenter, tr(pad.label));
	}
}

} // namespace gui

} // namespace lmms
