/*
 * FolderTrackView.cpp - view of a folder track
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

#include "FolderTrackView.h"

#include <QHBoxLayout>
#include <QToolButton>
#include <QVBoxLayout>

#include "FolderTrack.h"
#include "Knob.h"
#include "MixerChannelLcdSpinBox.h"
#include "TrackChannelControls.h"
#include "TrackContainerView.h"
#include "TrackLabelButton.h"
#include "embed.h"

namespace lmms::gui
{

FolderTrackView::FolderTrackView(FolderTrack* folder, TrackContainerView* tcv) :
	TrackView(folder, tcv)
{
	setFixedHeight(32);

	m_collapseButton = new QToolButton(getTrackSettingsWidget());
	m_collapseButton->setAutoRaise(true);
	m_collapseButton->setFixedSize(20, 20);
	m_collapseButton->setIconSize(QSize{16, 16});
	connect(m_collapseButton, &QToolButton::clicked, this, [this] {
		auto folder = castModel<FolderTrack>();
		folder->setCollapsed(!folder->isCollapsed());
	});

	auto tlb = new TrackLabelButton(this, getTrackSettingsWidget());
	tlb->setIcon(embed::getIconPixmap("folder"));
	tlb->show();

	m_mixerChannelNumber = new MixerChannelLcdSpinBox(2, getTrackSettingsWidget(), tr("Mixer channel"), this);
	m_mixerChannelNumber->setEnabled(false); // the folder's group channel is set up for it
	m_mixerChannelNumber->setVisible(trackContainerView()->trackHeadWidth() >= COMPACT_TRACK_WIDTH);
	connect(trackContainerView(), &TrackContainerView::trackHeadWidthChanged, this, [this](int width) {
		m_mixerChannelNumber->setVisible(width >= COMPACT_TRACK_WIDTH);
	});

	m_volumeKnob = new VolumeKnob(KnobType::Small17, tr("VOL"), getTrackSettingsWidget(),
		Knob::LabelRendering::LegacyFixedFontSize, tr("Folder volume"));
	m_volumeKnob->setHintText(tr("Volume:"), "%");

	m_panningKnob = new Knob(KnobType::Small17, tr("PAN"), getTrackSettingsWidget(),
		Knob::LabelRendering::LegacyFixedFontSize, tr("Panning"));
	m_panningKnob->setHintText(tr("Panning:"), "%");

	auto masterLayout = new QVBoxLayout(getTrackSettingsWidget());
	masterLayout->setContentsMargins(0, 1, 0, 0);
	auto layout = new QHBoxLayout();
	layout->setContentsMargins(0, 0, 0, 0);
	layout->setSpacing(3);
	layout->addWidget(m_collapseButton);
	layout->addWidget(tlb, 1);
	layout->addWidget(m_mixerChannelNumber);
	layout->addWidget(m_volumeKnob);
	layout->addWidget(m_panningKnob);
	layout->addSpacing(4);
	masterLayout->addLayout(layout);
	masterLayout->addSpacerItem(new QSpacerItem(0, 0, QSizePolicy::Minimum, QSizePolicy::Expanding));

	setModel(folder);
}




QMenu* FolderTrackView::createMixerMenu(QString title, QString newMixerLabel)
{
	(void)title;
	(void)newMixerLabel;
	return nullptr;
}




void FolderTrackView::modelChanged()
{
	TrackView::modelChanged();

	auto folder = castModel<FolderTrack>();
	m_mixerChannelNumber->setModel(folder->mixerChannelModel());
	m_channelControls = TrackChannelControls::replace(m_channelControls, folder, this, {.volume = m_volumeKnob,
		.panning = m_panningKnob, .mute = getTrackOperationsWidget()->muteButton(),
		.solo = getTrackOperationsWidget()->soloButton()});

	connect(folder, &FolderTrack::collapsedChanged, this, &FolderTrackView::updateCollapseButton,
		Qt::UniqueConnection);
	connect(folder, &FolderTrack::collapsedChanged, trackContainerView(), &TrackContainerView::realignTracks,
		Qt::UniqueConnection);
	updateCollapseButton();
}




void FolderTrackView::updateCollapseButton()
{
	const bool collapsed = castModel<FolderTrack>()->isCollapsed();
	m_collapseButton->setIcon(embed::getIconPixmap(collapsed ? "folder" : "folder_opened"));
	m_collapseButton->setToolTip(collapsed ? tr("Show the tracks in this folder") : tr("Hide the tracks in this folder"));
}

} // namespace lmms::gui
