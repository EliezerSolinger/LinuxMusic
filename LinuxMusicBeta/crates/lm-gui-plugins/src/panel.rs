//! The panel view: one GPUI view per open plugin window, drawing a `PanelSpec` faithfully and moving the plugin's
//! state through the `CorePort` only (I-22): reading the plugin node in the port's tree, changing parameters by
//! gestures and `param_write`, choices and switches by edits, and nothing else. It works the same in the client.
//!
//! Drawing: the background is one image (GPUI draws the PNG or the SVG at the screen's scale); each control is one
//! image rasterized for its state at the screen's scale and cached (`raster`, `widgets`); the texts (labels, the
//! value float, the menu, the value entry) are text. A panel of 30 controls is ~35 primitives a frame.
//!
//! Gestures (F-CTL-005, F-CTL-006, from `FloatModelEditorBase` and `AutomatableButton`):
//! - knob and LCD: drag vertically (0.4 % of the range a pixel; Shift a quarter), one drag = one gesture = one undo
//!   step, the value reaching the audio at once by `param_write`; the wheel (100 notches the whole range, Shift 10,
//!   Ctrl 1000, Alt 2000); double click types a value (a volume knob in dBFS); the middle button resets;
//!   Shift+drag carries the value to another control of the panel; the value float while dragging and after
//!   700 ms over a control, for 2 s;
//! - button and LED: a click selects (a group's button) or toggles; the middle button resets;
//! - every control: the right button opens its menu: reset, copy and paste the value, MIDI learn and forget
//!   (I-21.2), add or remove its automation lane (I-21.5) and, on knobs, linear or logarithmic travel (P-06).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    App, Bounds, Context, DispatchPhase, FocusHandle, Focusable, FontWeight, Hitbox, HitboxBehavior, IntoElement,
    KeyDownEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render, RenderImage,
    ScrollDelta, ScrollEvent, SharedString, Subscription, Task, TextAlign, TextRun, WeakEntity, Window, canvas, div,
    img, point, prelude::*, px, rgb, size,
};
use lm_port::{
    CorePort, EditOp, GestureId, Label, MapScope, MapTarget, Mapping, NodeId, NodeRef, NodeSchema, PathStep, Pending,
    Ref, Value,
};

use crate::raster::Canvas;
use crate::spec::{Bind, Kind, PanelSpec, Target, Text};
use crate::widgets::{self, Range, theme};
use crate::{PortHost, assets};

/// The value copied by "Copy value", for "Paste value" in any panel (the legacy keeps one for all controls)
static CLIPBOARD: Mutex<Option<f64>> = Mutex::new(None);

/// The hover delay and the display times of the value float (`FloatModelEditorBase::enterEvent`, `wheelEvent`)
pub const HOVER_DELAY: Duration = Duration::from_millis(700);
pub const HOVER_SHOW: Duration = Duration::from_millis(2000);
pub const WHEEL_SHOW: Duration = Duration::from_millis(1000);

/// Pixels of a trackpad's scroll that make one wheel notch (Qt's 120 of `angleDelta`)
const PIXELS_PER_NOTCH: f32 = 20.0;

/// A graph being drawn: one gesture, the samples as drawn so far
struct GraphDraw {
    ctrl: usize,
    gesture: GestureId,
    samples: Vec<f64>,
    last_x: f32,
    cursor: (f32, f32),
}

struct Drag {
    ctrl: usize,
    gesture: GestureId,
    last_y: f32,
    left_over: f64,
    value: f64,
}

/// What the value float shows and for whom
#[derive(Clone, Debug, PartialEq)]
struct Float {
    ctrl: usize,
    text: String,
}

/// An open context menu, at a panel point
struct Menu {
    ctrl: usize,
    x: f32,
    y: f32,
    /// a combo box's list of choices, not the control's menu
    choices: bool,
}

/// The value entry (`enterValue`) being typed
struct Entry {
    ctrl: usize,
    text: String,
}

/// What a panel asks of the window that hosts it
#[derive(Clone, Debug, PartialEq)]
pub enum PanelEvent {
    /// "MIDI binding…": open the interface's binding window on this target (lm-gui's `ui::midi::open_binding`)
    OpenBinding(MapTarget),
}

/// One entry of the context menu
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Reset,
    Copy,
    Paste,
    MidiLearn,
    MidiForget,
    MidiBinding,
    AddLane,
    RemoveLane,
    ToggleLog,
}

/// The panel of one plugin node
pub struct PluginPanel<H: PortHost> {
    host: gpui::Entity<H>,
    node: NodeId,
    spec: &'static PanelSpec,
    schema: Option<NodeSchema>,
    focus: FocusHandle,
    drag: Option<Drag>,
    graph: Option<GraphDraw>,
    /// The last x drawn on each graph (`m_lastCursorX`, for Shift+click lines)
    graph_last_x: HashMap<usize, f32>,
    /// A non-checkable button held down shows its active image
    pressed: Option<usize>,
    /// Shift+drag: the value carried, from this control
    carried: Option<(usize, f64)>,
    float: Option<Float>,
    hovered: Option<usize>,
    menu: Option<Menu>,
    entry: Option<Entry>,
    /// P-06: the controls the user switched between linear and logarithmic travel (a display option)
    toggled_log: HashSet<Target>,
    images: HashMap<(usize, u64), Arc<RenderImage>>,
    scroll_acc: f32,
    /// Files being imported for a file field: the request and the control
    imports: Vec<(lm_port::RequestId, usize)>,
    timer: Option<Task<()>>,
    _observe: Subscription,
}

impl<H: PortHost> PluginPanel<H> {
    pub fn new(host: gpui::Entity<H>, node: NodeId, spec: &'static PanelSpec, cx: &mut Context<Self>) -> Self {
        let schema = host.read(cx).port().plugin_schema(spec.plugin);
        // the port's pump notifies the host when something shown changed (an undo, automation, another client)
        let observe = cx.observe(&host, |p: &mut Self, _, cx| {
            p.collect_imports(cx);
            cx.notify();
        });
        PluginPanel {
            host,
            node,
            spec,
            schema,
            focus: cx.focus_handle(),
            drag: None,
            graph: None,
            graph_last_x: HashMap::new(),
            pressed: None,
            carried: None,
            float: None,
            hovered: None,
            menu: None,
            entry: None,
            toggled_log: HashSet::new(),
            images: HashMap::new(),
            scroll_acc: 0.0,
            imports: Vec::new(),
            timer: None,
            _observe: observe,
        }
    }

    pub fn node(&self) -> NodeId {
        self.node
    }

