//! A panel as data: the legacy view's size, its background and every control with its position, type and the
//! parameter or field it moves. Each `panels/<plugin>.rs` writes one, read off the legacy `*View.cpp` /
//! `*ControlDialog.cpp` (docs/plugins-visual.md lists where each number comes from).

use lm_port::{PeakBlock, Value};

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
    /// The panel's selected view (0 without views)
    pub view: u8,
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
    /// One button of a group on a stepped parameter: on when the parameter is `value`; a click sets it
    Step { param: &'static str, value: f64 },
    /// A switch on a boolean field (not automatable in the legacy either)
    Flag { field: &'static str },
    /// A push button on a stepped 0–1 parameter that is 1 while it is held (the legacy automatable button that is not
    /// checkable: the frequency shifter's resets)
    Momentary { param: &'static str },
    /// A push button without a model that changes several things at once (Organic's "Randomise")
    Press(PressFn),
    /// A selector of the panel's own view (a tab, the selected graph or string): panel-local state, not saved in the
    /// plugin; the controls with `Control::view` of this number are the ones shown
    View(u8),
    /// A push button that asks for a file and puts it in a resource field: an audio file (`action` absent) goes
    /// through the port's import; any other kind of file is sent as its bytes to the core's `action`
    /// (`{node, field, bytes}`, one undo step there), which reads it and sets the field
    Open { field: &'static str, action: Option<&'static str>, extensions: &'static [&'static str] },
    /// A push button that runs a core action of the plugin with `{node}` (SlicerT's "find slices")
    Action { action: &'static str },
}

impl Bind {
    pub fn target(&self) -> Target {
        match *self {
            Bind::Choice { field, .. } | Bind::Flag { field } => Target::Field(field),
            Bind::Switch { param } | Bind::Momentary { param } | Bind::Step { param, .. } => Target::Param(param),
            Bind::Press(_) | Bind::View(_) | Bind::Action { .. } => Target::Field(""),
            Bind::Open { field, .. } => Target::Field(field),
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
    /// A knob that turns an int field (`param` is then the field; BitInvader's sample length): the range is the
    /// field's
    pub int_field: bool,
    /// The legacy model's `setScaleLogarithmic(true)` (the knob's travel; the schema's `Scale::Log` is only how the
    /// generic editor shows it)
    pub log: bool,
    /// A `TempoSyncKnob`: the choice field of its tempo sync (`off` or a note length); the context menu offers the
    /// notes and moving the knob turns the sync off
    pub sync: Option<&'static str>,
    /// A switch parameter turned on when this knob moves (the Eq's band: moving its knob or fader turns it on)
    pub wakes: Option<&'static str>,
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
    /// Drawn as Guitarix's selector (a dark frame, the value in orange) instead of the LMMS combo box
    pub gx: bool,
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

/// A fader (`Fader` of the legacy, with its 23 px wide meters and its knob): on a parameter in dB (`linear` false) or
/// on a linear amplification (`linear` true, 0 is -inf and 1 is 0 dBFS)
#[derive(Clone, Debug)]
pub struct FaderSpec {
    pub param: &'static str,
    pub w: f32,
    pub h: f32,
    /// The model is an amplification, not dB (`modelIsLinear`)
    pub linear: bool,
    /// The two meters' peaks in the plugin's display frame (left, right index), absent: no meters
    pub meters: Option<(usize, usize)>,
    /// The unity line on the meters (`setRenderUnityLine`)
    pub unity_line: bool,
    /// The words of the float (`setHintText`) and its unit
    pub hint: Text,
    pub unit: &'static str,
    /// A switch parameter turned on when the fader moves
    pub wakes: Option<&'static str>,
}

/// A draggable handle on a vertical track (the legacy `Draggable`): a pixmap that sits where its model's value is
/// between the track's end points (`a` for the minimum, `b` for the maximum, panel y of the handle's center), and
/// moves the model as the mouse is dragged
#[derive(Clone, Debug)]
pub struct SliderSpec {
    pub param: &'static str,
    pub step: f64,
    pub a: f32,
    pub b: f32,
    pub handle: &'static Asset,
    /// The pixmap shown while the value is exactly 0 (`setDefaultValPixmap`)
    pub zero: Option<&'static Asset>,
    pub hint: Text,
    pub unit: &'static str,
}

/// A button the panel draws itself (a style-sheet button, a lamp, a tab): `draw(canvas, on)` paints its state
#[derive(Clone, Debug)]
pub struct ToggleSpec {
    pub bind: Bind,
    pub w: f32,
    pub h: f32,
    pub draw: fn(&mut crate::raster::Canvas, bool),
}

/// A text of a surface, painted by the panel (the CPU painter has no text): at its baseline, or centered
#[derive(Clone, Debug)]
pub struct SurfText {
    pub x: f32,
    pub y: f32,
    pub px: f32,
    pub text: String,
    pub color: u32,
    pub centered: bool,
}

/// What a surface sees of the plugin when it draws or is dragged
pub struct SurfaceCx<'a> {
    pub w: f32,
    pub h: f32,
    pub param: &'a dyn Fn(&str) -> f64,
    pub field: &'a dyn Fn(&str) -> Value,
    /// The plugin's live frame (`CorePort::display(node)`), empty when it has none
    pub frame: &'a [f32],
    /// The handle held or under the cursor
    pub active: Option<usize>,
    /// The parameter's range (min, max)
    pub range: &'a dyn Fn(&str) -> (f64, f64),
    /// The peaks of the resource a `PeaksSpec` names, for the window below (`None` while they load or without one)
    pub peaks: Option<&'a PeakBlock>,
    /// The part of the resource shown, as fractions of its length (a wave view's zoom); (0, 1) without a `PeaksSpec`
    pub window: (f64, f64),
    /// The project's sample rate, for the lengths in seconds
    pub rate: f64,
    /// The surface's primary parameter and its own number (`SurfaceSpec::tag`): one draw function serves several
    /// surfaces of a panel (Guitarix's meters, sliders and value displays)
    pub primary: &'static str,
    pub tag: u32,
    /// The panel-local zoom of a surface whose tag has [`SURFACE_ZOOM`] (the wheel changes it)
    pub zoom: f32,
    /// What the live frame was in the last seconds, newest last: (age in ms, the frame); only for a live surface
    pub history: &'a [(f32, Vec<f32>)],
}

/// A bit of [`SurfaceSpec::tag`]: the wheel over the surface changes a panel-local zoom value (`SurfaceCx::zoom`, not
/// saved in the plugin: the compressor's dB range). The tag's bits 0 to 7 are its first value, 8 to 15 the step and
/// 16 to 23 the largest; the smallest is the step. Wheel up lowers it
pub const SURFACE_ZOOM: u32 = 1 << 31;

/// The first value, the step and the largest of a zoom tag
pub fn zoom_of(tag: u32) -> (f32, f32, f32) {
    ((tag & 0xff) as f32, ((tag >> 8) & 0xff) as f32, ((tag >> 16) & 0xff) as f32)
}

impl std::fmt::Debug for SurfaceCx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SurfaceCx")
    }
}

impl SurfaceCx<'_> {
    /// The frame's `i`-th value, or `default` when there is no frame or it is not a number
    pub fn live(&self, i: usize, default: f32) -> f32 {
        self.frame.get(i).copied().filter(|v| !v.is_nan()).unwrap_or(default)
    }

    /// The parameter's position in its range, 0..1
    pub fn unit(&self, key: &str) -> f64 {
        let (lo, hi) = (self.range)(key);
        if hi > lo { (((self.param)(key) - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 }
    }
}

/// A handle of a surface, at a local point
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceHandle {
    pub x: f32,
    pub y: f32,
}

pub type SurfDrawFn = fn(&mut crate::raster::Canvas, &SurfaceCx<'_>);
pub type SurfTextsFn = fn(&SurfaceCx<'_>) -> Vec<SurfText>;
pub type SurfHandlesFn = fn(&SurfaceCx<'_>) -> Vec<SurfaceHandle>;
/// The parameters a handle moves to when dragged to a local point
pub type SurfDragFn = fn(&SurfaceCx<'_>, usize, f32, f32) -> Vec<(&'static str, f64)>;
/// The parameters the wheel over a handle moves to: the handle, the notches (up positive) and whether Ctrl is held
pub type SurfWheelFn = fn(&SurfaceCx<'_>, usize, f64, bool) -> Vec<(&'static str, f64)>;

/// A drawn area fed by the plugin's parameters, fields and live frame, with handles the user drags: the XY pad, a
/// transfer curve with its handle, the equalizer's curve, the wave view with its markers, the meters. Everything
/// it moves is a parameter (so mappable); it draws through the CPU painter and is cached by what it reads
#[derive(Clone, Debug)]
pub struct SurfaceSpec {
    pub w: f32,
    pub h: f32,
    /// The parameter that names the control (its menu and its MIDI target): the first the handles move
    pub primary: &'static str,
    /// The parameters and fields the drawing reads (the cache key; the surface is drawn again when they change)
    pub reads: &'static [&'static str],
    pub field_reads: &'static [&'static str],
    pub draw: SurfDrawFn,
    pub texts: Option<SurfTextsFn>,
    pub handles: Option<SurfHandlesFn>,
    pub drag: Option<SurfDragFn>,
    /// How near a click must be to a handle to take it (pixels); infinite: anywhere takes the nearest
    pub grab: f32,
    /// The press itself moves the handle (else only the moves that follow do, as the legacy `XyPad`)
    pub set_on_press: bool,
    /// Drawn again every frame (it shows the live frame)
    pub live: bool,
    /// The wheel moves the primary parameter (a slider handle), as a knob's
    pub wheel: bool,
    /// The wheel over a handle (the equalizer's bandwidth)
    pub wheel_edit: Option<SurfWheelFn>,
    /// The surface shows a resource's waveform (the panel asks the port for its peaks and keeps the window; the wheel
    /// zooms it as the legacy `AudioFileProcessorWaveView` does)
    pub peaks: Option<PeaksSpec>,
    /// A number the panel gives the surface for its draw function (`SurfaceCx::tag`)
    pub tag: u32,
    /// Markers the user places on the wave view (SlicerT's slice points)
    pub markers: Option<MarkersSpec>,
}

/// A list field of points, fractions of the shown resource from 0 to 1 and kept sorted: a click on the wave view
/// adds one, a drag moves one (the neighbours bound it) and a click on one with Shift, or the right button, removes
/// it; every change is one undo step (a drag is one gesture)
#[derive(Clone, Copy, Debug)]
pub struct MarkersSpec {
    pub field: &'static str,
    /// How near a click must be to a point to take it (pixels)
    pub grab: f32,
    /// The wave view's left edge and width in the surface (the graph's padding), for the pixel of a fraction
    pub x0: f32,
    pub width: f32,
    /// The first and last points are the ends of the resource (0 and 1): they are not moved and not removed
    pub fixed_ends: bool,
}

/// The resource field a wave view shows and the two parameters that bound the part that plays (fractions)
#[derive(Clone, Copy, Debug)]
pub struct PeaksSpec {
    pub field: &'static str,
    pub start: &'static str,
    pub end: &'static str,
    /// The wave's width in pixels (the pairs asked for)
    pub width: f32,
}

/// A text bound to a field of the state (a file's name, a patch's name): drawn by the panel
#[derive(Clone, Debug)]
pub struct ReadoutSpec {
    pub field: &'static str,
    pub w: f32,
    pub px: f32,
    pub color: u32,
    /// What it shows when the field is empty
    pub empty: Text,
    /// Only the file name of a resource's key or a path
    pub centered: bool,
}

/// A multi-line text the user types (the Xpressive expression editor): it edits the field of the panel's selected
/// view (`fields[view]`, the last one for a view past the end); the text is the field's, one edit for each key
#[derive(Clone, Debug)]
pub struct TextEditSpec {
    pub fields: &'static [&'static str],
    pub w: f32,
    pub h: f32,
    pub px: f32,
    pub color: u32,
}

/// A list of the entries the core lists for the plugin (the Sampler's presets of a bank file): the core's `action`
/// puts them into the status tree under `pluginInfo.<node>` (`{key, presets: [{bank, program, name}]}`); a click on
/// a row sets the `bank` and `program` fields in one step, and the wheel scrolls
#[derive(Clone, Debug)]
pub struct ListSpec {
    pub w: f32,
    pub h: f32,
    /// A row's height
    pub row: f32,
    pub bank: &'static str,
    pub program: &'static str,
    /// The core action that makes the listing (`{node}`), asked again when the file changes
    pub action: &'static str,
    /// The choice field that says which file field the listing reads
    pub format_field: &'static str,
    /// (the choice's key, the resource field of its file)
    pub files: &'static [(&'static str, &'static str)],
}

/// A bit of [`SurfaceSpec::tag`]: the surface shows what the live frame's counters did lately (a hit's light that fades):
/// its history keeps only the frames that changed, over the last 700 ms, and it is drawn again every 33 ms only while
/// one is that recent
pub const SURFACE_FADE: u32 = 1 << 30;

#[derive(Clone, Debug)]
pub enum Kind {
    List(ListSpec),
    TextEdit(TextEditSpec),
    Knob(KnobSpec),
    Led(LedSpec),
    Button(ButtonSpec),
    Lcd(LcdSpec),
    Combo(ComboSpec),
    Graph(GraphSpec),
    Fader(FaderSpec),
    Slider(SliderSpec),
    Toggle(ToggleSpec),
    Surface(SurfaceSpec),
    Readout(ReadoutSpec),
}

/// A fixed text of the view (a `QLabel`): its top-left corner, its size in pixels
#[derive(Clone, Debug)]
pub struct StaticText {
    pub x: f32,
    pub y: f32,
    pub px: f32,
    pub text: Text,
    pub color: u32,
    /// Only in this view of the panel (absent: always)
    pub view: Option<u8>,
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
    /// Only in this view of the panel (absent: always)
    pub view: Option<u8>,
    /// Only while this switch parameter is on (`true`) or off (`false`): a control the legacy view swaps for another
    /// when a model changes (SlewDistortion's linked slew knob, LOMM's feedback)
    pub when: Option<(&'static str, bool)>,
    /// Hidden while this choice field has this key (AudioFileProcessor's loop point, shown only while it loops)
    pub unless: Option<(&'static str, &'static str)>,
    /// Shown only while this choice field has this key (the compressor's blend knob, with the blend link mode)
    pub only: Option<(&'static str, &'static str)>,
}

impl Control {
    pub fn target(&self) -> Target {
        match &self.kind {
            Kind::Knob(k) if k.choices.is_some() || k.int_field => Target::Field(k.param),
            Kind::Knob(k) => Target::Param(k.param),
            Kind::Lcd(l) => Target::Param(l.param),
            Kind::Led(l) => l.bind.target(),
            Kind::Button(b) => b.bind.target(),
            Kind::Combo(c) => Target::Field(c.field),
            Kind::Graph(g) => Target::Field(g.field),
            Kind::Fader(f) => Target::Param(f.param),
            Kind::Slider(s) => Target::Param(s.param),
            Kind::Toggle(t) => t.bind.target(),
            Kind::Surface(s) => Target::Param(s.primary),
            Kind::Readout(r) => Target::Field(r.field),
            Kind::List(l) => Target::Field(l.bank),
            Kind::TextEdit(t) => Target::Field(t.fields[0]),
        }
    }

    /// A surface that only draws (no primary parameter): the mouse passes through it
    pub fn is_decoration(&self) -> bool {
        matches!(&self.kind, Kind::Surface(s) if s.primary.is_empty())
    }

    /// Whether it shows in `view`
    pub fn in_view(&self, view: u8) -> bool {
        self.view.is_none_or(|v| v == view)
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
            Kind::Fader(f) => (f.w, f.h),
            Kind::Slider(s) => {
                let img = crate::assets::image(s.handle);
                (img.width() as f32, (s.b - s.a).abs() + img.height() as f32)
            }
            Kind::Toggle(t) => (t.w, t.h),
            Kind::Surface(s) => (s.w, s.h),
            Kind::Readout(r) => (r.w, r.px + 4.0),
            Kind::List(l) => (l.w, l.h),
            Kind::TextEdit(t) => (t.w, t.h),
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
    /// The artwork of each view, by number, over `background` (Monstro's operators and matrix); a view past the end
    /// keeps `background`
    pub view_backgrounds: &'static [&'static Asset],
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
    /// An action of the core that reads the file into a field (`{node, field, bytes, name}`: the generic
    /// `plugin.load_into_field`)
    Action { action: &'static str, field: &'static str, extensions: &'static [&'static str] },
}

impl PanelSpec {
    /// The control at panel point `x`, `y` (the last one drawn wins, as Qt's stacking), in view 0
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.hit_if(x, y, |c| c.in_view(0))
    }

    /// The control at panel point `x`, `y` among those `shown` says are
    pub fn hit_if(&self, x: f32, y: f32, shown: impl Fn(&Control) -> bool) -> Option<usize> {
        self.controls.iter().enumerate().rev().find_map(|(i, c)| {
            let (w, h) = c.size();
            (shown(c) && !matches!(c.kind, Kind::Readout(_)) && x >= c.x && y >= c.y && x < c.x + w && y < c.y + h)
                .then_some(i)
        })
    }

    /// The artwork under the controls of `view`
    pub fn background_of(&self, view: u8) -> Option<&'static Asset> {
        self.view_backgrounds.get(view as usize).copied().or(self.background)
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
            int_field: false,
            log: false,
            sync: None,
            wakes: None,
        }),
        name: Text::new(name),
        tip: Text::EMPTY,
        view: None,
        when: None,
        unless: None,
        only: None,
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
        view: None,
        when: None,
        unless: None,
        only: None,
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
        view: None,
        when: None,
        unless: None,
        only: None,
    }
}

