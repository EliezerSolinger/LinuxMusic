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

protected:
	void paintEvent(QPaintEvent* event) override;
	void constructContextMenu(QMenu* menu) override;

private:
	void updateItem();

	AudioItem* m_item;
};

} // namespace lmms::gui

#endif // LMMS_GUI_AUDIO_ITEM_VIEW_H