    pub fn spec(&self) -> &'static PanelSpec {
        self.spec
    }

    // --- reading the state, through the port's tree

    fn state(&self, cx: &App) -> Option<Value> {
        self.host.read(cx).port().find(self.node).and_then(|n| n.get("state").cloned())
    }

    /// A parameter's value: the one being dragged, the tree's, or the schema's default (the tree omits defaults)
    pub fn param(&self, key: &str, cx: &App) -> f64 {
        if let Some(keys) = self.choice_keys(key) {
            if let Some(d) = &self.drag
                && self.spec.controls[d.ctrl].target().key() == key
            {
                return d.value;
            }
            let v = self.field(key, cx);
            return keys.iter().position(|k| v.as_text() == Some(*k)).unwrap_or(0) as f64;
        }
        if let Some(d) = &self.drag
            && self.spec.controls[d.ctrl].target() == Target::Param(self.spec.controls[d.ctrl].target().key())
            && self.spec.controls[d.ctrl].target().key() == key
        {
            return d.value;
        }
        self.state(cx)
            .and_then(|s| s.get("params").and_then(|p| p.get(key)).and_then(Value::as_real))
            .or_else(|| self.schema.as_ref().and_then(|s| s.find_param(key)).map(|p| p.def))
            .unwrap_or(0.0)
    }

    /// The keys of a knob that turns this choice field, if one does
    fn choice_keys(&self, field: &str) -> Option<&'static [&'static str]> {
        self.spec.controls.iter().find_map(|c| match &c.kind {
            Kind::Knob(k) if k.param == field => k.choices,
            _ => None,
        })
    }

    /// A field's value, or its default
    pub fn field(&self, key: &str, cx: &App) -> Value {
        self.state(cx)
            .and_then(|s| s.get(key).cloned())
            .or_else(|| self.schema.as_ref().and_then(|s| s.find_field(key)).and_then(|f| f.default.clone()))
            .unwrap_or(Value::NULL)
    }

    /// A graph's samples: the one being drawn, the tree's list, or its default
    pub fn samples(&self, ctrl: usize, cx: &App) -> Vec<f64> {
        if let Some(d) = &self.graph
            && d.ctrl == ctrl
        {
            return d.samples.clone();
        }
        let Kind::Graph(g) = &self.spec.controls[ctrl].kind else { return Vec::new() };
        let v = self.field(g.field, cx);
        let mut out: Vec<f64> = v.items().map(|x| x.as_real().unwrap_or(0.0)).collect();
        out.resize(g.length, 0.0);
        out
    }

    fn samples_value(samples: &[f64]) -> Value {
        Value::list(samples.iter().map(|v| Value::real(*v).unwrap_or_default()))
    }

    fn graph_down(&mut self, ctrl: usize, x: f32, y: f32, shift: bool, cx: &mut Context<Self>) {
        let Kind::Graph(g) = &self.spec.controls[ctrl].kind else { return };
        let c = &self.spec.controls[ctrl];
        let (lx, ly) = (x - c.x, y - c.y);
        let mut samples = self.samples(ctrl, cx);
        let last = self.graph_last_x.get(&ctrl).copied().unwrap_or(lx);
        if shift {
            widgets::graph_line(g, &mut samples, lx, ly, last);
        } else {
            widgets::graph_set(g, &mut samples, lx, ly);
        }
        let field = g.field;
        let label = Label::new(c.name.key);
        let op = self.field_op(field, Self::samples_value(&samples));
        let gesture = self.host.update(cx, |h, _| {
            let port = h.port_mut();
            let gid = port.begin_gesture(label);
            port.gesture_apply(&gid, vec![op]);
            gid
        });
        self.graph = Some(GraphDraw { ctrl, gesture, samples, last_x: lx, cursor: (lx, ly) });
        self.graph_last_x.insert(ctrl, lx);
        cx.notify();
    }

    fn graph_move(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        let Some(d) = &mut self.graph else { return };
        let c = &self.spec.controls[d.ctrl];
        let Kind::Graph(g) = &c.kind else { return };
        // `Graph::mouseMoveEvent`: the cursor kept inside the graph's 2-pixel border
        let lx = (x - c.x).clamp(2.0, g.w - 3.0);
        let ly = (y - c.y).clamp(2.0, g.h - 3.0);
        widgets::graph_line(g, &mut d.samples, lx, ly, d.last_x);
        d.last_x = lx;
        d.cursor = (lx, ly);
        let ctrl = d.ctrl;
        let value = Self::samples_value(&d.samples);
        let field = g.field;
        let op = self.field_op(field, value);
        self.graph_last_x.insert(ctrl, lx);
        let host = self.host.clone();
        let Some(d) = &self.graph else { return };
        host.update(cx, |h, _| h.port_mut().gesture_apply(&d.gesture, vec![op]));
        cx.notify();
    }

    fn graph_up(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.graph.take() {
            self.host.update(cx, |h, _| h.port_mut().gesture_commit(d.gesture));
            cx.notify();
        }
    }

    fn range(&self, ctrl: usize) -> Range {
        let c = &self.spec.controls[ctrl];
        let (key, step, center, log) = match &c.kind {
            Kind::Knob(k) => (k.param, k.step, k.center, k.log),
            Kind::Lcd(l) => (l.param, l.step, None, false),
            Kind::Led(_) | Kind::Button(_) | Kind::Combo(_) | Kind::Graph(_) => (c.target().key(), 1.0, None, false),
        };
        let p = self.schema.as_ref().and_then(|s| s.find_param(key));
        let (min, max) = match self.choice_keys(key) {
            Some(keys) => (0.0, keys.len().saturating_sub(1) as f64),
            None => p.map_or((0.0, 1.0), |p| (p.min, p.max)),
        };
        let log = log != self.toggled_log.contains(&c.target());
        Range { min, max, step, center: center.unwrap_or(min), log }
    }

    fn default_of(&self, t: Target) -> Value {
        let s = self.schema.as_ref();
        match t {
            Target::Param(k) => s.and_then(|s| s.find_param(k)).and_then(|p| Value::real(p.def).ok()),
            Target::Field(k) => s.and_then(|s| s.find_field(k)).and_then(|f| f.default.clone()),
        }
        .unwrap_or(Value::NULL)
    }

    fn is_on(&self, bind: Bind, cx: &App) -> bool {
        match bind {
            Bind::Choice { field, key } => self.field(field, cx).as_text() == Some(key),
            Bind::Switch { param } => self.param(param, cx) >= 0.5,
            Bind::Flag { field } => self.field(field, cx).as_bool() == Some(true),
            Bind::Press(_) => false,
        }
    }

    /// The text a control shows in its float and its menu (`currentValueToText` + the hint and unit)
    pub fn value_text(&self, ctrl: usize, cx: &App) -> String {
        let c = &self.spec.controls[ctrl];
        match &c.kind {
            Kind::Knob(k) => {
                let r = self.range(ctrl);
                let v = r.rounded(self.param(k.param, cx));
                if k.volume {
                    widgets::volume_text(v, k.zero_dbfs)
                } else if let Some(f) = k.unit_of {
                    format!("{}{}", widgets::qt_number(v), f(v))
                } else {
                    format!("{}{}", widgets::qt_number(v), unit(k.unit))
                }
            }
            Kind::Lcd(l) => widgets::qt_number(self.param(l.param, cx)),
            Kind::Combo(cb) => {
                let v = self.field(cb.field, cx);
                cb.items.iter().find(|i| v.as_text() == Some(i.key)).map(|i| i.text.show()).unwrap_or_default()
            }
            Kind::Graph(_) => String::new(),
            Kind::Led(_) | Kind::Button(_) => match c.target() {
                Target::Param(k) => widgets::qt_number(self.param(k, cx)),
                Target::Field(k) => match self.field(k, cx) {
                    v if v.as_bool().is_some() => if v.as_bool() == Some(true) { "1" } else { "0" }.into(),
                    v => v.as_text().unwrap_or("").to_owned(),
                },
            },
        }
    }

    fn float_text(&self, ctrl: usize, cx: &App) -> String {
        let c = &self.spec.controls[ctrl];
        match &c.kind {
            Kind::Knob(k) => {
                let hint = k.hint.show();
                format!("{} {}", hint, self.value_text(ctrl, cx)).trim().to_owned()
            }
            _ => c.tip.show(),
        }
    }

    // --- changing the state

    fn param_op(&self, key: &str, v: f64) -> Option<EditOp> {
        let value = Value::real(v).ok()?;
        Some(EditOp::Set {
            id: Ref::Id(self.node),
            path: vec![PathStep::key("state"), PathStep::key("params"), PathStep::key(key)],
            value,
        })
    }

    fn field_op(&self, key: &str, v: Value) -> EditOp {
        EditOp::Set { id: Ref::Id(self.node), path: vec![PathStep::key("state"), PathStep::key(key)], value: v }
    }

    /// One edit as one undo step, labelled with the control's name
    fn set(&mut self, ctrl: usize, value: Value, cx: &mut Context<Self>) {
        let c = &self.spec.controls[ctrl];
        let label = Label::new(c.name.key);
        let node = self.node;
        let (op, live) = match c.target() {
            Target::Param(k) => {
                let Some(v) = value.as_real() else { return };
                let r = self.range(ctrl);
                let v = r.fit(v);
                (self.param_op(k, v), Some((k, v)))
            }
            Target::Field(k) => {
                let value = match (self.choice_keys(k), value.as_real()) {
                    (Some(keys), Some(i)) => Value::text(keys[(i.round().max(0.0) as usize).min(keys.len() - 1)]),
                    _ => value,
                };
                (Some(self.field_op(k, value)), None)
            }
        };
        let Some(op) = op else { return };
        self.host.update(cx, |h, _| {
            let port = h.port_mut();
            if let Some((k, v)) = live {
                port.param_write(node, k, v as f32);
            }
            // a refusal (the node is gone, another client removed it) changes nothing; the view follows the tree
            let _ = port.apply(label, vec![op]);
        });
        cx.notify();
    }

    fn begin_drag(&mut self, ctrl: usize, y: f32, cx: &mut Context<Self>) {
        let key = self.spec.controls[ctrl].target().key();
        let value = self.param(key, cx);
        let label = Label::new(self.spec.controls[ctrl].name.key);
        let gesture = self.host.update(cx, |h, _| h.port_mut().begin_gesture(label));
        self.drag = Some(Drag { ctrl, gesture, last_y: y, left_over: 0.0, value });
        self.show_float(ctrl, None, cx);
    }

    fn drag_to(&mut self, y: f32, slow: bool, cx: &mut Context<Self>) {
        let Some(d) = &mut self.drag else { return };
        let dy = y - d.last_y;
        if dy == 0.0 {
            return;
        }
        d.last_y = y;
        let ctrl = d.ctrl;
        let r = self.range(ctrl);
        let Some(d) = &mut self.drag else { return };
        let v = r.drag(d.value, dy, slow, &mut d.left_over);
        if v == d.value {
            return;
        }
        d.value = v;
        let (op, live) = match self.spec.controls[ctrl].target() {
            Target::Param(key) => (self.param_op(key, v), Some(key)),
            Target::Field(key) => match self.choice_keys(key) {
                Some(keys) => (Some(self.field_op(key, Value::text(keys[(v as usize).min(keys.len() - 1)]))), None),
                None => (None, None),
            },
        };
        let Some(op) = op else { return };
        let node = self.node;
        let host = self.host.clone();
        let Some(d) = &self.drag else { return };
        host.update(cx, |h, _| {
            let port = h.port_mut();
            if let Some(key) = live {
                port.param_write(node, key, v as f32);
            }
            port.gesture_apply(&d.gesture, vec![op]);
        });
        self.show_float(ctrl, None, cx);
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.drag.take() {
            self.host.update(cx, |h, _| h.port_mut().gesture_commit(d.gesture));
            self.float = None;
            cx.notify();
        }
    }

    // --- the value float

    fn show_float(&mut self, ctrl: usize, hide_after: Option<Duration>, cx: &mut Context<Self>) {
        let text = self.float_text(ctrl, cx);
        self.float = (!text.is_empty()).then_some(Float { ctrl, text });
        self.timer = hide_after.map(|d| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(d).await;
                let _ = this.update(cx, |p, cx| {
                    if p.drag.is_none() {
                        p.float = None;
                        cx.notify();
                    }
                });
            })
        });
        cx.notify();
    }

    fn hover(&mut self, ctrl: Option<usize>, cx: &mut Context<Self>) {
        if self.hovered == ctrl || self.drag.is_some() {
            return;
        }
        self.hovered = ctrl;
        if self.float.as_ref().is_some_and(|f| Some(f.ctrl) != ctrl) {
            self.float = None;
            cx.notify();
        }
        self.timer = ctrl.map(|c| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(HOVER_DELAY).await;
                let _ = this.update(cx, |p, cx| {
                    if p.hovered == Some(c) && p.drag.is_none() && p.menu.is_none() {
                        p.show_float(c, Some(HOVER_SHOW), cx);
                    }
                });
            })
        });
    }

    // --- mouse

    fn mouse_down(&mut self, x: f32, y: f32, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.entry.is_some() || self.menu.is_some() {
            return;
        }
        let Some(ctrl) = self.spec.hit(x, y) else { return };
        let c = &self.spec.controls[ctrl];
        let m = ev.modifiers;
        if let Kind::Graph(_) = &c.kind {
            // a graph has no model menu in the legacy: only the left button draws
            if ev.button == MouseButton::Left {
                self.graph_down(ctrl, x, y, m.shift, cx);
            }
            return;
        }
        let press = matches!(&c.kind, Kind::Button(b) if matches!(b.bind, Bind::Press(_)));
        if press && ev.button != MouseButton::Left {
            // a push button has no model: no reset, no menu
            return;
        }
        match ev.button {
            MouseButton::Middle => {
                let t = c.target();
                self.set(ctrl, self.default_of(t), cx);
            }
            MouseButton::Right => {
                self.float = None;
                self.menu = Some(Menu { ctrl, x, y, choices: false });
                cx.notify();
            }
            MouseButton::Left => match &c.kind {
                Kind::Knob(_) | Kind::Lcd(_) => {
                    if ev.click_count >= 2 {
                        self.end_drag(cx);
                        self.open_entry(ctrl, window, cx);
                    } else if m.shift {
                        self.carried = Some((ctrl, self.param(c.target().key(), cx)));
                    } else if !m.control && !m.alt && !m.platform {
                        self.begin_drag(ctrl, y, cx);
                    }
                }
                Kind::Button(b) => {
                    let bind = b.bind;
                    self.click_button(ctrl, bind, ev.click_count, cx);
                }
                Kind::Led(l) => {
                    let bind = l.bind;
                    self.click_button(ctrl, bind, ev.click_count, cx);
                }
                Kind::Graph(_) => {}
                Kind::Combo(_) => {
                    // `ComboBox::mousePressEvent`: the list of the choices under the box
                    let c = &self.spec.controls[ctrl];
                    self.pressed = Some(ctrl);
                    self.float = None;
                    self.menu = Some(Menu { ctrl, x: c.x, y: c.y + widgets::COMBO_HEIGHT, choices: true });
                    cx.notify();
                }
            },
            _ => {}
        }
    }

    fn click_button(&mut self, ctrl: usize, bind: Bind, clicks: usize, cx: &mut Context<Self>) {
        match bind {
            Bind::Choice { key, .. } => {
                // a group's button: selecting the one already on changes nothing (`activateButton`)
                if !self.is_on(bind, cx) {
                    self.set(ctrl, Value::text(key), cx);
                }
                if clicks >= 2 {
                    self.double_clicked(ctrl, cx);
                }
            }
            Bind::Switch { .. } => {
                let on = self.is_on(bind, cx);
                self.set(ctrl, Value::real(if on { 0.0 } else { 1.0 }).unwrap_or_default(), cx);
            }
            Bind::Flag { .. } => {
                let on = self.is_on(bind, cx);
                self.set(ctrl, Value::bool(!on), cx);
            }
            Bind::Press(f) => {
                // a push button: its active image while held (`PixmapButton::mousePressEvent`), its edits in one step
                self.pressed = Some(ctrl);
                let mut seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0x9e37_79b9, |d| d.as_nanos() as u64)
                    | 1;
                let mut rand = move || {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (seed >> 11) as f64 / (1u64 << 53) as f64
                };
                let value = |k: &str| self.param(k, cx);
                let default = |k: &str| self.schema.as_ref().and_then(|s| s.find_param(k)).map_or(0.0, |p| p.def);
                let field = |k: &str| self.field(k, cx);
                let edits =
                    f(&mut crate::spec::PressCx { value: &value, field: &field, default: &default, rand: &mut rand });
                let ops: Vec<EditOp> = edits
                    .into_iter()
                    .filter_map(|(t, v)| match t {
                        Target::Param(k) => v.as_real().and_then(|x| self.param_op(k, x)),
                        Target::Field(k) => Some(self.field_op(k, v)),
                    })
                    .collect();
                let label = Label::new(self.spec.controls[ctrl].name.key);
                self.host.update(cx, |h, _| {
                    let _ = h.port_mut().apply(label, ops);
                });
                cx.notify();
            }
        }
    }

    /// `PixmapButton::doubleClicked` on a button with a file field: asks for an audio file, imports it through the
    /// port (in the client the port uploads it) and sets the field and the button's choice in one step
    fn double_clicked(&mut self, ctrl: usize, cx: &mut Context<Self>) {
        let Kind::Button(b) = &self.spec.controls[ctrl].kind else { return };
        if b.file_field.is_none() {
            return;
        }
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(Text::new("Open audio file").show().into()),
        });
        self.timer = Some(cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(mut chosen))) = paths.await else { return };
            let Some(path) = chosen.pop() else { return };
            let _ = this.update(cx, |p, cx| p.import_file(ctrl, path, cx));
        }));
    }

    /// Files dropped on the panel: the first with one of `extensions` goes to the core's `action` as its bytes
    /// (`{node, bytes}`), read here off the main thread: the file is on the interface's machine, which in the client
    /// is not the core's
    pub fn drop_files(
        &mut self,
        action: &'static str,
        extensions: &[&str],
        paths: &[std::path::PathBuf],
        cx: &mut Context<Self>,
    ) {
        let wanted = |p: &&std::path::PathBuf| {
            p.extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))
        };
        let Some(path) = paths.iter().find(wanted).cloned() else { return };
        let node = self.node;
        let read = cx.background_executor().spawn(async move {
            // a patch is a few bytes; anything past 1 MB is not one
            std::fs::read(&path).ok().filter(|b| b.len() <= 1 << 20)
        });
        self.timer = Some(cx.spawn(async move |this, cx| {
            let Some(bytes) = read.await else { return };
            let _ = this.update(cx, |p, cx| p.send_file(action, node, &bytes, cx));
        }));
    }

    /// Sends a file's bytes to the core's action for this plugin (`{node, bytes}`, one undo step there)
    pub fn send_file(&mut self, action: &'static str, node: NodeId, bytes: &[u8], cx: &mut Context<Self>) {
        let params = Value::object([
            ("node", Value::text(node.to_string())),
            ("bytes", Value::list(bytes.iter().map(|b| Value::int(i64::from(*b))))),
        ]);
        self.host.update(cx, |h, _| {
            let _ = h.port_mut().run_action(action.into(), params);
        });
    }

    /// Imports `path` for the file field of `ctrl` (also what the double click ends in; the tests call it)
    pub fn import_file(&mut self, ctrl: usize, path: std::path::PathBuf, cx: &mut Context<Self>) {
        let pending = self.host.update(cx, |h, _| h.port_mut().import(path));
        match pending {
            Pending::Ready(Ok(res)) => self.file_imported(ctrl, res, cx),
            Pending::Ready(Err(_)) => {}
            // over the network the answer comes later: the host hands it over (`PortHost::take_imported`)
            Pending::Waiting(id) => self.imports.push((id, ctrl)),
        }
    }

    fn file_imported(&mut self, ctrl: usize, res: lm_port::Resource, cx: &mut Context<Self>) {
        let c = &self.spec.controls[ctrl];
        let Kind::Button(b) = &c.kind else { return };
        let Some(field) = b.file_field else { return };
        let mut ops = vec![self.field_op(field, Value::resource(res))];
        if let Bind::Choice { field: choice, key } = b.bind {
            ops.push(self.field_op(choice, Value::text(key)));
        }
        let label = Label::new(c.name.key);
        self.host.update(cx, |h, _| {
            let _ = h.port_mut().apply(label, ops);
        });
        cx.notify();
    }

    /// The imports answered since the last frame (the client's port answers them in a later pump)
    fn collect_imports(&mut self, cx: &mut Context<Self>) {
        if self.imports.is_empty() {
            return;
        }
        let waiting = std::mem::take(&mut self.imports);
        for (id, ctrl) in waiting {
            match self.host.update(cx, |h, _| h.take_imported(id)) {
                Some(Ok(res)) => self.file_imported(ctrl, res, cx),
                Some(Err(_)) => {}
                None => self.imports.push((id, ctrl)),
            }
        }
    }

    fn mouse_move(&mut self, x: f32, y: f32, ev: &MouseMoveEvent, inside: bool, cx: &mut Context<Self>) {
        if self.graph.is_some() {
            self.graph_move(x, y, cx);
            return;
        }
        if self.drag.is_some() {
            self.drag_to(y, ev.modifiers.shift, cx);
            return;
        }
        if inside && ev.pressed_button.is_none() {
            let over = self.spec.hit(x, y);
            self.hover(over, cx);
        } else if !inside {
            self.hover(None, cx);
        }
    }

    fn mouse_up(&mut self, x: f32, y: f32, ev: &MouseUpEvent, cx: &mut Context<Self>) {
        if ev.button != MouseButton::Left {
            return;
        }
        self.end_drag(cx);
        self.graph_up(cx);
        self.pressed = None;
        if let Some((from, v)) = self.carried.take()
            && let Some(to) = self.spec.hit(x, y)
            && to != from
            && matches!(self.spec.controls[to].target(), Target::Param(_))
        {
            // Shift+drag dropped on another control: it takes the value (`dropEvent`, "float_value")
            self.set(to, Value::real(v).unwrap_or_default(), cx);
        }
    }

    fn scroll(&mut self, x: f32, y: f32, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let Some(ctrl) = self.spec.hit(x, y) else { return };
        if let Kind::Combo(cb) = &self.spec.controls[ctrl].kind {
            // `ComboBox::wheelEvent`: down selects the next choice, up the previous
            let down = match ev.delta {
                ScrollDelta::Lines(d) => d.y < 0.0,
                ScrollDelta::Pixels(d) => d.y < px(0.),
            };
            let v = self.field(cb.field, cx);
            let i = cb.items.iter().position(|it| v.as_text() == Some(it.key)).unwrap_or(0);
            let j = if down { (i + 1).min(cb.items.len() - 1) } else { i.saturating_sub(1) };
            if j != i {
                self.set(ctrl, Value::text(cb.items[j].key), cx);
            }
            return;
        }
        if !matches!(self.spec.controls[ctrl].kind, Kind::Knob(_) | Kind::Lcd(_)) {
            return;
        }
        let notches = match ev.delta {
            ScrollDelta::Lines(d) => {
                let v = if d.y != 0.0 { d.y } else { d.x };
                if v == 0.0 { 0 } else { v.signum() as i32 }
            }
            ScrollDelta::Pixels(d) => {
                let v = if d.y != px(0.) { d.y } else { d.x };
                self.scroll_acc += v.as_f32();
                let n = (self.scroll_acc / PIXELS_PER_NOTCH).trunc();
                self.scroll_acc -= n * PIXELS_PER_NOTCH;
                n as i32
            }
        };
        if notches == 0 {
            return;
        }
        let m = ev.modifiers;
        let steps = wheel_steps(m);
        let r = self.range(ctrl);
        let key = self.spec.controls[ctrl].target().key();
        let mut v = self.param(key, cx);
        for _ in 0..notches.unsigned_abs() {
            v = r.wheel(v, notches > 0, steps);
        }
        self.set(ctrl, Value::real(v).unwrap_or_default(), cx);
        self.show_float(ctrl, Some(WHEEL_SHOW), cx);
    }

    // --- the value entry (`enterValue`)

    fn open_entry(&mut self, ctrl: usize, window: &mut Window, cx: &mut Context<Self>) {
        let c = &self.spec.controls[ctrl];
        let v = self.param(c.target().key(), cx);
        let r = self.range(ctrl);
        let text = match &c.kind {
            Kind::Knob(k) if k.volume => {
                let ratio = r.rounded(v).abs() / k.zero_dbfs;
                let db = if ratio > 0.0 { 20.0 * ratio.log10() } else { -96.0 };
                format!("{db:.2}")
            }
            _ => format!("{:.*}", r.digits(), r.rounded(v)),
        };
        self.float = None;
        self.entry = Some(Entry { ctrl, text });
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn entry_bounds(&self, ctrl: usize) -> (f64, f64) {
        let c = &self.spec.controls[ctrl];
        let r = self.range(ctrl);
        match &c.kind {
            Kind::Knob(k) if k.volume => {
                let magnitude = r.max.max(r.min.abs());
                (-96.0, 20.0 * (magnitude / k.zero_dbfs).log10())
            }
            _ => (r.min, r.max),
        }
    }

    /// Applies the typed value: a number (comma or point), within the bounds; a volume knob's in dBFS
    pub fn commit_entry(&mut self, cx: &mut Context<Self>) {
        let Some(e) = self.entry.take() else { return };
        let Ok(typed) = e.text.trim().replace(',', ".").parse::<f64>() else {
            cx.notify();
            return;
        };
        let (lo, hi) = self.entry_bounds(e.ctrl);
        let typed = typed.clamp(lo, hi);
        let value = match &self.spec.controls[e.ctrl].kind {
            Kind::Knob(k) if k.volume => {
                let sign = if self.param(k.param, cx) < 0.0 { -1.0 } else { 1.0 };
                if typed <= -96.0 { 0.0 } else { sign * 10f64.powf(typed / 20.0) * k.zero_dbfs }
            }
            _ => typed,
        };
        self.set(e.ctrl, Value::real(value).unwrap_or_default(), cx);
    }

    fn key_down(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = &mut self.entry else {
            if ev.keystroke.key == "escape" && self.menu.take().is_some() {
                cx.notify();
            }
            return;
        };
        match ev.keystroke.key.as_str() {
            "enter" => self.commit_entry(cx),
            "escape" => self.entry = None,
            "backspace" => {
                e.text.pop();
            }
            _ => {
                if let Some(ch) = ev.keystroke.key_char.as_deref()
                    && ch.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+'))
                {
                    e.text.push_str(ch);
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    // --- the menu (F-CTL-005)

    /// The menu's entries for a control, with whether each is enabled
    pub fn menu_items(&self, ctrl: usize, cx: &App) -> Vec<(MenuItem, bool)> {
        let c = &self.spec.controls[ctrl];
        let param = matches!(c.target(), Target::Param(_));
        let mut items = vec![(MenuItem::Reset, true)];
        if param {
            items.push((MenuItem::Copy, true));
            items.push((MenuItem::Paste, clipboard().is_some()));
        }
        items.push((MenuItem::MidiLearn, true));
        items.push((MenuItem::MidiForget, self.mappings(ctrl, cx) > 0));
        items.push((MenuItem::MidiBinding, true));
        if param {
            if self.lane(ctrl, cx).is_some() {
                items.push((MenuItem::RemoveLane, true));
            } else {
                items.push((MenuItem::AddLane, self.owner(cx).is_some()));
            }
        }
        if matches!(c.kind, Kind::Knob(_)) {
            items.push((MenuItem::ToggleLog, true));
        }
        items
    }

    fn menu_label(&self, ctrl: usize, item: MenuItem, cx: &App) -> String {
        let c = &self.spec.controls[ctrl];
        let current = self.value_text(ctrl, cx);
        match item {
            MenuItem::Reset => {
                let d = self.default_of(c.target());
                let shown = match (&c.kind, d.as_real()) {
                    (Kind::Knob(k), Some(v)) if k.volume => widgets::volume_text(v, k.zero_dbfs),
                    (Kind::Knob(k), Some(v)) => format!("{}{}", widgets::qt_number(v), unit(k.unit)),
                    (_, Some(v)) => widgets::qt_number(v),
                    (_, None) => d.as_text().map(str::to_owned).unwrap_or_default(),
                };
                Text::new("&Reset (%1)").show().replace("%1", &shown).replace('&', "")
            }
            MenuItem::Copy => Text::new("&Copy value (%1)").show().replace("%1", &current).replace('&', ""),
            MenuItem::Paste => match clipboard() {
                Some(v) => Text::new("&Paste value (%1)").show().replace("%1", &widgets::qt_number(v)).replace('&', ""),
                None => Text::new("&Paste value").show().replace('&', ""),
            },
            MenuItem::MidiLearn => Text::new("MIDI learn").show(),
            MenuItem::MidiForget => Text::new("Remove MIDI mapping").show(),
            MenuItem::MidiBinding => Text::new("MIDI binding…").show(),
            MenuItem::AddLane => Text::new("Add automation lane").show(),
            MenuItem::RemoveLane => Text::new("Remove automation lane").show(),
            MenuItem::ToggleLog => {
                if self.range(ctrl).log { Text::new("Set linear") } else { Text::new("Set logarithmic") }.show()
            }
        }
    }

    /// Runs a menu entry
    pub fn run_menu(&mut self, ctrl: usize, item: MenuItem, cx: &mut Context<Self>) {
        self.menu = None;
        let c = &self.spec.controls[ctrl];
        let t = c.target();
        match item {
            MenuItem::Reset => self.set(ctrl, self.default_of(t), cx),
            MenuItem::Copy => {
                let v = self.param(t.key(), cx);
                if let Ok(mut c) = CLIPBOARD.lock() {
                    *c = Some(v);
                }
            }
            MenuItem::Paste => {
                if let Some(v) = clipboard() {
                    self.set(ctrl, Value::real(v).unwrap_or_default(), cx);
                }
            }
            MenuItem::MidiLearn => {
                // the next controller moved becomes the source (I-21.2); the session shows the learning and its end
                let target = self.map_target(ctrl);
                self.host.update(cx, |h, _| {
                    let _ = h.port_mut().midi_learn(target, MapScope::Project);
                });
            }
            MenuItem::MidiForget => {
                let target = self.map_target(ctrl);
                self.host.update(cx, |h, _| {
                    let _ = h.port_mut().unmap(&target, MapScope::Project);
                });
            }
            MenuItem::MidiBinding => cx.emit(PanelEvent::OpenBinding(self.map_target(ctrl))),
            MenuItem::AddLane => {
                if let Some(owner) = self.owner(cx) {
                    let lane = Value::object([
                        (
                            "target",
                            Value::object([
                                ("node", Value::text(self.node.to_string())),
                                ("param", Value::text(t.key())),
                            ]),
                        ),
                        ("points", Value::empty_list()),
                    ]);
                    let op = EditOp::Insert {
                        temp: 1,
                        parent: Ref::Id(owner),
                        list: "automation".into(),
                        index: usize::MAX,
                        node: lane,
                    };
                    let op = self.at_end(op, owner, cx);
                    let label = Label::new("Add automation lane");
                    self.host.update(cx, |h, _| {
                        let _ = h.port_mut().apply(label, vec![op]);
                    });
                }
            }
            MenuItem::RemoveLane => {
                if let Some(id) = self.lane(ctrl, cx) {
                    let label = Label::new("Remove automation lane");
                    self.host.update(cx, |h, _| {
                        let _ = h.port_mut().apply(label, vec![EditOp::Remove { id: Ref::Id(id) }]);
                    });
                }
            }
            MenuItem::ToggleLog => {
                if !self.toggled_log.remove(&t) {
                    self.toggled_log.insert(t);
                }
            }
        }
        cx.notify();
    }

    fn at_end(&self, op: EditOp, owner: NodeId, cx: &App) -> EditOp {
        let n = self.host.read(cx).port().find(owner).and_then(|o| o.get("automation").map(Value::len)).unwrap_or(0);
        match op {
            EditOp::Insert { temp, parent, list, node, .. } => EditOp::Insert { temp, parent, list, index: n, node },
            other => other,
        }
    }

    /// The control as a MIDI mapping's target (I-21.2): its parameter, or its field by path
    pub fn map_target(&self, ctrl: usize) -> MapTarget {
        let node = NodeRef::Id(self.node);
        match self.spec.controls[ctrl].target() {
            Target::Param(k) => MapTarget::Param { node, key: k.to_owned() },
            Target::Field(k) => MapTarget::Field { node, path: vec![PathStep::key("state"), PathStep::key(k)] },
        }
    }

    /// The project's mappings that move this control
    fn mappings(&self, ctrl: usize, cx: &App) -> usize {
        let want = self.map_target(ctrl);
        let root = self.host.read(cx).port().root().clone();
        let Some(list) = root.get("midiMap") else { return 0 };
        list.items().filter_map(|m| Mapping::from_value(m).ok()).filter(|m| m.target == want).count()
    }

    /// The track, channel or master whose subtree holds this plugin: where its automation lanes live (I-21.5)
    fn owner(&self, cx: &App) -> Option<NodeId> {
        let root = self.host.read(cx).port().root().clone();
        let me = self.node.to_string();
        let mut found = None;
        let mut owners: Vec<(Vec<PathStep>, NodeId)> = Vec::new();
        root.walk(&mut |path, v| {
            if v.get("automation").is_some_and(Value::is_list)
                && let Some(id) = v.get("id").and_then(Value::as_text).and_then(|s| NodeId::parse(s).ok())
            {
                owners.push((path.to_vec(), id));
            }
            if v.get("plugin").is_some() && v.get("id").and_then(Value::as_text) == Some(me.as_str()) {
                found = Some(path.to_vec());
            }
        });
        let at = found?;
        owners.into_iter().filter(|(p, _)| at.starts_with(p)).max_by_key(|(p, _)| p.len()).map(|(_, id)| id)
    }

    /// The automation lane of this control's parameter, if there is one
    fn lane(&self, ctrl: usize, cx: &App) -> Option<NodeId> {
        let Target::Param(key) = self.spec.controls[ctrl].target() else { return None };
        let owner = self.owner(cx)?;
        let o = self.host.read(cx).port().find(owner)?;
        let me = self.node.to_string();
        o.get("automation")?.items().find_map(|l| {
            let t = l.get("target")?;
            (t.get("node").and_then(Value::as_text) == Some(me.as_str())
                && t.get("param").and_then(Value::as_text) == Some(key))
            .then(|| l.get("id").and_then(Value::as_text).and_then(|s| NodeId::parse(s).ok()))
            .flatten()
        })
    }

    // --- drawing

    /// The image of a control in its present state, at the screen's scale (cached by the state)
    fn control_image(&mut self, ctrl: usize, scale: f32, cx: &App) -> Arc<RenderImage> {
        let c = &self.spec.controls[ctrl];
        let (w, h) = c.size();
        let (state_key, draw): (u64, Draw) = match &c.kind {
            Kind::Knob(k) => {
                let r = self.range(ctrl);
                let angle = r.angle(self.param(k.param, cx));
                let center = r.angle(r.center);
                let style = k.style.clone();
                let key =
                    k.style.cache_key() ^ ((angle as i64 + 1000) as u64) << 20 ^ ((center as i64 + 1000) as u64) << 40;
                (key, Box::new(move |cv| widgets::draw_knob(cv, &style, angle, center, true)))
            }
            Kind::Button(b) => {
                let on = self.is_on(b.bind, cx) || self.pressed == Some(ctrl);
                let asset = if on { b.active } else { b.inactive };
                (on as u64, Box::new(move |cv| cv.pixmap(assets::image(asset), 0.0, 0.0, false)))
            }
            Kind::Led(l) => {
                let on = self.is_on(l.bind, cx);
                let asset = if on { l.color.asset() } else { &assets::lmms::LED_OFF };
                (on as u64, Box::new(move |cv| cv.pixmap(assets::image(asset), 0.0, 0.0, false)))
            }
            Kind::Lcd(l) if l.frac > 0 => {
                let v = self.param(l.param, cx);
                let texts = widgets::lcd_float_texts(v, l.frac);
                let hsh = texts
                    .0
                    .bytes()
                    .chain(texts.1.bytes())
                    .fold(0u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64));
                let (sheet, whole, frac) = (l.sheet, l.digits, l.frac);
                (hsh, Box::new(move |cv| widgets::draw_lcd_float(cv, sheet, whole, frac, &texts, true)))
            }
            Kind::Lcd(l) => {
                let v = self.param(l.param, cx);
                let text = widgets::lcd_text(v, l.digits, l.leading_zero);
                let mut hsh = 0u64;
                for b in text.bytes() {
                    hsh = hsh.wrapping_mul(131).wrapping_add(b as u64);
                }
                let (sheet, digits) = (l.sheet, l.digits);
                (hsh, Box::new(move |cv| widgets::draw_lcd(cv, sheet, digits, &text, true)))
            }
            Kind::Graph(g) => {
                let samples = self.samples(ctrl, cx);
                let cursor = self.graph.as_ref().filter(|d| d.ctrl == ctrl).map(|d| d.cursor);
                let mut h = std::hash::DefaultHasher::new();
                use std::hash::{Hash, Hasher};
                for v in &samples {
                    v.to_bits().hash(&mut h);
                }
                cursor.map(|(x, y)| (x.to_bits(), y.to_bits())).hash(&mut h);
                let g = g.clone();
                (h.finish(), Box::new(move |cv| widgets::draw_graph(cv, &g, &samples, cursor)))
            }
            Kind::Combo(cb) => {
                let v = self.field(cb.field, cx);
                let i = cb.items.iter().position(|it| v.as_text() == Some(it.key));
                let icon = i.and_then(|i| cb.items[i].icon);
                let pressed = self.pressed == Some(ctrl) && self.menu.is_some();
                let w = cb.w;
                (
                    i.map_or(u64::MAX, |i| i as u64) ^ (pressed as u64) << 32,
                    Box::new(move |cv| {
                        widgets::draw_combo(cv, w, icon.map(assets::image), pressed);
                    }),
                )
            }
        };
        let key = (ctrl, state_key ^ (scale.to_bits() as u64).rotate_left(7));
        if let Some(i) = self.images.get(&key) {
            return Arc::clone(i);
        }
        let mut canvas = Canvas::new(w, h, scale);
        draw(&mut canvas);
        let image = canvas.into_image();
        // one image per state seen; a knob has at most 271 angles, so the cache stays small
        self.images.insert(key, Arc::clone(&image));
        image
    }

    fn texts(&self, cx: &App) -> Vec<Txt> {
        let mut out = Vec::new();
        let base = |x: f32, y: f32, px: f32, text: String, color: u32, centered: bool| Txt {
            x,
            y,
            px,
            text,
            color,
            centered,
            top: false,
        };
        for c in &self.spec.controls {
            let (w, h) = c.size();
            match &c.kind {
                Kind::Knob(k) => {
                    if let Some(l) = &k.label {
                        // the legacy label: at the bottom of the widget, 2 pixels above its edge (`drawLabel`)
                        out.push(base(
                            c.x + w / 2.0,
                            c.y + h + 10.0 - 2.0,
                            theme::SMALL_FONT_PX,
                            l.show(),
                            0xffffff,
                            true,
                        ));
                    }
                }
                Kind::Led(l) => {
                    let t = l.text.show();
                    if !t.is_empty() {
                        // paintLegacy: a shadow (64, 64, 64) one pixel down-right, then white
                        out.push(base(c.x + w + 4.0, c.y + 11.0, theme::DEFAULT_FONT_PX, t.clone(), 0x404040, false));
                        out.push(base(c.x + w + 3.0, c.y + 10.0, theme::DEFAULT_FONT_PX, t, 0xffffff, false));
                    }
                }
                Kind::Lcd(l) => {
                    if let Some(t) = &l.label {
                        let t = t.show();
                        out.push(base(
                            c.x + w / 2.0 + 1.0,
                            c.y + h + 9.0,
                            theme::DEFAULT_FONT_PX,
                            t.clone(),
                            0x404040,
                            true,
                        ));
                        out.push(base(c.x + w / 2.0, c.y + h + 8.0, theme::DEFAULT_FONT_PX, t, 0xffffff, true));
                    }
                }
                Kind::Combo(cb) => {
                    let current = self.field(cb.field, cx);
                    if let Some(item) = cb.items.iter().find(|i| current.as_text() == Some(i.key)) {
                        let tx = c.x + combo_text_x(item);
                        // `(height + fontMetrics.height) / 2 - 4`, the shadow one pixel down-right in (64, 64, 64)
                        let y = c.y + ((widgets::COMBO_HEIGHT + 15.0) / 2.0).floor() - 4.0;
                        let t = item.text.show();
                        out.push(base(tx + 1.0, y + 1.0, theme::DEFAULT_FONT_PX, t.clone(), 0x404040, false));
                        out.push(base(tx, y, theme::DEFAULT_FONT_PX, t, 0xe0e0e0, false));
                    }
                }
                Kind::Button(_) | Kind::Graph(_) => {}
            }
        }
        for l in &self.spec.labels {
            out.push(Txt { x: l.x, y: l.y, px: l.px, text: l.text.show(), color: l.color, centered: false, top: true });
        }
        out
    }
}

/// Where a combo box's text starts: after the icon (`tx = 5`, plus the icon's width and 3)
fn combo_text_x(item: &crate::spec::ComboItem) -> f32 {
    match item.icon {
        Some(a) => {
            let img = assets::image(a);
            let w = if img.height() > 16 {
                (img.width() as f32 * 16.0 / img.height() as f32).round()
            } else {
                img.width() as f32
            };
            5.0 + w + 3.0
        }
        None => 5.0,
    }
}

/// A text the panel paints: at its baseline (`top` false, as QPainter::drawText) or its top-left (a QLabel)
#[derive(Clone, Debug)]
struct Txt {
    x: f32,
    y: f32,
    px: f32,
    text: String,
    color: u32,
    centered: bool,
    top: bool,
}

/// A unit as the legacy hint writes it (`" " + tr("semitones")`): the word translated, the space kept
fn unit(u: &'static str) -> String {
    let word = u.trim_start();
    if word.chars().any(char::is_alphabetic) {
        format!("{}{}", &u[..u.len() - word.len()], crate::i18n::tr(word))
    } else {
        u.to_owned()
    }
}

fn clipboard() -> Option<f64> {
    CLIPBOARD.lock().ok().and_then(|c| *c)
}

/// The panel's own texts (menu, value entry), for the translation test
pub const TEXTS: &[&str] = &[
    "&Reset (%1)",
    "&Copy value (%1)",
    "&Paste value (%1)",
    "&Paste value",
    "MIDI learn",
    "Remove MIDI mapping",
    "MIDI binding…",
    "Add automation lane",
    "Remove automation lane",
    "Set linear",
    "Set logarithmic",
    "Set value",
    "Please enter a new value between %1 and %2:",
    "Please enter a new value between %1 dBFS and %2 dBFS:",
    "Cancel",
    "OK",
    "Open audio file",
];

/// `FloatModelEditorBase::wheelEvent`: the notches of a whole sweep for the modifiers held
pub fn wheel_steps(m: Modifiers) -> f64 {
    if m.shift && !m.control && !m.alt {
        10.0
    } else if m.control && !m.shift && !m.alt {
        1000.0
    } else if m.alt && !m.shift && !m.control {
        2000.0
    } else {
        100.0
    }
}

impl<H: PortHost> std::fmt::Debug for PluginPanel<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginPanel")
            .field("plugin", &self.spec.plugin)
            .field("node", &self.node)
            .finish_non_exhaustive()
    }
}

impl<H: PortHost> gpui::EventEmitter<PanelEvent> for PluginPanel<H> {}

impl<H: PortHost> Focusable for PluginPanel<H> {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<H: PortHost> Render for PluginPanel<H> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let spec = self.spec;
        let (w, h) = spec.size;
        let scale = window.scale_factor();
        let images: Vec<(Bounds<Pixels>, Arc<RenderImage>)> = (0..spec.controls.len())
            .map(|i| {
                let c = &spec.controls[i];
                let (cw, ch) = c.size();
                (Bounds::new(point(px(c.x), px(c.y)), size(px(cw), px(ch))), self.control_image(i, scale, cx))
            })
            .collect();
        let texts = self.texts(cx);
        let weak = cx.entity().downgrade();

        let mut root = div()
            .id(SharedString::from(format!("panel-{}", spec.plugin)))
            .debug_selector(move || format!("panel-{}", spec.plugin))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .relative()
            .flex_none()
            .w(px(w))
            .h(px(h))
            .bg(rgb(spec.fill));
        if let Some(crate::spec::FileDrop::Action { action, extensions }) = spec.file_drop {
            root = root.on_drop(cx.listener(move |p, paths: &gpui::ExternalPaths, _, cx| {
                p.drop_files(action, extensions, paths.paths(), cx);
            }));
        }
        if let Some(bg) = spec.background {
            root = root.child(img(bg.gpui_image()).absolute().top_0().left_0().w(px(w)).h(px(h)));
        }
        root = root.child(
            canvas(
                move |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                move |bounds, hitbox, window, cx| paint(bounds, hitbox, images, texts, weak, window, cx),
            )
            .absolute()
            .top_0()
            .left_0()
            .w(px(w))
            .h(px(h)),
        );
        // the controls' places, for the tests and the accessibility tree (no paint, no hitbox)
        for c in &spec.controls {
            let (cw, ch) = c.size();
            let id = c.id;
            root = root.child(
                div().absolute().left(px(c.x)).top(px(c.y)).w(px(cw)).h(px(ch)).debug_selector(move || id.to_string()),
            );
        }
        if let Some(f) = &self.float {
            let c = &spec.controls[f.ctrl];
            let (cw, _) = c.size();
            root = root.child(
                div()
                    .absolute()
                    .left(px(c.x + cw + 2.0))
                    .top(px(c.y))
                    .p(px(3.))
                    .bg(rgb(theme::FLOAT_BG))
                    .text_color(rgb(theme::FLOAT_TEXT))
                    .text_size(px(12.))
                    .whitespace_nowrap()
                    .debug_selector(|| "value-float".into())
                    .child(f.text.clone()),
            );
        }
        if let Some(m) = &self.menu {
            root = if m.choices {
                root.child(self.render_choices(m.ctrl, m.x, m.y, cx))
            } else {
                root.child(self.render_menu(m.ctrl, m.x, m.y, cx))
            };
        }
        if let Some(e) = &self.entry {
            root = root.child(self.render_entry(e.ctrl, &e.text, cx));
        }
        root
    }
}

