/*
 * AmpAndIrControls.cpp - the Amp & IR plugin's controls and their window
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

#include "AmpAndIrControls.h"

#include <QDesktopServices>
#include <QDir>
#include <QUrl>
#include <QDomElement>
#include <QFileInfo>
#include <QGridLayout>
#include <QLabel>
#include <QMenu>
#include <QMessageBox>
#include <QPushButton>

#include "AmpAndIr.h"
#include "ConfigManager.h"
#include "FileDialog.h"
#include "Knob.h"
#include "PathUtil.h"
#include "Song.h"

namespace lmms
{

AmpAndIrControls::AmpAndIrControls(AmpAndIrEffect* effect) :
	EffectControls(effect),
	m_inputModel(0.f, -24.f, 24.f, 0.1f, this, tr("Input gain")),
	m_outputModel(0.f, -24.f, 24.f, 0.1f, this, tr("Output gain")),
	m_effect(effect)
{
	m_inputModel.setCenterValue(0.f);
	m_outputModel.setCenterValue(0.f);
}


auto AmpAndIrControls::setImpulseResponse(const QString& path) -> QString
{
	if (path.isEmpty()) { m_effect->clearImpulseResponse(); }
	else if (const auto error = m_effect->loadImpulseResponse(path); !error.isEmpty()) { return error; }
	m_impulseResponse = path.isEmpty() ? QString{} : PathUtil::toShortestRelative(PathUtil::toAbsolute(path));
	emit impulseResponseChanged();
	return {};
}


auto AmpAndIrControls::setAmpModel(const QString& path) -> QString
{
	if (const auto error = m_effect->loadAmpModel(path); !error.isEmpty()) { return error; }
	m_ampModel = path.isEmpty() ? QString{} : PathUtil::toShortestRelative(PathUtil::toAbsolute(path));
	emit ampModelChanged();
	return {};
}


void AmpAndIrControls::saveSettings(QDomDocument& doc, QDomElement& parent)
{
	m_inputModel.saveSettings(doc, parent, "input");
	m_outputModel.saveSettings(doc, parent, "output");
	parent.setAttribute("ir", m_impulseResponse);
	parent.setAttribute("nam", m_ampModel);
}


void AmpAndIrControls::loadSettings(const QDomElement& parent)
{
	m_inputModel.loadSettings(parent, "input");
	m_outputModel.loadSettings(parent, "output");
	// (a missing file: reported, the rest of the effect loads)
	if (const auto error = setAmpModel(parent.attribute("nam")); !error.isEmpty()) { Engine::getSong()->collectError(error); }
	if (const auto error = setImpulseResponse(parent.attribute("ir")); !error.isEmpty())
	{
		Engine::getSong()->collectError(error);
	}
}


auto AmpAndIrControls::createView() -> gui::EffectControlDialog*
{
	return new gui::AmpAndIrControlDialog(this);
}


namespace gui
{

AmpAndIrControlDialog::AmpAndIrControlDialog(AmpAndIrControls* controls) :
	EffectControlDialog(controls),
	m_controls(controls)
{
	auto layout = new QGridLayout(this);
	layout->setContentsMargins(8, 8, 8, 8);
	layout->setHorizontalSpacing(8);
	auto makeKnob = [this](FloatModel* model, const QString& label, const QString& hint) {
		auto knob = new Knob(KnobType::Bright26, label, this);
		knob->setModel(model);
		knob->setHintText(hint, " dB");
		return knob;
	};
	// IN, then the amp, then the IR, then OUT: the signal's way
	layout->addWidget(makeKnob(&controls->m_inputModel, tr("IN"), tr("Input gain:")), 0, 0, 2, 1);
	m_ampName = new QLabel(this);
	m_ampName->setMinimumWidth(150);
	layout->addWidget(m_ampName, 0, 1);
	auto amp = new QPushButton(tr("Amp model..."), this);
	connect(amp, &QPushButton::clicked, this, &AmpAndIrControlDialog::chooseAmpModel);
	layout->addWidget(amp, 1, 1);
	m_name = new QLabel(this);
	m_name->setMinimumWidth(150);
	layout->addWidget(m_name, 0, 2);
	auto choose = new QPushButton(tr("Impulse response..."), this);
	connect(choose, &QPushButton::clicked, this, &AmpAndIrControlDialog::chooseImpulseResponse);
	layout->addWidget(choose, 1, 2);
	layout->addWidget(makeKnob(&controls->m_outputModel, tr("OUT"), tr("Output gain:")), 0, 3, 2, 1);
	layout->setSizeConstraint(QLayout::SetFixedSize);

	const auto showNames = [this] {
		const auto ir = m_controls->impulseResponse();
		m_name->setText(ir.isEmpty() ? tr("No impulse response") : QFileInfo{ir}.completeBaseName());
		const auto amp = m_controls->ampModel();
		m_ampName->setText(amp.isEmpty() ? tr("No amp model") : QFileInfo{amp}.completeBaseName());
	};
	connect(controls, &AmpAndIrControls::impulseResponseChanged, this, showNames);
	connect(controls, &AmpAndIrControls::ampModelChanged, this, showNames);
	showNames();
}


void AmpAndIrControlDialog::warn(const QString& error)
{
	if (!error.isEmpty()) { QMessageBox::warning(this, tr("Amp & IR"), error); }
}


void AmpAndIrControlDialog::chooseAmpModel()
{
	auto menu = QMenu{this};
	menu.addAction(tr("From a file (.nam)..."), this, [this] {
		const auto path = FileDialog::getOpenFileName(this, tr("Amp model"), QString{},
			tr("Neural Amp Modeler models (*.nam)"));
		if (!path.isEmpty()) { warn(m_controls->setAmpModel(path)); }
	});
	// Captures of real amps are the community's, each under its author's terms: none comes with the program
	menu.addAction(tr("Find amp models online (TONE3000)..."), this, [] {
		QDesktopServices::openUrl(QUrl{"https://www.tone3000.com"});
	});
	if (!m_controls->ampModel().isEmpty())
	{
		menu.addAction(tr("None"), this, [this] { warn(m_controls->setAmpModel({})); });
	}
	menu.exec(QCursor::pos());
}


void AmpAndIrControlDialog::chooseImpulseResponse()
{
	// The library that comes with the program, or a file
	auto menu = QMenu{this};
	const auto library = QDir{ConfigManager::inst()->factorySamplesDir() + "impulse-responses"};
	for (const auto& folder : library.entryList(QDir::Dirs | QDir::NoDotAndDotDot))
	{
		auto* sub = menu.addMenu(folder);
		for (const auto& file : QDir{library.filePath(folder)}.entryInfoList({"*.wav"}, QDir::Files))
		{
			sub->addAction(file.completeBaseName(), this, [this, path = file.absoluteFilePath()] {
				warn(m_controls->setImpulseResponse(path));
			});
		}
	}
	menu.addSeparator();
	menu.addAction(tr("From a file..."), this, [this] {
		const auto path = FileDialog::getOpenFileName(this, tr("Impulse response"), QString{},
			tr("Audio files (*.wav *.flac *.aif *.aiff *.ogg)"));
		if (!path.isEmpty()) { warn(m_controls->setImpulseResponse(path)); }
	});
	if (!m_controls->impulseResponse().isEmpty())
	{
		menu.addAction(tr("None"), this, [this] { warn(m_controls->setImpulseResponse({})); });
	}
	menu.exec(QCursor::pos());
}

} // namespace gui

} // namespace lmms
