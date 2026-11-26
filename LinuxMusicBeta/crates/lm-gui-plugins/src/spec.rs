//! A panel as data: the legacy view's size, its background and every control with its position, type and the
//! parameter or field it moves. Each `panels/<plugin>.rs` writes one, read off the legacy `*View.cpp` /
//! `*ControlDialog.cpp` (docs/plugins-visual.md lists where each number comes from).

use lm_port::Value;

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

/// What a push button sees: the parameters' present values and defaults, and random numbers in [0, 1)
pub struct PressCx<'a> {
    pub value: &'a dyn Fn(&str) -> f64,
    /// A field's value (a graph's samples, a choice)
    pub field: &'a dyn Fn(&str) -> Value,
    pub default: &'a dyn Fn(&str) -> f64,
    pub rand: &'a mut dyn FnMut() -> f64,
}

impl std::fmt::Debug for PressCx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PressCx")
    }
}

/// What a pressed button changes (one undo step)
pub type PressFn = fn(&mut PressCx<'_>) -> Vec<(Target, Value)>;

/// What a button does
#[derive(Clone, Copy, Debug)]
pub enum Bind {
    /// One button of a group (`AutomatableButtonGroup`): on when the choice field holds `key`; a click selects it
    Choice { field: &'static str, key: &'static str },
    /// A switch on a stepped 0–1 parameter (the legacy automatable `BoolModel`): a click toggles it
    Switch { param: &'static str },
    /// A switch on a boolean field (not automatable in the legacy either)
    Flag { field: &'static str },
    /// A push button on a stepped 0–1 parameter that is 1 while it is held (the legacy automatable button that is not
    /// checkable: the frequency shifter's resets)
    Momentary { param: &'static str },
    /// A push button without a model that changes several things at once (Organic's "Randomise")
    Press(PressFn),
}

impl Bind {
    pub fn target(&self) -> Target {
        match *self {
            Bind::Choice { field, .. } | Bind::Flag { field } => Target::Field(field),
            Bind::Switch { param } | Bind::Momentary { param } => Target::Param(param),
            Bind::Press(_) => Target::Field(""),
        }
    }
}

/// A knob (`Knob`, `VolumeKnob`, `TempoSyncKnob`)
#[derive(Clone, Debug)]
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
    /// A `VolumeKnob`: its text is dBFS of `value / zero_dbfs`, and the entry asks in dBFS
    pub volume: bool,
    /// The value that is 0 dBFS (`setZeroDbfsPoint`, 100 by default)
    pub zero_dbfs: f64,
    /// A unit worked out from the value (the legacy views that call `setHintText` again on every change: Sid's
    /// times and cutoff in Hz)
    pub unit_of: Option<fn(f64) -> String>,
    /// The label under the pixmap (`setLabel`), drawn at `SMALL_FONT_SIZE`
    pub label: Option<Text>,
    /// A knob that turns a choice field (`param` is then the field): the knob's value is the index of the field's
    /// key in this list (Organic's waveform knobs; the legacy had a number, the new state a key, U30)
    pub choices: Option<&'static [&'static str]>,
    /// The legacy model's `setScaleLogarithmic(true)` (the knob's travel; the schema's `Scale::Log` is only how the
    /// generic editor shows it)
    pub log: bool,
}

/// An LED check box (`LedCheckBox`), with its text to the right
#[derive(Clone, Debug)]
pub struct LedSpec {
    pub color: LedColor,
    pub text: Text,
    pub bind: Bind,
}

/// A pixmap button (`PixmapButton`): its two images; `checkable` false shows the active image while pressed
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug)]
pub struct LcdSpec {
    pub sheet: LcdSheet,
    pub digits: usize,
    pub param: &'static str,
    pub label: Option<Text>,
    pub leading_zero: bool,
    /// A `LcdFloatSpinBox`: the digits after the point (`digits` are then the whole part's); 0 for the integer one
    pub frac: usize,
    /// The model's step (the drag and the wheel round to it)
    pub step: f64,
}

/// One entry of a combo box: the choice's key, its text and its icon
#[derive(Clone, Debug)]
pub struct ComboItem {
    pub key: &'static str,
    pub text: Text,
    pub icon: Option<&'static Asset>,
}

/// A combo box (`ComboBox`) on a choice field, `w` wide (its height is `ComboBox::DEFAULT_HEIGHT`, 22)
#[derive(Clone, Debug)]
pub struct ComboSpec {
    pub field: &'static str,
    pub w: f32,
    pub items: Vec<ComboItem>,
}

