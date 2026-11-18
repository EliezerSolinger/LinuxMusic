/*
 * DrumMachines.h - the drum machines of the DrumSynth kits as an instrument
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

#ifndef LMMS_DRUM_MACHINES_H
#define LMMS_DRUM_MACHINES_H

#include <array>
#include <atomic>
#include <span>
#include <vector>

#include <QColor>
#include <QImage>
#include <QPixmap>
#include <QMutex>

#include "ComboBoxModel.h"
#include "Instrument.h"
#include "InstrumentView.h"
#include "SampleFrame.h"

namespace lmms
{

/**
 * The drum machines of the DrumSynth kits that come with the program (data/samples/drumsynth: CR-78, CR-8000,
 * TR-606, TR-77, TR-808, TR-909, LinnDrum, and the Electro and R&B kits), the CR-78 recorded, and kits in the style of
 * the SP-1200 and the MPC60 (samplers: the program's own samples given their character), each piece on its note of the General MIDI
 * drum map (the variations with no note there on the notes after it, from 82), so that drum parts written for
 * any General MIDI kit play.
 *
 * The pieces are synthesized from their DrumSynth files when a kit is chosen (on the main thread), then played from
 * memory on the audio thread with voices made beforehand: no allocation there. A piece is louder by its velocity
 * (its first half millisecond fading in: the DrumSynth pieces start on a step, which clicks);
 * a hi-hat hit chokes the hi-hat that rings (closed, pedal and open hi-hats in one group), as on the machines.
 */
class DrumMachinesInstrument : public LegacyInstrument
{
	Q_OBJECT
public:
	//! A piece of a kit: its DrumSynth file, the note it plays on, its name on the panel, and its choke group
	struct Piece
	{
		const char* file;
		int note;
		const char* label;
		int choke = 0; //!< pieces in the same group (not 0) choke each other: the hi-hats
	};

	//! How a kit's machine is drawn: its body, its panel, its accent, and the colors of its pads
	struct Look
	{
		QColor body;
		QColor panel;
		QColor accent;
		QColor text;
		std::array<QColor, 4> pads;
	};

	struct Kit
	{
		const char* name;
		const char* folder; //!< in data/samples: drumsynth/<machine> (DrumSynth files), drummachines/<machine> (recordings)
		Look look;
		std::vector<Piece> pieces;
		//! A photo of the machine (embedded, photos/: see photos/CREDITS.md) and its credit, none if there is none
		const char* photo = nullptr;
		const char* credit = nullptr;
	};

	static auto kits() -> const std::vector<Kit>&;

	explicit DrumMachinesInstrument(InstrumentTrack* track);
	~DrumMachinesInstrument() override;

	void saveSettings(QDomDocument& doc, QDomElement& elem) override;
	void loadSettings(const QDomElement& elem) override;
	QString nodeName() const override;
	gui::PluginView* instantiateView(QWidget* parent) override;

	auto wantsEvents() const -> bool override { return true; }
	auto activate(double sampleRate, f_cnt_t maxFrameCount) -> bool override;
	//! Back to rest (an export, a new project): every voice silent [audio thread]
	void reset() override;

	auto kitModel() -> ComboBoxModel& { return m_kitModel; }
	//! The kit that plays (its pieces synthesized), or -1 while none could be
	auto loadedKit() const -> int { return m_loadedKit.load(std::memory_order_relaxed); }

	//! The velocity of the last hit of each piece of the kit since the previous call (0 for none), for the view
	auto takeHits() -> std::vector<int>;

	static constexpr int MaxPieces = 24;

signals:
	void kitChanged();

private slots:
	//! Synthesizes the chosen kit's pieces and puts them in place of the ones that played [main thread]
	void loadKit();

private:
	void playImpl(std::span<SampleFrame> out, InputEvents events) override;
	void noteOn(int note, float velocity);
	void render(std::span<SampleFrame> out);

	//! A piece synthesized: its frames, its note and its choke group
	struct Sound
	{
		std::vector<SampleFrame> frames;
		int note = -1;
		int choke = 0;
	};

	//! A piece playing: which, where, how loud; choked, it fades out
	struct Voice
	{
		int sound = -1;
		std::size_t position = 0;
		float gain = 0.f;
		int fade = 0; //!< (frames left of a choke's fade, 0 for none)
	};
	static constexpr int Voices = 64;
	static constexpr int ChokeFrames = 64;

	ComboBoxModel m_kitModel;
	sample_rate_t m_sampleRate = 0;

	//! The sounds that play, replaced by loadKit() while m_soundsMutex is locked
	std::vector<Sound> m_sounds;
	std::array<int, 128> m_soundOfNote{}; //!< (the sound each note plays, -1 for none)
	std::array<Voice, Voices> m_voices{};
	QMutex m_soundsMutex;
	std::atomic<int> m_loadedKit = -1;
	sample_rate_t m_loadedRate = 0;
	std::size_t m_fadeInFrames = 1; //!< (half a millisecond)

	std::array<std::atomic<int>, MaxPieces> m_hits{};
};


namespace gui
{

class ComboBox;

/**
 * The kit's machine: its photo, filling the view (the CR-78 has none with a free license: its panel drawn), and, over
 * its lower part, a pad for
 * each of its pieces, which lights up when the piece is hit and plays it when clicked.
 */
class DrumMachinesView : public InstrumentView
{
	Q_OBJECT
public:
	DrumMachinesView(DrumMachinesInstrument* instrument, QWidget* parent);

	auto sizeHint() const -> QSize override { return {640, 360}; }

protected:
	void paintEvent(QPaintEvent* event) override;
	void timerEvent(QTimerEvent* event) override;
	void mousePressEvent(QMouseEvent* event) override;
	void mouseReleaseEvent(QMouseEvent* event) override;

private:
	void modelChanged() override;
	//! The machine's area, and each pad's in it; its photo's above the pads, when it has one
	auto machineRect() const -> QRect;
	auto photoRect() const -> QRect;
	//! The band the pads are on, over the photo
	auto padsRect(int count) const -> QRect;
	auto hasPhoto() const -> bool;
	auto padRect(int index, int count) const -> QRect;
	//! The pad at @p pos, or -1
	auto padAt(const QPoint& pos) const -> int;

	ComboBox* m_kitBox;
	mutable int m_photoKit = -1;
	mutable QPixmap m_photo; //!< (the shown kit's)
	mutable QImage m_backdrop; //!< (the photo blurred, behind it when it is small)
	std::vector<float> m_glow;
	int m_heldNote = -1;
};

} // namespace gui

} // namespace lmms

#endif // LMMS_DRUM_MACHINES_H
