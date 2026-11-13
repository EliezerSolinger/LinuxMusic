/*
 * AudioItemView.h
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

#ifndef LMMS_GUI_AUDIO_ITEM_VIEW_H
#define LMMS_GUI_AUDIO_ITEM_VIEW_H

#include "ClipView.h"

namespace lmms
{
class AudioItem;
}

namespace lmms::gui
{

/**
 * An audio item on its track (docs/plano-gravacao.md, section 11): its waveform from the file's peaks, its fades and
 * its gain; the menu chooses where its audio is (RAM or disk) and resets its gain and fades.
 */
class AudioItemView : public ClipView
{
	Q_OBJECT
public:
	AudioItemView(AudioItem* item, TrackView* view);

	auto verticalPlace(int contentHeight) const -> std::pair<int, int> override;

protected:
	void paintEvent(QPaintEvent* event) override;
	void constructContextMenu(QMenu* menu) override;
	void mousePressEvent(QMouseEvent* event) override;
	void mouseMoveEvent(QMouseEvent* event) override;
	void mouseReleaseEvent(QMouseEvent* event) override;

private:
	void updateItem();

	//! The handles on the item's top edge: its fades at the corners, its gain along it (as REAPER's)
	enum class Handle { None, FadeIn, FadeOut, Gain };
	auto handleAt(QPoint position) const -> Handle;
	auto pixelsPerMs() const -> double;
	//! Its fades as heard: at least the overlap with the items next to it on its lane (the crossfade)
	auto heardFades() const -> std::pair<double, double>;

	AudioItem* m_item;
	Handle m_dragging = Handle::None;
	QPoint m_dragStart;
	double m_startValue = 0;
	//! While dragging a handle, the value shown (applied as one undo step when let go)
	double m_gainDb = 0;
	double m_fadeInMs = 0;
	double m_fadeOutMs = 0;
};

} // namespace lmms::gui

#endif // LMMS_GUI_AUDIO_ITEM_VIEW_H
