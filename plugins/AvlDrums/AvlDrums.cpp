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
#include <QMouseEvent>
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
#include "MidiEvent.h"
#include "MidiPort.h"
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
	{"Black Pearl", "Black_Pearl_4_LV2.sf2", "black_pearl.png", "black_pearl.map.png", true},
	{"Red Zeppelin", "Red_Zeppelin_4_LV2.sf2", "red_zeppelin.png", "red_zeppelin.map.png", true},
	{"Blonde Bop", "Blonde_Bop_LV2.sf2", "blonde_bop.png", "blonde_bop.map.png", true},
	{"Blonde Bop Hot Rod", "Blonde_Bop_HR_LV2.sf2", "blonde_bop_hr.png", "blonde_bop.map.png", true},
	{"Buskman's Holiday (percussion)", "Buskmans_Holiday_LV2.sf2", "buskman.png", "buskman.map.png", false},
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

constexpr int ControlsHeight = 34;
constexpr int DrumPieces = 26; // notes 36 to 61

/**
 * The maps of avldrums.lv2 color each piece's area: a red value R means color index (R - 10) / 9, and this table
 * gives the piece (note - 36) of each color index. Neighboring color indexes of the same drum are its different
 * hits (e.g. the center and the edge of the snare).
 */
constexpr auto MapPieces = std::array<int, DrumPieces>{
	0, 2, 4, 1, 3, 12, 10, 6, 7, 5, 8, 11, 9, 13, 14, 16, 23, 15, 17, 18, 22, 21, 20, 24, 19, 25
};

auto mapPiece(QRgb pixel) -> int
{
	const int red = qRed(pixel);
	if (red < 10) { return -1; }
	const int color = (red - 10) / 9;
	return color < DrumPieces ? MapPieces[color] : -1;
}

} // namespace


AvlDrumsView::AvlDrumsView(AvlDrumsInstrument* instrument, QWidget* parent) :
	InstrumentView(instrument, parent)
{
	setAutoFillBackground(true);
	auto pal = palette();
	pal.setColor(QPalette::Window, QColor{0x16, 0x18, 0x1b});
	setPalette(pal);
	setMinimumSize(420, ControlsHeight + 200);

	m_kitBox = new ComboBox(this, tr("Drum kit"));
	m_kitBox->setGeometry(8, 6, 230, ComboBox::DEFAULT_HEIGHT);

	m_multiOutputBox = new LedCheckBox(tr("Output per drum piece (9 channels)"), this, tr("Output per drum piece"),
		LedCheckBox::LedColor::Green);
	m_multiOutputBox->move(250, 10);
	m_multiOutputBox->setToolTip(tr("Kick, snare, hi-hat, tom and floor tom (mono), cymbals and percussion "
		"(stereo) get their own outputs, which the pin connector can route to the track's auxiliary channels"));

	setToolTip(tr("Click a drum to play it"));

	connect(instrument, &AvlDrumsInstrument::kitChanged, this, [this] {
		updateMultiOutputAvailability();
		loadKitImages();
		update();
	});

	startTimer(33);
	modelChanged();
}




void AvlDrumsView::modelChanged()
{
	auto drums = castModel<AvlDrumsInstrument>();
	m_kitBox->setModel(&drums->kitModel());
	m_multiOutputBox->setModel(&drums->multiOutputModel());
	updateMultiOutputAvailability();
	loadKitImages();
}




void AvlDrumsView::updateMultiOutputAvailability()
{
	auto drums = castModel<AvlDrumsInstrument>();
	const auto kit = std::clamp(drums->kitModel().value(), 0, static_cast<int>(AvlDrumsInstrument::Kits.size()) - 1);
	m_multiOutputBox->setEnabled(AvlDrumsInstrument::Kits[kit].hasMultiOutput);
}




void AvlDrumsView::loadKitImages()
{
	auto drums = castModel<AvlDrumsInstrument>();
	const auto index = std::clamp(drums->kitModel().value(), 0, static_cast<int>(AvlDrumsInstrument::Kits.size()) - 1);
	if (index == m_imagesKit) { return; }
	m_imagesKit = index;

	const auto& kit = AvlDrumsInstrument::Kits[index];
	const auto dir = ConfigManager::inst()->dataDir() + "avldrums/images/";
	m_photo = QImage{dir + kit.photo};
	m_map = QImage{dir + kit.map}.convertToFormat(QImage::Format_RGB32);

	// Each piece's area of the map, as a highlight over the photo
	auto bounds = std::array<QRect, AvlDrumsInstrument::PieceCount>{};
	for (int y = 0; y < m_map.height(); ++y)
	{
		const auto line = reinterpret_cast<const QRgb*>(m_map.constScanLine(y));
		for (int x = 0; x < m_map.width(); ++x)
		{
			const int piece = mapPiece(line[x]);
			if (piece < 0) { continue; }
			auto& rect = bounds[piece + 36 - AvlDrumsInstrument::FirstNote];
			rect = rect.united(QRect{x, y, 1, 1});
		}
	}

	const auto glow = QColor{255, 214, 120};
	for (int p = 0; p < AvlDrumsInstrument::PieceCount; ++p)
	{
		auto& highlight = m_highlights[p];
		highlight.rect = bounds[p];
		highlight.image = QImage{};
		if (bounds[p].isEmpty()) { continue; }

		highlight.image = QImage{bounds[p].size(), QImage::Format_ARGB32_Premultiplied};
		highlight.image.fill(Qt::transparent);
		for (int y = 0; y < bounds[p].height(); ++y)
		{
			const auto src = reinterpret_cast<const QRgb*>(m_map.constScanLine(bounds[p].y() + y)) + bounds[p].x();
			const auto dst = reinterpret_cast<QRgb*>(highlight.image.scanLine(y));
			for (int x = 0; x < bounds[p].width(); ++x)
			{
				if (mapPiece(src[x]) + 36 - AvlDrumsInstrument::FirstNote != p) { continue; }
				dst[x] = qPremultiply(qRgba(glow.red(), glow.green(), glow.blue(), 170));
			}
		}
	}
}




