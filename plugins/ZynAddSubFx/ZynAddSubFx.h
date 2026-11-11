/*
 * ZynAddSubFx.h - ZynAddSubFX-embedding plugin
 *
 * Copyright (c) 2008-2010 Tobias Doerffel <tobydox/at/users.sourceforge.net>
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

#ifndef ZYNADDSUBFX_H
#define ZYNADDSUBFX_H

#include <QMap>
#include <QMutex>

#include <optional>
#include <string>

#include <globals.h>

#include "AutomatableModel.h"
#include "Instrument.h"
#include "InstrumentView.h"
#include "RemotePlugin.h"

class QPushButton;

namespace lmms
{


class LocalZynAddSubFx;
class NotePlayHandle;

namespace gui
{
class Knob;
class LedCheckBox;
class ZynAddSubFxView;
}

class ZynAddSubFxRemotePlugin
	: public RemotePlugin
{
	Q_OBJECT
public:
	ZynAddSubFxRemotePlugin();

	bool processMessage( const message & _m ) override;

signals:
	void clickedCloseButton();
};


/**
 * Hosts ZynAddSubFX, either in-process or, while its own GUI is shown, in a RemoteZynAddSubFx process.
 *
 * It has no inputs and a stereo output, which the remote process writes into the host's buffers in
 * shared memory.
 */
class ZynAddSubFxInstrument : public Instrument
{
	Q_OBJECT
public:
	ZynAddSubFxInstrument( InstrumentTrack * _instrument_track );
	~ZynAddSubFxInstrument() override;

	bool handleMidiEvent( const MidiEvent& event, const TimePos& time = TimePos(), f_cnt_t offset = 0 ) override;

	void saveSettings( QDomDocument & _doc, QDomElement & _parent ) override;
	void loadSettings( const QDomElement & _this ) override;

	void loadFile( const QString & _file ) override;


	QString nodeName() const override;

	gui::PluginView* instantiateView( QWidget * _parent ) override;

	// AudioProcessor implementation
	auto activate(double sampleRate, f_cnt_t maxFrameCount) -> bool override;
	auto process(ProcessContext& context) -> ProcessStatus override;
	auto channelCount(bool isInput) const -> ch_cnt_t override { return isInput ? 0 : 2; }
	auto channelGroupCount(bool isInput) const -> group_cnt_t override { return isInput ? 0 : 1; }
	auto getChannelGroup(group_cnt_t index, bool isInput, AudioBuffer::ChannelGroup& group) const
		-> ch_cnt_t override
	{
		(void)index;
		(void)group;
		return isInput ? 0 : 2;
	}
	auto getInplacePair(ch_cnt_t inputIndex) const -> std::optional<ch_cnt_t> override
	{
		(void)inputIndex;
		return std::nullopt;
	}
	auto usesSharedMemoryBuffers() const -> bool override { return m_hasGUI; }
	void onSharedMemoryChange(const SharedBufferInfo& info) override;

	//! Switches between the in-process synth and the remote one showing ZynAddSubFX's GUI [main thread]
	void setShowGUI(bool show);
	auto showsGUI() const -> bool { return m_hasGUI; }

private slots:
	void reloadPlugin();

	void updatePitchRange();

	void updatePortamento();
	void updateFilterFreq();
	void updateFilterQ();
	void updateBandwidth();
	void updateFmGain();
	void updateResCenterFreq();
	void updateResBandwidth();


private:
	void initPlugin();
	void sendControlChange( MidiControllers midiCtl, float value );

	bool m_hasGUI;
	QMutex m_pluginMutex;
	LocalZynAddSubFx * m_localPlugin;
	ZynAddSubFxRemotePlugin * m_remotePlugin;

	//! The sample rate and period size the synth was created with
	sample_rate_t m_sampleRate = 0;
	f_cnt_t m_bufferSize = 0;

	//! Where the host put the audio buffers for the remote process
	std::string m_sharedBufferKey;
	ch_cnt_t m_sharedBufferChannels = 0;

	FloatModel m_portamentoModel;
	FloatModel m_filterFreqModel;
	FloatModel m_filterQModel;
	FloatModel m_bandwidthModel;
	FloatModel m_fmGainModel;
	FloatModel m_resCenterFreqModel;
	FloatModel m_resBandwidthModel;
	BoolModel m_forwardMidiCcModel;

	QMap<int, bool> m_modifiedControllers;

	friend class gui::ZynAddSubFxView;


signals:
	void settingsChanged();

} ;


namespace gui
{


class ZynAddSubFxView : public InstrumentViewFixedSize
{
	Q_OBJECT
public:
	ZynAddSubFxView( Instrument * _instrument, QWidget * _parent );


protected:
	void dragEnterEvent( QDragEnterEvent * _dee ) override;
	void dropEvent( QDropEvent * _de ) override;


private:
	void modelChanged() override;

	QPushButton * m_toggleUIButton;
	Knob * m_portamento;
	Knob * m_filterFreq;
	Knob * m_filterQ;
	Knob * m_bandwidth;
	Knob * m_fmGain;
	Knob * m_resCenterFreq;
	Knob * m_resBandwidth;
	LedCheckBox * m_forwardMidiCC;


private slots:
	void toggleUI();

} ;


} // namespace gui

} // namespace lmms

#endif