/// How a graph draws its samples (`Graph::Style`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphStyle {
    /// steps, not antialiased
    Nearest,
    /// lines, antialiased, the last back to the first
    Linear,
    /// lines, antialiased
    LinearNonCyclic,
    /// filled bars with a line on top
    Bar,
}

/// A drawable graph (`Graph`) on a list field of reals: the user draws the samples with the mouse
#[derive(Clone, Debug)]
pub struct GraphSpec {
    pub field: &'static str,
    pub w: f32,
    pub h: f32,
    pub min: f64,
    pub max: f64,
    pub length: usize,
    /// The parameter that sets how many of the samples show (MultitapEcho's Steps), absent: all of them
    pub length_param: Option<&'static str>,
    pub style: GraphStyle,
    /// `setGraphColor`
    pub color: crate::raster::Rgba,
    /// The pixmap under the curve (the plugin's `setPalette` background brush: BitInvader, WaveShaper...)
    pub background: Option<&'static Asset>,
    /// `setForeground`: a pixmap over the curve (the plugin's grid)
    pub foreground: Option<&'static Asset>,
    /// The samples' step (`graphModel`'s; 0: any value)
    pub step: f64,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Knob(KnobSpec),
    Led(LedSpec),
    Button(ButtonSpec),
    Lcd(LcdSpec),
    Combo(ComboSpec),
    Graph(GraphSpec),
}

/// A fixed text of the view (a `QLabel`): its top-left corner, its size in pixels
#[derive(Clone, Debug)]
pub struct StaticText {
    pub x: f32,
    pub y: f32,
    pub px: f32,
    pub text: Text,
    pub color: u32,
}

/// One control at its place
#[derive(Clone, Debug)]
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
            Kind::Knob(k) if k.choices.is_some() => Target::Field(k.param),
            Kind::Knob(k) => Target::Param(k.param),
            Kind::Lcd(l) => Target::Param(l.param),
            Kind::Led(l) => l.bind.target(),
            Kind::Button(b) => b.bind.target(),
            Kind::Combo(c) => Target::Field(c.field),
            Kind::Graph(g) => Target::Field(g.field),
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
            Kind::Lcd(l) if l.frac > 0 => crate::widgets::lcd_float_size(l.sheet, l.digits, l.frac),
            Kind::Lcd(l) => crate::widgets::lcd_size(l.sheet, l.digits),
            Kind::Combo(c) => (c.w, crate::widgets::COMBO_HEIGHT),
            Kind::Graph(g) => (g.w, g.h),
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
    /// The view's fixed texts
    pub labels: Vec<StaticText>,
    /// What a file dropped on the panel does (the legacy views' `dropEvent`)
    pub file_drop: Option<FileDrop>,
}