impl<H: PortHost> PluginPanel<H> {
    fn render_menu(&self, ctrl: usize, x: f32, y: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &self.spec.controls[ctrl];
        let items = self.menu_items(ctrl, cx);
        let mut menu = div()
            .id("control-menu")
            .debug_selector(|| "control-menu".into())
            .occlude()
            .absolute()
            .left(px(x))
            .top(px(y))
            .min_w(px(180.))
            .bg(rgb(theme::MENU_BG))
            .border_1()
            .border_color(rgb(theme::MENU_CAPTION))
            .text_size(px(12.))
            .text_color(rgb(theme::MENU_TEXT))
            .on_mouse_down_out(cx.listener(|p, _, _, cx| {
                p.menu = None;
                cx.notify();
            }))
            // the caption (`CaptionMenu`): the model's name
            .child(
                div()
                    .px(px(8.))
                    .py(px(3.))
                    .bg(rgb(theme::MENU_CAPTION))
                    .font_weight(FontWeight::BOLD)
                    .child(c.name.show()),
            );
        for (i, (item, enabled)) in items.into_iter().enumerate() {
            let label = self.menu_label(ctrl, item, cx);
            let sel = format!("menu-{}", menu_name(item));
            let mut row =
                div().id(("menu-item", i)).debug_selector(move || sel.clone()).px(px(8.)).py(px(3.)).child(label);
            if enabled {
                row = row.hover(|s| s.bg(rgb(theme::MENU_HOVER))).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |p, _, _, cx| {
                        cx.stop_propagation();
                        p.run_menu(ctrl, item, cx);
                    }),
                );
            } else {
                row = row.text_color(rgb(theme::MENU_DISABLED));
            }
            menu = menu.child(row);
        }
        menu
    }

    /// A combo box's list (`ComboBox::mousePressEvent`'s QMenu): every choice with its icon; a click selects it
    fn render_choices(&self, ctrl: usize, x: f32, y: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let Kind::Combo(cb) = &self.spec.controls[ctrl].kind else { return div().into_any_element() };
        let current = self.field(cb.field, cx);
        let mut list = div()
            .id("combo-list")
            .debug_selector(|| "combo-list".into())
            .occlude()
            .absolute()
            .left(px(x))
            .top(px(y))
            .min_w(px(cb.w))
            .bg(rgb(theme::MENU_BG))
            .border_1()
            .border_color(rgb(theme::MENU_CAPTION))
            .text_size(px(theme::DEFAULT_FONT_PX))
            .text_color(rgb(theme::MENU_TEXT))
            .on_mouse_down_out(cx.listener(|p, _, _, cx| {
                p.menu = None;
                p.pressed = None;
                cx.notify();
            }));
        for (i, item) in cb.items.iter().enumerate() {
            let key = item.key;
            let sel = format!("choice-{key}");
            let on = current.as_text() == Some(key);
            let mut row = div()
                .id(("combo-item", i))
                .debug_selector(move || sel.clone())
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(6.))
                .py(px(2.))
                .hover(|s| s.bg(rgb(theme::MENU_HOVER)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |p, _, _, cx| {
                        cx.stop_propagation();
                        p.menu = None;
                        p.pressed = None;
                        p.set(ctrl, Value::text(key), cx);
                    }),
                );
            if on {
                row = row.bg(rgb(theme::MENU_CAPTION));
            }
            if let Some(icon) = item.icon {
                row = row.child(img(icon.gpui_image()).w(px(assets::image(icon).width() as f32)).h(px(16.)));
            }
            list = list.child(row.child(item.text.show()));
        }
        list.into_any_element()
    }

    fn render_entry(&self, ctrl: usize, text: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let (lo, hi) = self.entry_bounds(ctrl);
        let volume = matches!(&self.spec.controls[ctrl].kind, Kind::Knob(k) if k.volume);
        let prompt = if volume {
            Text::new("Please enter a new value between %1 dBFS and %2 dBFS:")
        } else {
            Text::new("Please enter a new value between %1 and %2:")
        }
        .show()
        .replace("%1", &widgets::qt_number(lo))
        .replace("%2", &widgets::qt_number(hi));
        let (w, h) = self.spec.size;
        div()
            .id("value-entry")
            .debug_selector(|| "value-entry".into())
            .occlude()
            .absolute()
            .left(px((w - 240.0).max(0.0) / 2.0))
            .top(px((h - 96.0).max(0.0) / 2.0))
            .w(px(240.))
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .bg(rgb(theme::DIALOG_BG))
            .border_1()
            .border_color(rgb(theme::DIALOG_BORDER))
            .text_size(px(12.))
            .text_color(rgb(theme::DIALOG_TEXT))
            .child(div().font_weight(FontWeight::BOLD).child(Text::new("Set value").show()))
            .child(div().child(prompt))
            .child(
                div()
                    .px(px(4.))
                    .py(px(2.))
                    .bg(rgb(theme::DIALOG_FIELD))
                    .border_1()
                    .border_color(rgb(theme::DIALOG_BORDER))
                    .debug_selector(|| "value-entry-text".into())
                    .child(format!("{text}|")),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.))
                    .child(
                        div()
                            .id("entry-cancel")
                            .px(px(8.))
                            .border_1()
                            .border_color(rgb(theme::DIALOG_BORDER))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|p, _, _, cx| {
                                    cx.stop_propagation();
                                    p.entry = None;
                                    cx.notify();
                                }),
                            )
                            .child(Text::new("Cancel").show()),
                    )
                    .child(
                        div()
                            .id("entry-ok")
                            .debug_selector(|| "value-entry-ok".into())
                            .px(px(8.))
                            .border_1()
                            .border_color(rgb(theme::DIALOG_BORDER))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|p, _, _, cx| {
                                    cx.stop_propagation();
                                    p.commit_entry(cx);
                                }),
                            )
                            .child(Text::new("OK").show()),
                    ),
            )
    }
}

