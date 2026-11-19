/*
 * TrackView.cpp - implementation of TrackView class
 *
 * Copyright (c) 2004-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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


#include "TrackView.h"

#include <QTimer>

#include <QApplication>
#include <QContextMenuEvent>
#include <QAbstractButton>
#include <QMenu>
#include <QPushButton>
#include <QVBoxLayout>
#include <QHBoxLayout>
#include <QMouseEvent>
#include <QPainter>
#include <QStyleOption>


#include "AudioEngine.h"
#include "AutomatableButton.h"
#include "ConfigManager.h"
#include "DataFile.h"
#include "DeprecationHelper.h"
#include "Engine.h"
#include "FadeButton.h"
#include "GuiApplication.h"
#include "MainWindow.h"
#include "EffectChain.h"
#include "Mixer.h"
#include "StringPairDrag.h"
#include "Track.h"
#include "TrackGrip.h"
#include "TrackContainer.h"
#include "TrackContainerView.h"
#include "ClipView.h"


namespace lmms::gui
{

static_assert(TrackView::TwoRowHeight == DEFAULT_TRACK_HEIGHT, "a track starts with its two rows");


TrackView::TrackView(Track* track, TrackContainerView* tcv)
	: QWidget(tcv->contentWidget())
	, ModelView(nullptr, this)
	, m_track(track)
	, m_trackContainerView(tcv)
	, m_trackOperationsWidget(this)
	, m_trackSettingsWidget(this)
	, m_trackContentWidget(this)
{
	setAutoFillBackground( true );
	QPalette pal;
	pal.setColor( backgroundRole(), QColor( 32, 36, 40 ) );
	setPalette( pal );

	m_trackSettingsWidget.setAutoFillBackground( true );

	auto layout = new QHBoxLayout(this);
	layout->setContentsMargins(0, 0, 0, 0);
	layout->setSpacing( 0 );
	layout->addWidget( &m_trackOperationsWidget );
	layout->addWidget( &m_trackSettingsWidget );
	layout->addWidget( &m_trackContentWidget, 1 );
	setFixedHeight( m_track->getHeight() );

	resizeEvent( nullptr );

	// Lanes take room (the subclasses set their own height first: checked once they are made)
	connect(m_track, &Track::lanesChanged, this, &TrackView::fitLanes);
	QTimer::singleShot(0, this, &TrackView::fitLanes);

	setAcceptDrops( true );
	setMouseTracking(true);
	m_trackSettingsWidget.setMouseTracking(true);
	m_trackSettingsWidget.setMouseTracking(true);
	setAttribute( Qt::WA_DeleteOnClose, true );


	connect( m_track, SIGNAL(destroyedTrack()), this, SLOT(close()));
	connect( m_track,
		SIGNAL(clipAdded(lmms::Clip*)),
			this, SLOT(createClipView(lmms::Clip*)),
			Qt::QueuedConnection );

	// The track looks muted while it is silent in the mix (Track::isMutedInMix())
	connect(m_trackOperationsWidget.m_muteBtn, &QPushButton::toggled, &m_trackContentWidget,
		qOverload<>(&QWidget::update));
	connect(m_trackOperationsWidget.m_muteBtn, &QPushButton::toggled, this, &TrackView::muteChanged);
	
	connect(m_trackContainerView, &TrackContainerView::trackHeadWidthChanged, this, &TrackView::updateWidth);

	// Tracks in folders are indented
	auto indent = [this] { m_trackSettingsWidget.setContentsMargins(FolderIndent * m_track->folderDepth(), 0, 0, 0); };
	connect(m_track, &Track::folderDepthChanged, this, indent);
	indent();

	auto trackGrip = m_trackOperationsWidget.getTrackGrip();
	connect(trackGrip, &TrackGrip::grabbed, this, &TrackView::onTrackGripGrabbed);
	connect(trackGrip, &TrackGrip::released, this, &TrackView::onTrackGripReleased);

	// create views for already existing clips
	for (const auto& clip : m_track->m_clips)
	{
		createClipView(clip);
	}

	m_trackContainerView->addTrackView( this );
}




void TrackView::resizeEvent( QResizeEvent * re )
{
	m_trackOperationsWidget.setFixedSize(TRACK_OP_WIDTH, height() - 1);
	m_trackSettingsWidget.setFixedSize(m_trackContainerView->trackHeadWidth() - m_trackOperationsWidget.width(), height() - 1);

	m_trackContentWidget.setFixedHeight(height());
	fitHeaderRows();
}




namespace
{

/**
 * The level of the track's mixer channel, on the right edge of its header: its RMS (the mixer's strip takes the
 * channel's peaks, so they are left to it), green, then yellow and red near full scale
 */
