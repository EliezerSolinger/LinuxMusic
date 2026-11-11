/*
 * AvlDrums.h - the AVL drum kits as an instrument
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

#ifndef LMMS_AVL_DRUMS_H
#define LMMS_AVL_DRUMS_H

#include <array>
#include <atomic>
#include <cstdint>

#include <QMutex>

#include "AutomatableModel.h"
#include "ComboBoxModel.h"
#include "Instrument.h"
#include "InstrumentView.h"

class QPushButton;

struct _fluid_hashtable_t;
struct _fluid_synth_t;

namespace lmms
{

/**
 * Plays one of the AVL drum kits (SF2 files in data/avldrums) with FluidSynth.
 *
 * - Stereo: one stereo output with the whole kit.
 * - Multi-output (not for the percussion kit): 9 outputs in 7 channel groups, each its own output group in the
 *   pin connector: kick, snare, hi-hat, tom and floor tom (mono), cymbals and percussion (stereo). By default all
 *   of them go to the main track channels; routing them to auxiliary pairs lets each piece go to its own mixer
 *   channel through sends.
 *
 * Notes 36 to 61 play the drums (35 is the stick click of the Blonde Bop kits), with five velocity layers.
 */
class AvlDrumsInstrument : public Instrument
{
	Q_OBJECT
public:
	struct Kit
	{
		const char* name;
		const char* file;		//!< SF2 in data/avldrums
		const char* photo;		//!< photo of the kit in data/avldrums/images
		const char* map;		//!< where each drum piece is on the photo (see AvlDrumsView)
		bool hasMultiOutput;
	};
	static const std::array<Kit, 5> Kits;

	//! Drum pieces shown by the view: one per note from FirstNote
	static constexpr int FirstNote = 35;
	static constexpr int PieceCount = 27;

	explicit AvlDrumsInstrument(InstrumentTrack* track);
	~AvlDrumsInstrument() override;

	void saveSettings(QDomDocument& doc, QDomElement& elem) override;
	void loadSettings(const QDomElement& elem) override;
	QString nodeName() const override;
	gui::PluginView* instantiateView(QWidget* parent) override;

	// AudioProcessor implementation
	auto wantsEvents() const -> bool override { return true; }
	auto activate(double sampleRate, f_cnt_t maxFrameCount) -> bool override;
	auto process(ProcessContext& context) -> ProcessStatus override;
	auto channelCount(bool isInput) const -> ch_cnt_t override;
	auto channelGroupCount(bool isInput) const -> group_cnt_t override;
	auto getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup& group) const
		-> ch_cnt_t override;
	auto getInplacePair(ch_cnt_t inputIndex) const -> std::optional<ch_cnt_t> override;
	auto channelGroupName(group_cnt_t index, bool isInput) const -> std::string override;

	auto kitModel() -> ComboBoxModel& { return m_kitModel; }
	auto multiOutputModel() -> BoolModel& { return m_multiOutputModel; }

	//! Whether the current kit is loaded and plays
	auto isLoaded() const -> bool { return m_loaded.load(std::memory_order_relaxed); }

	//! The velocity of the last hit of each piece since the previous call (0 for none), for the view
	auto takeHits() -> std::array<int, PieceCount>;

signals:
	//! A kit was loaded or failed to load
	void kitChanged();

private slots:
	//! Creates the synth for the selected kit and output mode [main thread]
	void loadKit();

private:
	void destroySynth();
	void applyEvent(const PluginEvent& event);
	void render(AudioBuffer& out, f_cnt_t offset, f_cnt_t frames);
	auto multiOutputActive() const -> bool;

	ComboBoxModel m_kitModel;
	BoolModel m_multiOutputModel;

	//! The synth and how it was created; replaced by loadKit() while m_synthMutex is locked
	_fluid_hashtable_t* m_settings = nullptr;
	_fluid_synth_t* m_synth = nullptr;
	bool m_synthMultiOutput = false;
	sample_rate_t m_sampleRate = 0;
	int m_loadedKit = -1;
	sample_rate_t m_loadedSampleRate = 0;
	QMutex m_synthMutex;
	std::atomic<bool> m_loaded = false;

	std::array<std::atomic<int>, PieceCount> m_hits{};
};


namespace gui
{

class ComboBox;
class LedCheckBox;

//! Kit and output selection, and the kit's pieces lighting up when they are hit
/**
 * The kit selection and a photo of the kit, as in avldrums.lv2: the pieces light up when they are hit, and
 * clicking a piece plays it.
 */
class AvlDrumsView : public InstrumentView
{
	Q_OBJECT
public:
	AvlDrumsView(AvlDrumsInstrument* instrument, QWidget* parent);

	auto sizeHint() const -> QSize override { return {640, 360}; }

protected:
	void paintEvent(QPaintEvent* event) override;
	void timerEvent(QTimerEvent* event) override;
	void mousePressEvent(QMouseEvent* event) override;
	void mouseReleaseEvent(QMouseEvent* event) override;

private:
	//! A piece's area on the photo, as a highlight image in the photo's coordinates
	struct Highlight
	{
		QRect rect;
		QImage image;
	};

	void modelChanged() override;
	void updateMultiOutputAvailability();

	//! Loads the photo and the map of the selected kit
	void loadKitImages();

	//! Where the photo is drawn: as large as fits below the controls, keeping its proportions
	auto photoRect() const -> QRect;

	void resizeEvent(QResizeEvent* event) override;

	//! The note of the piece at @p pos, or -1
	auto noteAt(const QPoint& pos) const -> int;

	ComboBox* m_kitBox;
	LedCheckBox* m_multiOutputBox;
	QPushButton* m_channelPerPieceButton;

	int m_imagesKit = -1;
	QImage m_photo;
	QImage m_map;
	std::array<Highlight, AvlDrumsInstrument::PieceCount> m_highlights;

	std::array<float, AvlDrumsInstrument::PieceCount> m_glow{};
	int m_heldNote = -1;
};

} // namespace gui

} // namespace lmms

#endif // LMMS_AVL_DRUMS_H