impl Control {
    /// A combo box drawn as Guitarix's selector
    #[must_use]
    pub fn gx(mut self) -> Self {
        if let Kind::Combo(c) = &mut self.kind {
            c.gx = true;
        }
        self
    }

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

    /// A knob over an int field (see [`KnobSpec::int_field`])
    #[must_use]
    pub fn int_field(mut self) -> Self {
        if let Some(k) = self.knob_mut() {
            k.int_field = true;
        }
        self
    }

    /// A knob or fader that turns the switch parameter `param` on when it moves
    #[must_use]
    pub fn wakes(mut self, param: &'static str) -> Self {
        match &mut self.kind {
            Kind::Knob(k) => k.wakes = Some(param),
            Kind::Fader(f) => f.wakes = Some(param),
            _ => {}
        }
        self
    }

    /// A `TempoSyncKnob` whose sync is the choice field `field`
    #[must_use]
    pub fn synced(mut self, field: &'static str) -> Self {
        if let Some(k) = self.knob_mut() {
            k.sync = Some(field);
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

    /// Hidden while the choice field is `value`
    #[must_use]
    pub fn hidden_while(mut self, field: &'static str, value: &'static str) -> Self {
        self.unless = Some((field, value));
        self
    }

    /// Shown only while the choice field is `value`
    #[must_use]
    pub fn only_while(mut self, field: &'static str, value: &'static str) -> Self {
        self.only = Some((field, value));
        self
    }

    /// A readout in the small font
    #[must_use]
    pub fn small(mut self) -> Self {
        if let Kind::Readout(r) = &mut self.kind {
            r.px = crate::widgets::theme::SMALL_FONT_PX;
        }
        self
    }

    /// Shown only while the switch parameter is `on`
    #[must_use]
    pub fn when(mut self, param: &'static str, on: bool) -> Self {
        self.when = Some((param, on));
        self
    }

    /// Shown only in this view of the panel
    #[must_use]
    pub fn in_view_n(mut self, v: u8) -> Self {
        self.view = Some(v);
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
        kind: Kind::Combo(ComboSpec { field, w, items, gx: false }),
        name: Text::new(name),
        tip: Text::EMPTY,
        view: None,
        when: None,
        unless: None,
        only: None,
    }
}

/// A fixed text (`QLabel`) at its top-left corner, white, `DEFAULT_FONT_SIZE`
pub fn label(x: f32, y: f32, text: &'static str) -> StaticText {
    StaticText { x, y, px: crate::widgets::theme::DEFAULT_FONT_PX, text: Text::new(text), color: 0xd1d8e4, view: None }
}

/// A graph at `x`, `y`
pub fn graph(spec: GraphSpec, x: f32, y: f32, name: &'static str) -> Control {
    Control {
        id: spec.field,
        x,
        y,
        kind: Kind::Graph(spec),
        name: Text::new(name),
        tip: Text::EMPTY,
        view: None,
        when: None,
        unless: None,
        only: None,
    }
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
        view: None,
        when: None,
        unless: None,
        only: None,
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
        view: None,
        when: None,
        unless: None,
        only: None,
    }
}

/// A fader at `x`, `y` on `param` (the legacy 23 px wide `Fader`; `h` its height)
pub fn fader(param: &'static str, x: f32, y: f32, h: f32, name: &'static str, hint: &'static str) -> Control {
    Control {
        id: param,
        x,
        y,
        kind: Kind::Fader(FaderSpec {
            param,
            w: 23.0,
            h,
            linear: false,
            meters: None,
            unity_line: false,
            hint: Text::new(hint),
            unit: " dBFS",
            wakes: None,
        }),
        name: Text::new(name),
        tip: Text::EMPTY,
        view: None,
        when: None,
        unless: None,
        only: None,
    }
}

impl Control {
    fn fader_mut(&mut self) -> Option<&mut FaderSpec> {
        match &mut self.kind {
            Kind::Fader(f) => Some(f),
            _ => None,
        }
    }

