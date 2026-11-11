/*
 * RemotePlugin.h - base class providing RPC like mechanisms
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

#ifndef LMMS_REMOTE_PLUGIN_H
#define LMMS_REMOTE_PLUGIN_H

#include <span>
#include <QThread>
#include <QProcess>
#include <QRecursiveMutex>

#include "RemotePluginBase.h"
#include "AudioProcessor.h"
#include "SharedMemory.h"
#include "LmmsTypes.h"

namespace lmms
{

class MidiEvent;
class RemotePlugin;
class SampleFrame;

class ProcessWatcher : public QThread
{
	Q_OBJECT
public:
	explicit ProcessWatcher(RemotePlugin* plugin);
	~ProcessWatcher() override = default;

	void stop()
	{
		m_quit = true;
		quit();
	}

	void reset()
	{
		m_quit = false;
	}

private:
	void run() override;

	RemotePlugin * m_plugin;
	volatile bool m_quit;

} ;


class LMMS_EXPORT RemotePlugin : public QObject, public RemotePluginBase
{
	Q_OBJECT
public:
	RemotePlugin();
	~RemotePlugin() override;

	inline bool isRunning()
	{
#ifdef DEBUG_REMOTE_PLUGIN
		return true;
#else
		return m_process.state() != QProcess::NotRunning;
#endif // DEBUG_REMOTE_PLUGIN
	}

	bool init( const QString &pluginExecutable, bool waitForInitDoneMsg, QStringList extraArgs = {} );

	inline void waitForHostInfoGotten()
	{
		m_failed = waitForMessage( IdHostInfoGotten ).id
							!= IdHostInfoGotten;
	}

	void waitForInitDone(bool busyWaiting = true);

	bool processMessage( const message & _m ) override;

	/**
	 * Lets the remote process run one period on the shared audio buffer (see setSharedAudioBuffer()).
	 *
	 * @returns false if the remote process could not process, in which case the outputs were not written
	 */
	bool process();

	/**
	 * Tells the remote process where its audio buffers are: a shared memory block created by the host
	 * (PluginInstance) holding a channel buffer table followed by the channel buffers.
	 */
	void setSharedAudioBuffer(const AudioProcessor::SharedBufferInfo& info);

	//! The channel counts the remote process reported, see channelCountsChanged()
	auto remoteChannelsIn() const -> ch_cnt_t { return m_remoteChannelsIn; }
	auto remoteChannelsOut() const -> ch_cnt_t { return m_remoteChannelsOut; }

	void processMidiEvent( const MidiEvent&, const f_cnt_t _offset );

	void updateSampleRate( sample_rate_t _sr )
	{
		lock();
		sendMessage( message( IdSampleRateInformation ).addInt( _sr ) );
		waitForMessage( IdInformationUpdated, true );
		unlock();
	}


	virtual void toggleUI()
	{
		lock();
		sendMessage( IdToggleUI );
		unlock();
	}

	int isUIVisible()
	{
		lock();
		sendMessage( IdIsUIVisible );
		unlock();
		message m = waitForMessage( IdIsUIVisible );
		return m.id != IdIsUIVisible ? -1 : m.getInt() ? 1 : 0;
	}

	inline bool failed() const
	{
		return m_failed;
	}

	inline void lock()
	{
		m_commMutex.lock();
	}

	inline void unlock()
	{
		m_commMutex.unlock();
	}


public slots:
	virtual void showUI();
	virtual void hideUI();

signals:
	//! The remote process reported new input/output channel counts; the plugin should rescan its audio ports
	void channelCountsChanged(lmms::ch_cnt_t channelsIn, lmms::ch_cnt_t channelsOut);

protected:
	bool m_failed;

private:
	QProcess m_process;
	ProcessWatcher m_watcher;

	QString m_exec;
	QStringList m_args;

	QRecursiveMutex m_commMutex;

	//! Whether the remote process was given a shared audio buffer
	bool m_hasSharedAudioBuffer = false;

	ch_cnt_t m_remoteChannelsIn = 0;
	ch_cnt_t m_remoteChannelsOut = 0;

#ifndef SYNC_WITH_SHM_FIFO
	int m_server;
	QString m_socketFile;
#endif // not SYNC_WITH_SHM_FIFO

	friend class ProcessWatcher;


private slots:
	void processFinished( int exitCode, QProcess::ExitStatus exitStatus );
	void processErrored(QProcess::ProcessError err );
} ;

inline std::string QSTR_TO_STDSTR(QString const& qstr)
{
	return qstr.toStdString();
}

} // namespace lmms

#endif // LMMS_REMOTE_PLUGIN_H
