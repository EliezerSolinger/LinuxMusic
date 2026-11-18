/*
 * AmplifikationLiteControls.cpp - Amplifikation Lite's knobs, switches and panel
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

#include "AmplifikationLiteControls.h"

#include <cmath>

#include <QDialog>
#include <QDomElement>
#include <QFileInfo>
#include <QLabel>
#include <QMessageBox>
#include <QMouseEvent>
#include <QPainter>
#include <QPushButton>
#include <QVBoxLayout>

#include "AmplifikationLite.h"
#include "FileDialog.h"
#include "PathUtil.h"
#include "Song.h"
#include "embed.h"

namespace lmms
{

AmplifikationLiteControls::AmplifikationLiteControls(AmplifikationLiteEffect* effect) :
	EffectControls(effect),
	m_input(0.2f, 0.f, 1.f, 0.001f, this, tr("Input")),
	m_channel(0.f, 0.f, 2.f, 1.f, this, tr("Channel")),
	m_gain(0.5f, 0.f, 1.f, 0.001f, this, tr("Gain")),
	m_bass(0.5f, 0.f, 1.f, 0.001f, this, tr("Bass")),
	m_middle(0.5f, 0.f, 1.f, 0.001f, this, tr("Middle")),
	m_treble(0.5f, 0.f, 1.f, 0.001f, this, tr("Treble")),
	m_presence(0.5f, 0.f, 1.f, 0.001f, this, tr("Presence")),
	m_volume(0.5f, 0.f, 1.f, 0.001f, this, tr("Volume")),
	m_cabinet(0.f, 0.f, 2.f, 1.f, this, tr("Cabinet")),
	m_effect(effect)
{
}


auto AmplifikationLiteControls::settings() const -> amplite::AmpLiteDsp::Settings
{
	auto s = amplite::AmpLiteDsp::Settings{};
	s.channel = static_cast<int>(std::lround(m_channel.value()));
	s.input = m_input.value();
	s.gain = m_gain.value();
	s.bass = m_bass.value();
	s.middle = m_middle.value();
	s.treble = m_treble.value();
	s.presence = m_presence.value();
	s.volume = m_volume.value();
	s.internalCabinet = cabinet() == Cabinet::Internal;
	return s;
}


auto AmplifikationLiteControls::cabinet() const -> Cabinet
{
	return static_cast<Cabinet>(std::clamp(static_cast<int>(std::lround(m_cabinet.value())), 0, 2));
}


auto AmplifikationLiteControls::setImpulseResponse(const QString& path) -> QString
{
	if (path.isEmpty()) { m_effect->clearImpulseResponse(); }
	else if (const auto error = m_effect->loadImpulseResponse(path); !error.isEmpty()) { return error; }
	m_impulseResponse = path.isEmpty() ? QString{} : PathUtil::toShortestRelative(PathUtil::toAbsolute(path));
	emit impulseResponseChanged();
	return {};
}


void AmplifikationLiteControls::saveSettings(QDomDocument& doc, QDomElement& parent)
{
	m_input.saveSettings(doc, parent, "input");
	m_channel.saveSettings(doc, parent, "channel");
	m_gain.saveSettings(doc, parent, "gain");
	m_bass.saveSettings(doc, parent, "bass");
	m_middle.saveSettings(doc, parent, "middle");
	m_treble.saveSettings(doc, parent, "treble");
	m_presence.saveSettings(doc, parent, "presence");
	m_volume.saveSettings(doc, parent, "volume");
	m_cabinet.saveSettings(doc, parent, "cabinet");
	parent.setAttribute("ir", m_impulseResponse);
}


void AmplifikationLiteControls::loadSettings(const QDomElement& parent)
{
	m_input.loadSettings(parent, "input");
	m_channel.loadSettings(parent, "channel");
	m_gain.loadSettings(parent, "gain");
	m_bass.loadSettings(parent, "bass");
	m_middle.loadSettings(parent, "middle");
	m_treble.loadSettings(parent, "treble");
	m_presence.loadSettings(parent, "presence");
	m_volume.loadSettings(parent, "volume");
	m_cabinet.loadSettings(parent, "cabinet");
	if (const auto error = setImpulseResponse(parent.attribute("ir")); !error.isEmpty())
	{
		Engine::getSong()->collectError(error);
	}
}


auto AmplifikationLiteControls::createView() -> gui::EffectControlDialog*
{
	return new gui::AmplifikationLiteControlDialog(this);
}


namespace gui
{

namespace
{

// The original's panel (600 x 230): each control's top-left corner, matched to the pixel against a screenshot
// of the original (Amplifikation-Lite-1.1.1-Screenshot.jpeg)
constexpr int KnobSize = 36;
constexpr QPoint InputKnob{53, 146};
constexpr QPoint GainKnob{168, 146};
constexpr QPoint BassKnob{236, 146};
constexpr QPoint MiddleKnob{293, 146};
constexpr QPoint TrebleKnob{350, 146};
constexpr QPoint PresenceKnob{406, 146};
constexpr QPoint VolumeKnob{475, 146};
constexpr QPoint ChannelSwitch{141, 145};
constexpr QPoint CabinetSwitch{525, 145};
constexpr QPoint IrButton{557, 160};
constexpr QRect AboutArea{562, 198, 26, 26};
constexpr int SwitchFrameHeight = 39;

} // namespace


FilmstripKnob::FilmstripKnob(const QPixmap& strip, int frameSize, QWidget* parent) :
	FloatModelEditorBase(DirectionOfManipulation::Vertical, parent),
	m_strip(strip),
	m_frameSize(frameSize),
	m_frames(std::max(1, strip.height() / frameSize))
{
	setFixedSize(frameSize, frameSize);
	setCursor(Qt::PointingHandCursor);
}


void FilmstripKnob::paintEvent(QPaintEvent*)
{
	const auto state = model() ? std::clamp(static_cast<double>(model()->normValue(model()->value())), 0., 1.) : 0.;
	const int frame = static_cast<int>(std::lround(state * (m_frames - 1)));
	auto p = QPainter{this};
	p.drawPixmap(0, 0, m_strip, 0, frame * m_frameSize, m_frameSize, m_frameSize);
}


ToggleSwitch::ToggleSwitch(const QPixmap& strip, QWidget* parent) :
	FloatModelEditorBase(DirectionOfManipulation::Vertical, parent),
	m_strip(strip)
{
	setFixedSize(strip.width(), SwitchFrameHeight);
	setCursor(Qt::PointingHandCursor);
}


void ToggleSwitch::paintEvent(QPaintEvent*)
{
	const int position = model() ? std::clamp(static_cast<int>(std::lround(model()->value())), 0, 2) : 0;
	auto p = QPainter{this};
	p.drawPixmap(0, 0, m_strip, 0, position * SwitchFrameHeight, m_strip.width(), SwitchFrameHeight);
}


void ToggleSwitch::mousePressEvent(QMouseEvent* event)
{
	if (event->button() != Qt::LeftButton || !model())
	{
		FloatModelEditorBase::mousePressEvent(event);
		return;
	}
	// a click above the middle moves the lever up one position, below it down one
	const int position = std::clamp(static_cast<int>(std::lround(model()->value())), 0, 2);
	const int step = event->position().y() < height() / 2.0 ? -1 : 1;
	model()->setValue(static_cast<float>(std::clamp(position + step, 0, 2)));
	update();
}


void ToggleSwitch::mouseMoveEvent(QMouseEvent*) {}
void ToggleSwitch::mouseReleaseEvent(QMouseEvent*) {}


void ToggleSwitch::wheelEvent(QWheelEvent* event)
{
	if (!model()) { return; }
	const int step = event->angleDelta().y() > 0 ? -1 : 1;
	model()->setValue(std::clamp(model()->value() + step, 0.f, 2.f));
	event->accept();
	update();
}


AmplifikationLiteControlDialog::AmplifikationLiteControlDialog(AmplifikationLiteControls* controls) :
	EffectControlDialog(controls),
	m_controls(controls)
{
	const auto background = PLUGIN_NAME::getIconPixmap("background");
	setAutoFillBackground(true);
	auto palette = QPalette{};
	palette.setBrush(backgroundRole(), background);
	setPalette(palette);
	setFixedSize(background.size());

	const auto strip = PLUGIN_NAME::getIconPixmap("silverknob");
	auto knob = [&](FloatModel* model, QPoint corner, const QString& hint) {
		auto* k = new FilmstripKnob(strip, KnobSize, this);
		k->setModel(model);
		k->setHintText(hint, "");
		k->move(corner);
	};
	knob(&controls->m_input, InputKnob, tr("Input:"));
	knob(&controls->m_gain, GainKnob, tr("Gain:"));
	knob(&controls->m_bass, BassKnob, tr("Bass:"));
	knob(&controls->m_middle, MiddleKnob, tr("Middle:"));
	knob(&controls->m_treble, TrebleKnob, tr("Treble:"));
	knob(&controls->m_presence, PresenceKnob, tr("Presence:"));
	knob(&controls->m_volume, VolumeKnob, tr("Volume:"));

	const auto switchStrip = PLUGIN_NAME::getIconPixmap("silverswitch");
	auto* channel = new ToggleSwitch(switchStrip, this);
	channel->setModel(&controls->m_channel);
	channel->move(ChannelSwitch);
	channel->setToolTip(tr("Channel: clean, rhythm, lead"));
	auto* cabinet = new ToggleSwitch(switchStrip, this);
	cabinet->setModel(&controls->m_cabinet);
	cabinet->move(CabinetSwitch);
	cabinet->setToolTip(tr("Cabinet: INT (built-in), OFF, EXT (a loaded impulse response)"));

	// the IR's load button: only there when the cabinet is on EXT
	auto* ir = new QPushButton(this);
	ir->setIcon(PLUGIN_NAME::getIconPixmap("irload"));
	ir->setIconSize(QSize(28, 28));
	ir->setFixedSize(28, 28);
	ir->setFlat(true);
	ir->setStyleSheet("background: transparent; border: none;");
	ir->setCursor(Qt::PointingHandCursor);
	ir->move(IrButton);
	connect(ir, &QPushButton::clicked, this, &AmplifikationLiteControlDialog::chooseImpulseResponse);
	const auto showIr = [ir, controls] {
		const auto path = controls->impulseResponse();
		ir->setToolTip(path.isEmpty() ? tr("Load the EXT cabinet's impulse response")
			: tr("EXT cabinet: %1 (click to load another)").arg(QFileInfo{path}.completeBaseName()));
		ir->setVisible(controls->cabinet() == AmplifikationLiteControls::Cabinet::External);
	};
	connect(controls, &AmplifikationLiteControls::impulseResponseChanged, this, showIr);
	connect(&controls->m_cabinet, &FloatModel::dataChanged, this, showIr);
	showIr();

	auto* about = new QPushButton(this);
	about->setFlat(true);
	about->setStyleSheet("background: transparent; border: none;");
	about->setGeometry(AboutArea);
	about->setCursor(Qt::PointingHandCursor);
	about->setToolTip(tr("About"));
	connect(about, &QPushButton::clicked, this, &AmplifikationLiteControlDialog::showAbout);
}


void AmplifikationLiteControlDialog::showAbout()
{
	auto dialog = QDialog{this};
	dialog.setWindowTitle(tr("Amplifikation Lite"));
	auto* layout = new QVBoxLayout(&dialog);
	auto* image = new QLabel(&dialog);
	image->setPixmap(PLUGIN_NAME::getIconPixmap("about"));
	layout->addWidget(image);
	auto* note = new QLabel(tr("Modeled in LinuxMusic from measurements of Kuassa's Amplifikation Lite. "
		"Artwork by Kuassa, used with permission."), &dialog);
	note->setWordWrap(true);
	layout->addWidget(note);
	dialog.exec();
}


void AmplifikationLiteControlDialog::chooseImpulseResponse()
{
	const auto path = FileDialog::getOpenFileName(this, tr("Impulse response"), QString{},
		tr("Audio files (*.wav *.flac *.aif *.aiff *.ogg)"));
	if (path.isEmpty()) { return; }
	if (const auto error = m_controls->setImpulseResponse(path); !error.isEmpty())
	{
		QMessageBox::warning(this, tr("Amplifikation Lite"), error);
	}
}

} // namespace gui

} // namespace lmms
