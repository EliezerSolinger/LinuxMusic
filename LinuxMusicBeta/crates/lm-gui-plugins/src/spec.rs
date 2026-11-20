//! A panel as data: the legacy view's size, its background and every control with its position, type and the
//! parameter or field it moves. Each `panels/<plugin>.rs` writes one, read off the legacy `*View.cpp` /
//! `*ControlDialog.cpp` (docs/plugins-visual.md lists where each number comes from).

use crate::assets::Asset;
use crate::widgets::{KnobStyle, LcdSheet, LedColor};

/// A translatable text: a legacy English source string (the key of the legacy `.ts` translations and of
/// `locales/pt-BR/panels.txt`) and the number that replaces its `%1`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Text {
    pub key: &'static str,
    pub arg: Option<u32>,
}

impl Text {
    pub const fn new(key: &'static str) -> Text {
        Text { key, arg: None }
    }

    pub const fn n(key: &'static str, arg: u32) -> Text {
        Text { key, arg: Some(arg) }
    }

    pub const EMPTY: Text = Text::new("");

    /// The shown text
    pub fn show(&self) -> String {
        let t = crate::i18n::tr(self.key);
        match self.arg {
            Some(a) => t.replace("%1", &a.to_string()),
            None => t.to_string(),
        }
    }
}

/// What a control moves in the plugin's state (`state.params.<key>` or `state.<key>`): the same target a MIDI
/// mapping names (I-21.2: `MapTarget::Param` or `MapTarget::Field`)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// A parameter (automatable, a slot in the engine)
    Param(&'static str),
    /// A field of the state: a choice (by its text key, U30) or a switch
    Field(&'static str),
}

impl Target {
    pub fn key(&self) -> &'static str {
        match self {
            Target::Param(k) | Target::Field(k) => k,
        }
    }
}

/// What a button does
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bind {
    /// One button of a group (`AutomatableButtonGroup`): on when the choice field holds `key`; a click selects it
    Choice { field: &'static str, key: &'static str },
    /// A switch on a stepped 0–1 parameter (the legacy automatable `BoolModel`): a click toggles it
    Switch { param: &'static str },
    /// A switch on a boolean field (not automatable in the legacy either)
    Flag { field: &'static str },
}

impl Bind {
    pub fn target(&self) -> Target {
        match *self {
            Bind::Choice { field, .. } | Bind::Flag { field } => Target::Field(field),
            Bind::Switch { param } => Target::Param(param),
        }
    }
}

/// A knob (`Knob`, `VolumeKnob`, `TempoSyncKnob`)
#[derive(Clone, Debug, PartialEq)]
pub struct KnobSpec {
    pub style: KnobStyle,
    pub param: &'static str,
    /// The legacy model's step (the drag, the wheel and the text round to it)
    pub step: f64,
    /// The arc's origin (`setCenterValue`), absent: the minimum
    pub center: Option<f64>,
    /// `setHintText(text, unit)`
    pub hint: Text,
    pub unit: &'static str,
    /// A `VolumeKnob`: its text is dBFS of `value / 100`, and the entry asks in dBFS
    pub volume: bool,
    /// The label under the pixmap (`setLabel`), drawn at `SMALL_FONT_SIZE`
    pub label: Option<Text>,
}

/// An LED check box (`LedCheckBox`), with its text to the right
#[derive(Clone, Debug, PartialEq)]
pub struct LedSpec {
    pub color: LedColor,
    pub text: Text,
    pub bind: Bind,
}

/// A pixmap button (`PixmapButton`): its two images; `checkable` false shows the active image while pressed
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonSpec {
    pub active: &'static Asset,
    pub inactive: &'static Asset,
    pub bind: Bind,
    /// A double click chooses an audio file for this resource field (TripleOscillator's user wave,
    /// `oscUserDefWaveDblClick`): the file is imported through the port and the field and the button's choice are
    /// set in one step
    pub file_field: Option<&'static str>,
}

/// An LCD spin box (`LcdSpinBox`) on a stepped parameter
#[derive(Clone, Debug, PartialEq)]
pub struct LcdSpec {
    pub sheet: LcdSheet,
    pub digits: usize,
    pub param: &'static str,
    pub label: Option<Text>,
    pub leading_zero: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Knob(KnobSpec),
    Led(LedSpec),
    Button(ButtonSpec),
    Lcd(LcdSpec),
}

/// One control at its place
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    /// The test name (`debug_selector`) and the stable name of the control in the panel
    pub id: &'static str,
    pub x: f32,
    pub y: f32,
    pub kind: Kind,
    /// The model's display name: the context menu's caption and the undo step's label
    pub name: Text,
    /// The tooltip (`setToolTip`)
    pub tip: Text,
}

impl Control {
    pub fn target(&self) -> Target {
        match &self.kind {
            Kind::Knob(k) => Target::Param(k.param),
            Kind::Lcd(l) => Target::Param(l.param),
            Kind::Led(l) => l.bind.target(),
            Kind::Button(b) => b.bind.target(),
        }
    }

    /// Its size on the panel (the pixmaps' sizes, or the styled knob's fixed size)
    pub fn size(&self) -> (f32, f32) {
        match &self.kind {
            Kind::Knob(k) => k.style.size(),
            Kind::Led(l) => {
                let img = crate::assets::image(l.color.asset());
                (img.width() as f32, img.height() as f32)
            }
            Kind::Button(b) => {
                let a = crate::assets::image(b.active);
                let i = crate::assets::image(b.inactive);
                (a.width().max(i.width()) as f32, a.height().max(i.height()) as f32)
            }
            Kind::Lcd(l) => crate::widgets::lcd_size(l.sheet, l.digits),
        }
    }
}

/// A plugin's panel
#[derive(Debug)]
pub struct PanelSpec {
    pub plugin: &'static str,
    /// The legacy view's fixed size
    pub size: (f32, f32),
    /// The legacy background (`setPalette` with the artwork), PNG or SVG
    pub background: Option<&'static Asset>,
    /// The fill under it (the parts the artwork does not cover)
    pub fill: u32,
    pub controls: Vec<Control>,
}

impl PanelSpec {
    /// The control at panel point `x`, `y` (the last one drawn wins, as Qt's stacking)
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.controls.iter().enumerate().rev().find_map(|(i, c)| {
            let (w, h) = c.size();
            (x >= c.x && y >= c.y && x < c.x + w && y < c.y + h).then_some(i)
        })
    }

    /// The controls of one target (a group's buttons share one)
    pub fn controls_of(&self, t: Target) -> impl Iterator<Item = (usize, &Control)> {
        self.controls.iter().enumerate().filter(move |(_, c)| c.target() == t)
    }
}
