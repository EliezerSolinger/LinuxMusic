/*
 * ProjectStateTest.cpp
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

#include <QDomDocument>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QtTest>

#include "AudioEngine.h"
#include "DataFile.h"
#include "Engine.h"
#include "SerializingObject.h"
#include "Song.h"

using namespace lmms;

//! Parts of a project kept by someone other than the song (the GUI's editors and notes, see Song::addProjectState())
class ProjectStateTest : public QObject
{
	Q_OBJECT

	//! A part of the project like the GUI's: a name and a value
	class State : public SerializingObject
	{
	public:
		explicit State(QString name) : m_name{std::move(name)} {}
		auto nodeName() const -> QString override { return m_name; }
		void saveSettings(QDomDocument&, QDomElement& element) override { element.setAttribute("value", value); }
		void loadSettings(const QDomElement& element) override { value = element.attribute("value"); ++loads; }
		QString value;
		int loads = 0;

	private:
		QString m_name;
	};

	QTemporaryDir m_directory;

	//! A project with what the GUI keeps, and the song's loop
	auto writeProject() -> QString
	{
		Engine::getSong()->createNewProject();
		const auto path = m_directory.filePath("project.mmp");
		if (!Engine::getSong()->saveProjectFile(path)) { return {}; }
		auto file = DataFile{path};
		auto addElement = [&](const QString& name, const QString& value) {
			// (in place of the default template's)
			for (auto old = file.content().firstChildElement(name); !old.isNull(); old = file.content().firstChildElement(name))
			{
				file.content().removeChild(old);
			}
			auto element = file.createElement(name);
			element.setAttribute("value", value);
			auto child = file.createElement("child");
			child.appendChild(file.createTextNode("text kept as it is"));
			element.appendChild(child);
			file.content().appendChild(element);
		};
		addElement("ControllerRackView", "rack");
		addElement("pianoroll", "roll");
		addElement("projectnotes", "notes");
		addElement("fromanewerversion", "unknown");
		auto timeline = file.content().firstChildElement("timeline");
		if (timeline.isNull())
		{
			timeline = file.createElement("timeline");
			file.content().appendChild(timeline);
		}
		timeline.setAttribute("lp0pos", 192);
		timeline.setAttribute("lp1pos", 576);
		timeline.setAttribute("lpstate", 1);
		return file.writeFile(path) ? path : QString{};
	}

	static auto text(const QDomElement& element) -> QString
	{
		auto out = QString{};
		auto stream = QTextStream{&out};
		element.save(stream, 0);
		return out;
	}

	//! The element @p name of the project @p path, as text (empty if none)
	static auto element(const QString& path, const QString& name) -> QString
	{
		auto file = DataFile{path};
		const auto found = file.content().firstChildElement(name);
		return found.isNull() ? QString{} : text(found);
	}

private slots:
	void initTestCase()
	{
		QVERIFY(m_directory.isValid());
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	//! Without a GUI, what the GUI keeps in a project comes back unchanged when it is saved again
	void withoutTheGuiItsPartsOfTheProjectAreKept()
	{
		const auto path = writeProject();
		QVERIFY(!path.isEmpty());
		Engine::getSong()->loadProject(path);
		const auto again = m_directory.filePath("again.mmp");
		QVERIFY(Engine::getSong()->saveProjectFile(again));
		for (const auto& name : {"ControllerRackView", "pianoroll", "projectnotes", "fromanewerversion"})
		{
			const auto before = element(path, name);
			QVERIFY2(!before.isEmpty(), name);
			QCOMPARE(element(again, name), before);
		}
	}

	//! The song's loop is the song's: it loads and saves without a GUI too
	void theLoopLoadsAndSavesWithoutTheGui()
	{
		const auto path = writeProject();
		QVERIFY(!path.isEmpty());
		Engine::getSong()->loadProject(path);
		const auto& timeline = Engine::getSong()->getTimeline(Song::PlayMode::Song);
		QCOMPARE(timeline.loopBegin().getTicks(), 192);
		QCOMPARE(timeline.loopEnd().getTicks(), 576);
		const auto again = m_directory.filePath("again.mmp");
		QVERIFY(Engine::getSong()->saveProjectFile(again));
		QVERIFY(element(again, "timeline").contains("lp1pos=\"576\""));
	}

	//! A part added by the GUI is loaded by it, saved by it (once, in the order added), and not kept besides
	void aPartAddedIsLoadedAndSavedByItsKeeper()
	{
		auto roll = State{"pianoroll"};
		auto rack = State{"ControllerRackView"};
		Engine::getSong()->addProjectState(&rack);
		Engine::getSong()->addProjectState(&roll);
		const auto removeStates = qScopeGuard([&] {
			Engine::getSong()->removeProjectState(&rack);
			Engine::getSong()->removeProjectState(&roll);
		});
		const auto path = writeProject();
		QVERIFY(!path.isEmpty());
		const auto loadsBefore = roll.loads;
		Engine::getSong()->loadProject(path);
		QCOMPARE(roll.value, QString{"roll"});
		QCOMPARE(rack.value, QString{"rack"});
		QCOMPARE(roll.loads, loadsBefore + 1);

		roll.value = "changed";
		const auto again = m_directory.filePath("again.mmp");
		QVERIFY(Engine::getSong()->saveProjectFile(again));
		auto saved = DataFile{again};
		const auto content = saved.content();
		QCOMPARE(content.elementsByTagName("pianoroll").count(), 1);
		QCOMPARE(content.firstChildElement("pianoroll").attribute("value"), QString{"changed"});
		// In the order added, before the elements kept for nobody
		auto names = QStringList{};
		for (auto child = content.firstChildElement(); !child.isNull(); child = child.nextSiblingElement())
		{
			names << child.nodeName();
		}
		QVERIFY(names.indexOf("ControllerRackView") < names.indexOf("pianoroll"));
		QVERIFY(names.indexOf("pianoroll") < names.indexOf("projectnotes"));
		QVERIFY(names.indexOf("projectnotes") < names.indexOf("timeline"));
	}

	//! A new project starts without the old one's kept parts, and the views are told before it is cleared
	void clearingTheProjectTellsTheViewsAndForgetsKeptParts()
	{
		const auto path = writeProject();
		QVERIFY(!path.isEmpty());
		Engine::getSong()->loadProject(path);
		auto spy = QSignalSpy{Engine::getSong(), &Song::aboutToClearProject};
		Engine::getSong()->createNewProject();
		QVERIFY(spy.count() >= 1);
		const auto again = m_directory.filePath("new.mmp");
		QVERIFY(Engine::getSong()->saveProjectFile(again));
		// (a new project has the default template's notes, not the old project's)
		QVERIFY(!element(again, "projectnotes").contains("value=\"notes\""));
		QVERIFY(element(again, "fromanewerversion").isEmpty());
	}
};

QTEST_GUILESS_MAIN(ProjectStateTest)
#include "ProjectStateTest.moc"
