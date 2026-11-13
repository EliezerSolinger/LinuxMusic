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

#include <QDir>
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
	const auto error = m_effect->loadImpulseResponse(path);
	if (!error.isEmpty()) { return error; }
	m_impulseResponse = PathUtil::toShortestRelative(PathUtil::toAbsolute(path));
	emit impulseResponseChanged();
	return {};
}


void AmpAndIrControls::saveSettings(QDomDocument& doc, QDomElement& parent)
{
	m_inputModel.saveSettings(doc, parent, "input");
	m_outputModel.saveSettings(doc, parent, "output");
	parent.setAttribute("ir", m_impulseResponse);
}


void AmpAndIrControls::loadSettings(const QDomElement& parent)
{
	m_inputModel.loadSettings(parent, "input");
	m_outputModel.loadSettings(parent, "output");
	if (const auto ir = parent.attribute("ir"); !ir.isEmpty())
	{
		if (const auto error = setImpulseResponse(ir); !error.isEmpty()) { Engine::getSong()->collectError(error); }
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
	auto makeKnob = [this](FloatModel* model, const QString& label, const QString& hint) {
		auto knob = new Knob(KnobType::Bright26, label, this);
		knob->setModel(model);
		knob->setHintText(hint, " dB");
		return knob;
	};
	layout->addWidget(makeKnob(&controls->m_inputModel, tr("IN"), tr("Input gain:")), 0, 0, 2, 1);
	m_name = new QLabel(this);
	m_name->setMinimumWidth(160);
	layout->addWidget(m_name, 0, 1);
	auto choose = new QPushButton(tr("Impulse response..."), this);
	connect(choose, &QPushButton::clicked, this, &AmpAndIrControlDialog::choose);
	layout->addWidget(choose, 1, 1);
	layout->addWidget(makeKnob(&controls->m_outputModel, tr("OUT"), tr("Output gain:")), 0, 2, 2, 1);
	layout->setSizeConstraint(QLayout::SetFixedSize);

	const auto showName = [this] {
		const auto ir = m_controls->impulseResponse();
		m_name->setText(ir.isEmpty() ? tr("No impulse response") : QFileInfo{ir}.completeBaseName());
	};
	connect(controls, &AmpAndIrControls::impulseResponseChanged, this, showName);
	showName();
}


void AmpAndIrControlDialog::choose()
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
				if (const auto error = m_controls->setImpulseResponse(path); !error.isEmpty())
				{
					QMessageBox::warning(this, tr("Impulse response"), error);
				}
			});
		}
	}
	menu.addSeparator();
	menu.addAction(tr("From a file..."), this, [this] {
		const auto path = FileDialog::getOpenFileName(this, tr("Impulse response"), QString{},
			tr("Audio files (*.wav *.flac *.aif *.aiff *.ogg)"));
		if (path.isEmpty()) { return; }
		if (const auto error = m_controls->setImpulseResponse(path); !error.isEmpty())
		{
			QMessageBox::warning(this, tr("Impulse response"), error);
		}
	});
	menu.exec(QCursor::pos());
}

} // namespace gui

} // namespace lmms
