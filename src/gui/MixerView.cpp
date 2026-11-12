/*
 * MixerView.cpp - effect-mixer-view for LMMS
 *
 * Copyright (c) 2008-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#include "MixerView.h"

#include <QButtonGroup>
#include <QGridLayout>
#include <QHBoxLayout>
#include <QLayout>
#include <QLineEdit>
#include <QPushButton>
#include <QScrollArea>
#include <QStyle>
#include <QKeyEvent>
#include <QStackedLayout>
#include <QStackedWidget>
#include <QVBoxLayout>

#include "EffectRackView.h"
#include "Engine.h"
#include "Fader.h"
#include "GuiApplication.h"
#include "Knob.h"
#include "LedCheckBox.h"
#include "InstrumentTrack.h"
#include "MainWindow.h"
#include "Mixer.h"
#include "MixerChannelView.h"
#include "PatternStore.h"
#include "SampleTrack.h"
#include "SendButtonIndicator.h"
#include "Song.h"
#include "SubWindow.h"
#include "TrackContainer.h" // For TrackContainer::TrackList typedef
#include "embed.h"

namespace lmms::gui
{


MixerView::MixerView(Mixer* mixer) :
	QWidget(),
	ModelView(nullptr, this),
	SerializingObjectHook(),
	m_mixer(mixer)
{
	mixer->setHook(this);

	// Channels made or removed elsewhere, e.g. with tracks (see Mixer::setUpTrackChannel())
	connect(mixer, &Mixer::channelCreated, this, [this](int) { addMissingChannelViews(); });
	connect(mixer, &Mixer::channelDeleted, this, [this](int index) { removeChannelView(index); });
	connect(mixer, &Mixer::dataChanged, this, &MixerView::updateGroupVisibility);
	connect(mixer, &Mixer::channelsSwapped, this, &MixerView::swapChannelViews);

	//QPalette pal = palette();
	//pal.setColor(QPalette::Window, QColor(72, 76, 88));
	//setPalette(pal);

	setAutoFillBackground(true);

	setWindowTitle(tr("Mixer"));
	setWindowIcon(embed::getIconPixmap("mixer"));

	// main-layout
	auto ml = new QHBoxLayout{this};

	// Set margins
	ml->setContentsMargins(0, 4, 0, 0);

	// Channel area
	m_channelAreaWidget = new QWidget;
	chLayout = new QHBoxLayout(m_channelAreaWidget);
	chLayout->setSizeConstraint(QLayout::SetMinimumSize);
	chLayout->setSpacing(0);
	chLayout->setContentsMargins(0, 0, 0, 0);
	chLayout->setAlignment(Qt::AlignLeft);
	m_channelAreaWidget->setLayout(chLayout);

	// create rack layout before creating the first channel
	m_racksWidget = new QWidget;
	m_racksLayout = new QStackedLayout(m_racksWidget);
	m_racksLayout->setContentsMargins(0, 0, 0, 0);
	m_racksWidget->setLayout(m_racksLayout);

	// add master channel
	m_mixerChannelViews.resize(mixer->numChannels());
	MixerChannelView * masterView = new MixerChannelView(this, this, 0);
	connectToSoloAndMute(0);
	m_mixerChannelViews[0] = masterView;

	m_racksLayout->addWidget(m_mixerChannelViews[0]->m_effectRackView);

	ml->addWidget(masterView, 0);

	auto mixerChannelSize = masterView->sizeHint();

	// add mixer channels
	for (int i = 1; i < m_mixerChannelViews.size(); ++i)
	{
		m_mixerChannelViews[i] = new MixerChannelView(m_channelAreaWidget, this, i);
		connectToSoloAndMute(i);
		chLayout->addWidget(m_mixerChannelViews[i]);
	}

	// add the scrolling section to the main layout
	// class solely for scroll area to pass key presses down
	class ChannelArea : public QScrollArea
	{
		public:
			ChannelArea(QWidget* parent, MixerView* mv) :
				QScrollArea(parent), m_mv(mv) {}
			~ChannelArea() override = default;
			void keyPressEvent(QKeyEvent* e) override
			{
				m_mv->keyPressEvent(e);
			}
		private:
			MixerView* m_mv;
	};
	channelArea = new ChannelArea(this, this);
	channelArea->setWidget(m_channelAreaWidget);
	channelArea->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
	channelArea->setFrameStyle(QFrame::NoFrame);
	channelArea->setMinimumWidth(mixerChannelSize.width() * 6);
	channelArea->setWidgetResizable(true);

	int const scrollBarExtent = style()->pixelMetric(QStyle::PM_ScrollBarExtent);
	// As tall as the tallest strip (in analog mode, the master has no desk's strip)
	auto updateMinimumHeight = [this, scrollBarExtent] {
		int height = 0;
		for (const auto view : m_mixerChannelViews) { height = std::max(height, view->sizeHint().height()); }
		channelArea->setMinimumHeight(height + scrollBarExtent);
	};
	updateMinimumHeight();
	connect(mixer->analogModel(), &BoolModel::dataChanged, this, updateMinimumHeight, Qt::QueuedConnection);
	connect(mixer, &Mixer::channelCreated, this, updateMinimumHeight, Qt::QueuedConnection);

	ml->addWidget(channelArea, 1);

	// show the add new mixer channel button
	auto newChannelBtn = new QPushButton(embed::getIconPixmap("new_channel"), QString(), this);
	newChannelBtn->setObjectName("newChannelBtn");
	newChannelBtn->setSizePolicy(QSizePolicy::MinimumExpanding, QSizePolicy::Expanding);
	newChannelBtn->setFixedWidth(mixerChannelSize.width());
	newChannelBtn->setFocusPolicy(Qt::NoFocus);
	connect(newChannelBtn, SIGNAL(clicked()), this, SLOT(addNewChannel()));
	ml->addWidget(newChannelBtn, 0);


	// The selected channel's strip settings, above the stacked layout of the channels' effect racks
	auto channelSettings = new QWidget{this};
	channelSettings->setFixedWidth(EffectRackView::DEFAULT_WIDTH);
	auto settingsLayout = new QGridLayout{channelSettings};
	settingsLayout->setContentsMargins(6, 2, 6, 4);
	settingsLayout->setHorizontalSpacing(8);
	settingsLayout->setVerticalSpacing(2);

	m_trimKnob = new Knob{KnobType::Bright26, tr("TRIM"), channelSettings, Knob::LabelRendering::LegacyFixedFontSize,
		tr("Input trim")};
	m_trimKnob->setHintText(tr("Input trim:"), " dB");
	m_widthKnob = new Knob{KnobType::Bright26, tr("WIDTH"), channelSettings, Knob::LabelRendering::LegacyFixedFontSize,
		tr("Stereo width")};
	m_widthKnob->setHintText(tr("Stereo width:"), "%");
	m_panKnob = new Knob{KnobType::Bright26, tr("PAN"), channelSettings, Knob::LabelRendering::LegacyFixedFontSize,
		tr("Panning")};
	m_panKnob->setHintText(tr("Panning:"), "%");
	m_swapChannelsBox = new LedCheckBox{tr("Swap L/R"), channelSettings, tr("Swap left/right"),
		LedCheckBox::LedColor::Green};

	settingsLayout->addWidget(m_trimKnob, 0, 0, 2, 1, Qt::AlignCenter);
	settingsLayout->addWidget(m_swapChannelsBox, 0, 1, 2, 1, Qt::AlignVCenter);
	settingsLayout->addWidget(m_widthKnob, 0, 2, 2, 1, Qt::AlignCenter);
	settingsLayout->addWidget(m_panKnob, 0, 3, 2, 1, Qt::AlignCenter);

	// The mixer's mode: an analog desk (with a desk's strip in every channel) or a digital one (effects only)
	auto modeRow = new QHBoxLayout{};
	modeRow->setContentsMargins(6, 4, 6, 2);
	modeRow->setSpacing(0);
	auto modeButtons = new QButtonGroup{this};
	auto analogButton = new QPushButton{tr("ANALOG"), this};
	auto digitalButton = new QPushButton{tr("DIGITAL"), this};
	analogButton->setToolTip(tr("Analog desk: every channel has gain, filters, gate, compressor and EQ built in"));
	digitalButton->setToolTip(tr("Digital desk: no built-in processing, only the channels' effects"));
	for (const auto button : {analogButton, digitalButton})
	{
		button->setObjectName("mixerModeButton");
		button->setCheckable(true);
		button->setFocusPolicy(Qt::NoFocus);
		modeButtons->addButton(button);
		modeRow->addWidget(button);
	}
	const auto analogModel = mixer->analogModel();
	auto showMode = [analogModel, analogButton, digitalButton] {
		(analogModel->value() ? analogButton : digitalButton)->setChecked(true);
	};
	showMode();
	connect(analogModel, &BoolModel::dataChanged, this, showMode);
	connect(analogButton, &QPushButton::clicked, this, [analogModel] { analogModel->setValue(true); });
	connect(digitalButton, &QPushButton::clicked, this, [analogModel] { analogModel->setValue(false); });

	auto rightColumn = new QVBoxLayout{};
	rightColumn->setContentsMargins(0, 0, 0, 0);
	rightColumn->setSpacing(0);
	rightColumn->addLayout(modeRow);
	rightColumn->addWidget(channelSettings);
	rightColumn->addWidget(m_racksWidget, 1);
	ml->addLayout(rightColumn);

	setCurrentMixerChannel(m_mixerChannelViews[0]);

	updateGeometry();

	auto* mainWindow = getGUI()->mainWindow();

	// timer for updating faders
	connect(mainWindow, &MainWindow::periodicUpdate, this, &MixerView::updateFaders);

	layout()->setSizeConstraint(QLayout::SetMinimumSize);

	// we want to receive dataChanged-signals in order to update
	setModel(mixer);
}




int MixerView::addNewChannel()
{
	// add new mixer channel and redraw the form.
	Mixer * mix = getMixer();

	// The view is added when the mixer announces the channel
	int newChannelIndex = mix->createChannel();
	updateMixerChannel(newChannelIndex);

	return newChannelIndex;
}




void MixerView::addMissingChannelViews()
{
	// Only once the views exist; refreshDisplay() makes them all
	if (m_mixerChannelViews.isEmpty()) { return; }

	for (int i = m_mixerChannelViews.size(); i < getMixer()->numChannels(); ++i)
	{
		m_mixerChannelViews.push_back(new MixerChannelView(m_channelAreaWidget, this, i));
		connectToSoloAndMute(i);
		chLayout->addWidget(m_mixerChannelViews[i]);
		m_racksLayout->addWidget(m_mixerChannelViews[i]->m_effectRackView);
		updateMixerChannel(i);
	}
}




void MixerView::removeChannelView(int index)
{
	if (index <= 0 || index >= m_mixerChannelViews.size()) { return; }

	// The channel's models are gone, which disconnected them from the view already
	const int selected = m_currentMixerChannel ? m_currentMixerChannel->channelIndex() : 0;

	chLayout->removeWidget(m_mixerChannelViews[index]);
	m_racksLayout->removeWidget(m_mixerChannelViews[index]->m_effectRackView);
	// delete the view later to prevent a crash when deleting from its context menu
	m_mixerChannelViews[index]->m_effectRackView->hide();
	m_mixerChannelViews[index]->hide();
	m_mixerChannelViews[index]->deleteLater();
	m_channelAreaWidget->adjustSize();

	// make sure every channel knows what index it is
	for (int i = index + 1; i < m_mixerChannelViews.size(); ++i)
	{
		m_mixerChannelViews[i]->setChannelIndex(i - 1);
	}
	m_mixerChannelViews.remove(index);

	setCurrentMixerChannel(std::min<int>(selected > index ? selected - 1 : selected, m_mixerChannelViews.size() - 1));
}


void MixerView::refreshDisplay()
{
	// delete all views and re-add them
	for (int i = 1; i<m_mixerChannelViews.size(); ++i)
	{
		// First disconnect from the solo/mute models.
		disconnectFromSoloAndMute(i);

		auto * mixerChannelView = m_mixerChannelViews[i];
		chLayout->removeWidget(mixerChannelView);
		m_racksLayout->removeWidget(mixerChannelView->m_effectRackView);

		delete mixerChannelView;
	}
	m_channelAreaWidget->adjustSize();

	// re-add the views
	m_mixerChannelViews.resize(getMixer()->numChannels());
	for (int i = 1; i < m_mixerChannelViews.size(); ++i)
	{
		m_mixerChannelViews[i] = new MixerChannelView(m_channelAreaWidget, this, i);
		connectToSoloAndMute(i);

		chLayout->addWidget(m_mixerChannelViews[i]);
		m_racksLayout->addWidget(m_mixerChannelViews[i]->m_effectRackView);
	}

	// set selected mixer channel to 0
	setCurrentMixerChannel(0);

	// update all mixer lines
	for (int i = 0; i < m_mixerChannelViews.size(); ++i)
	{
		updateMixerChannel(i);
	}
	updateGroupVisibility();
}




auto MixerView::isGroupCollapsed(int index) const -> bool
{
	return index > 0 && index < getMixer()->numChannels()
		&& m_collapsedGroups.contains(getMixer()->mixerChannel(index));
}




void MixerView::toggleGroup(int index)
{
	if (index <= 0 || index >= getMixer()->numChannels()) { return; }
	const auto channel = getMixer()->mixerChannel(index);
	if (!m_collapsedGroups.erase(channel)) { m_collapsedGroups.insert(channel); }
	updateGroupVisibility();
}




void MixerView::updateGroupVisibility()
{
	const auto mixer = getMixer();

	// Forget groups that are gone
	std::erase_if(m_collapsedGroups, [mixer](const MixerChannel* group) {
		for (int i = 1; i < mixer->numChannels(); ++i)
		{
			if (mixer->mixerChannel(i) == group) { return !group->m_groupChannel; }
		}
		return true;
	});

	// A channel is hidden if its main output goes to a group that is collapsed or hidden itself
	auto hidden = std::vector<int>(mixer->numChannels(), -1); // -1 unknown, 0 shown, 1 hidden
	auto isHidden = [&](auto& self, int index, int depth) -> bool {
		if (hidden[index] != -1) { return hidden[index] == 1; }
		bool result = false;
		for (const MixerRoute* route : mixer->mixerChannel(index)->m_sends)
		{
			if (route->sourcePair() != 0 || route->destinationPair() != 0) { continue; }
			const auto receiver = route->receiver();
			if (!receiver->m_groupChannel || depth > mixer->numChannels()) { continue; }
			if (m_collapsedGroups.contains(receiver) || self(self, receiver->index(), depth + 1))
			{
				result = true;
				break;
			}
		}
		hidden[index] = result ? 1 : 0;
		return result;
	};

	for (int i = 1; i < m_mixerChannelViews.size() && i < mixer->numChannels(); ++i)
	{
		m_mixerChannelViews[i]->setVisible(!isHidden(isHidden, i, 0));
		m_mixerChannelViews[i]->update();
	}
}

void MixerView::saveSettings(QDomDocument& doc, QDomElement& domElement)
{
	MainWindow::saveWidgetState(this, domElement);
}




void MixerView::loadSettings(const QDomElement& domElement)
{
	MainWindow::restoreWidgetState(this, domElement);
}





void MixerView::toggledSolo()
{
	// The mixer applies solo itself (see Mixer::applySolo())
	updateAllMixerChannels();
}


void MixerView::toggledMute()
{
	updateAllMixerChannels();
}

Mixer* MixerView::getMixer() const
{
	return m_mixer;
}

void MixerView::updateAllMixerChannels()
{
	for (int i = 0; i < m_mixerChannelViews.size(); ++i)
	{
		m_mixerChannelViews[i]->update();
	}
}

void MixerView::connectToSoloAndMute(int channelIndex)
{
	auto * mixerChannel = getMixer()->mixerChannel(channelIndex);

	connect(&mixerChannel->m_muteModel, &BoolModel::dataChanged, this, &MixerView::toggledMute, Qt::DirectConnection);
	connect(&mixerChannel->m_soloModel, &BoolModel::dataChanged, this, &MixerView::toggledSolo, Qt::DirectConnection);
}

void MixerView::disconnectFromSoloAndMute(int channelIndex)
{
	auto * mixerChannel = getMixer()->mixerChannel(channelIndex);

	disconnect(&mixerChannel->m_muteModel, &BoolModel::dataChanged, this, &MixerView::toggledMute);
	disconnect(&mixerChannel->m_soloModel, &BoolModel::dataChanged, this, &MixerView::toggledSolo);
}


void MixerView::setCurrentMixerChannel(MixerChannelView* channel)
{
	// select
	m_currentMixerChannel = channel;
	m_racksLayout->setCurrentWidget(m_mixerChannelViews[channel->channelIndex()]->m_effectRackView);

	const auto mixerChannel = getMixer()->mixerChannel(channel->channelIndex());
	m_trimKnob->setModel(&mixerChannel->m_trimModel);
	m_swapChannelsBox->setModel(&mixerChannel->m_swapChannelsModel);
	m_widthKnob->setModel(&mixerChannel->m_widthModel);
	m_panKnob->setModel(&mixerChannel->m_panModel);

	// set up send knob
	for (int i = 0; i < m_mixerChannelViews.size(); ++i)
	{
		updateMixerChannel(i);
	}
}


void MixerView::updateMixerChannel(int index)
{
	const auto mixer = getMixer();

	const auto currentIndex = m_currentMixerChannel->channelIndex();
	const auto thisLine = m_mixerChannelViews[index];
	thisLine->setToolTip(getMixer()->mixerChannel(index)->m_name);

	const auto sendModelCurrentToThis = mixer->channelSendModel(currentIndex, index);
	if (sendModelCurrentToThis == nullptr)
	{
		thisLine->m_sendKnob->setVisible(false);
		thisLine->m_sendArrow->setVisible(false);
	}
	else
	{
		thisLine->m_sendKnob->setVisible(true);
		thisLine->m_sendKnob->setModel(sendModelCurrentToThis);
		thisLine->m_sendArrow->setVisible(true);
	}

	const auto sendModelThisToCurrent = mixer->channelSendModel(index, currentIndex);
	if (sendModelThisToCurrent)
	{
		thisLine->m_receiveArrowOrSendButton->setVisible(true);
		thisLine->m_receiveArrowOrSendButton->setCurrentIndex(thisLine->m_receiveArrowStackedIndex);
	}
	else
	{
		thisLine->m_receiveArrowOrSendButton->setVisible(!mixer->isInfiniteLoop(currentIndex, index));
		thisLine->m_receiveArrowOrSendButton->setCurrentIndex(thisLine->m_sendButtonStackedIndex);
	}

	thisLine->m_sendButton->updateLightStatus();
	thisLine->m_renameLineEdit->setText(thisLine->elideName(thisLine->mixerChannel()->m_name));
	thisLine->update();

	updateGroupVisibility();
}


void MixerView::deleteChannel(int index)
{
	// can't delete master
	if (index == 0) return;

	// Disconnect from the solo/mute models of the channel we are about to delete
	disconnectFromSoloAndMute(index);

	// remember selected line
	int selLine = m_currentMixerChannel->channelIndex();

	Mixer* mixer = getMixer();
	// in case the deleted channel is soloed or the remaining
	// channels will be left in a muted state
	mixer->clearChannel(index);

	// delete the real channel; its view goes away when the mixer announces it (see removeChannelView())
	mixer->deleteChannel(index);

	// select the next channel
	if (selLine >= m_mixerChannelViews.size())
	{
		selLine = m_mixerChannelViews.size() - 1;
	}
	setCurrentMixerChannel(selLine);
}

void MixerView::deleteUnusedChannels()
{
	Mixer* mix = getMixer();

	// Check all channels except master, delete those with no incoming sends
	for (int i = m_mixerChannelViews.size() - 1; i > 0; --i)
	{
		if (!mix->isChannelInUse(i))
		{
			deleteChannel(i);
		}
	}
}

void MixerView::moveChannelLeft(int index)
{
	// can't move master or first channel left or last channel right
	if (index <= 1 || index >= m_mixerChannelViews.size()) { return; }

	// The views follow when the mixer announces the swap (see swapChannelViews())
	m_mixer->moveChannelLeft(index);
}




void MixerView::swapChannelViews(int index, int otherIndex)
{
	if (index > otherIndex) { std::swap(index, otherIndex); }
	if (index <= 0 || otherIndex >= m_mixerChannelViews.size() || otherIndex != index + 1) { return; }

	const auto layoutIndex = chLayout->indexOf(m_mixerChannelViews[otherIndex]);
	assert(layoutIndex >= 1);

	chLayout->removeWidget(m_mixerChannelViews[otherIndex]);
	chLayout->insertWidget(layoutIndex - 1, m_mixerChannelViews[otherIndex]);

	m_mixerChannelViews[otherIndex]->setChannelIndex(index);
	m_mixerChannelViews[index]->setChannelIndex(otherIndex);
	std::swap(m_mixerChannelViews[index], m_mixerChannelViews[otherIndex]);
}



void MixerView::moveChannelRight(int index)
{
	moveChannelLeft(index + 1);
}


void MixerView::renameChannel(int index)
{
	m_mixerChannelViews[index]->renameChannel();
}



void MixerView::keyPressEvent(QKeyEvent * e)
{
	auto adjustCurrentFader = [this](const Qt::KeyboardModifiers& modifiers, Fader::AdjustmentDirection direction)
	{
		auto* mixerChannel = currentMixerChannel();

		if (mixerChannel)
		{
			mixerChannel->fader()->adjust(modifiers, direction);
		}
	};

	switch(e->key())
	{
		case Qt::Key_Delete:
			deleteChannel(m_currentMixerChannel->channelIndex());
			break;
		case Qt::Key_Left:
			if (e->modifiers() & Qt::AltModifier)
			{
				moveChannelLeft(m_currentMixerChannel->channelIndex());
			}
			else
			{
				// select channel to the left
				setCurrentMixerChannel(m_currentMixerChannel->channelIndex() - 1);
			}
			break;
		case Qt::Key_Right:
			if (e->modifiers() & Qt::AltModifier)
			{
				moveChannelRight(m_currentMixerChannel->channelIndex());
			}
			else
			{
				// select channel to the right
				setCurrentMixerChannel(m_currentMixerChannel->channelIndex() + 1);
			}
			break;
		case Qt::Key_Up:
		case Qt::Key_Plus:
			adjustCurrentFader(e->modifiers(), Fader::AdjustmentDirection::Up);
			break;
		case Qt::Key_Down:
		case Qt::Key_Minus:
			adjustCurrentFader(e->modifiers(), Fader::AdjustmentDirection::Down);
			break;
		case Qt::Key_Insert:
			if (e->modifiers() & Qt::ShiftModifier)
			{
				addNewChannel();
			}
			break;
		case Qt::Key_Enter:
		case Qt::Key_Return:
		case Qt::Key_F2:
			renameChannel(m_currentMixerChannel->channelIndex());
			break;
		default:
			e->ignore();
			break;
	}
}



void MixerView::setCurrentMixerChannel(int channel)
{
	if (channel >= 0 && channel < m_mixerChannelViews.size())
	{
		setCurrentMixerChannel(m_mixerChannelViews[channel]);
	}
}



void MixerView::clear()
{
	for (auto i = m_mixerChannelViews.size() - 1; i > 0; --i) { deleteChannel(i); }
	getMixer()->clearChannel(0);

	m_mixerChannelViews[0]->reset();

	refreshDisplay();
}




void MixerView::updateFaders()
{
	Mixer * m = getMixer();

	for (int i = 0; i < m_mixerChannelViews.size(); ++i)
	{
		// No new period was measured since the last update: keep the meters as they are
		const auto peaks = m->mixerChannel(i)->m_meter.takePeaks();
		if (!peaks) { continue; }

		// Rise immediately, fall off gradually
		auto fader = m_mixerChannelViews[i]->m_fader;
		const float fallOff = 1.25;
		fader->setPeak_L(std::max((*peaks)[0], fader->getPeak_L() / fallOff));
		fader->setPeak_R(std::max((*peaks)[1], fader->getPeak_R() / fallOff));
	}
}


} // namespace lmms::gui