class TrackLevelMeter : public QWidget
{
public:
	TrackLevelMeter(Track* track, const IntModel* channel, QWidget* parent) :
		QWidget(parent), m_track(track), m_channel(channel)
	{
		setFixedWidth(6);
		setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Expanding);
		connect(getGUI()->mainWindow(), &MainWindow::periodicUpdate, this, [this] {
			const auto level = currentLevel();
			if (std::abs(level - m_shown) > 0.002f) { m_shown = level; update(); }
		});
	}

protected:
	void paintEvent(QPaintEvent*) override
	{
		auto p = QPainter{this};
		const auto r = rect().adjusted(1, 3, -1, -3);
		p.fillRect(r, QColor{11, 13, 15});
		// (in dB from -60 to 0: as a meter reads, not as the amplitude)
		const auto db = m_shown > 0 ? 20.f * std::log10(m_shown) : -60.f;
		const auto fraction = std::clamp((db + 60.f) / 60.f, 0.f, 1.f);
		const auto filled = static_cast<int>(r.height() * fraction);
		if (filled <= 0) { return; }
		auto gradient = QLinearGradient{QPointF(0, r.bottom()), QPointF(0, r.top())};
		gradient.setColorAt(0, QColor{47, 209, 111});
		gradient.setColorAt(0.75, QColor{47, 209, 111});
		gradient.setColorAt(0.9, QColor{242, 215, 58});
		gradient.setColorAt(1, QColor{232, 69, 60});
		p.fillRect(QRect{r.left(), r.bottom() - filled + 1, r.width(), filled}, gradient);
	}

private:
	auto currentLevel() const -> float
	{
		const auto mixer = Engine::mixer();
		auto* channel = mixer->trackChannel(m_track);
		if (!channel && m_channel)
		{
			channel = mixer->mixerChannel(std::clamp(m_channel->value(), 0, mixer->numChannels() - 1));
		}
		return channel ? std::max(channel->m_meter.rms(0), channel->m_meter.rms(1)) : 0.f;
	}

	Track* m_track;
	const IntModel* m_channel;
	float m_shown = 0.f;
};

} // namespace


void TrackView::buildHeader(const HeaderParts& parts)
{
	auto* settings = getTrackSettingsWidget();
	auto* outer = new QHBoxLayout(settings);
	outer->setContentsMargins(4, 2, 2, 1);
	outer->setSpacing(4);
	auto* rows = new QVBoxLayout();
	rows->setContentsMargins(0, 0, 0, 0);
	rows->setSpacing(1);
	outer->addLayout(rows, 1);

	const auto addTo = [](QHBoxLayout* row, QWidget* widget, int stretch = 0) {
		if (!widget) { return; }
		row->addWidget(widget, stretch);
		widget->show();
	};

	// On top: arming, the name, mute and solo (they were in the column on the left, with the gear)
	auto* first = new QHBoxLayout();
	first->setContentsMargins(0, 0, 0, 0);
	first->setSpacing(3);
	addTo(first, parts.leading);
	addTo(first, parts.arm);
	// (the name in a field of the row's height, its icon small, as in REAPER: the knobs' row below keeps its room)
	if (auto* label = qobject_cast<QAbstractButton*>(parts.label))
	{
		label->setFixedHeight(20);
		label->setIconSize(QSize{16, 16});
	}
	addTo(first, parts.label, 1);
	auto* mute = m_trackOperationsWidget.m_muteBtn;
	auto* solo = m_trackOperationsWidget.m_soloBtn;
	mute->setParent(settings);
	solo->setParent(settings);
	addTo(first, mute);
	addTo(first, solo);
	rows->addLayout(first);

	// Below: the knobs, then FX and I/O
	m_secondRow = new QWidget(settings);
	auto* second = new QHBoxLayout(m_secondRow);
	second->setContentsMargins(0, 0, 0, 0);
	second->setSpacing(4);
	// (the knobs at their size: an input meter between them and FX hides itself while the track is not armed)
	for (auto* knob : {parts.volume, parts.panning})
	{
		if (knob) { knob->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed); }
	}
	addTo(second, parts.volume);
	addTo(second, parts.panning);
	if (parts.middle) { addTo(second, parts.middle, 1); }
	second->addStretch(parts.middle ? 0 : 1);
	addTo(second, parts.fx);
	addTo(second, parts.io);
	rows->addWidget(m_secondRow);

	// When tall: the rest (input, monitor, recording)
	if (!parts.extra.empty())
	{
		m_thirdRow = new QWidget(settings);
		auto* third = new QHBoxLayout(m_thirdRow);
		third->setContentsMargins(0, 0, 0, 0);
		third->setSpacing(4);
		for (auto* widget : parts.extra) { addTo(third, widget); }
		third->addStretch(1);
		rows->addWidget(m_thirdRow);
	}
	rows->addStretch(1);

	// The level, on the right edge (a track with a mixer channel)
	if (parts.channel || Engine::mixer()->trackChannel(m_track))
	{
		outer->addWidget(new TrackLevelMeter(m_track, parts.channel, settings));
	}
	fitHeaderRows();
}