    /// A fader with the output meters over `frame[l]`, `frame[r]` (`EqFader`)
    #[must_use]
    pub fn meters(mut self, l: usize, r: usize) -> Self {
        if let Some(f) = self.fader_mut() {
            f.meters = Some((l, r));
        }
        self
    }

    /// A fader on a linear amplification
    #[must_use]
    pub fn linear(mut self) -> Self {
        if let Some(f) = self.fader_mut() {
            f.linear = true;
        }
        self
    }

    /// The unit of the fader's float (`setHintText`'s second word)
    #[must_use]
    pub fn fader_unit(mut self, u: &'static str) -> Self {
        if let Some(f) = self.fader_mut() {
            f.unit = u;
        }
        self
    }

    /// The fader's unity line
    #[must_use]
    pub fn unity_line(mut self) -> Self {
        if let Some(f) = self.fader_mut() {
            f.unity_line = true;
        }
        self
    }
}

/// A surface at `x`, `y`
pub fn surface(spec: SurfaceSpec, id: &'static str, x: f32, y: f32, name: &'static str) -> Control {
    Control {
        id,
        x,
        y,
        kind: Kind::Surface(spec),
        name: Text::new(name),
        tip: Text::EMPTY,
        view: None,
        when: None,
        unless: None,
        only: None,
    }
}

/// A text editor over the fields of the views at `x`, `y`
pub fn text_edit(
    fields: &'static [&'static str],
    id: &'static str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    name: &'static str,
) -> Control {
    Control {
        id,
        x,
        y,
        kind: Kind::TextEdit(TextEditSpec { fields, w, h, px: crate::widgets::theme::SMALL_FONT_PX, color: 0xffffff }),
        name: Text::new(name),
        tip: Text::EMPTY,
        view: None,
        when: None,
        unless: None,
        only: None,
    }
}

