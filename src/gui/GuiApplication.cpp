/*
 * GuiApplication.cpp
 *
 * Copyright (c) 2014 Lukas W <lukaswhl/at/gmail.com>
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

#include "GuiApplication.h"

#include "ExportWizard.h"

#include <QFileInfo>

#include "lmmsversion.h"

#include "LmmsStyle.h"
#include "LmmsPalette.h"

#include "AutomationEditor.h"
#include "BrowserPanel.h"
#include "ConfigManager.h"
#include "ControllerRackView.h"
#include "InstrumentTrackView.h"
#include "InstrumentTrackWindow.h"
#include "MixerView.h"
#include "MainWindow.h"
#include "MicrotunerConfig.h"
#include "PatternEditor.h"
#include "PianoRoll.h"
#include "ProjectNotes.h"
#include "PluginBrowser.h"
#include "Song.h"
#include "SongEditor.h"
#include "SongMarkers.h"
#include "TabWidget.h"

#include <QApplication>
#include <QDebug>
#include <QDir>
#include <QtGlobal>
#include <QHBoxLayout>
#include <QLabel>
#include <QMessageBox>
#include <QSplashScreen>
#include <QSocketNotifier>
#include <QThread>
#include <QTimer>

#ifdef LMMS_BUILD_WIN32
#include <io.h>
#include <stdio.h>
#include <windows.h>
#else
#include <sys/socket.h>
#include <unistd.h>
#endif

namespace lmms
{


namespace gui
{

GuiApplication* getGUI()
{
	return GuiApplication::instance();
}


GuiApplication* GuiApplication::s_instance = nullptr;

GuiApplication* GuiApplication::instance()
{
	return s_instance;
}

bool GuiApplication::isWayland()
{
	return QGuiApplication::platformName().contains("wayland");
}



namespace
{

//! Quits once the main loop runs: a project still loading (its events processed before the loop starts) would make
//! an exit before it do nothing
void quitWhenTheMainLoopRuns()
{
	if (QThread::currentThread()->loopLevel() > 0) { QApplication::exit(0); }
	else { QTimer::singleShot(100, qApp, &quitWhenTheMainLoopRuns); }
}

} // namespace




GuiApplication::GuiApplication()
{
	// Immediately register our SIGINT handler
	createSocketNotifier();

	// prompt the user to create the LMMS working directory (e.g. ~/Documents/lmms) if it doesn't exist
	if ( !ConfigManager::inst()->hasWorkingDir() &&
		QMessageBox::question( nullptr,
				tr( "Working directory" ),
				tr( "The LinuxMusic working directory %1 does not "
				"exist. Create it now? You can change the directory "
				"later via Edit -> Settings." ).arg( ConfigManager::inst()->workingDir() ),
					QMessageBox::Yes | QMessageBox::No, QMessageBox::Yes ) == QMessageBox::Yes)
	{
		ConfigManager::inst()->createWorkingDir();
	}
	// Init style and palette
	QDir::addSearchPath("artwork", ConfigManager::inst()->themeDir());
	QDir::addSearchPath("artwork", ConfigManager::inst()->defaultThemeDir());
	QDir::addSearchPath("artwork", ":/artwork");

	auto lmmsstyle = new LmmsStyle();
	QApplication::setStyle(lmmsstyle);

	auto lmmspal = new LmmsPalette(nullptr, lmmsstyle);
	auto lpal = new QPalette(lmmspal->palette());

	QApplication::setPalette( *lpal );
	LmmsStyle::s_palette = lpal;

#ifdef LMMS_BUILD_APPLE
	QApplication::setAttribute(Qt::AA_DontShowIconsInMenus, true);
#endif

	// Show splash screen
	QSplashScreen splashScreen( embed::getIconPixmap( "splash" ) );
	splashScreen.setFixedSize(splashScreen.pixmap().size());
	splashScreen.show();

	QHBoxLayout layout;
	layout.setAlignment(Qt::AlignBottom);
	splashScreen.setLayout(&layout);

	// Create a left-aligned label for loading progress 
	// & a right-aligned label for version info
	QLabel loadingProgressLabel;
	m_loadingProgressLabel = &loadingProgressLabel;
	QLabel versionLabel(MainWindow::tr( "Version %1" ).arg( LMMS_VERSION ));

	loadingProgressLabel.setAlignment(Qt::AlignLeft);
	versionLabel.setAlignment(Qt::AlignRight);

	layout.addWidget(&loadingProgressLabel);
	layout.addWidget(&versionLabel);

	// may have long gaps between future frames, so force update now
	splashScreen.update();
	qApp->processEvents();

	connect(Engine::inst(), SIGNAL(initProgress(const QString&)), 
		this, SLOT(displayInitProgress(const QString&)));

	// Init central engine which handles all components of LMMS
	Engine::init(false);

	s_instance = this;

	displayInitProgress(tr("Preparing UI"));

	// Screenshot runs (see below) must not stop at the first-run setup dialog
	if (!qEnvironmentVariable("LINUXMUSIC_SCREENSHOT").isEmpty())
	{
		ConfigManager::inst()->setValue("app", "configured", "1");
	}

	m_mainWindow = new MainWindow;
	connect(m_mainWindow, SIGNAL(destroyed(QObject*)), this, SLOT(childDestroyed(QObject*)));
	connect(m_mainWindow, SIGNAL(initProgress(const QString&)), 
		this, SLOT(displayInitProgress(const QString&)));

	displayInitProgress(tr("Preparing song editor"));
	m_songEditor = new SongEditorWindow(Engine::getSong());
	connect(m_songEditor, SIGNAL(destroyed(QObject*)), this, SLOT(childDestroyed(QObject*)));

	displayInitProgress(tr("Preparing mixer"));
	m_mixerView = new MixerView(Engine::mixer());
	connect(m_mixerView, SIGNAL(destroyed(QObject*)), this, SLOT(childDestroyed(QObject*)));

	displayInitProgress(tr("Preparing controller rack"));
	m_controllerRackView = new ControllerRackView;
	connect(m_controllerRackView, SIGNAL(destroyed(QObject*)), this, SLOT(childDestroyed(QObject*)));

	displayInitProgress(tr("Preparing project notes"));
	m_projectNotes = new ProjectNotes;
	connect(m_projectNotes, SIGNAL(destroyed(QObject*)), this, SLOT(childDestroyed(QObject*)));

	displayInitProgress(tr("Preparing microtuner"));
	m_microtunerConfig = new MicrotunerConfig;
	connect(m_microtunerConfig, SIGNAL(destroyed(QObject*)), this, SLOT(childDestroyed(QObject*)));

	displayInitProgress(tr("Preparing pattern editor"));
	m_patternEditor = new PatternEditorWindow(Engine::patternStore());
	connect(m_patternEditor, SIGNAL(destroyed(QObject*)), this, SLOT(childDestroyed(QObject*)));

	displayInitProgress(tr("Preparing piano roll"));
	m_pianoRoll = new PianoRollWindow();
	connect(m_pianoRoll, SIGNAL(destroyed(QObject*)), this, SLOT(childDestroyed(QObject*)));

	displayInitProgress(tr("Preparing automation editor"));
	m_automationEditor = new AutomationEditorWindow;
	connect(m_automationEditor, SIGNAL(destroyed(QObject*)), this, SLOT(childDestroyed(QObject*)));

	splashScreen.finish(m_mainWindow);
	m_mainWindow->finalize();

	m_loadingProgressLabel = nullptr;

	// Development aid: LINUXMUSIC_SCREENSHOT=<file.png> saves an image of the main window shortly
	// after startup and quits, which allows checking the UI without screen capture permissions
	if (const auto screenshotPath = qEnvironmentVariable("LINUXMUSIC_SCREENSHOT"); !screenshotPath.isEmpty())
	{
		// LINUXMUSIC_SCREENSHOT_ACTION=instrument opens the first instrument window before the screenshot
		const auto screenshotAction = qEnvironmentVariable("LINUXMUSIC_SCREENSHOT_ACTION");
		// =pattern shows the Pattern Editor
		if (screenshotAction == "pattern")
		{
			QTimer::singleShot(2500, m_mainWindow, [this] {
				QMetaObject::invokeMethod(m_mainWindow, "togglePatternEditorWin", Q_ARG(bool, true));
			});
		}
		// =pins opens the pin connector of the first instrument instead, =effects its effects tab
		if (screenshotAction == "instrument" || screenshotAction == "pins" || screenshotAction == "effects")
		{
			QTimer::singleShot(2500, m_mainWindow, [this, screenshotAction] {
				const auto views = m_mainWindow->findChildren<InstrumentTrackView*>();
				if (views.isEmpty()) { return; }
				if (screenshotAction == "pins") { QMetaObject::invokeMethod(views.first(), "togglePinConnector"); }
				else { QMetaObject::invokeMethod(views.first(), "toggleInstrumentWindow", Q_ARG(bool, true)); }
				if (screenshotAction == "effects")
				{
					views.first()->getInstrumentTrackWindow()->tabWidgetParent()->setActiveTab(3);
				}
			});
		}
		// =effects-browser shows the browser's effects page
		if (screenshotAction == "export-run")
		{
			// The whole export (render, then WAV and FLAC) into the screenshot's folder, then quit
			QTimer::singleShot(2500, m_mainWindow, [screenshotPath, this] {
				auto wizard = ExportWizard{m_mainWindow};
				// LINUXMUSIC_EXPORT_CLIP=1: only the first track's first clip
				if (qEnvironmentVariableIntValue("LINUXMUSIC_EXPORT_CLIP") && !Engine::getSong()->tracks().empty())
				{
					auto* track = Engine::getSong()->tracks().front();
					if (track->numOfClips() > 0)
					{
						const auto* clip = track->getClip(0);
						wizard.setOnly(track, std::pair{clip->startPosition(), clip->endPosition()});
					}
				}
				wizard.setFolderAndName(QFileInfo{screenshotPath}.absolutePath(), "export-run");
				wizard.accept();
				quitWhenTheMainLoopRuns();
			});
			return;
		}
		if (screenshotAction == "export")
		{
			// The export wizard, on its formats page, saved by itself
			QTimer::singleShot(2500, m_mainWindow, [screenshotPath, this] {
				auto wizard = new ExportWizard{m_mainWindow};
				wizard->show();
				wizard->next();
				QTimer::singleShot(800, wizard, [wizard, screenshotPath] {
					wizard->grab().save(screenshotPath);
					quitWhenTheMainLoopRuns();
				});
			});
			return;
		}
		if (screenshotAction == "settings")
		{
			// (LINUXMUSIC_SETTINGS_TAB=<n> opens it at a tab)
			QTimer::singleShot(2500, m_mainWindow, [this] {
				m_mainWindow->showSettingsPage(qEnvironmentVariableIntValue("LINUXMUSIC_SETTINGS_TAB"));
			});
		}
		if (screenshotAction == "regions")
		{
			// Markers and regions on the ruler, the second region as the cycle
			QTimer::singleShot(2500, m_mainWindow, [] {
				auto& markers = Engine::getSong()->markers();
				const auto bar = TimePos::ticksPerBar();
				markers.add(0, "Start");
				markers.add(4 * bar, "Drop");
				markers.addRegion(0, 4 * bar, "Intro");
				markers.addRegion(4 * bar, 12 * bar, "Chorus", QColor{200, 90, 120});
				markers.addRegion(6 * bar, 8 * bar, "Break");
				auto& timeline = Engine::getSong()->getTimeline(Song::PlayMode::Song);
				timeline.setLoopPoints(TimePos{4 * bar}, TimePos{12 * bar});
				timeline.setLoopEnabled(true);
			});
		}
		if (screenshotAction == "effects-browser")
		{
			QTimer::singleShot(2500, m_mainWindow, [this] {
				m_mainWindow->showEffectsExplorer(); // the mixing page, with its explorer
			});
		}
		// LINUXMUSIC_SCREENSHOT_DELAY=<ms> waits longer, e.g. for a large project to load
		const int delay = qEnvironmentVariableIntValue("LINUXMUSIC_SCREENSHOT_DELAY");
		QTimer::singleShot(delay > 0 ? delay : 4000, m_mainWindow, [this, screenshotPath, screenshotAction] {
			// =plugins captures the plugin browser of the side bar instead of the main window
			auto browser = m_mainWindow->findChild<PluginBrowser*>();
			if (screenshotAction == "plugins" && browser)
			{
				browser->resize(320, 1000);
				browser->grab().save(screenshotPath);
			}
			// =mixer captures the mixer
			else if ((screenshotAction == "mixer" || screenshotAction == "mixer-channel") && m_mixerView)
			{
				// mixer-channel: with the first channel selected (its settings and effects beside the strips)
				if (screenshotAction == "mixer-channel") { m_mixerView->setCurrentMixerChannel(1); }
				m_mixerView->resize(1100, m_mixerView->minimumSizeHint().height());
				m_mixerView->grab().save(screenshotPath);
			}
			else { m_mainWindow->grab().save(screenshotPath); }

			quitWhenTheMainLoopRuns();
		});
	}
}


GuiApplication::~GuiApplication()
{
	s_instance = nullptr;
}


void GuiApplication::displayInitProgress(const QString &msg)
{
	Q_ASSERT(m_loadingProgressLabel != nullptr);
	
	m_loadingProgressLabel->setText(msg);
	// must force a UI update and process events, as there may be long gaps between processEvents() calls during init
	m_loadingProgressLabel->repaint();
	qApp->processEvents();
}

void GuiApplication::childDestroyed(QObject *obj)
{
	// when any object that can be reached via getGUI()->mainWindow(), getGUI()->mixerView(), etc
	//   is destroyed, ensure that their accessor functions will return null instead of a garbage pointer.
	if (obj == m_mainWindow)
	{
		m_mainWindow = nullptr;
	}
	else if (obj == m_mixerView)
	{
		m_mixerView = nullptr;
	}
	else if (obj == m_songEditor)
	{
		m_songEditor = nullptr;
	}
	else if (obj == m_automationEditor)
	{
		m_automationEditor = nullptr;
	}
	else if (obj == m_patternEditor)
	{
		m_patternEditor = nullptr;
	}
	else if (obj == m_pianoRoll)
	{
		m_pianoRoll = nullptr;
	}
	else if (obj == m_projectNotes)
	{
		m_projectNotes = nullptr;
	}
	else if (obj == m_microtunerConfig)
	{
		m_microtunerConfig = nullptr;
	}
	else if (obj == m_controllerRackView)
	{
		m_controllerRackView = nullptr;
	}
}

void GuiApplication::sigintHandler(int)
{
#ifdef LMMS_BUILD_WIN32
	char message[] = "Sorry, SIGINT is unhandled on this platform\n";
	std::ignore = _write(_fileno(stderr), message, sizeof(message));
#else
	char a = 1;
	std::ignore = ::write(s_sigintFd[0], &a, sizeof(a));
#endif
}

// Create our unix signal notifiers
void GuiApplication::createSocketNotifier()
{
#ifdef LMMS_BUILD_WIN32
	// no-op
#else
	if (::socketpair(AF_UNIX, SOCK_STREAM, 0, s_sigintFd))
	{
		qFatal("Couldn't create SIGINT socketpair");
		return;
	}

	// Listen on the file descriptor for SIGINT
	m_sigintNotifier = new QSocketNotifier(s_sigintFd[1], QSocketNotifier::Read, this);
	connect(m_sigintNotifier, SIGNAL(activated(QSocketDescriptor)), this, SLOT(sigintOccurred()), Qt::QueuedConnection);
#endif
}

// Handle the shutdown event
void GuiApplication::sigintOccurred()
{
	m_sigintNotifier->setEnabled(false);
	qDebug() << "Shutting down...";
	// cleanup, etc
	qApp->exit(3);
	m_sigintNotifier->setEnabled(true);
}

#ifdef LMMS_BUILD_WIN32
QFont GuiApplication::getWin32SystemFont()
{
	auto metrics = NONCLIENTMETRICSW{ .cbSize = sizeof(NONCLIENTMETRICSW) };
	SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, sizeof(NONCLIENTMETRICSW), &metrics, 0);
	int pointSize = static_cast<int>(metrics.lfMessageFont.lfHeight);
	if ( pointSize < 0 )
	{
		// height is in pixels, convert to points
		HDC hDC = GetDC( nullptr );
		pointSize = MulDiv(std::abs(pointSize), 72, GetDeviceCaps(hDC, LOGPIXELSY));
		ReleaseDC( nullptr, hDC );
	}

	return QFont{QString::fromWCharArray(metrics.lfMessageFont.lfFaceName), pointSize};
}
#endif


} // namespace gui

} // namespace lmms
