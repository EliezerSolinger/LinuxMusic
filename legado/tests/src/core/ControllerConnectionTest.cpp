/*
 * ControllerConnectionTest.cpp
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

#include <memory>

#include <QDomDocument>
#include <QPointer>
#include <QtTest>

#include "AudioEngine.h"
#include "AutomatableModel.h"
#include "ControllerConnection.h"
#include "Engine.h"
#include "LfoController.h"
#include "MidiController.h"
#include "Song.h"

using namespace lmms;

//! Who owns a controller connection and its controller, and what happens when one of them goes
class ControllerConnectionTest : public QObject
{
	Q_OBJECT

	//! The model's state, as a project keeps it
	static auto saved(AutomatableModel& model) -> QDomDocument
	{
		auto doc = QDomDocument{};
		auto element = doc.createElement("model");
		doc.appendChild(element);
		model.saveSettings(doc, element, "value");
		return doc;
	}

	//! A model whose connection holds a controller of @p type of its own (a MIDI controller's state otherwise)
	static auto connectionHolding(Controller::ControllerType type) -> QDomDocument
	{
		auto model = FloatModel{0.f, 0.f, 1.f, 0.01f};
		model.setControllerConnection(std::make_unique<ControllerConnection>(std::make_unique<MidiController>(nullptr)));
		auto doc = saved(model);
		auto controller = doc.documentElement().firstChildElement("connection").firstChildElement("value")
			.firstChildElement();
		controller.setAttribute("type", static_cast<int>(type));
		return doc;
	}

private slots:
	void initTestCase()
	{
		Engine::init(true);
		Engine::audioEngine()->stopProcessing();
	}

	void cleanupTestCase()
	{
		Engine::destroy();
	}

	//! A song's controller removed: the connections to it go with it, and their models know
	void removingAControllerDeletesItsConnections()
	{
		auto* song = Engine::getSong();
		auto* lfo = new LfoController(song);
		song->addController(lfo);
		auto model = FloatModel{0.f, 0.f, 1.f, 0.01f};
		model.setControllerConnection(std::make_unique<ControllerConnection>(lfo));
		const auto connection = QPointer<ControllerConnection>{model.controllerConnection()};
		QCOMPARE(lfo->connectionCount(), 1);

		song->removeController(lfo);
		QVERIFY(connection.isNull());
		QVERIFY(model.controllerConnection() == nullptr);
	}

	//! A connection moved to another controller no longer depends on the first
	void aConnectionMovedOnOutlivesItsFirstController()
	{
		auto* song = Engine::getSong();
		auto* first = new LfoController(song);
		auto* second = new LfoController(song);
		song->addController(first);
		song->addController(second);
		auto model = FloatModel{0.f, 0.f, 1.f, 0.01f};
		model.setControllerConnection(std::make_unique<ControllerConnection>(first));
		model.controllerConnection()->setController(second);
		QCOMPARE(first->connectionCount(), 0);
		QCOMPARE(second->connectionCount(), 1);

		song->removeController(first);
		QVERIFY(model.controllerConnection() != nullptr);
		QCOMPARE(model.controllerConnection()->getController(), second);

		song->removeController(second);
		QVERIFY(model.controllerConnection() == nullptr);
	}

	//! Its state loaded again (as by undo), a model has one connection: the one there was is deleted
	void loadingAgainReplacesTheConnection()
	{
		auto* song = Engine::getSong();
		auto* lfo = new LfoController(song);
		song->addController(lfo);
		auto model = FloatModel{0.f, 0.f, 1.f, 0.01f};
		model.setControllerConnection(std::make_unique<ControllerConnection>(lfo));
		const auto first = QPointer<ControllerConnection>{model.controllerConnection()};
		const auto doc = saved(model);

		model.loadSettings(doc.documentElement(), "value");
		QVERIFY(first.isNull());
		QVERIFY(model.controllerConnection() != nullptr);
		QCOMPARE(model.controllerConnection()->getController(), lfo);
		QCOMPARE(lfo->connectionCount(), 1);

		song->removeController(lfo);
		QVERIFY(model.controllerConnection() == nullptr);
	}

	//! A MIDI controller is the connection's own: it is saved and loaded with it, and deleted with it
	void aMidiControllerIsTheConnectionsOwn()
	{
		auto model = FloatModel{0.f, 0.f, 1.f, 0.01f};
		auto midi = std::make_unique<MidiController>(nullptr);
		const auto controller = QPointer<Controller>{midi.get()};
		model.setControllerConnection(std::make_unique<ControllerConnection>(std::move(midi)));
		const auto doc = saved(model);

		auto loaded = FloatModel{0.f, 0.f, 1.f, 0.01f};
		loaded.loadSettings(doc.documentElement(), "value");
		QVERIFY(loaded.controllerConnection() != nullptr);
		QCOMPARE(loaded.controllerConnection()->getController()->type(), Controller::ControllerType::Midi);
		QVERIFY(loaded.controllerConnection()->getController() != controller.data());

		model.setControllerConnection(nullptr);
		QVERIFY(controller.isNull());
	}

	//! A connection in a file that holds another kind of controller (or none known) is left without one
	void aConnectionHoldingAnotherControllerIsLeftWithout()
	{
		for (const auto type : {Controller::ControllerType::Lfo, Controller::ControllerType::Peak,
			Controller::ControllerType::Dummy, static_cast<Controller::ControllerType>(57)})
		{
			const auto doc = connectionHolding(type);
			auto model = FloatModel{0.f, 0.f, 1.f, 0.01f};
			model.loadSettings(doc.documentElement(), "value");
			QVERIFY(model.controllerConnection() != nullptr);
			QCOMPARE(model.controllerConnection()->getController()->type(), Controller::ControllerType::Dummy);
		}
	}

	//! A connection given no controller has the dummy one
	void noControllerIsTheDummy()
	{
		auto connection = ControllerConnection{static_cast<Controller*>(nullptr)};
		QCOMPARE(connection.getController()->type(), Controller::ControllerType::Dummy);
		connection.setController(static_cast<Controller*>(nullptr));
		connection.setController(std::unique_ptr<Controller>{});
		QCOMPARE(connection.getController()->type(), Controller::ControllerType::Dummy);
	}
};

QTEST_GUILESS_MAIN(ControllerConnectionTest)
#include "ControllerConnectionTest.moc"
