/*
 * MixerChannelView.h
 *
 * Copyright (c) 2024 saker
 *
 * This file is part of LMMS - https://lmms.io
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

#ifndef LMMS_GUI_MIXER_CHANNEL_VIEW_H
#define LMMS_GUI_MIXER_CHANNEL_VIEW_H

#include <array>

#include <QWidget>

#include "Mixer.h"

class QLabel;
class QLineEdit;
class QStackedWidget;

namespace lmms {
class MixerChannel;
}

namespace lmms::gui {
class AutomatableButton;
class EffectRackView;
class Fader;
class DeskKnob;
class Knob;
class LcdWidget;
class MixerView;
class PeakIndicator;
class SendButtonIndicator;


class MixerChannelView : public QWidget
{
	Q_OBJECT
	Q_PROPERTY(QBrush backgroundActive READ backgroundActive WRITE setBackgroundActive)
	Q_PROPERTY(QColor strokeOuterActive READ strokeOuterActive WRITE setStrokeOuterActive)
	Q_PROPERTY(QColor strokeOuterInactive READ strokeOuterInactive WRITE setStrokeOuterInactive)
	Q_PROPERTY(QColor strokeInnerActive READ strokeInnerActive WRITE setStrokeInnerActive)
	Q_PROPERTY(QColor strokeInnerInactive READ strokeInnerInactive WRITE setStrokeInnerInactive)
public:
	MixerChannelView(QWidget* parent, MixerView* mixerView, int channelIndex);

	//! The width of a channel's strip (the master's is wider)
	static constexpr int StripWidth = 58;

	//! Everything above the panning knob (on the master, the master section), which the mixer makes as tall in
	//! every strip so that the faders line up
	auto upperArea() const -> QWidget* { return m_upperArea; }

	void paintEvent(QPaintEvent* event) override;
	void contextMenuEvent(QContextMenuEvent*) override;
	void mousePressEvent(QMouseEvent*) override;
	void dragEnterEvent(QDragEnterEvent* event) override;
	void dropEvent(QDropEvent* event) override;
	void mouseDoubleClickEvent(QMouseEvent*) override;
	void keyPressEvent(QKeyEvent* ke) override;

	void reset();

	int channelIndex() const { return m_channelIndex; }
	void setChannelIndex(int index);

	QBrush backgroundActive() const { return m_backgroundActive; }
	void setBackgroundActive(const QBrush& c) { m_backgroundActive = c; }

	QColor strokeOuterActive() const { return m_strokeOuterActive; }
	void setStrokeOuterActive(const QColor& c) { m_strokeOuterActive = c; }

	QColor strokeOuterInactive() const { return m_strokeOuterInactive; }
	void setStrokeOuterInactive(const QColor& c) { m_strokeOuterInactive = c; }

	QColor strokeInnerActive() const { return m_strokeInnerActive; }
	void setStrokeInnerActive(const QColor& c) { m_strokeInnerActive = c; }

	QColor strokeInnerInactive() const { return m_strokeInnerInactive; }
	void setStrokeInnerInactive(const QColor& c) { m_strokeInnerInactive = c; }

	Fader* fader() const { return m_fader; }

public slots:
	void renameChannel();
	void resetColor();
	void selectColor();
	void randomizeColor();

private slots:
	void renameFinished();

	//! Shows what the strip has in the mixer's mode (see Mixer::isAnalog())
	void updateMode();

	//! Points the FX send knobs at the channel's sends to the effect returns
	void updateFxSends();
	void removeChannel();
	void removeUnusedChannels();
	void moveChannelLeft();
	void moveChannelRight();

private:
	bool confirmRemoval(int index);
	QString elideName(const QString& name);
	MixerChannel* mixerChannel() const;
	auto isMasterChannel() const -> bool { return m_channelIndex == 0; }

	//! Where a group channel's marker is (see MixerView::toggleGroup())
	auto groupMarkerRect() const -> QRect;

private:
	SendButtonIndicator* m_sendButton;
	QLabel* m_receiveArrow;
	QStackedWidget* m_receiveArrowOrSendButton;
	int m_receiveArrowStackedIndex = -1;
	int m_sendButtonStackedIndex = -1;

	Knob* m_sendKnob;
	Knob* m_panKnob;
	DeskKnob* m_deskPanKnob;	//!< the panning knob in analog mode
	LcdWidget* m_channelNumberLcd;
	//! Widths of the name label (and the panning knob's background) and of the peak level above the fader
	static constexpr int NameWidth = 50;
	static constexpr int PeakWidth = 26;

	QLineEdit* m_renameLineEdit;
	QLabel* m_sendArrow;
	AutomatableButton* m_muteButton;
	AutomatableButton* m_soloButton;
	AutomatableButton* m_phaseInvertButton;

	//! Analog mode: the strip of an analog desk, and the effects as LEDs; digital mode: the effects by name
	QWidget* m_analogSection;
	QWidget* m_effectLeds;
	QWidget* m_effectSlotList;
	QWidget* m_upperArea = nullptr;

	//! The master's PRE faders, left and right (see MasterSection)
	std::array<Fader*, 2> m_preFaders{};
	static constexpr int PreFaderWidth = 24;
	static constexpr int PostFaderWidth = 30;

	//! The sends to the effect returns (see Mixer::fxReturns())
	QWidget* m_fxSends;
	std::array<Knob*, Mixer::FxReturnCount> m_fxSendKnobs{};
	PeakIndicator* m_peakIndicator = nullptr;
	Fader* m_fader;
	EffectRackView* m_effectRackView;
	MixerView* m_mixerView;
	int m_channelIndex = 0;
	bool m_inRename = false;

	QBrush m_backgroundActive;
	QColor m_strokeOuterActive;
	QColor m_strokeOuterInactive;
	QColor m_strokeInnerActive;
	QColor m_strokeInnerInactive;

	friend class MixerView;
};
} // namespace lmms::gui

#endif // LMMS_GUI_MIXER_CHANNEL_VIEW_H
