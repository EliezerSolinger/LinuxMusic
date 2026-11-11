/*
 * Vestige.h - instrument VeSTige for hosting VST-plugins
 *
 * Copyright (c) 2005-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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


#ifndef _VESTIGE_H
#define _VESTIGE_H


#include <string>

#include <QMutex>

#include "Instrument.h"
#include "InstrumentView.h"
#include "SubWindow.h"


class QGridLayout;
class QMdiSubWindow;
class QPushButton;
class QScrollArea;

namespace lmms
{

class FloatModel;
class VstPlugin;

namespace gui
{
class ManageVestigeInstrumentView;
class PixmapButton;
class VestigeInstrumentView;
class VstPluginKnob;
} // namespace gui


/**
 * Hosts a VST instrument, which runs in a RemoteVstPlugin process.
 *
 * The plugin's audio inputs and outputs, as the remote process reports them, are exposed to the pin
 * connector. The remote process reads and writes the host's buffers in shared memory.
 * Without a plugin, the instrument has a silent stereo output.
 */
class VestigeInstrument : public Instrument
{
	Q_OBJECT
public:
	VestigeInstrument( InstrumentTrack * _instrument_track );
	~VestigeInstrument() override;

	void saveSettings( QDomDocument & _doc, QDomElement & _parent ) override;
	void loadSettings( const QDomElement & _this ) override;

	QString nodeName() const override;

	void loadFile( const QString & _file ) override;

	bool handleMidiEvent( const MidiEvent& event, const TimePos& time, f_cnt_t offset = 0 ) override;

	gui::PluginView* instantiateView( QWidget * _parent ) override;

	// AudioProcessor implementation
	auto process(ProcessContext& context) -> ProcessStatus override;
	auto channelCount(bool isInput) const -> ch_cnt_t override;
	auto channelGroupCount(bool isInput) const -> group_cnt_t override { return channelCount(isInput) > 0 ? 1 : 0; }
	auto getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup& group) const
		-> ch_cnt_t override
	{
		(void)index;
		(void)group;
		return channelCount(isInput);
	}
	auto getInplacePair(ch_cnt_t inputIndex) const -> std::optional<ch_cnt_t> override
	{
		(void)inputIndex;
		return std::nullopt;
	}
	auto usesSharedMemoryBuffers() const -> bool override { return m_plugin != nullptr; }
	void onSharedMemoryChange(const SharedBufferInfo& info) override;

protected slots:
	void setParameter( lmms::Model * action );
	void handleConfigChange( QString cls, QString attr, QString value );
	void reloadPlugin();

private:
	void closePlugin();

	//! Lets the host set up buffers for the plugin's channel counts [main thread]
	void rescanAudioPorts();

	//! Where the host put the audio buffers for the remote process
	std::string m_sharedBufferKey;
	SharedBufferInfo m_sharedBufferInfo{};

	VstPlugin * m_plugin;
	QMutex m_pluginMutex;

	QString m_pluginDLL;
	gui::SubWindow* m_subWindow;
	QScrollArea * m_scrollArea;
	FloatModel ** knobFModel;
	QObject * p_subWindow;
	int paramCount;


	friend class gui::VestigeInstrumentView;
	friend class gui::ManageVestigeInstrumentView;

} ;


namespace gui
{

class ManageVestigeInstrumentView : public InstrumentViewFixedSize
{
	Q_OBJECT
public:
	ManageVestigeInstrumentView( Instrument * _instrument, QWidget * _parent, VestigeInstrument * m_vi2 );
	~ManageVestigeInstrumentView() override;

protected slots:
	void syncPlugin();
	void displayAutomatedOnly();
	void setParameter( lmms::Model * action );
	void closeWindow();

protected:
	virtual void dragEnterEvent( QDragEnterEvent * _dee );
	virtual void dropEvent( QDropEvent * _de );
	virtual void paintEvent( QPaintEvent * _pe );

private:
	VestigeInstrument * m_vi;

	QWidget *widget;
	QGridLayout * l;
	QPushButton * m_syncButton;
	QPushButton * m_displayAutomatedOnly;
	QPushButton * m_closeButton;
	std::vector<VstPluginKnob*> m_vstKnobs;
};


class VestigeInstrumentView : public InstrumentViewFixedSize
{
	Q_OBJECT
public:
	VestigeInstrumentView( Instrument * _instrument, QWidget * _parent );
	virtual ~VestigeInstrumentView() = default;


protected slots:
	void updateMenu();
	void openPlugin();
	void managePlugin();
	void openPreset();
	void savePreset();
	void nextProgram();
	void previousProgram();
	void selPreset();
	void toggleGUI();
	void noteOffAll();


protected:
	virtual void dragEnterEvent( QDragEnterEvent * _dee );
	virtual void dropEvent( QDropEvent * _de );
	virtual void paintEvent( QPaintEvent * _pe );


private:
	virtual void modelChanged();


	VestigeInstrument * m_vi;

	int lastPosInMenu;

	PixmapButton * m_openPluginButton;
	PixmapButton * m_openPresetButton;
	QPushButton* m_rolLPresetButton;
	QPushButton* m_rolRPresetButton;
	QPushButton * m_selPresetButton;
	QPushButton * m_toggleGUIButton;
	PixmapButton * m_managePluginButton;
	PixmapButton * m_savePresetButton;

	Instrument * _instrument2;
	QWidget * _parent2;

} ;


} // namespace gui

} // namespace lmms

#endif
