/*
 * SongEditor.cpp - basic window for song-editing
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

#include "SongEditor.h"

#include "TrackContentWidget.h"

#include "Clipboard.h"
#include "ProjectJournal.h"
#include "Shortcuts.h"
#include "TempoLaneWidget.h"

#include <cmath>

#include <QAction>
#include <QDomElement>
#include <QCursor>
#include <QKeyEvent>
#include <QLabel>
#include <QMdiArea>
#include <QScrollBar>
#include <QSlider>
#include <QTimeLine>

#include "ActionGroup.h"
#include "AudioEngine.h"
#include "AutomatableSlider.h"
#include "ClipView.h"
#include "ComboBox.h"
#include "ConfigManager.h"
#include "CPULoadWidget.h"
#include "DeprecationHelper.h"
#include "embed.h"
#include "GuiApplication.h"
#include "LcdSpinBox.h"
#include "MainWindow.h"
#include "MeterDialog.h"
#include "Oscilloscope.h"
#include "PianoRoll.h"
#include "PositionLine.h"
#include "SubWindow.h"
#include "TextFloat.h"
#include "TimeDisplayWidget.h"
#include "TimeLineWidget.h"
#include "TrackView.h"

namespace lmms::gui
{

namespace
{

constexpr int MIN_PIXELS_PER_BAR = 4;
constexpr int MAX_PIXELS_PER_BAR = 400;
constexpr int ZOOM_STEPS = 200;

constexpr std::array SNAP_SIZES{8.f, 4.f, 2.f, 1.f, 1/2.f, 1/4.f, 1/8.f, 1/16.f};
constexpr std::array PROPORTIONAL_SNAP_SIZES{64.f, 32.f, 16.f, 8.f, 4.f, 2.f, 1.f, 1/2.f, 1/4.f, 1/8.f, 1/16.f, 1/32.f, 1/64.f};

}



SongEditor::SongEditor( Song * song ) :
	TrackContainerView( song ),
	m_song( song ),
	m_zoomingModel(new IntModel(calculateZoomSliderValue(DEFAULT_PIXELS_PER_BAR), 0, ZOOM_STEPS, nullptr, tr("Zoom"))),
	m_snappingModel(new ComboBoxModel()),
	m_proportionalSnap( false ),
	m_scrollBack( false ),
	m_smoothScroll( ConfigManager::inst()->value( "ui", "smoothscroll" ).toInt() ),
	m_mode(EditMode::Draw),
	m_origin(),
	m_scrollPos(),
	m_mousePos(),
	m_rubberBandStartTrackview(0),
	m_rubberbandStartTimePos(0),
	m_rubberbandPixelsPerBar(DEFAULT_PIXELS_PER_BAR),
	m_selectRegion(false)
{
	// Set up timeline
	m_timeLine = new TimeLineWidget(trackHeadWidth(), 32, pixelsPerBar(),
		m_song->getTimeline(Song::PlayMode::Song),
		m_currentPosition, this
	);
	connect(this, &TrackContainerView::positionChanged, m_timeLine, qOverload<>(&QWidget::update));
	connect(m_timeLine->timeline(), &Timeline::positionChanged, this, &SongEditor::updatePosition);
	connect( m_timeLine, SIGNAL(regionSelectedFromPixels(int,int)),
			this, SLOT(selectRegionFromPixels(int,int)));
	connect( m_timeLine, SIGNAL(selectionFinished()),
			 this, SLOT(stopRubberBand()));
	connect(this, &TrackContainerView::trackHeadWidthChanged, m_timeLine, [this](int width){ m_timeLine->setXOffset(width); });
	connect(this, &TrackContainerView::trackHeadWidthChanged, this, &SongEditor::updatePositionLine);

	// when tracks realign, adjust height of position line
	connect(this, &TrackContainerView::tracksRealigned, this, &SongEditor::updatePositionLine);

	m_positionLine = new PositionLine(this, Song::PlayMode::Song);
	static_cast<QVBoxLayout *>( layout() )->insertWidget( 1, m_timeLine );

	connect( m_song, SIGNAL(playbackStateChanged()),
			 m_positionLine, SLOT(update()));

	// When zoom changes, update position line
	// But we must convert pixels per bar to a zoom factor where 1.0 is 100%
	connect(this, &SongEditor::pixelsPerBarChanged, m_positionLine,
		[this]() {
			m_positionLine->zoomChange(pixelsPerBar() / float(DEFAULT_PIXELS_PER_BAR));
			updatePositionLine();
		});

	// Ensure loop markers snap to same increments as clips. Zoom & proportional
	// snap changes are handled in zoomingChanged() and toggleProportionalSnap()
	connect(m_snappingModel, &ComboBoxModel::dataChanged,
		[this]() { m_timeLine->setSnapSize(getSnapSize()); });



	static_cast<QVBoxLayout *>( layout() )->insertWidget( 0, m_timeLine );

	// The tempo map's lane, under the ruler and on its scale
	m_tempoLane = new TempoLaneWidget(m_timeLine, &m_song->tempoMap(), trackHeadWidth(), this);
	static_cast<QVBoxLayout*>(layout())->insertWidget(1, m_tempoLane);
	connect(this, &TrackContainerView::positionChanged, m_tempoLane, qOverload<>(&QWidget::update));
	connect(this, &TrackContainerView::trackHeadWidthChanged, m_tempoLane, &TempoLaneWidget::setHeadWidth);
	connect(m_song, &Song::tempoChanged, m_tempoLane, qOverload<>(&QWidget::update));

	m_leftRightScroll = new QScrollBar( Qt::Horizontal, this );
	m_leftRightScroll->setMinimum(0);
	m_leftRightScroll->setMaximum(0);
	m_leftRightScroll->setSingleStep(1);
	m_leftRightScroll->setPageStep(20 * TimePos::ticksPerBar());
	static_cast<QVBoxLayout *>( layout() )->addWidget( m_leftRightScroll );
	connect( m_leftRightScroll, SIGNAL(valueChanged(int)),
					this, SLOT(scrolled(int)));
	connect( m_song, SIGNAL(lengthChanged(int)),
			this, SLOT(updateScrollBar(int)));
	connect(m_leftRightScroll, SIGNAL(valueChanged(int)),this, SLOT(updateRubberband()));
	connect(contentWidget()->verticalScrollBar(), SIGNAL(valueChanged(int)),this, SLOT(updateRubberband()));
	connect(m_timeLine, SIGNAL(selectionFinished()), this, SLOT(stopSelectRegion()));


	// Set up zooming model
	m_zoomingModel->setParent(this);
	m_zoomingModel->setJournalling(false);
	connect(m_zoomingModel, SIGNAL(dataChanged()), this, SLOT(zoomingChanged()));
	m_zoomingModel->setInitValue(ConfigManager::inst()->value("ui","songeditorzoom",QString::number(calculateZoomSliderValue(DEFAULT_PIXELS_PER_BAR))).toInt());


	// Set up snapping model
	m_snappingModel->setParent(this);
	for (float bars : SNAP_SIZES)
	{
		if (bars > 1.0f)
		{
			m_snappingModel->addItem(QString("%1 Bars").arg(bars));
		}
		else if (bars == 1.0f)
		{
			m_snappingModel->addItem( "1 Bar" );
		}
		else
		{
			m_snappingModel->addItem(QString("1/%1 Bar").arg(1 / bars));
		}
	}
	m_snappingModel->setInitValue(ConfigManager::inst()->value("ui", "songeditorsnap", QString::number(m_snappingModel->findText( "1/4 Bar" ))).toInt());

	setFocusPolicy( Qt::StrongFocus );
	setFocus();
}

SongEditor::~SongEditor()
{
	ConfigManager::inst()->setValue("ui", "songeditorzoom", QString::number(m_zoomingModel->value()));
	ConfigManager::inst()->setValue("ui", "songeditorsnap", QString::number(m_snappingModel->value()));
}


void SongEditor::saveSettings( QDomDocument& doc, QDomElement& element )
{
	MainWindow::saveWidgetState( parentWidget(), element );
	element.setAttribute("trackheadwidth", trackHeadWidth());
}

void SongEditor::loadSettings( const QDomElement& element )
{
	MainWindow::restoreWidgetState(parentWidget(), element);
	setTrackHeadWidth(element.attribute("trackheadwidth", QString::number(trackHeadWidth())).toInt());
}




float SongEditor::getSnapSize() const
{
	float snapSize = SNAP_SIZES[m_snappingModel->value()];

	// If proportional snap is on, we snap to finer values when zoomed in
	if (m_proportionalSnap)
	{
		// Finds the closest available snap size
		const float optimalSize = snapSize * DEFAULT_PIXELS_PER_BAR / pixelsPerBar();
		return *std::min_element(PROPORTIONAL_SNAP_SIZES.begin(), PROPORTIONAL_SNAP_SIZES.end(), [optimalSize](float a, float b)
		{
			return std::abs(a - optimalSize) < std::abs(b - optimalSize);
		});
	}

	return snapSize;
}

QString SongEditor::getSnapSizeString() const
{
	float bars = getSnapSize();

	if (bars < 1)
	{
		return QString(tr("1/%1 Bar")).arg(round(1 / bars));
	}
	else if (bars >= 2)
	{
		return QString(tr("%1 Bars")).arg(bars);
	}
	else
	{
		return QString("1 Bar");
	}
}

void SongEditor::scrolled( int new_pos )
{
	update();
	emit positionChanged(m_currentPosition = TimePos(new_pos));
	updatePositionLine();
}




void SongEditor::selectRegionFromPixels(int xStart, int xEnd)
{
	if (!m_selectRegion)
	{
		m_selectRegion = true;

		//deselect all clips
		for (auto &it : findChildren<selectableObject *>()) { it->setSelected(false); }

		rubberBand()->setEnabled(true);
		rubberBand()->show();

		//we save the position of scrollbars, mouse position and zooming level
		m_origin = QPoint(xStart, 0);
		m_scrollPos = QPoint(m_leftRightScroll->value(), contentWidget()->verticalScrollBar()->value());
		m_rubberbandPixelsPerBar = pixelsPerBar();

		//calculate the song position where the mouse was clicked
		m_rubberbandStartTimePos = TimePos((xStart - trackHeadWidth())
											/ pixelsPerBar() * TimePos::ticksPerBar())
											+ m_currentPosition;
		m_rubberBandStartTrackview = 0;
	}
	//the current mouse position within the borders of song editor
	m_mousePos = QPoint(qMax(trackHeadWidth(), qMin(xEnd, width()))
						, std::numeric_limits<int>::max());
	updateRubberband();
}




void SongEditor::stopSelectRegion()
{
	m_selectRegion = false;
}




void SongEditor::updateRubberband()
{
	if (rubberBandActive())
	{
		int originX = m_origin.x();

		//take care of the zooming
		if (m_rubberbandPixelsPerBar != pixelsPerBar())
		{
			originX = trackHeadWidth() + (originX - trackHeadWidth()) * pixelsPerBar() / m_rubberbandPixelsPerBar;
		}

		//take care of the scrollbar position
		int hs = (m_leftRightScroll->value() - m_scrollPos.x()) * pixelsPerBar() / TimePos::ticksPerBar();
		int vs = contentWidget()->verticalScrollBar()->value() - m_scrollPos.y();

		//the adjusted origin point
		QPoint origin = QPoint(qMax(originX - hs, trackHeadWidth()), m_origin.y() - vs);

		//paint the rubber band rect
		rubberBand()->setGeometry(QRect(origin,
										contentWidget()->mapFromParent(QPoint(m_mousePos.x(), m_mousePos.y()))
										).normalized());

		//the index of the TrackView the mouse is hover
		int rubberBandTrackview = trackIndexFromSelectionPoint(m_mousePos.y());

		//the time position the mouse is hover
		TimePos rubberbandTimePos = TimePos((qMin(m_mousePos.x(), width()) - trackHeadWidth())
											  / pixelsPerBar() * TimePos::ticksPerBar())
											  + m_currentPosition;

		//are clips in the rect of selection?
		for (auto &it : findChildren<selectableObject *>())
		{
			auto clip = dynamic_cast<ClipView*>(it);
			if (clip)
			{
				auto indexOfTrackView = trackViews().indexOf(clip->getTrackView());
				bool isBeetweenRubberbandViews = indexOfTrackView >= qMin(m_rubberBandStartTrackview, rubberBandTrackview)
											  && indexOfTrackView <= qMax(m_rubberBandStartTrackview, rubberBandTrackview);
				bool isBeetweenRubberbandTimePos = clip->getClip()->endPosition() >= qMin(m_rubberbandStartTimePos, rubberbandTimePos)
											  && clip->getClip()->startPosition() <= qMax(m_rubberbandStartTimePos, rubberbandTimePos);
				it->setSelected(isBeetweenRubberbandViews && isBeetweenRubberbandTimePos);
			}
		}
	}
}




void SongEditor::setEditMode( EditMode mode )
{
	m_mode = mode;
}

void SongEditor::setEditModeDraw()
{
	setEditMode(EditMode::Draw);
}

void SongEditor::setEditModeKnife()
{
	setEditMode(EditMode::Knife);
}

void SongEditor::setEditModeSelect()
{
	setEditMode(EditMode::Select);
}

void SongEditor::toggleProportionalSnap()
{
	m_proportionalSnap = !m_proportionalSnap;
	m_timeLine->setSnapSize(getSnapSize());

	emit proportionalSnapChanged();
}




void SongEditor::splitAtPlayhead()
{
	// The selected clips the playhead crosses; with none selected, every clip it crosses (as REAPER's S)
	const auto at = m_song->getPlayPos(Song::PlayMode::Song);
	auto targets = QVector<ClipView*>{};
	const auto selected = selectedObjects();
	for (const auto view : trackViews())
	{
		for (const auto clipView : view->getTrackContentWidget()->clipViews())
		{
			const auto clip = clipView->getClip();
			if (clip->startPosition() >= at || clip->endPosition() <= at) { continue; }
			if (!selected.isEmpty() && !clipView->isSelected()) { continue; }
			targets.push_back(clipView);
		}
	}
	if (targets.isEmpty()) { return; }
	const auto group = ProjectJournal::Group{Engine::projectJournal()};
	for (const auto clipView : targets) { clipView->splitClip(at - clipView->getClip()->startPosition()); }
}




void SongEditor::copyPaste(int key)
{
	// The selected clips, the first (the topmost, leftmost) being the one they are copied around
	auto selected = QVector<ClipView*>{};
	for (const auto object : selectedObjects())
	{
		if (auto view = dynamic_cast<ClipView*>(object)) { selected.push_back(view); }
	}
	std::sort(selected.begin(), selected.end(), [](ClipView* a, ClipView* b) {
		const auto& tracks = Engine::getSong()->tracks();
		const auto ia = std::find(tracks.begin(), tracks.end(), a->getClip()->getTrack());
		const auto ib = std::find(tracks.begin(), tracks.end(), b->getClip()->getTrack());
		if (ia != ib) { return ia < ib; }
		return a->getClip()->startPosition() < b->getClip()->startPosition();
	});

	// The track a paste goes to: the first selected clip's, else the one under the mouse, else the first
	const auto targetTrack = [&]() -> TrackView* {
		if (!selected.isEmpty()) { return selected.front()->getTrackView(); }
		const auto mouse = mapFromGlobal(QCursor::pos());
		for (const auto view : trackViews())
		{
			const auto area = QRect{view->mapTo(this, QPoint{0, 0}), view->size()};
			if (area.contains(mouse)) { return view; }
		}
		return trackViews().isEmpty() ? nullptr : trackViews().front();
	};
	const auto paste = [&](const TimePos& at) {
		if (auto view = targetTrack())
		{
			view->getTrackContentWidget()->pasteSelection(at, Clipboard::getMimeData(), false, at);
		}
	};

	switch (key)
	{
	case Qt::Key_C:
		if (!selected.isEmpty()) { selected.front()->copy(selected); }
		break;
	case Qt::Key_X:
		if (!selected.isEmpty()) { selected.front()->cut(selected); }
		break;
	case Qt::Key_V:
		// At the playhead
		paste(m_song->getPlayPos(Song::PlayMode::Song));
		break;
	case Qt::Key_D:
	{
		// Duplicate: right after the selection
		if (selected.isEmpty()) { break; }
		auto end = TimePos{0};
		auto start = selected.front()->getClip()->startPosition();
		for (const auto view : selected)
		{
			end = std::max(end, view->getClip()->endPosition());
			start = std::min(start, view->getClip()->startPosition());
		}
		selected.front()->copy(selected);
		// (to the start of the bar after it, as clips are laid out)
		paste(TimePos{start.getTicks() + TimePos{end.getTicks() - start.getTicks()}.nextFullBar() * TimePos::ticksPerBar()});
		break;
	}
	default:
		break;
	}
}




void SongEditor::keyPressEvent( QKeyEvent * ke )
{
	bool isShiftPressed = ke->modifiers() & Qt::ShiftModifier;
	if( isShiftPressed &&
						( ke->key() == Qt::Key_Insert || ke->key() == Qt::Key_Enter || ke->key() == Qt::Key_Return ) )
	{
		m_song->insertBar();
	}
	else if( isShiftPressed &&
						( ke->key() == Qt::Key_Delete || ke->key() == Qt::Key_Backspace ) )
	{
		m_song->removeBar();
	}
	else if( ke->key() == Qt::Key_Left )
	{
		tick_t t = m_song->getPlayPos(Song::PlayMode::Song).getTicks() - TimePos::ticksPerBar();
		if( t >= 0 )
		{
			m_song->setPlayPos( t, Song::PlayMode::Song );
			if (!m_song->isPlaying()) { m_song->setPlayPos(t, Song::PlayMode::None); }
		}
	}
	else if( ke->key() == Qt::Key_Right )
	{
		tick_t t = m_song->getPlayPos(Song::PlayMode::Song).getTicks() + TimePos::ticksPerBar();
		if( t < MaxSongLength )
		{
			m_song->setPlayPos( t, Song::PlayMode::Song );
			if (!m_song->isPlaying()) { m_song->setPlayPos(t, Song::PlayMode::None); }
		}
	}
	else if( ke->key() == Qt::Key_Home )
	{
		m_song->setPlayPos( 0, Song::PlayMode::Song );
		if (!m_song->isPlaying()) { m_song->setPlayPos(0, Song::PlayMode::None); }
	}
	else if( ke->key() == Qt::Key_Delete || ke->key() == Qt::Key_Backspace )
	{
		QVector<selectableObject *> so = selectedObjects();
		for (const auto& selectedClip : so)
		{
			auto clipv = dynamic_cast<ClipView*>(selectedClip);
			clipv->remove();
		}
	}
	else if( ke->key() == Qt::Key_A && ke->modifiers() & Qt::ControlModifier )
	{
		selectAllClips( !isShiftPressed );
	}
	else if (Shortcuts::matches("split", ke))
	{
		splitAtPlayhead();
	}
	else if (Shortcuts::matches("marker", ke))
	{
		// As in REAPER: a marker at the playhead, named after its number (renamed with a double-click)
		auto& markers = m_song->markers();
		const auto tick = m_song->getPlayPos(Song::PlayMode::Song).getTicks();
		markers.add(tick, tr("Marker %1").arg(markers.markers().size() + 1));
		m_song->setModified();
	}
	else if (Shortcuts::matches("region", ke))
	{
		// A region over the cycle (named after its number; renamed with a double-click)
		const auto& timeline = m_song->getTimeline(Song::PlayMode::Song);
		auto& markers = m_song->markers();
		if (timeline.loopEnd() > timeline.loopBegin())
		{
			markers.addRegion(timeline.loopBegin().getTicks(), timeline.loopEnd().getTicks(),
				tr("Region %1").arg(markers.regions().size() + 1));
			m_song->setModified();
		}
	}
	else if (ke->key() >= Qt::Key_1 && ke->key() <= Qt::Key_9 && ke->modifiers() == Qt::NoModifier)
	{
		// As in REAPER: to marker 1 to 9
		const auto index = static_cast<std::size_t>(ke->key() - Qt::Key_1);
		const auto& markers = m_song->markers().markers();
		if (index < markers.size())
		{
			const auto tick = markers[index].tick;
			m_song->setPlayPos(tick, Song::PlayMode::Song);
			if (!m_song->isPlaying()) { m_song->setPlayPos(tick, Song::PlayMode::None); }
		}
	}
	else if (Shortcuts::matches("copy", ke)) { copyPaste(Qt::Key_C); }
	else if (Shortcuts::matches("cut", ke)) { copyPaste(Qt::Key_X); }
	else if (Shortcuts::matches("paste", ke)) { copyPaste(Qt::Key_V); }
	else if (Shortcuts::matches("duplicate", ke)) { copyPaste(Qt::Key_D); }
	else if( ke->key() == Qt::Key_Escape )
	{
		selectAllClips( false );
	}
	else if (ke->key() == Qt::Key_0 && ke->modifiers() & Qt::ControlModifier)
	{
		m_zoomingModel->reset();
	}
	else
	{
		QWidget::keyPressEvent(ke);
	}
}



void SongEditor::adjustLeftRightScoll(int value)
{
	m_leftRightScroll->setValue(m_leftRightScroll->value()
						- value * DEFAULT_PIXELS_PER_BAR / pixelsPerBar());
}


void SongEditor::wheelEvent( QWheelEvent * we )
{
	// As in REAPER: over the clips, the wheel zooms horizontally, Ctrl+wheel the tracks' height, Alt+wheel scrolls
	// horizontally (Qt hands it as a horizontal wheel), Shift+wheel scrolls vertically, as does the wheel over the
	// tracks' heads; a horizontal wheel (a trackpad) scrolls horizontally
	const auto posX = we->position().toPoint().x();
	const bool overClips = posX > trackHeadWidth();
	const bool vertical = std::abs(we->angleDelta().y()) >= std::abs(we->angleDelta().x());
	const auto mods = we->modifiers() & (Qt::ControlModifier | Qt::ShiftModifier | Qt::AltModifier);
	if (overClips && vertical && mods == Qt::ControlModifier)
	{
		// The tracks' height, all together
		const int direction = we->angleDelta().y() > 0 ? 1 : -1;
		for (const auto view : trackViews())
		{
			const int height = view->getTrack()->getHeight();
			view->resizeToHeight(std::clamp(height + direction * std::max(4, height / 8), MINIMAL_TRACK_HEIGHT, 256));
		}
		we->accept();
		return;
	}
	if (overClips && vertical && mods == Qt::NoModifier)
	{
		int x = posX - trackHeadWidth();
		// tick based on the mouse x-position where the scroll wheel was used
		int tick = x / pixelsPerBar() * TimePos::ticksPerBar();

		// move zoom slider (pixelsPerBar will change automatically)
		int step = 5;
		// when Alt is pressed, wheelEvent returns delta for x coordinate (mimics horizontal mouse wheel)
		int direction = (we->angleDelta().y() + we->angleDelta().x()) > 0 ? 1 : -1;
		m_zoomingModel->incValue(step * direction);

		// scroll to zooming around cursor's tick
		int newTick = static_cast<int>(x / pixelsPerBar() * TimePos::ticksPerBar());
		m_leftRightScroll->setValue(m_leftRightScroll->value() + tick - newTick);

		// update timeline
		m_timeLine->setPixelsPerBar(pixelsPerBar());
		// and make sure, all Clip's are resized and relocated
		realignTracks();
	}

	// FIXME: Reconsider if determining orientation is necessary in Qt6.
	else if (std::abs(we->angleDelta().x()) > std::abs(we->angleDelta().y())) // scrolling is horizontal
	{
		adjustLeftRightScoll(we->angleDelta().x());
	}
	else
	{
		// Vertical: the scroll area scrolls the tracks
		we->ignore();
		return;
	}
	we->accept();
}




void SongEditor::mousePressEvent(QMouseEvent *me)
{
	const auto pos = position(me);
	
	if (allowRubberband())
	{
		//we save the position of scrollbars, mouse position and zooming level
		m_scrollPos = QPoint(m_leftRightScroll->value(), contentWidget()->verticalScrollBar()->value());
		m_origin = contentWidget()->mapFromParent(pos);
		m_rubberbandPixelsPerBar = pixelsPerBar();

		//paint the rubberband
		rubberBand()->setEnabled(true);
		rubberBand()->setGeometry(QRect(m_origin, QSize()));
		rubberBand()->show();

		//the trackView(index) and the time position where the mouse was clicked
		m_rubberBandStartTrackview = trackIndexFromSelectionPoint(pos.y());
		m_rubberbandStartTimePos = TimePos((pos.x() - trackHeadWidth())
											/ pixelsPerBar() * TimePos::ticksPerBar())
											+ m_currentPosition;
	}
	QWidget::mousePressEvent(me);
}




void SongEditor::mouseMoveEvent(QMouseEvent *me)
{
	m_mousePos = position(me);
	updateRubberband();
	QWidget::mouseMoveEvent(me);
}




void SongEditor::mouseReleaseEvent(QMouseEvent *me)
{
	rubberBand()->hide();
	rubberBand()->setEnabled(false);
	QWidget::mouseReleaseEvent(me);
}








void SongEditor::updateScrollBar(int len)
{
	m_leftRightScroll->setMaximum(len * TimePos::ticksPerBar());
}




static inline void animateScroll( QScrollBar *scrollBar, int newVal, bool smoothScroll )
{
	if( smoothScroll == false )
	{
		scrollBar->setValue( newVal );
	}
	else
	{
		// do smooth scroll animation using QTimeLine
		auto t = scrollBar->findChild<QTimeLine*>();
		if( t == nullptr )
		{
			t = new QTimeLine( 600, scrollBar );
			t->setFrameRange( scrollBar->value(), newVal );
			t->connect( t, SIGNAL(finished()), SLOT(deleteLater()));

			scrollBar->connect( t, SIGNAL(frameChanged(int)), SLOT(setValue(int)));

			t->start();
		}
		else
		{
			// smooth scrolling is still active, therefore just update the end frame
			t->setEndFrame( newVal );
		}
	}
}




void SongEditor::updatePosition()
{
	const TimePos& t = m_timeLine->timeline()->pos();
	if ((m_song->isPlaying() && m_song->m_playMode == Song::PlayMode::Song)
							|| m_scrollBack)
	{
		m_smoothScroll = ConfigManager::inst()->value( "ui", "smoothscroll" ).toInt();
		const int w = width() - trackHeadWidth()
							- contentWidget()->verticalScrollBar()->width(); // width of right scrollbar
		
		if (m_timeLine->autoScroll() == TimeLineWidget::AutoScrollState::Stepped)
		{
			const auto nextPosition = m_currentPosition + w * TimePos::ticksPerBar() / pixelsPerBar();
			if (t > nextPosition || t < m_currentPosition) 
			{
				animateScroll(m_leftRightScroll, t.getTicks(), m_smoothScroll);
			}
		}
		else if (m_timeLine->autoScroll() == TimeLineWidget::AutoScrollState::Continuous)
		{
			m_leftRightScroll->setValue(std::max(t.getTicks() - w * TimePos::ticksPerBar() / pixelsPerBar() / 2, 0.0f));
		}
		m_scrollBack = false;
	}

	updatePositionLine();
}




void SongEditor::updatePositionLine()
{
	const int x = m_timeLine->markerX(m_timeLine->timeline()->pos());
	if (x >= trackHeadWidth() - 1)
	{
		m_positionLine->show();
		m_positionLine->move( x-( m_positionLine->width() - 1 ), m_timeLine->height() );
	}
	else
	{
		m_positionLine->hide();
	}

	m_positionLine->setFixedHeight(totalHeightOfTracks());
}




int SongEditor::calculatePixelsPerBar() const
{
	// What we need to raise 2 by to get MIN_PIXELS_PER_BAR and MAX_PIXELS_PER_BAR
	static const double minExp = std::log2(MIN_PIXELS_PER_BAR);
	static const double maxExp = std::log2(MAX_PIXELS_PER_BAR);
	static const double stepsInv = 1 / static_cast<double>(ZOOM_STEPS) * (maxExp - minExp);
	double exponent = m_zoomingModel->value() * stepsInv + minExp;

	double ppb = std::exp2(exponent);

	return static_cast<int>(std::round(ppb));
}




int SongEditor::calculateZoomSliderValue(int pixelsPerBar) const
{
	// What we need to raise 2 by to get MIN_PIXELS_PER_BAR and MAX_PIXELS_PER_BAR
	static const double minExp = std::log2(MIN_PIXELS_PER_BAR);
	static const double maxExp = std::log2(MAX_PIXELS_PER_BAR);
	double exponent = std::log2(pixelsPerBar);

	double sliderValue = (exponent - minExp) / (maxExp - minExp) * ZOOM_STEPS;

	return static_cast<int>(std::round(sliderValue));
}




void SongEditor::zoomingChanged()
{
	int ppb = calculatePixelsPerBar();
	setPixelsPerBar(ppb);

	m_timeLine->setPixelsPerBar(ppb);
	m_tempoLane->update();
	realignTracks();
	updateRubberband();
	m_timeLine->setSnapSize(getSnapSize());

	emit pixelsPerBarChanged(ppb);
}


void SongEditor::selectAllClips( bool select )
{
	QVector<selectableObject *> so = select ? rubberBand()->selectableObjects() : rubberBand()->selectedObjects();
	for( int i = 0; i < so.count(); ++i )
	{
		so.at(i)->setSelected( select );
	}
}




bool SongEditor::allowRubberband() const
{
	return m_mode == EditMode::Select;
}




bool SongEditor::knifeMode() const
{
	return m_mode == EditMode::Knife;
}




int SongEditor::trackIndexFromSelectionPoint(int yPos)
{
	const TrackView * tv = trackViewAt(yPos - m_timeLine->height());
	return tv ? indexOfTrackView(tv)
			  : yPos < m_timeLine->height() ? 0
											: trackViews().count();
}




int SongEditor::indexOfTrackView(const TrackView *tv)
{
	return static_cast<int>(std::distance(trackViews().begin(),
										  std::find(trackViews().begin(), trackViews().end(), tv)));
}




ComboBoxModel *SongEditor::snappingModel() const
{
	return m_snappingModel;
}




SongEditorWindow::SongEditorWindow(Song* song) :
	Editor(false, false, false),
	m_editor(new SongEditor(song)),
	m_crtlAction( nullptr ),
	m_snapSizeLabel( new QLabel( m_toolBar ) )
{
	setWindowTitle( tr( "Song-Editor" ) );
	setWindowIcon( embed::getIconPixmap( "songeditor" ) );

	setCentralWidget(m_editor);
	setAcceptDrops(true);
	m_toolBar->setAcceptDrops(true);

	// The transport stopped (from here, the transport bar or a shortcut): the piano roll's recording ends, and the
	// timeline's recording marks go
	connect(&song->transport(), &Transport::stopped, this, [this] {
		getGUI()->pianoRoll()->stopRecording();
		m_editor->m_timeLine->setRecording(false);
		m_editor->m_positionLine->setRecording(false);
	});
	connect(m_toolBar, SIGNAL(dragEntered(QDragEnterEvent*)), m_editor, SLOT(dragEnterEvent(QDragEnterEvent*)));
	connect(m_toolBar, SIGNAL(dropped(QDropEvent*)), m_editor, SLOT(dropEvent(QDropEvent*)));

	// Set up buttons
	m_playAction->setToolTip(tr("Play song (Space)"));
	m_stopAction->setToolTip(tr( "Stop song (Space)" ));

	// Track actions
	DropToolBar *trackActionsToolBar = addDropToolBarToTop(tr("Track actions"));

	m_addPatternTrackAction = new QAction(embed::getIconPixmap("add_pattern_track"),
									 tr("Add pattern-track"), this);

	m_addSampleTrackAction = new QAction(embed::getIconPixmap("add_sample_track"),
										 tr("Add sample-track"), this);
	m_addSampleTrackAction->setToolTip(tr("Add a sample track: short samples on the grid"));

	m_addAudioTrackAction = new QAction(embed::getIconPixmap("add_audio_track"), tr("Add audio track"), this);
	m_addAudioTrackAction->setToolTip(tr("Add an audio track: recordings and audio files, with lanes and fades"));
	connect(m_addAudioTrackAction, SIGNAL(triggered()), m_editor->m_song, SLOT(addAudioTrack()));

	m_addAutomationTrackAction = new QAction(embed::getIconPixmap("add_automation"),
											 tr("Add automation-track"), this);

	connect(m_addPatternTrackAction, SIGNAL(triggered()), m_editor->m_song, SLOT(addPatternTrack()));
	connect(m_addSampleTrackAction, SIGNAL(triggered()), m_editor->m_song, SLOT(addSampleTrack()));
	connect(m_addAutomationTrackAction, SIGNAL(triggered()), m_editor->m_song, SLOT(addAutomationTrack()));

	m_addFolderTrackAction = new QAction(embed::getIconPixmap("folder"), tr("Add folder track"), this);
	connect(m_addFolderTrackAction, SIGNAL(triggered()), m_editor->m_song, SLOT(addFolderTrack()));

	trackActionsToolBar->addAction( m_addPatternTrackAction );
	trackActionsToolBar->addAction(m_addAudioTrackAction);
	trackActionsToolBar->addAction( m_addSampleTrackAction );
	trackActionsToolBar->addAction( m_addAutomationTrackAction );
	trackActionsToolBar->addAction(m_addFolderTrackAction);


	// Edit actions
	DropToolBar *editActionsToolBar = addDropToolBarToTop(tr("Edit actions"));

	m_editModeGroup = new ActionGroup(this);
	m_drawModeAction = m_editModeGroup->addAction(embed::getIconPixmap("edit_draw"), tr("Draw mode"));
	m_knifeModeAction = m_editModeGroup->addAction(embed::getIconPixmap("edit_knife"), tr("Knife mode (split clips)"));
	m_selectModeAction = m_editModeGroup->addAction(embed::getIconPixmap("edit_select"), tr("Edit mode (select and move)"));
	m_drawModeAction->setChecked(true);

	connect(m_drawModeAction, SIGNAL(triggered()), m_editor, SLOT(setEditModeDraw()));
	connect(m_knifeModeAction, SIGNAL(triggered()), m_editor, SLOT(setEditModeKnife()));
	connect(m_selectModeAction, SIGNAL(triggered()), m_editor, SLOT(setEditModeSelect()));

	editActionsToolBar->addAction( m_drawModeAction );
	editActionsToolBar->addAction( m_knifeModeAction );
	editActionsToolBar->addAction( m_selectModeAction );

	DropToolBar *timeLineToolBar = addDropToolBarToTop(tr("Timeline controls"));
	m_editor->m_timeLine->addToolButtons(timeLineToolBar);

	DropToolBar *insertActionsToolBar = addDropToolBarToTop(tr("Bar insert controls"));
	m_insertBarAction = new QAction(embed::getIconPixmap("insert_bar"), tr("Insert bar"), this);
	m_removeBarAction = new QAction(embed::getIconPixmap("remove_bar"), tr("Remove bar"), this);
	insertActionsToolBar->addAction( m_insertBarAction );
	insertActionsToolBar->addAction( m_removeBarAction );
	connect(m_insertBarAction, SIGNAL(triggered()), song, SLOT(insertBar()));
	connect(m_removeBarAction, SIGNAL(triggered()), song, SLOT(removeBar()));

	DropToolBar *zoomToolBar = addDropToolBarToTop(tr("Zoom controls"));

	auto zoom_lbl = new QLabel(m_toolBar);
	zoom_lbl->setPixmap( embed::getIconPixmap( "zoom" ) );

	// Set slider zoom
	m_zoomingSlider = new AutomatableSlider(m_toolBar, tr("Zoom"));
	m_zoomingSlider->setModel(m_editor->m_zoomingModel);
	m_zoomingSlider->setOrientation(Qt::Horizontal);
	m_zoomingSlider->setPageStep(1);
	m_zoomingSlider->setFocusPolicy(Qt::NoFocus);
	m_zoomingSlider->setFixedSize(100, 26);
	m_zoomingSlider->setToolTip(tr("Zoom"));
	m_zoomingSlider->setContextMenuPolicy(Qt::NoContextMenu);
	connect(m_editor->m_zoomingModel, SIGNAL(dataChanged()), this, SLOT(updateSnapLabel()));

	zoomToolBar->addWidget( zoom_lbl );
	zoomToolBar->addWidget(m_zoomingSlider);

	DropToolBar *snapToolBar = addDropToolBarToTop(tr("Snap controls"));
	//Set up quantization/snapping selector
	m_snappingComboBox = new ComboBox( m_toolBar );
	m_snappingComboBox->setFixedSize( 80, ComboBox::DEFAULT_HEIGHT );
	m_snappingComboBox->setModel(m_editor->m_snappingModel);
	m_snappingComboBox->setToolTip(tr("Clip snapping size"));
	m_snappingComboBox->setIcon(embed::getIconPixmap("quantize"));
	connect(m_editor->snappingModel(), SIGNAL(dataChanged()), this, SLOT(updateSnapLabel()));

	m_setProportionalSnapAction = new QAction(embed::getIconPixmap("proportional_snap"),
											 tr("Toggle proportional snap on/off"), this);
	m_setProportionalSnapAction->setCheckable(true);
	m_setProportionalSnapAction->setChecked(false);
	connect(m_setProportionalSnapAction, SIGNAL(triggered()), m_editor, SLOT(toggleProportionalSnap()));
	connect(m_setProportionalSnapAction, SIGNAL(triggered()), this, SLOT(updateSnapLabel()));

	snapToolBar->addWidget( m_snappingComboBox );
	snapToolBar->addSeparator();
	snapToolBar->addAction( m_setProportionalSnapAction );

	snapToolBar->addSeparator();
	snapToolBar->addWidget( m_snapSizeLabel );

	connect(song, SIGNAL(projectLoaded()), this, SLOT(adjustUiAfterProjectLoad()));
	connect(this, SIGNAL(resized()), m_editor, SLOT(updatePositionLine()));
}

QSize SongEditorWindow::sizeHint() const
{
	return {900, 300};
}

void SongEditorWindow::updateSnapLabel(){
	if (m_setProportionalSnapAction->isChecked())
	{
		m_snapSizeLabel->setText(QString("Snap: ") + m_editor->getSnapSizeString());
		m_snappingComboBox->setToolTip(tr("Base snapping size"));
	}
	else
	{
		m_snappingComboBox->setToolTip(tr("Clip snapping size"));
		m_snapSizeLabel->clear();
	}
}




void SongEditorWindow::syncEditMode(){
	m_editModeGroup->checkedAction()->trigger();
}




void SongEditorWindow::resizeEvent(QResizeEvent *event)
{
	emit resized();
}


void SongEditorWindow::changeEvent(QEvent *event)
{
	QWidget::changeEvent(event);
	if (event->type() == QEvent::WindowStateChange)
	{
		m_editor->realignTracks();
	}
}


void SongEditorWindow::play()
{
	emit playTriggered();
	Engine::getSong()->transport().play();
}




void SongEditorWindow::stop()
{
	// The rest follows the transport's stopped(), whoever stops it
	Engine::getSong()->transport().stop();
}




void SongEditorWindow::lostFocus()
{
	if( m_crtlAction )
	{
		m_crtlAction->setChecked( true );
		m_crtlAction->trigger();
	}
}




void SongEditorWindow::adjustUiAfterProjectLoad()
{
	// make sure to bring us to front as the song editor is the central
	// widget in a song and when just opening a song in order to listen to
	// it, it's very annoying to manually bring up the song editor each time
	parentWidget()->show();
	m_editor->scrolled(0);
}


} // namespace lmms::gui