fn menu_name(item: MenuItem) -> &'static str {
    match item {
        MenuItem::Reset => "reset",
        MenuItem::Copy => "copy",
        MenuItem::Paste => "paste",
        MenuItem::MidiLearn => "midi-learn",
        MenuItem::MidiForget => "midi-forget",
        MenuItem::MidiBinding => "midi-binding",
        MenuItem::AddLane => "add-lane",
        MenuItem::RemoveLane => "remove-lane",
        MenuItem::ToggleLog => "toggle-log",
    }
}

/// How a control's image is drawn for its present state
type Draw = Box<dyn Fn(&mut Canvas)>;

/// Paints the controls' images and texts, and listens to the mouse for the whole panel (a drag goes on outside it)
fn paint<H: PortHost>(
    bounds: Bounds<Pixels>,
    hitbox: Hitbox,
    images: Vec<(Bounds<Pixels>, Arc<RenderImage>)>,
    texts: Vec<Txt>,
    weak: WeakEntity<PluginPanel<H>>,
    window: &mut Window,
    cx: &mut App,
) {
    let o = bounds.origin;
    for (b, image) in images {
        let at = Bounds::new(o + b.origin, b.size);
        let _ = window.paint_image(at, at, Default::default(), image, 0, false);
    }
    let font = window.text_style().font();
    let font_id = window.text_system().resolve_font(&font);
    for t in texts {
        let run = TextRun {
            len: t.text.len(),
            font: font.clone(),
            color: rgb(t.color).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let size_px = px(t.px);
        let ascent = window.text_system().ascent(font_id, size_px);
        let descent = window.text_system().descent(font_id, size_px);
        let line = window.text_system().shape_line(t.text.into(), size_px, &[run], None);
        let lw = line.width.as_f32();
        let left = if t.centered { t.x - lw / 2.0 } else { t.x };
        // a line box exactly ascent + descent high: its baseline is one ascent below its top
        let top = if t.top { px(t.y) } else { px(t.y) - ascent };
        let _ = line.paint(o + point(px(left), top), ascent + descent.abs(), TextAlign::Left, None, window, cx);
    }
    let local = move |p: gpui::Point<Pixels>| ((p.x - o.x).as_f32(), (p.y - o.y).as_f32());
    let w = weak.clone();
    let hb = hitbox.clone();
    window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !hb.is_hovered(window) {
            return;
        }
        let (x, y) = local(ev.position);
        let _ = w.update(cx, |p, cx| p.mouse_down(x, y, ev, window, cx));
    });
    let w = weak.clone();
    let hb = hitbox.clone();
    window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        let (x, y) = local(ev.position);
        let inside = hb.is_hovered(window);
        let _ = w.update(cx, |p, cx| p.mouse_move(x, y, ev, inside, cx));
    });
    let w = weak.clone();
    window.on_mouse_event(move |ev: &MouseUpEvent, phase, _, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        let (x, y) = local(ev.position);
        let _ = w.update(cx, |p, cx| p.mouse_up(x, y, ev, cx));
    });
    let w = weak;
    window.on_mouse_event(move |ev: &ScrollEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !hitbox.should_handle_scroll(window) {
            return;
        }
        let (x, y) = local(ev.position);
        let _ = w.update(cx, |p, cx| p.scroll(x, y, ev, cx));
    });
}

/// What `CorePort` answers a panel needs (the tests check it): the plugin's node and its schema
pub fn port_has_plugin(port: &dyn CorePort, node: NodeId, plugin: &str) -> bool {
    port.find(node).and_then(|n| n.get("plugin").and_then(Value::as_text).map(|p| p == plugin)).unwrap_or(false)
        && port.plugin_schema(plugin).is_some()
}
