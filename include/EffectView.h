/*
 * EffectView.h - view-component for an effect
 *
 * Copyright (c) 2006-2007 Danny McRae <khjklujn/at/users.sourceforge.net>
 * Copyright (c) 2007-2010 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef LMMS_GUI_EFFECT_VIEW_H
#define LMMS_GUI_EFFECT_VIEW_H

#include "AutomatableModel.h"
#include "PluginView.h"
#include "Effect.h"

class QGraphicsOpacityEffect;
class QGroupBox;
class QLabel;
class QPushButton;
class QMdiSubWindow;

namespace lmms::gui
{

class EffectControlDialog;
class Knob;
class LedCheckBox;
class PinConnector;
class TempoSyncKnob;


class EffectView : public PluginView
{
	Q_OBJECT
public:
	//! @param audioPorts the effect's pin connector (owned by its PluginInstance), if any
	EffectView(Effect* _model, QWidget* _parent, AudioPortsModel* audioPorts = nullptr);
	~EffectView() override;

	inline Effect * effect()
	{
		return castModel<Effect>();
	}
	inline const Effect * effect() const
	{
		return castModel<Effect>();
	}

	//! Shows the slot menu (move, replace, presets, sidechain source, remove) at the mouse
	void showMenu();

	static constexpr int DEFAULT_WIDTH = 215;
	static constexpr int DEFAULT_HEIGHT = 60;
	
	void mouseMoveEvent(QMouseEvent* event) override;
	void mousePressEvent(QMouseEvent* event) override;
	void mouseReleaseEvent(QMouseEvent* event) override;

public slots:
	void editControls();
	void togglePinConnector();
	void moveUp();
	void moveDown();
	void deletePlugin();
	void corruptStateUpdate();

signals:
	void movedUp(EffectView* view);
	void movedDown(EffectView* view);
	void deletedPlugin(EffectView* view);
	//! The slot menu asked to replace the effect with another one, or with a saved preset
	void replaceRequested(EffectView* view);
	void loadPresetRequested(EffectView* view);
	void savePresetRequested(EffectView* view);

protected:
	void contextMenuEvent( QContextMenuEvent * _me ) override;
	void paintEvent( QPaintEvent * _pe ) override;
	void modelChanged() override;


private:
	QPixmap m_bg;
	LedCheckBox * m_bypass;
	Knob * m_wetDry;
	TempoSyncKnob * m_autoQuit;
	QMdiSubWindow * m_subWindow;
	EffectControlDialog * m_controlView;
	QPushButton* m_pinConnectorButton = nullptr;
	AudioPortsModel* m_audioPorts = nullptr; //!< owned by the effect's PluginInstance
	PinConnector* m_pinConnectorView = nullptr;

	bool m_dragging;
	QGraphicsOpacityEffect* m_opacityEffect;

} ;


} // namespace lmms::gui

#endif // LMMS_GUI_EFFECT_VIEW_H