auto AvlDrumsView::photoRect() const -> QRect
{
	const auto area = QRect{0, ControlsHeight, width(), height() - ControlsHeight};
	if (m_photo.isNull() || area.width() <= 0 || area.height() <= 0) { return area; }

	auto size = m_photo.size().scaled(area.size(), Qt::KeepAspectRatio);
	return QRect{QPoint{area.x() + (area.width() - size.width()) / 2, area.y() + (area.height() - size.height()) / 2},
		size};
}




auto AvlDrumsView::noteAt(const QPoint& pos) const -> int
{
	const auto rect = photoRect();
	if (m_map.isNull() || !rect.contains(pos)) { return -1; }

	const int x = (pos.x() - rect.x()) * m_map.width() / rect.width();
	const int y = (pos.y() - rect.y()) * m_map.height() / rect.height();
	if (!m_map.rect().contains(x, y)) { return -1; }

	const int piece = mapPiece(m_map.pixel(x, y));
	return piece < 0 ? -1 : 36 + piece;
}




void AvlDrumsView::mousePressEvent(QMouseEvent* event)
{
	if (event->button() != Qt::LeftButton) { return InstrumentView::mousePressEvent(event); }

	const int note = noteAt(event->position().toPoint());
	if (note < 0) { return; }

	auto track = castModel<AvlDrumsInstrument>()->instrumentTrack();
	if (!track) { return; }

	m_heldNote = note;
	track->processInEvent(MidiEvent(MidiNoteOn, -1, note, track->midiPort()->baseVelocity()));
}




void AvlDrumsView::mouseReleaseEvent(QMouseEvent* event)
{
	if (m_heldNote < 0) { return InstrumentView::mouseReleaseEvent(event); }

	if (auto track = castModel<AvlDrumsInstrument>()->instrumentTrack())
	{
		track->processInEvent(MidiEvent(MidiNoteOff, -1, m_heldNote, 0));
	}
	m_heldNote = -1;
}




void AvlDrumsView::timerEvent(QTimerEvent*)
{
	const auto hits = castModel<AvlDrumsInstrument>()->takeHits();
	bool changed = false;
	for (int piece = 0; piece < AvlDrumsInstrument::PieceCount; ++piece)
	{
		const float level = std::max(m_glow[piece] * 0.85f, hits[piece] / 127.f);
		const float shown = level < 0.02f ? 0.f : level;
		changed = changed || shown != m_glow[piece];
		m_glow[piece] = shown;
	}
	if (changed) { update(); }
}




void AvlDrumsView::paintEvent(QPaintEvent*)
{
	auto p = QPainter{this};
	p.setRenderHint(QPainter::SmoothPixmapTransform);

	auto drums = castModel<AvlDrumsInstrument>();
	const auto rect = photoRect();
	if (!drums->isLoaded() || m_photo.isNull())
	{
		p.setPen(QColor{0xe0, 0x60, 0x50});
		p.setFont(adjustedToPixelSize(font(), 11));
		p.drawText(rect, Qt::AlignCenter | Qt::TextWordWrap, tr("The drum kit could not be loaded"));
		return;
	}

	p.drawImage(rect, m_photo);

	// The pieces that were hit light up
	const auto scaleX = static_cast<double>(rect.width()) / m_photo.width();
	const auto scaleY = static_cast<double>(rect.height()) / m_photo.height();
	const auto mapScaleX = static_cast<double>(m_photo.width()) / std::max(m_map.width(), 1);
	const auto mapScaleY = static_cast<double>(m_photo.height()) / std::max(m_map.height(), 1);
	p.setCompositionMode(QPainter::CompositionMode_Screen);
	for (int piece = 0; piece < AvlDrumsInstrument::PieceCount; ++piece)
	{
		const auto& highlight = m_highlights[piece];
		if (m_glow[piece] <= 0.f || highlight.image.isNull()) { continue; }

		const auto target = QRectF{
			rect.x() + highlight.rect.x() * mapScaleX * scaleX,
			rect.y() + highlight.rect.y() * mapScaleY * scaleY,
			highlight.rect.width() * mapScaleX * scaleX,
			highlight.rect.height() * mapScaleY * scaleY};
		p.setOpacity(m_glow[piece]);
		p.drawImage(target, highlight.image);
	}
}

} // namespace gui

} // namespace lmms