/// A list of the plugin's presets at `x`, `y`
pub fn list(spec: ListSpec, id: &'static str, x: f32, y: f32, name: &'static str) -> Control {
    Control {
        id,
        x,
        y,
        kind: Kind::List(spec),
        name: Text::new(name),
        tip: Text::EMPTY,
        view: None,
        when: None,
        unless: None,
        only: None,
    }
}

/// A text bound to a field
pub fn readout(field: &'static str, x: f32, y: f32, w: f32, color: u32, empty: &'static str) -> Control {
    Control {
        id: field,
        x,
        y,
        kind: Kind::Readout(ReadoutSpec {
            field,
            w,
            px: crate::widgets::theme::DEFAULT_FONT_PX,
            color,
            empty: Text::new(empty),
            centered: false,
        }),
        name: Text::new(field),
        tip: Text::EMPTY,
        view: None,
        when: None,
        unless: None,
        only: None,
    }
}

/// A button drawn by `draw` at `x`, `y`, `w` by `h`
#[allow(clippy::too_many_arguments)]
pub fn toggle(
    bind: Bind,
    id: &'static str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    draw: fn(&mut crate::raster::Canvas, bool),
    name: &'static str,
) -> Control {
    Control {
        id,
        x,
        y,
        kind: Kind::Toggle(ToggleSpec { bind, w, h, draw }),
        name: Text::new(name),
        tip: Text::EMPTY,
        view: None,
        when: None,
        unless: None,
        only: None,
    }
}

/// A slider handle at `x` on a vertical track from `a` (the minimum) to `b` (the maximum)
#[allow(clippy::too_many_arguments)]
pub fn slider(
    param: &'static str,
    x: f32,
    a: f32,
    b: f32,
    handle: &'static Asset,
    zero: Option<&'static Asset>,
    name: &'static str,
    hint: &'static str,
) -> Control {
    let ph = crate::assets::image(handle).height() as f32;
    Control {
        id: param,
        x,
        y: (a.min(b) - ph / 2.0).floor(),
        kind: Kind::Slider(SliderSpec { param, step: 0.0001, a, b, handle, zero, hint: Text::new(hint), unit: "" }),
        name: Text::new(name),
        tip: Text::EMPTY,
        view: None,
        when: None,
        unless: None,
        only: None,
    }
}