auto TrackView::makeFxButton(const EffectChain* chain, std::function<void()> open) -> QWidget*
{
	auto* button = new QPushButton(tr("FX"), getTrackSettingsWidget());
	button->setObjectName("btn-fx");
	button->setFocusPolicy(Qt::NoFocus);
	button->setCursor(Qt::PointingHandCursor);
	button->setCheckable(true);
	button->setToolTip(tr("The track's effects"));
	const auto refresh = [button, chain] {
		const auto count = chain ? static_cast<int>(chain->effects().size()) : 0;
		const auto text = count > 0 ? tr("FX %1").arg(count) : tr("FX");
		if (button->text() != text) { button->setText(text); }
		// (lit while it has effects, as REAPER's)
		button->setChecked(count > 0);
	};
	refresh();
	connect(getGUI()->mainWindow(), &MainWindow::periodicUpdate, button, refresh);
	connect(button, &QPushButton::clicked, button, [refresh, open = std::move(open)] {
		open();
		refresh();
	});
	return button;
}




auto TrackView::makeIoButton(std::function<void(QMenu*)> fill) -> QWidget*
{
	auto* button = new QPushButton(tr("I/O"), getTrackSettingsWidget());
	button->setObjectName("btn-io");
	button->setFocusPolicy(Qt::NoFocus);
	button->setCursor(Qt::PointingHandCursor);
	button->setToolTip(tr("Where the track takes its input from and goes to: its mixer channel, MIDI"));
	auto* menu = new QMenu(button);
	connect(menu, &QMenu::aboutToShow, button, [menu, fill = std::move(fill)] {
		menu->clear();
		fill(menu);
	});
	button->setMenu(menu);
	return button;
}




void TrackView::fitHeaderRows()
{
	// (by the height of the track's own lane: its lanes past the first add below)
	const auto own = m_lanesExpanded && m_singleLaneHeight > 0 ? m_singleLaneHeight : height();
	if (m_secondRow) { m_secondRow->setVisible(own >= CompactHeight); }
	if (m_thirdRow) { m_thirdRow->setVisible(own >= LargeHeight); }
}




void TrackView::contextMenuEvent(QContextMenuEvent* event)
{
	// (on the header only: the clips have their own menus)
	if (!m_trackSettingsWidget.geometry().contains(event->pos())
		&& !m_trackOperationsWidget.geometry().contains(event->pos()))
	{
		return QWidget::contextMenuEvent(event);
	}
	showTrackMenu(event->globalPos());
}




void TrackView::showTrackMenu(const QPoint& where)
{
	m_trackOperationsWidget.showMenu(where);
}




void TrackView::update()
{
	m_trackContentWidget.update();
	if( !m_trackContainerView->fixedClips() )
	{
		m_trackContentWidget.changePosition();
	}
	QWidget::update();
}




QMenu * TrackView::createMixerMenu(QString title, QString newMixerLabel)
{
	Q_UNUSED(title)
	Q_UNUSED(newMixerLabel)
	return nullptr;
}




bool TrackView::close()
{
	m_trackContainerView->removeTrackView( this );
	return QWidget::close();
}