/// A file dropped on a panel
#[derive(Clone, Copy, Debug)]
pub enum FileDrop {
    /// An action of the core that reads the file (`{node, path}`), as OpulenZ's `.sbi` import
    Action { action: &'static str, extensions: &'static [&'static str] },
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

// --- builders, so a panel reads like the legacy view's constructor

/// A knob on `param` at `x`, `y`: `name` the model's name, `hint` and `unit` its `setHintText`
#[allow(clippy::too_many_arguments)]
pub fn knob(
    param: &'static str,
    x: f32,
    y: f32,
    style: KnobStyle,
    step: f64,
    name: &'static str,
    hint: &'static str,
    unit: &'static str,
) -> Control {
    Control {
        id: param,
        x,
        y,
        kind: Kind::Knob(KnobSpec {
            style,
            param,
            step,
            center: None,
            hint: Text::new(hint),
            unit,
            volume: false,
            zero_dbfs: 100.0,
            unit_of: None,
            label: None,
            choices: None,
            log: false,
        }),
        name: Text::new(name),
        tip: Text::EMPTY,
    }
}

/// An LED check box at `x`, `y`
pub fn led(
    bind: Bind,
    id: &'static str,
    x: f32,
    y: f32,
    color: LedColor,
    text: &'static str,
    name: &'static str,
) -> Control {
    Control {
        id,
        x,
        y,
        kind: Kind::Led(LedSpec { color, text: Text::new(text), bind }),
        name: Text::new(name),
        tip: Text::EMPTY,
    }
}

/// A pixmap button at `x`, `y`
pub fn button(
    bind: Bind,
    id: &'static str,
    x: f32,
    y: f32,
    active: &'static Asset,
    inactive: &'static Asset,
    name: &'static str,
) -> Control {
    Control {
        id,
        x,
        y,
        kind: Kind::Button(ButtonSpec { active, inactive, bind, file_field: None }),
        name: Text::new(name),
        tip: Text::EMPTY,
    }
}

impl Control {
    fn knob_mut(&mut self) -> Option<&mut KnobSpec> {
        match &mut self.kind {
            Kind::Knob(k) => Some(k),
            _ => None,
        }
    }

    /// `setLabel`
    #[must_use]
    pub fn label(mut self, l: &'static str) -> Self {
        if let Some(k) = self.knob_mut() {
            k.label = Some(Text::new(l));
        }
        self
    }

    /// A `VolumeKnob`
    #[must_use]
    pub fn volume(mut self) -> Self {
        if let Some(k) = self.knob_mut() {
            k.volume = true;
        }
        self
    }

    /// A `VolumeKnob` whose 0 dBFS is at `zero` (`setZeroDbfsPoint`)
    #[must_use]
    pub fn volume_at(mut self, zero: f64) -> Self {
        if let Some(k) = self.knob_mut() {
            k.volume = true;
            k.zero_dbfs = zero;
        }
        self
    }

    /// A knob over a choice field (see [`KnobSpec::choices`])
    #[must_use]
    pub fn choices(mut self, keys: &'static [&'static str]) -> Self {
        if let Some(k) = self.knob_mut() {
            k.choices = Some(keys);
        }
        self
    }

    /// A unit worked out from the value
    #[must_use]
    pub fn unit_of(mut self, f: fn(f64) -> String) -> Self {
        if let Some(k) = self.knob_mut() {
            k.unit_of = Some(f);
        }
        self
    }

    /// `setCenterValue`
    #[must_use]
    pub fn center(mut self, v: f64) -> Self {
        if let Some(k) = self.knob_mut() {
            k.center = Some(v);
        }
        self
    }

    /// `setScaleLogarithmic(true)` on the legacy model
    #[must_use]
    pub fn log(mut self) -> Self {
        if let Some(k) = self.knob_mut() {
            k.log = true;
        }
        self
    }

    /// `setToolTip`
    #[must_use]
    pub fn tip(mut self, t: &'static str) -> Self {
        self.tip = Text::new(t);
        self
    }

    /// The number of the `%1` in the name, hint and tooltip
    #[must_use]
    pub fn n(mut self, n: u32) -> Self {
        self.name.arg = Some(n);
        self.tip.arg = Some(n);
        if let Some(k) = self.knob_mut() {
            k.hint.arg = Some(n);
        }
        self
    }
}

/// A combo box at `x`, `y`
pub fn combo(field: &'static str, x: f32, y: f32, w: f32, name: &'static str, items: Vec<ComboItem>) -> Control {
    Control {
        id: field,
        x,
        y,
        kind: Kind::Combo(ComboSpec { field, w, items }),
        name: Text::new(name),
        tip: Text::EMPTY,
    }
}

/// A fixed text (`QLabel`) at its top-left corner, white, `DEFAULT_FONT_SIZE`
pub fn label(x: f32, y: f32, text: &'static str) -> StaticText {
    StaticText { x, y, px: crate::widgets::theme::DEFAULT_FONT_PX, text: Text::new(text), color: 0xd1d8e4 }
}

/// A graph at `x`, `y`
pub fn graph(spec: GraphSpec, x: f32, y: f32, name: &'static str) -> Control {
    Control { id: spec.field, x, y, kind: Kind::Graph(spec), name: Text::new(name), tip: Text::EMPTY }
}

/// An integer LCD spin box (`LcdSpinBox`) of `digits` digits on a stepped parameter
pub fn lcd(param: &'static str, x: f32, y: f32, sheet: LcdSheet, digits: usize, name: &'static str) -> Control {
    Control {
        id: param,
        x,
        y,
        kind: Kind::Lcd(LcdSpec { sheet, digits, param, label: None, leading_zero: false, frac: 0, step: 1.0 }),
        name: Text::new(name),
        tip: Text::EMPTY,
    }
}

/// A float LCD spin box (`LcdFloatSpinBox`) of `whole` + `frac` digits; `step` the model's step
#[allow(clippy::too_many_arguments)]
pub fn lcd_float(
    param: &'static str,
    x: f32,
    y: f32,
    sheet: LcdSheet,
    whole: usize,
    frac: usize,
    step: f64,
    name: &'static str,
) -> Control {
    Control {
        id: param,
        x,
        y,
        kind: Kind::Lcd(LcdSpec { sheet, digits: whole, param, label: None, leading_zero: false, frac, step }),
        name: Text::new(name),
        tip: Text::EMPTY,
    }
}
