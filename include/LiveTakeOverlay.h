/*
 * LiveTakeOverlay.h
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

#ifndef LMMS_GUI_LIVE_TAKE_OVERLAY_H
#define LMMS_GUI_LIVE_TAKE_OVERLAY_H

#include <QTimer>
#include <QWidget>

namespace lmms
{
class Track;
}

namespace lmms::gui
{

class TrackView;

/**
 * Over a track's clips while it records (docs/plano-gravacao.md, "Cache de picos"): the take growing from where it
 * began to where the song is, in red, with its waveform as it is written to disk. Transparent to the mouse; shown
 * only while the song records this track.
 */
class LiveTakeOverlay : public QWidget
{
	Q_OBJECT
public:
	LiveTakeOverlay(TrackView* trackView, QWidget* content);

protected:
	void paintEvent(QPaintEvent* event) override;
	auto eventFilter(QObject* watched, QEvent* event) -> bool override;

private:
	void refresh();
	auto xOf(double tick) const -> double;

	TrackView* m_trackView;
	QTimer m_timer;
};

} // namespace lmms::gui

#endif // LMMS_GUI_LIVE_TAKE_OVERLAY_H