void TrackView::modelChanged()
{
	m_track = castModel<Track>();
	Q_ASSERT( m_track != nullptr );
	connect( m_track, SIGNAL(destroyedTrack()), this, SLOT(close()));
	// The track's own mute and solo, on its source (see TrackContainer::updateSilence())
	m_trackOperationsWidget.m_muteBtn->setModel(&m_track->m_mutedModel);
	m_trackOperationsWidget.m_soloBtn->setModel(&m_track->m_soloModel);
	// Another track's solo or a folder's mute may silence this one
	connect(m_track->trackContainer(), &TrackContainer::silenceChanged, this, &TrackView::muteChanged,
		Qt::UniqueConnection);
	connect(m_track->trackContainer(), &TrackContainer::silenceChanged, &m_trackContentWidget,
		qOverload<>(&QWidget::update), Qt::UniqueConnection);
	ModelView::modelChanged();
	// Its height, and its lanes' (see fitLanes())
	m_lanesExpanded = false;
	setFixedHeight( m_track->getHeight() );
	fitLanes();
}




void TrackView::dragEnterEvent( QDragEnterEvent * dee )
{
	StringPairDrag::processDragEnterEvent( dee, "track_" +
					QString::number( static_cast<int>(m_track->type()) ) );
}




void TrackView::dropEvent( QDropEvent * de )
{
	QString type = StringPairDrag::decodeKey( de );
	QString value = StringPairDrag::decodeValue( de );
	if( type == ( "track_" + QString::number( static_cast<int>(m_track->type()) ) ) )
	{
		// value contains our XML-data so simply create a
		// DataFile which does the rest for us...
		DataFile dataFile( value.toUtf8() );
		// Without the audio engine's change lock: each piece the track restores takes it for its own moment (its
		// clips, its instrument, see Track::loadTrack() and InstrumentTrack::swapInstrument())
		m_track->restoreState( dataFile.content().firstChild().toElement() );
		de->accept();
	}
}




void TrackView::fitLanes()
{
	const auto lanes = m_track->laneCount();
	if (!m_lanesExpanded) { m_singleLaneHeight = height(); }
	if (lanes > 1)
	{
		setFixedHeight(m_singleLaneHeight + (lanes - 1) * LaneHeight);
		m_lanesExpanded = true;
	}
	else if (m_lanesExpanded)
	{
		setFixedHeight(m_singleLaneHeight);
		m_lanesExpanded = false;
	}
	m_trackContentWidget.update();
}




void TrackView::mousePressEvent( QMouseEvent * me )
{
	const auto pos = position(me);

	// If previously dragged too small, restore on shift-leftclick
	if( height() < DEFAULT_TRACK_HEIGHT &&
		me->modifiers() & Qt::ShiftModifier &&
		me->button() == Qt::LeftButton )
	{
		setFixedHeight( DEFAULT_TRACK_HEIGHT );
		m_track->setHeight( DEFAULT_TRACK_HEIGHT );
	}


	int widgetTotal = m_trackContainerView->trackHeadWidth();

	if (m_trackContainerView->allowRubberband() == true  && pos.x() > widgetTotal)
	{
		QWidget::mousePressEvent( me );
	}
	else if( me->button() == Qt::LeftButton )
	{
		if (me->x() > widgetTotal - ResizeGripWidth && me->x() <= widgetTotal)
		{
			m_action = Action::ResizeHorizontal;
		}
		else if (me->modifiers() & Qt::ShiftModifier || (me->y() > height() - ResizeGripWidth && me->x() <= widgetTotal))
		{
			m_action = Action::ResizeVertical;
			QCursor::setPos(mapToGlobal(QPoint(pos.x(), height())));
			QApplication::setOverrideCursor(Qt::SizeVerCursor);
		}

		me->accept();
	}
	else
	{
		QWidget::mousePressEvent( me );
	}
}




void TrackView::mouseMoveEvent( QMouseEvent * me )
{

	const auto pos = position(me);

	int widgetTotal = m_trackContainerView->trackHeadWidth();
	if (m_trackContainerView->allowRubberband() == true && pos.x() > widgetTotal)
	{
		QWidget::mouseMoveEvent( me );
	}
	else if( m_action == Action::Move )
	{
		// look which track-widget the mouse-cursor is over
		const int yPos =
			m_trackContainerView->contentWidget()->mapFromGlobal(globalPosition(me)).y();
		const TrackView * trackAtY = m_trackContainerView->trackViewAt( yPos );

		// debug code
		//	qDebug( "y position %d", yPos );

		const auto container = m_trackContainerView->model();
		auto addUndoStep = [this, container] {
			if (!m_movedByGrip) { container->addLayoutCheckPoint(); }
			m_movedByGrip = true;
		};

		// a track-widget not equal to ourself (or what we hold)? then move us up/down there, with what we hold
		if (trackAtY != nullptr && trackAtY != this && !container->isInFolder(trackAtY->getTrack(), m_track))
		{
			addUndoStep();
			container->moveTrackStep(m_track, pos.y() >= 0);
		}

		// Sideways: into the folder above or out of the folder
		const int x = globalPosition(me).x();
		if (std::abs(x - m_gripLevelX) >= 3 * FolderIndent)
		{
			const int delta = x > m_gripLevelX ? 1 : -1;
			const int depth = m_track->folderDepth();
			if (delta > 0 || depth > 0)
			{
				addUndoStep();
				container->changeFolderDepth(m_track, delta, false);
			}
			if (m_track->folderDepth() != depth) { m_gripLevelX = x; }
		}
	}
	else if (m_action == Action::ResizeVertical)
	{
		resizeToHeight(pos.y());
		setCursor(Qt::SizeVerCursor);
	}
	else if (m_action == Action::ResizeHorizontal)
	{
		m_trackContainerView->setTrackHeadWidth(pos.x());
		setCursor(Qt::SizeHorCursor);
	}
	else if (pos.x() > widgetTotal - ResizeGripWidth && pos.x() < widgetTotal)
	{
		setCursor(Qt::SizeHorCursor);
	}
	else if (pos.y() > height() - ResizeGripWidth && pos.x() < widgetTotal)
	{
		setCursor(Qt::SizeVerCursor);
	}
	else
	{
		setCursor(Qt::ArrowCursor);
	}

	if( height() < DEFAULT_TRACK_HEIGHT )
	{
		setToolTip(m_track->m_name);
	}
}



void TrackView::mouseReleaseEvent( QMouseEvent * me )
{
	m_action = Action::None;
	while( QApplication::overrideCursor() != nullptr )
	{
		QApplication::restoreOverrideCursor();
	}
	m_trackOperationsWidget.update();
	setCursor(Qt::ArrowCursor);

	QWidget::mouseReleaseEvent( me );
}

void TrackView::wheelEvent(QWheelEvent* we)
{
	// Note: we add the values because one of them will be 0. If the alt modifier
	// is pressed x is non-zero and otherwise y.
	const int deltaY = we->angleDelta().x() + we->angleDelta().y();
	int const direction = deltaY < 0 ? -1 : 1;

	auto const modKeys = we->modifiers();
	int stepSize = modKeys == (Qt::ControlModifier | Qt::AltModifier) ? 1 : modKeys == (Qt::ShiftModifier | Qt::AltModifier) ? 5 : 0;

	if (stepSize != 0)
	{
		resizeToHeight(height() + stepSize * direction);
		we->accept();
		return;
	}
	we->ignore();
}




void TrackView::paintEvent( QPaintEvent * pe )
{
	QStyleOption opt;
	opt.initFrom( this );
	QPainter p( this );
	style()->drawPrimitive( QStyle::PE_Widget, &opt, &p, this );
}




void TrackView::createClipView( Clip * clip )
{
	ClipView * tv = clip->createView( this );
	if( clip->getSelectViewOnCreate() == true )
	{
		tv->setSelected( true );
	}
	clip->selectViewOnCreate( false );
}




void TrackView::muteChanged()
{
	FadeButton * indicator = getActivityIndicator();
	if (indicator) { setIndicatorMute(indicator, m_track->isMutedInMix()); }
}


void TrackView::onTrackGripGrabbed()
{
	m_action = Action::Move;
	m_movedByGrip = false;
	m_gripLevelX = QCursor::pos().x();
}

void TrackView::onTrackGripReleased()
{
	m_action = Action::None;
}



void TrackView::setIndicatorMute(FadeButton* indicator, bool muted)
{
	indicator->setMuted(muted);
}


void TrackView::resizeToHeight(int h)
{
	setFixedHeight(qMax<int>(h, MINIMAL_TRACK_HEIGHT));
	m_trackContainerView->realignTracks();
	m_track->setHeight(height());
}

void TrackView::updateWidth(int width)
{
	m_trackSettingsWidget.setFixedWidth(width - m_trackOperationsWidget.width());
}

} // namespace lmms::gui
