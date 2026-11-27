//! Owner: agent C. The instrument window (approved design 05): a docked panel with the track's name and colour, the
//! instrument and "Swap", the presets on top (previous, next, the list, save), text tabs (Instrument, Envelopes and
//! filter, Note effects, Effects (n), MIDI, Tuning) and a fixed footer (volume, pitch, bend range, where the
//! outputs go, the test keyboard).
//!
//! The first half of the file is THE GENERIC PLUGIN EDITOR, shared with the effect window: the controls come from
//! the plugin's schema through the port (`CorePort::plugin_schema`): a knob for a real parameter (linear or log), a
//! stepper for a stepped one, a switch for a 0–1 stepped one or a bool field, a list for a choice field (by key,
//! U30), with the unit and the automation mark, and the gestures of every control (F-CTL-005/006): drag (Shift 4×
//! slower; one gesture, one undo step), middle click = default, double click = type a value, wheel = 100 steps
//! (Shift 10, Ctrl/Cmd 1000), Shift+drag copies the value onto another control, right click = the menu (default,
//! copy, paste, automate / remove the lane, linear/log display, P-06).
//!
//! The window knows only ids (U38): it reads the root through the port, follows the track by a `Feed`, closes when
//! the track goes away, and every change is an `EditOp` (I-15).

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, AnyView, App, Bounds, Context, Entity, FocusHandle, FontWeight, Global, KeyDownEvent, KeyUpEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, Render, ScrollEvent,
    SharedString, Subscription, WeakEntity, Window, anchored, canvas, deferred, div, fill, point, prelude::*, px, size,
};
use gpui_component::Sizable;
use gpui_component::input::{Input, InputEvent, InputState};
use lm_gui_plugins::Gesture;
use lm_port::{
    BrowsePage, EditOp, Feed, FieldKind, Label, Loading, NodeId, NodeSchema, Pending, PluginInfo, Ref,
    Scale, Target, Topic, Value,
};

mod note_effects;

use super::live::{self, Live};
use super::{Screen, ScreenCx};
use crate::core::Core;
use crate::model::{self, TrackView};
use crate::ui::{THEME, button, c, caption, knob_face, loading_component, tab, tr, trf};
use crate::view;

// =================================================================================================================
// The generic plugin editor (shared with screens/effect.rs)
// =================================================================================================================

/// What a control edits on its node
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// `state.params.<key>`: a real parameter
    Param(&'static str),
    /// `state.<key>`: a choice or bool field
    Field(&'static str),
    /// the track's own `volume` (linear gain; shown in dB)
    TrackVolume,
    /// an insert's `wet` (0..1; absent: all wet), the legacy EffectView's W/D
    Wet,
    /// a track's MIDI controller `cc.N` of the CC rack (F-IJN-025), stored at `track.cc.cN`
    Cc(u8),
}

#[derive(Clone, Debug, PartialEq)]
pub enum CtlKind {
    Knob { min: f64, max: f64, def: f64, log: bool },
    Stepper { min: f64, max: f64, def: f64 },
    Switch { def: bool },
    Choice { options: &'static [&'static str] },
    Volume,
}

/// One control as drawn
#[derive(Clone, Debug)]
pub struct Ctl {
    pub node: NodeId,
    /// the plugin's id ("" for the track's volume): the labels and the display option are by plugin
    pub plugin: Arc<str>,
    pub slot: Slot,
    pub kind: CtlKind,
    pub unit: &'static str,
    pub automatable: bool,
    /// the current value (a choice's key is in `choice`)
    pub value: f64,
    pub choice: Arc<str>,
    pub automated: bool,
    pub enabled: bool,
    /// a text in place of the value (a synced time shows its note)
    pub shown: Option<String>,
}

/// `f64::clamp` panics when a plugin's schema has min above max or NaN; this one does not
fn fit(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

impl Ctl {
    pub fn key(&self) -> &'static str {
        match self.slot {
            Slot::Param(k) | Slot::Field(k) => k,
            Slot::TrackVolume => "volume",
            Slot::Wet => "wet",
            Slot::Cc(n) => intern(&format!("cc.{n}")),
        }
    }

    fn default_value(&self) -> f64 {
        match &self.kind {
            CtlKind::Knob { def, .. } | CtlKind::Stepper { def, .. } => *def,
            CtlKind::Switch { def } => f64::from(u8::from(*def)),
            CtlKind::Volume | CtlKind::Choice { .. } => 0.0,
        }
    }

    fn clamp(&self, v: f64) -> f64 {

        match &self.kind {
            // a MIDI controller's value is a whole number
            CtlKind::Knob { min, max, .. } if matches!(self.slot, Slot::Cc(_)) => fit(v, *min, *max).round(),
            CtlKind::Knob { min, max, .. } => fit(v, *min, *max),
            CtlKind::Stepper { min, max, .. } => fit(v, *min, *max).round(),
            CtlKind::Switch { .. } => f64::from(u8::from(v > 0.5)),
            CtlKind::Volume => v.clamp(-60.0, 12.0),
            CtlKind::Choice { .. } => v,
        }
    }

    /// 0..1 of the knob's travel, with the display option (P-06) applied
    fn to_t(&self, v: f64, flip: bool) -> f64 {
        match self.kind {
            CtlKind::Knob { min, max, log, .. } => {
                if (log != flip) && min > 0.0 {
                    ((v / min).ln() / (max / min).ln()).clamp(0.0, 1.0)
                } else {
                    ((v - min) / (max - min)).clamp(0.0, 1.0)
                }
            }
            CtlKind::Stepper { min, max, .. } => ((v - min) / (max - min)).clamp(0.0, 1.0),
            CtlKind::Volume => view::db_to_travel(v),
            _ => v,
        }
    }

    fn value_at(&self, t: f64, flip: bool) -> f64 {
        let t = t.clamp(0.0, 1.0);
        let v = match self.kind {
            CtlKind::Knob { min, max, log, .. } => {
                if (log != flip) && min > 0.0 {
                    min * (max / min).powf(t)
                } else {
                    min + t * (max - min)
                }
            }
            CtlKind::Stepper { min, max, .. } => min + t * (max - min),
            CtlKind::Volume => view::travel_to_db(t).max(-60.0),
            _ => t,
        };
        self.clamp(v)
    }

    /// The value as shown, with its unit
    pub fn format(&self, v: f64) -> String {
        if let CtlKind::Volume = self.kind {
            return view::format_db(v);
        }
        let stepped = matches!(self.kind, CtlKind::Stepper { .. }) || matches!(self.slot, Slot::Cc(_));
        match self.unit {
            "Hz" if v >= 1000.0 => format!("{:.2} kHz", v / 1000.0),
            "Hz" if v < 10.0 => format!("{v:.2} Hz"),
            "Hz" => format!("{v:.0} Hz"),
            "ms" if v >= 1000.0 => format!("{:.2} s", v / 1000.0),
            "ms" => format!("{v:.0} ms"),
            "dB" => format!("{v:+.1} dB"),
            "cents" => format!("{v:+.0} ct"),
            "st" => format!("{v:.0} st"),
            "%" => format!("{v:.0} %"),
            "" if stepped => format!("{v:.0}"),
            "" => format!("{v:.2}"),
            u => format!("{v:.2} {u}"),
        }
    }

    /// The edit that sets it
    pub fn op(&self, v: f64) -> EditOp {
        match self.slot {
            Slot::Param(k) => EditOp::set(self.node, &["state", "params", k], Value::real(v).unwrap_or_default()),
            Slot::Field(k) => EditOp::set(self.node, &["state", k], Value::bool(v > 0.5)),
            Slot::TrackVolume => {
                EditOp::set(self.node, &["volume"], Value::real(view::db_to_gain(v).min(4.0)).unwrap_or_default())
            }
            Slot::Wet => EditOp::set(self.node, &["wet"], Value::real(v.clamp(0.0, 1.0)).unwrap_or_default()),
            Slot::Cc(n) => {
                EditOp::set(self.node, &["cc", intern(&format!("c{n}"))], Value::real(v.round()).unwrap_or_default())
            }
        }
    }

    /// The control as a MIDI mapping's target (I-21.2)
    pub fn map_target(&self) -> lm_port::MapTarget {
        match self.slot {
            Slot::Param(k) => crate::ui::midi::param(self.node, k),
            Slot::Field(k) => crate::ui::midi::field(self.node, &["state", k]),
            Slot::TrackVolume => crate::ui::midi::param(self.node, "volume"),
            // lm-doc's parameter key of an insert's mix
            Slot::Wet => crate::ui::midi::param(self.node, "insert.wet"),
            Slot::Cc(_) => crate::ui::midi::param(self.node, self.key()),
        }
    }

    fn op_choice(&self, key: &str) -> EditOp {
        EditOp::set(self.node, &["state", self.key()], Value::text(key))
    }
}

/// An interned `&'static str` (labels made from a plugin's id and a key: few, owned by the symbol table for the
/// life of the program, lm_base::intern)
pub fn intern(text: &str) -> &'static str {
    lm_port::intern(text)
}

/// A translation with fallbacks: the first key that has one, else the fallback as it is
pub fn tr_first(keys: &[&str], fallback: &str) -> SharedString {
    for k in keys {
        let k = intern(k);
        let t = tr(k);
        if t != k {
            return SharedString::from(t);
        }
    }
    SharedString::from(fallback.to_string())
}

/// A parameter's or field's label: `param.<plugin>.<key>`, then `param.<key>`, then the key
pub fn label_of(plugin: &str, key: &str) -> SharedString {
    tr_first(&[&format!("param.{plugin}.{key}"), &format!("param.{key}")], key)
}

/// A choice's label: `choice.<key>.<option>`, then `choice.<option>`, then the option
pub fn choice_label(key: &str, option: &str) -> SharedString {
    tr_first(&[&format!("choice.{key}.{option}"), &format!("choice.{option}")], option)
}

/// A plugin's name: `plugin.<name key>` from `PluginInfo`, else the id
pub fn plugin_name(plugins: &[PluginInfo], id: &str) -> SharedString {
    let key = plugins.iter().find(|p| &*p.id == id).map_or(id, |p| &p.name);
    tr_first(&[&format!("plugin.{key}")], key)
}

/// The schemas the windows read, by plugin (they do not change while the program runs)
#[derive(Default)]
struct Schemas(HashMap<Arc<str>, Option<Arc<NodeSchema>>>);
impl Global for Schemas {}

pub fn schema_of(core: &Entity<Core>, plugin: &Arc<str>, cx: &mut App) -> Option<Arc<NodeSchema>> {
    if let Some(x) = cx.try_global::<Schemas>().and_then(|s| s.0.get(plugin)) {
        return x.clone();
    }
    let got = core.read(cx).port().plugin_schema(plugin).map(Arc::new);
    cx.default_global::<Schemas>().0.insert(plugin.clone(), got.clone());
    got
}

/// The automation lanes of a node's parameters: (owner node, lane id, param key), from the tracks' and the
/// channels' `automation` lists (the automation lives with the owner, base spec item 2)
pub fn lanes_of(root: &Value, node: NodeId) -> Vec<(NodeId, NodeId, Arc<str>)> {
    let node_text = node.to_string();
    let mut out = Vec::new();
    let mut scan = |owner: &Value| {
        let Some(oid) = model::id_of(owner) else { return };
        for a in model::list(owner, "automation") {
            let Some(t) = a.get("target") else { continue };
            if t.get("node").and_then(Value::as_text) == Some(node_text.as_str())
                && let Some(lid) = model::id_of(a)
            {
                out.push((oid, lid, model::text(t, "param")));
            }
        }
    };
    for t in model::list(root, "tracks") {
        scan(t);
    }
    if let Some(chs) = model::at(root, &["mixer", "channels"]) {
        for ch in chs.items() {
            scan(ch);
        }
    }
    if let Some(m) = model::at(root, &["mixer", "master"]) {
        scan(m);
    }
    out
}

/// Who owns a plugin node: the track of an instrument, the channel of an insert
pub fn owner_of(root: &Value, node: NodeId) -> Option<NodeId> {
    model::tracks(root)
        .into_iter()
        .find(|t| t.instrument.as_ref().is_some_and(|(i, _)| *i == node))
        .map(|t| t.id)
        .or_else(|| model::channels(root).into_iter().find(|c| c.inserts.iter().any(|x| x.id == node)).map(|c| c.id))
        // a track's own parameters (its CC rack's controllers) have their lanes on the track
        .or_else(|| model::tracks(root).into_iter().find(|t| t.id == node).map(|t| t.id))
}

/// The controls of a plugin node from its schema: the parameters, then the choice and bool fields
pub fn controls_of(node: NodeId, plugin_node: &Value, schema: &NodeSchema, root: &Value) -> Vec<Ctl> {
    let plugin = model::text(plugin_node, "plugin");
    let state = plugin_node.get("state").cloned().unwrap_or_default();
    let params = state.get("params").cloned().unwrap_or_default();
    let lanes = lanes_of(root, node);
    let mut out = Vec::new();
    for p in &schema.params {
        let kind = if p.scale == Scale::Stepped && p.min == 0.0 && p.max == 1.0 {
            CtlKind::Switch { def: p.def > 0.5 }
        } else if p.scale == Scale::Stepped {
            CtlKind::Stepper { min: p.min, max: p.max, def: p.def }
        } else {
            CtlKind::Knob { min: p.min, max: p.max, def: p.def, log: p.scale == Scale::Log }
        };
        out.push(Ctl {
            node,
            plugin: plugin.clone(),
            slot: Slot::Param(p.key),
            kind,
            unit: p.unit,
            automatable: p.automatable,
            value: params.get(p.key).and_then(Value::as_real).unwrap_or(p.def),
            choice: Arc::from(""),
            automated: lanes.iter().any(|(_, _, k)| &**k == p.key),
            enabled: true,
            shown: None,
        });
    }
    for f in &schema.fields {
        let (kind, value, choice) = match &f.kind {
            FieldKind::Choice(options) => {
                let def: Arc<str> = f
                    .default
                    .as_ref()
                    .and_then(Value::as_shared_text)
                    .cloned()
                    .unwrap_or_else(|| Arc::from(options.first().copied().unwrap_or("")));
                let cur = state.get(f.key).and_then(Value::as_shared_text).cloned().unwrap_or(def);
                (CtlKind::Choice { options }, 0.0, cur)
            }
            FieldKind::Bool => {
                let def = f.default.as_ref().and_then(Value::as_bool).unwrap_or(false);
                let v = state.get(f.key).and_then(Value::as_bool).unwrap_or(def);
                (CtlKind::Switch { def }, f64::from(u8::from(v)), Arc::from(""))
            }
            _ => continue,
        };
        out.push(Ctl {
            node,
            plugin: plugin.clone(),
            slot: Slot::Field(f.key),
            kind,
            unit: "",
            automatable: false,
            value,
            choice,
            automated: false,
            enabled: true,
            shown: None,
        });
    }
    out
}

/// A knob drag in progress: one gesture from mouse down to mouse up
pub struct ParamDrag {
    ctl: Ctl,
    start_y: f32,
    start_t: f64,
    gesture: Gesture,
}

/// A popup of a window: the shared ones (a choice's list, a control's menu) or the window's own
#[derive(Clone, Debug)]
pub enum Popup<C> {
    Choice(Ctl),
    Param(Ctl),
    Own(C),
}

pub struct EditorState<C> {
    pub drag: Option<ParamDrag>,
    pub popup: Option<(Popup<C>, Point<Pixels>)>,
    /// typing a value (double click): the control and its field
    pub typing: Option<(Ctl, Entity<InputState>, Subscription)>,
    /// Shift+drag: a value carried to another control
    pub carried: Option<f64>,
}

impl<C> Default for EditorState<C> {
    fn default() -> Self {
        EditorState { drag: None, popup: None, typing: None, carried: None }
    }
}

/// A window that hosts generic controls
pub trait ParamHost<C: Clone + 'static>: Render + Sized + 'static {
    /// The keymap context of the window's mouse actions ("Instrument", "Effect")
    const KEYMAP: &'static str;
    fn core(&self) -> &Entity<Core>;
    fn editor(&mut self) -> &mut EditorState<C>;
    fn editor_ref(&self) -> &EditorState<C>;
    /// the window's own popup items
    fn own_menu(&self, popup: &C, cx: &Context<Self>) -> Vec<MenuItem<Self>>;
    /// the workspace, for the MIDI binding window
    fn workspace(&self) -> WeakEntity<crate::workspace::Workspace>;
}

/// The value clipboard and the display options of the controls (P-06: linear/log per control is the interface's
/// option, not the schema's)
#[derive(Default)]
struct EditorGlobals {
    clipboard: Option<f64>,
    flipped: HashSet<(Arc<str>, &'static str)>,
}
impl Global for EditorGlobals {}

fn flipped(c_: &Ctl, cx: &App) -> bool {
    cx.try_global::<EditorGlobals>().is_some_and(|g| g.flipped.contains(&(c_.plugin.clone(), c_.key())))
}

/// Applies edits as one undo step. Over the network the answer comes in a later pump (`Waiting`): the window shows
/// the change when the root has it; a refusal comes back as a notice
pub fn apply_ops<V: 'static>(core: &Entity<Core>, label: &'static str, ops: Vec<EditOp>, cx: &mut Context<V>) -> bool {
    let r = core.update(cx, |core, _| {
        let p = core.port_mut().apply(Label::new(label), ops);
        core.settle(p)
    });
    cx.notify();
    matches!(r, Pending::Ready(Ok(_)) | Pending::Waiting(_))
}

/// Runs a core action (presets...) by id
pub fn run_action<V: 'static>(core: &Entity<Core>, action: &str, params: Value, cx: &mut Context<V>) {
    core.update(cx, |core, _| {
        core.report(|pt| pt.run_action(action.into(), params));
    });
    cx.notify();
}

fn current(core: &Core, c_: &Ctl) -> f64 {
    let port = core.port();
    match c_.slot {
        Slot::TrackVolume => model::tracks(port.root())
            .into_iter()
            .find(|t| t.id == c_.node)
            .map_or(0.0, |t| view::gain_to_db(t.volume).max(-60.0)),
        Slot::Param(k) => port
            .find(c_.node)
            .and_then(|n| n.get("state").and_then(|s| s.get("params")).and_then(|p| p.get(k)).and_then(Value::as_real))
            .unwrap_or(c_.value),
        Slot::Wet => port.find(c_.node).and_then(|n| n.get("wet").and_then(Value::as_real)).unwrap_or(1.0),
        Slot::Cc(n) => port
            .find(c_.node)
            .and_then(|t| t.get("cc").and_then(|cc| cc.get(&format!("c{n}"))).and_then(Value::as_real))
            .unwrap_or(c_.value),
        Slot::Field(k) => port
            .find(c_.node)
            .and_then(|n| n.get("state").and_then(|s| s.get(k)).and_then(Value::as_bool))
            .map_or(c_.value, |b| f64::from(u8::from(b))),
    }
}

pub fn begin_drag<C: Clone + 'static, V: ParamHost<C>>(
    v: &mut V,
    c_: Ctl,
    ev: &MouseDownEvent,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    cx.stop_propagation();
    let core = v.core().clone();
    let value = current(core.read(cx), &c_);
    if crate::keymap::fires(V::KEYMAP, "param.carry", ev.button, ev.click_count, &ev.modifiers) {
        // Shift+drag: carry the value to another control (F-CTL-006)
        v.editor().carried = Some(value);
        return;
    }
    if crate::keymap::fires(V::KEYMAP, "param.type", ev.button, ev.click_count, &ev.modifiers) {
        start_typing(v, c_, value, window, cx);
        return;
    }
    let flip = flipped(&c_, cx);
    let prev = v.editor().drag.take();
    let gesture = core.update(cx, |core, _| {
        if let Some(d) = prev {
            core.gesture_commit(d.gesture);
        }
        core.begin_gesture(Label::new("Change a parameter"))
    });
    let start_t = c_.to_t(value, flip);
    v.editor().drag = Some(ParamDrag { ctl: c_, start_y: ev.position.y.as_f32(), start_t, gesture });
    cx.notify();
}

/// On the window's root: the drag's moves (the value reaches the audio's slot at once, and the document by the
/// gesture)
pub fn drag_move<C: Clone + 'static, V: ParamHost<C>>(v: &mut V, ev: &MouseMoveEvent, cx: &mut Context<V>) {
    let Some(d) = &v.editor_ref().drag else { return };
    let c_ = d.ctl.clone();
    // 200 px for the whole travel; Shift: 4x slower (F-CTL-006)
    let slow = if crate::keymap::holds(V::KEYMAP, "param.fine", &ev.modifiers) { 4.0 } else { 1.0 };
    let t = d.start_t - f64::from((ev.position.y.as_f32() - d.start_y) / (200.0 * slow));
    let value = c_.value_at(t, flipped(&c_, cx));
    let core = v.core().clone();
    core.update(cx, |core, _| {
        if matches!(c_.slot, Slot::Param(_) | Slot::Cc(_)) {
            core.port_mut().param_write(c_.node, c_.key(), value as f32);
        }
        if let Some(d) = &v.editor_ref().drag {
            core.gesture_apply(d.gesture, vec![c_.op(value)]);
        }
    });
    cx.notify();
}

/// On the window's root: the end of a drag, or a carried value dropped nowhere
pub fn drag_end<C: Clone + 'static, V: ParamHost<C>>(v: &mut V, cx: &mut Context<V>) {
    let core = v.core().clone();
    if let Some(d) = v.editor().drag.take() {
        core.update(cx, |core, _| core.gesture_commit(d.gesture));
        cx.notify();
    }
    v.editor().carried = None;
}

fn start_typing<C: Clone + 'static, V: ParamHost<C>>(
    v: &mut V,
    c_: Ctl,
    value: f64,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    let text = if matches!(c_.kind, CtlKind::Stepper { .. }) { format!("{value:.0}") } else { format!("{value:.2}") };
    let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
    input.update(cx, |i, cx| i.focus(window, cx));
    let core = v.core().clone();
    let ctl = c_.clone();
    let sub = cx.subscribe(&input, move |this: &mut V, input, ev: &InputEvent, cx| match ev {
        InputEvent::PressEnter { .. } => {
            if let Some(x) = parse_value(&input.read(cx).value()) {
                apply_ops(&core, "Type a value", vec![ctl.op(ctl.clamp(x))], cx);
            }
            this.editor().typing = None;
            cx.notify();
        }
        InputEvent::Blur => {
            this.editor().typing = None;
            cx.notify();
        }
        _ => {}
    });
    v.editor().typing = Some((c_, input, sub));
    cx.notify();
}

/// A typed value, with or without its unit: "1,2 kHz", "350 ms", "-3 dB"
pub fn parse_value(text: &str) -> Option<f64> {
    let t = text.trim().to_lowercase();
    let (num, mul) = match t.strip_suffix("khz") {
        Some(n) => (n.to_string(), 1000.0),
        None => (t.clone(), 1.0),
    };
    let num: String = num.chars().filter(|ch| ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+' | ',')).collect();
    num.replace(',', ".").parse::<f64>().ok().map(|x| x * mul)
}

/// The automation mark, and the MIDI mark when a controller is mapped to it
fn marks(c_: &Ctl, mapped: bool) -> impl IntoElement {
    div()
        .flex()
        .gap(px(2.))
        .h(px(8.))
        .when(c_.automated, |d| d.child(div().size(px(6.)).mt(px(1.)).rounded(px(3.)).bg(c(THEME.orange))))
        .when(mapped, |d| d.child(div().text_size(px(7.5)).text_color(c(THEME.cyan)).child("MIDI")))
}

fn ctl_id(c_: &Ctl) -> SharedString {
    SharedString::from(format!("{}-{}", c_.node, c_.key()))
}

/// One control by its kind
pub fn control<C: Clone + 'static, V: ParamHost<C>>(this: &V, c_: Ctl, cx: &Context<V>) -> AnyElement {
    let id = ctl_id(&c_);
    let sel = {
        let id = id.clone();
        move || format!("ctl-{id}")
    };
    let label = match c_.slot {
        Slot::TrackVolume => SharedString::from(tr("Volume")),
        Slot::Wet => SharedString::from(tr("W/D")),
        Slot::Cc(n) => SharedString::from(format!("CC {n}")),
        _ => label_of(&c_.plugin, c_.key()),
    };
    let value_color = if c_.automated { THEME.orange } else { THEME.text };
    let mapped = !crate::ui::midi::mappings_of(this.core().read(cx).port(), &c_.map_target()).is_empty();
    let typing = this
        .editor_ref()
        .typing
        .as_ref()
        .filter(|(t, _, _)| t.node == c_.node && t.slot == c_.slot)
        .map(|t| t.1.clone());
    let value_text: AnyElement = match typing {
        Some(input) => div().w(px(64.)).child(crate::ui::text_entry(Input::new(&input).small())).into_any_element(),
        None => div()
            .text_size(px(10.))
            .text_color(c(value_color))
            .whitespace_nowrap()
            .child(c_.shown.clone().unwrap_or_else(|| c_.format(c_.value)))
            .into_any_element(),
    };
    let small_label = |l: SharedString| div().text_size(px(9.5)).text_color(c(THEME.text2)).child(l);
    let body = match c_.kind.clone() {
        CtlKind::Stepper { min, max, .. } => {
            let step = |d: f64, s: &'static str| {
                let cc = c_.clone();
                button(SharedString::from(format!("{id}-{s}")), s, false, 0).on_click(cx.listener(
                    move |v: &mut V, _, _, cx| {
                        let core = v.core().clone();
                        let now = current(core.read(cx), &cc);
                        apply_ops(&core, "Change a parameter", vec![cc.op(fit(now + d, min, max))], cx);
                    },
                ))
            };
            let cc = c_.clone();
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .min_w(px(64.))
                .child(div().flex().gap(px(4.)).child(small_label(label)).child(marks(&c_, mapped)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(3.))
                        .child(step(-1.0, "−"))
                        .child(
                            div()
                                .id(SharedString::from(format!("{id}-v")))
                                .debug_selector(sel)
                                .min_w(px(34.))
                                .h(px(18.))
                                .px(px(4.))
                                .rounded(px(3.))
                                .bg(c(THEME.lcd_bg))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_ns_resize()
                                .child(value_text)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |v: &mut V, ev: &MouseDownEvent, w, cx| {
                                        begin_drag(v, cc.clone(), ev, w, cx)
                                    }),
                                ),
                        )
                        .child(step(1.0, "+")),
                )
                .into_any_element()
        }
        CtlKind::Knob { .. } | CtlKind::Volume => {
            let t = c_.to_t(c_.value, flipped(&c_, cx));
            let cc = c_.clone();
            let cw = c_.clone();
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(1.))
                .w(px(64.))
                .child(marks(&c_, mapped))
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-k")))
                        .debug_selector(sel)
                        .cursor_ns_resize()
                        .child(knob_face(t, 28.0))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |v: &mut V, ev: &MouseDownEvent, w, cx| {
                                begin_drag(v, cc.clone(), ev, w, cx)
                            }),
                        )
                        .on_scroll(cx.listener(move |v: &mut V, ev: &ScrollEvent, _, cx| {
                            // the wheel: 100 steps for the whole travel; Shift 10, Ctrl/Cmd 1000 (F-CTL-006)
                            let dy = ev.delta.pixel_delta(px(16.)).y.as_f32();
                            if dy == 0.0 {
                                return;
                            }
                            let ids = ["param.wheel_coarse", "param.wheel_fine", "param.wheel"];
                            let steps = match crate::keymap::pick_wheel(V::KEYMAP, &ids, &ev.modifiers) {
                                Some("param.wheel_coarse") => 10.0,
                                Some("param.wheel_fine") => 1000.0,
                                Some(_) => 100.0,
                                None => return,
                            };
                            let core = v.core().clone();
                            let flip = flipped(&cw, cx);
                            let now = current(core.read(cx), &cw);
                            let t = cw.to_t(now, flip) + f64::from(dy.signum()) / steps;
                            apply_ops(&core, "Change a parameter", vec![cw.op(cw.value_at(t, flip))], cx);
                            cx.stop_propagation();
                        })),
                )
                .child(small_label(label).max_w(px(62.)).overflow_hidden().whitespace_nowrap())
                .child(value_text)
                .into_any_element()
        }
        CtlKind::Switch { .. } => {
            let on = c_.value > 0.5;
            let cc = c_.clone();
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .min_w(px(64.))
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-b")))
                        .debug_selector(sel)
                        .w(px(24.))
                        .h(px(13.))
                        .rounded(px(7.))
                        .bg(c(if on { THEME.green } else { THEME.bg4 }))
                        .cursor_pointer()
                        .child(
                            div()
                                .size(px(9.))
                                .mt(px(2.))
                                .ml(px(if on { 13. } else { 2. }))
                                .rounded(px(5.))
                                .bg(c(THEME.text)),
                        )
                        .on_click(cx.listener(move |v: &mut V, _, _, cx| {
                            let core = v.core().clone();
                            apply_ops(&core, "Switch", vec![cc.op(if on { 0.0 } else { 1.0 })], cx);
                        })),
                )
                .child(div().text_size(px(10.5)).text_color(c(if on { THEME.text } else { THEME.text2 })).child(label))
                .child(marks(&c_, mapped))
                .into_any_element()
        }
        CtlKind::Choice { .. } => {
            let cc = c_.clone();
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .min_w(px(84.))
                .child(div().flex().gap(px(4.)).child(small_label(label)).child(marks(&c_, mapped)))
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-c")))
                        .debug_selector(sel)
                        .h(px(20.))
                        .px(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .rounded(px(4.))
                        .bg(c(THEME.bg3))
                        .border_1()
                        .border_color(c(THEME.line2))
                        .text_size(px(11.))
                        .cursor_pointer()
                        .child(choice_label(c_.key(), &c_.choice))
                        .child(div().text_color(c(THEME.text3)).child("▾"))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |v: &mut V, ev: &MouseDownEvent, _, cx| {
                                v.editor().popup = Some((Popup::Choice(cc.clone()), ev.position));
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        ),
                )
                .into_any_element()
        }
    };
    // the gestures every control shares: the menu, the middle click, a carried value dropped here
    let (cm, cd, cu) = (c_.clone(), c_.clone(), c_.clone());
    let wrapped = div()
        .id(SharedString::from(format!("{id}-w")))
        .child(body)
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |v: &mut V, ev: &MouseDownEvent, _, cx| {
                v.editor().popup = Some((Popup::Param(cm.clone()), ev.position));
                cx.stop_propagation();
                cx.notify();
            }),
        )
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |v: &mut V, _, _, cx| {
                if !matches!(cd.kind, CtlKind::Choice { .. }) {
                    let core = v.core().clone();
                    apply_ops(&core, "Back to the default", vec![cd.op(cd.default_value())], cx);
                }
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |v: &mut V, _: &MouseUpEvent, _, cx| {
                if let Some(x) = v.editor().carried.take()
                    && !matches!(cu.kind, CtlKind::Choice { .. })
                {
                    let core = v.core().clone();
                    apply_ops(&core, "Copy a value", vec![cu.op(cu.clamp(x))], cx);
                }
            }),
        );
    if c_.enabled { wrapped.into_any_element() } else { wrapped.opacity(0.4).into_any_element() }
}

/// A card of controls with its title
pub fn group_card(title: impl Into<SharedString>, controls: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(8.))
        .rounded(px(6.))
        .bg(c(THEME.bg2))
        .child(div().text_size(px(11.)).font_weight(FontWeight::BOLD).text_color(c(THEME.text2)).child(title.into()))
        .child(div().flex().flex_wrap().gap(px(8.)).items_end().children(controls))
        .into_any_element()
}

/// The controls in cards by the first dotted segment of their keys (`osc1.vol` → `osc1`), in schema order
pub fn grouped<C: Clone + 'static, V: ParamHost<C>>(
    this: &V,
    ctls: Vec<Ctl>,
    title: &str,
    cx: &Context<V>,
) -> Vec<AnyElement> {
    let mut groups: Vec<(String, Vec<Ctl>)> = Vec::new();
    for k in ctls {
        let g = k.key().split_once('.').map_or(String::new(), |(g, _)| g.to_string());
        match groups.iter_mut().find(|(x, _)| *x == g) {
            Some((_, v)) => v.push(k),
            None => groups.push((g, vec![k])),
        }
    }
    groups
        .into_iter()
        .map(|(g, ks)| {
            let t = if g.is_empty() {
                SharedString::from(title.to_string())
            } else {
                tr_first(&[&format!("group.{g}")], &g)
            };
            group_card(t, ks.into_iter().map(|k| control(this, k, cx)).collect())
        })
        .collect()
}

/// What a popup item does
pub type MenuAction<V> = Rc<dyn Fn(&mut V, &mut Window, &mut Context<V>)>;

/// A popup item
pub struct MenuItem<V> {
    pub label: SharedString,
    pub checked: bool,
    pub enabled: bool,
    pub action: MenuAction<V>,
}

impl<V> MenuItem<V> {
    pub fn new(
        label: impl Into<SharedString>,
        action: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    ) -> Self {
        MenuItem { label: label.into(), checked: false, enabled: true, action: Rc::new(action) }
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    /// a line that is not clickable (a heading)
    pub fn heading(label: impl Into<SharedString>) -> Self {
        MenuItem { label: label.into(), checked: false, enabled: false, action: Rc::new(|_, _, _| {}) }
    }
}

fn shared_menu<C: Clone + 'static, V: ParamHost<C>>(this: &V, p: &Popup<C>, cx: &Context<V>) -> Vec<MenuItem<V>> {
    match p {
        Popup::Choice(c_) => {
            let CtlKind::Choice { options } = &c_.kind else { return Vec::new() };
            options
                .iter()
                .map(|o| {
                    let cc = c_.clone();
                    let o: &'static str = o;
                    MenuItem::new(choice_label(c_.key(), o), move |v: &mut V, _, cx| {
                        let core = v.core().clone();
                        apply_ops(&core, "Choose", vec![cc.op_choice(o)], cx);
                    })
                    .checked(*c_.choice == *o)
                })
                .collect()
        }
        Popup::Param(c_) => {
            let is_choice = matches!(c_.kind, CtlKind::Choice { .. });
            let def = c_.default_value();
            let clip = cx.try_global::<EditorGlobals>().and_then(|g| g.clipboard);
            let (c1, c2, c3) = (c_.clone(), c_.clone(), c_.clone());
            let paste = match clip {
                Some(x) => trf("Paste the value ({0})", &[&c_.format(c_.clamp(x))]),
                None => tr("Paste the value").to_string(),
            };
            let mut items = vec![
                MenuItem::new(trf("Back to the default ({0})", &[&c_.format(def)]), move |v: &mut V, _, cx| {
                    let core = v.core().clone();
                    apply_ops(&core, "Back to the default", vec![c1.op(def)], cx);
                })
                .enabled(!is_choice),
                MenuItem::new(tr("Copy the value"), move |v: &mut V, _, cx| {
                    let x = current(v.core().read(cx), &c2);
                    cx.default_global::<EditorGlobals>().clipboard = Some(x);
                })
                .enabled(!is_choice),
                MenuItem::new(paste, move |v: &mut V, _, cx| {
                    if let Some(x) = clip {
                        let core = v.core().clone();
                        apply_ops(&core, "Paste a value", vec![c3.op(c3.clamp(x))], cx);
                    }
                })
                .enabled(clip.is_some() && !is_choice),
            ];
            if let Slot::Param(_) | Slot::Cc(_) = c_.slot {
                let key = c_.key();
                let port = this.core().read(cx).port();
                let root = port.root();
                let lane = lanes_of(root, c_.node).into_iter().find(|(_, _, k)| &**k == key);
                let owner = owner_of(root, c_.node);
                let node = c_.node;
                let count = owner.and_then(|o| port.find(o)).map_or(0, |o| model::list(&o, "automation").count());
                items.push(match lane {
                    Some((_, lid, _)) => MenuItem::new(tr("Remove the automation lane"), move |v: &mut V, _, cx| {
                        let core = v.core().clone();
                        apply_ops(&core, "Remove the automation", vec![EditOp::Remove { id: Ref::Id(lid) }], cx);
                    }),
                    None => MenuItem::new(tr("Automate (add a lane)"), move |v: &mut V, _, cx| {
                        let Some(owner) = owner else { return };
                        let target =
                            Value::object([("node", Value::text(node.to_string())), ("param", Value::text(key))]);
                        let lane =
                            Value::object([("target", target), ("on", Value::bool(true)), ("points", Value::list([]))]);
                        let op = EditOp::Insert {
                            temp: 1,
                            parent: Ref::Id(owner),
                            list: "automation".into(),
                            index: count,
                            node: lane,
                        };
                        let core = v.core().clone();
                        apply_ops(&core, "Automate", vec![op], cx);
                    })
                    .enabled(c_.automatable && owner.is_some()),
                });
            }
            // MIDI learn, remove the mapping, the binding window (I-21.2; F-CTL-004/005)
            let target = c_.map_target();
            let mapped = !crate::ui::midi::mappings_of(this.core().read(cx).port(), &target).is_empty();
            let (t1, t2, t3) = (target.clone(), target.clone(), target);
            items.push(MenuItem::new(tr("MIDI learn"), move |v: &mut V, _, cx| {
                let core = v.core().clone();
                crate::ui::midi::learn(&core, t1.clone(), lm_port::MapScope::Project, cx);
            }));
            if mapped {
                items.push(MenuItem::new(tr("Remove MIDI mapping"), move |v: &mut V, _, cx| {
                    let core = v.core().clone();
                    crate::ui::midi::unmap(&core, &t2, cx);
                }));
            }
            if let lm_port::MapTarget::Param { .. } = &t3 {
                let t4 = t3.clone();
                items.push(MenuItem::new(tr("Modulate…"), move |v: &mut V, window, cx| {
                    let (ws, core) = (v.workspace(), v.core().clone());
                    crate::ui::midi::open_modulator(&core, &ws, t4.clone(), window, cx);
                }));
            }
            items.push(MenuItem::new(tr("MIDI binding…"), move |v: &mut V, window, cx| {
                let ws = v.workspace();
                crate::ui::midi::open_binding(&ws, t3.clone(), window, cx);
            }));
            if let CtlKind::Knob { min, log, .. } = c_.kind
                && min > 0.0
            {
                let on = flipped(c_, cx);
                let key = (c_.plugin.clone(), c_.key());
                items.push(
                    MenuItem::new(tr("Logarithmic scale"), move |_: &mut V, _, cx| {
                        let g = cx.default_global::<EditorGlobals>();
                        if !g.flipped.remove(&key) {
                            g.flipped.insert(key.clone());
                        }
                        cx.notify();
                    })
                    .checked(log != on),
                );
            }
            items
        }
        Popup::Own(o) => this.own_menu(o, cx),
    }
}

/// The popup layer of a window: over everything, at the mouse; a click outside closes it
pub fn popup_layer<C: Clone + 'static, V: ParamHost<C>>(this: &V, cx: &Context<V>) -> Option<AnyElement> {
    let (p, at) = this.editor_ref().popup.clone()?;
    let items = shared_menu(this, &p, cx);
    Some(
        deferred(
            anchored().position(at).snap_to_window_with_margin(px(8.)).child(
                div()
                    .id("popup")
                    .debug_selector(|| "popup".into())
                    .occlude()
                    .min_w(px(170.))
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .py(px(4.))
                    .rounded(px(6.))
                    .bg(c(THEME.bg3))
                    .border_1()
                    .border_color(c(THEME.line2))
                    .shadow_lg()
                    .on_mouse_down_out(cx.listener(|v: &mut V, _, _, cx| {
                        v.editor().popup = None;
                        cx.notify();
                    }))
                    .children(items.into_iter().enumerate().map(|(k, it)| {
                        let action = it.action.clone();
                        let enabled = it.enabled;
                        div()
                            .id(("popup-item", k))
                            .debug_selector(move || format!("popup-item-{k}"))
                            .px(px(10.))
                            .py(px(3.))
                            .flex()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .text_color(c(if enabled { THEME.text } else { THEME.text3 }))
                            .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(c(THEME.bg4))))
                            .child(div().w(px(10.)).text_color(c(THEME.green)).child(if it.checked {
                                "✓"
                            } else {
                                ""
                            }))
                            .child(it.label)
                            .on_click(cx.listener(move |v: &mut V, _, window, cx| {
                                if enabled {
                                    v.editor().popup = None;
                                    action(v, window, cx);
                                    cx.notify();
                                }
                            }))
                    })),
            ),
        )
        .with_priority(1)
        .into_any_element(),
    )
}

/// The loading component's "Cancel": the operations a target's loading waits on
pub fn cancel_target(core: &Entity<Core>, target: Target, cx: &mut App) {
    core.update(cx, |core, _| {
        let ops: Vec<_> = core.port().activity().into_iter().filter(|a| a.target == target).map(|a| a.id).collect();
        for op in ops {
            core.port_mut().cancel(op);
        }
    });
}

/// The chip of a plugin: its initials, coloured by kind (purple: third-party, outside the program)
pub fn plugin_chip(info: Option<&PluginInfo>, id: &str) -> impl IntoElement {
    let initials: String =
        id.chars().filter(|ch| ch.is_ascii_alphanumeric()).take(2).collect::<String>().to_uppercase();
    let color = match info {
        Some(i) if i.third_party => THEME.purple,
        Some(i) if i.instrument => THEME.blue,
        Some(_) => THEME.cyan,
        None => THEME.red,
    };
    div()
        .px(px(4.))
        .h(px(15.))
        .flex()
        .items_center()
        .rounded(px(3.))
        .bg(c(color))
        .text_color(c(THEME.on_accent))
        .text_size(px(9.))
        .font_weight(FontWeight::BOLD)
        .child(initials)
}

/// A message in a window (an instrument a tab does not apply to)
pub fn notice(text: impl Into<SharedString>) -> AnyElement {
    div()
        .p(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(c(THEME.yellow))
        .text_size(px(11.))
        .text_color(c(THEME.yellow))
        .child(text.into())
        .into_any_element()
}

// =================================================================================================================
// The instrument window
// =================================================================================================================

/// The instrument functions' keys (lm-plugin `instrument::schema`): they go to their own tabs
const ENV_KEYS: [&str; 13] = [
    "Predelay",
    "Attack",
    "Hold",
    "Decay",
    "Sustain",
    "Release",
    "Amount",
    "LfoPredelay",
    "LfoAttack",
    "LfoSpeed",
    "LfoAmount",
    "LfoX100",
    "LfoEnvAmount",
];
const FILTER_KEYS: [&str; 4] = ["filterOn", "filterType", "cutoff", "resonance"];
const VOICE_KEYS: [&str; 2] = ["voiceMode", "glide"];
const TUNING_KEYS: [&str; 2] = ["pitch", "bendRange"];

fn is_function_key(k: &str) -> bool {
    let env = ["vol", "cut", "res"].iter().any(|t| {
        k.strip_prefix(t)
            .is_some_and(|rest| ENV_KEYS.contains(&rest) || matches!(rest, "LfoShape" | "LfoSync" | "LfoWave"))
    });
    env || FILTER_KEYS.contains(&k) || VOICE_KEYS.contains(&k) || TUNING_KEYS.contains(&k)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Instrument,
    Envelopes,
    NoteEffects,
    Effects,
    Midi,
    Tuning,
}

impl Tab {
    pub const ALL: [Tab; 6] = [Tab::Instrument, Tab::Envelopes, Tab::NoteEffects, Tab::Effects, Tab::Midi, Tab::Tuning];
    fn title(self) -> &'static str {
        match self {
            Tab::Instrument => "Instrument",
            Tab::Envelopes => "Envelopes and filter",
            Tab::NoteEffects => "Note effects",
            Tab::Effects => "Effects",
            Tab::Midi => "MIDI",
            Tab::Tuning => "Tuning",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum IwPopup {
    Tracks,
    Swap,
    Presets,
    /// where a track output goes, by its index
    Output(usize),
}

/// A preset of the browser service: its entry, its name, whether it is the factory's
pub type PresetEntry = (lm_port::EntryId, Arc<str>, bool);

/// A note's name (MIDI numbering, C4 = 60)
pub fn note_name(k: f64) -> String {
    const N: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let k = k.round() as i32;
    format!("{}{}", N[k.rem_euclid(12) as usize], k.div_euclid(12) - 1)
}

/// The last preset loaded on a node, as the window shows it (the document keeps no preset's name: a preset is a
/// state, piece 7a)
#[derive(Default)]
struct PresetNames(HashMap<NodeId, Arc<str>>);
impl Global for PresetNames {}

fn preset_name(node: NodeId, cx: &App) -> Option<Arc<str>> {
    cx.try_global::<PresetNames>().and_then(|n| n.0.get(&node).cloned())
}

pub struct InstrumentWindow {
    scx: ScreenCx,
    track: Option<NodeId>,
    feed: Option<Feed>,
    tab: Tab,
    /// the envelope's target: volume, cutoff, resonance
    target: usize,
    keyboard: bool,
    held: Option<u8>,
    /// the notes this window has sounding (I-15), released on mouse-up, focus loss and close
    live: Live,
    /// the computer keys held, by key name
    played: HashMap<String, u8>,
    /// the window's focus: the computer keyboard plays the instrument while it has it (F-IJN-022)
    focus: FocusHandle,
    keys_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    kit_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    lit: Option<(usize, Instant)>,
    /// the presets of the current plugin (the browser service's Presets page), refreshed when the list opens
    presets: Vec<PresetEntry>,
    editor: EditorState<IwPopup>,
    /// The plugin's own panel (I-22, lm-gui-plugins), for the instrument it was made for
    panel: Option<(NodeId, Arc<str>, AnyView, Subscription)>,
    _subs: Vec<Subscription>,
}

impl ParamHost<IwPopup> for InstrumentWindow {
    const KEYMAP: &'static str = "Instrument";

    fn core(&self) -> &Entity<Core> {
        &self.scx.core
    }
    fn editor(&mut self) -> &mut EditorState<IwPopup> {
        &mut self.editor
    }
    fn editor_ref(&self) -> &EditorState<IwPopup> {
        &self.editor
    }
    fn workspace(&self) -> WeakEntity<crate::workspace::Workspace> {
        self.scx.workspace.clone()
    }
    fn own_menu(&self, p: &IwPopup, cx: &Context<Self>) -> Vec<MenuItem<Self>> {
        let port = self.scx.core.read(cx).port();
        let root = port.root();
        let Some(track) = self.track else { return Vec::new() };
        let tracks = model::tracks(root);
        match p {
            IwPopup::Tracks => tracks
                .iter()
                .filter(|t| t.instrument.is_some())
                .map(|t| {
                    let id = t.id;
                    MenuItem::new(t.name.to_string(), move |w: &mut Self, _, cx| w.set_track(Some(id), cx))
                        .checked(id == track)
                })
                .collect(),
            IwPopup::Swap => {
                let cur = tracks.iter().find(|t| t.id == track).and_then(|t| t.instrument.clone()).map(|(_, p)| p);
                let plugins = port.plugins();
                let mut list: Vec<&PluginInfo> = plugins.iter().filter(|p| p.instrument).collect();
                list.sort_by_key(|p| (p.third_party, p.category.clone(), p.id.clone()));
                let mut items = Vec::new();
                let mut last: Option<Arc<str>> = None;
                for p in list {
                    if last.as_ref() != Some(&p.category) {
                        items.push(MenuItem::heading(tr_first(&[&format!("category.{}", p.category)], &p.category)));
                        last = Some(p.category.clone());
                    }
                    let id = p.id.clone();
                    let checked = cur.as_deref() == Some(&*id);
                    items.push(
                        MenuItem::new(plugin_name(&plugins, &p.id), move |w: &mut Self, _, cx| w.swap(&id, cx))
                            .checked(checked),
                    );
                }
                items
            }
            IwPopup::Presets => {
                let Some((node, _)) = tracks.iter().find(|t| t.id == track).and_then(|t| t.instrument.clone()) else {
                    return Vec::new();
                };
                let cur = preset_name(node, cx);
                let mut items = vec![MenuItem::heading(tr("Factory presets"))];
                let mut user = false;
                for (entry, name, factory) in &self.presets {
                    if !factory && !user {
                        items.push(MenuItem::heading(tr("Your presets")));
                        user = true;
                    }
                    let (entry, nm) = (*entry, name.clone());
                    items.push(
                        MenuItem::new(name.to_string(), move |w: &mut Self, _, cx| {
                            w.load_preset(node, entry, nm.clone(), cx)
                        })
                        .checked(cur.as_deref() == Some(&**name)),
                    );
                }
                if self.presets.is_empty() {
                    items.push(MenuItem::heading(tr("No presets")));
                }
                items
            }
            IwPopup::Output(k) => {
                let Some(t) = tracks.iter().find(|t| t.id == track) else { return Vec::new() };
                let Some(o) = t.outputs.get(*k) else { return Vec::new() };
                let (output, to) = (o.id, o.to);
                model::channels(root)
                    .into_iter()
                    .map(|ch| {
                        let channel = ch.id;
                        MenuItem::new(ch.name.to_string(), move |w: &mut Self, _, cx| {
                            let core = w.scx.core.clone();
                            let op = EditOp::set(output, &["to"], Value::text(channel.to_string()));
                            apply_ops(&core, "Route an output", vec![op], cx);
                        })
                        .checked(channel == to)
                    })
                    .collect()
            }
        }
    }
}

impl InstrumentWindow {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&scx.core, |_, _, cx| cx.notify());
        // the notes end when the window closes
        let core = scx.core.clone();
        let release = cx.on_release(move |w, cx| w.live.release_all(&core, cx));
        // the target: a track, or its instrument; else the first track with an instrument
        let first = {
            let tracks = model::tracks(scx.core.read(cx).port().root());
            scx.target
                .and_then(|t| tracks.iter().find(|x| x.id == t || x.instrument.as_ref().is_some_and(|(i, _)| *i == t)))
                .or_else(|| tracks.iter().find(|t| t.instrument.is_some()))
                .map(|t| t.id)
        };
        // the tab it showed last time (the interface document)
        let tab = {
            let ui = scx.core.read(cx).port().ui();
            let want = ui.get("instrument").and_then(|i| i.get("tab")).and_then(lm_port::Value::as_text);
            Tab::ALL.into_iter().find(|t| Some(format!("{t:?}").as_str()) == want).unwrap_or(Tab::Instrument)
        };
        let mut w = InstrumentWindow {
            scx,
            track: None,
            feed: None,
            tab,
            target: 0,
            keyboard: false,
            held: None,
            live: Live::default(),
            played: HashMap::new(),
            focus: cx.focus_handle(),
            keys_bounds: Rc::new(Cell::new(None)),
            kit_bounds: Rc::new(Cell::new(None)),
            lit: None,
            presets: Vec::new(),
            editor: EditorState::default(),
            panel: None,
            _subs: vec![observe, release],
        };
        // the notes end when the focus or the window's activation is lost
        let blur = cx.on_blur(&w.focus, window, |w, _, cx| w.release_notes(cx));
        let active = cx.observe_window_activation(window, |w, window, cx| {
            if !window.is_window_active() {
                w.release_notes(cx);
            }
        });
        w._subs.extend([blur, active]);
        window.focus(&w.focus, cx);
        w.set_track(first, cx);
        w
    }

    /// Keeps the panel of the instrument shown, remade when the node or the plugin changes (I-22)
    fn update_panel(&mut self, instrument: Option<(NodeId, Arc<str>)>, window: &mut Window, cx: &mut Context<Self>) {
        let want = instrument.filter(|(_, p)| lm_gui_plugins::has_panel(p));
        if self.panel.as_ref().map(|(n, p, ..)| (*n, p.clone())) == want {
            return;
        }
        self.panel = want.and_then(|(node, plugin)| {
            let entity = lm_gui_plugins::open_entity(&plugin, node, self.scx.core.clone(), cx)?;
            let workspace = self.scx.workspace.clone();
            let core = self.scx.core.clone();
            let sub =
                cx.subscribe_in(&entity, window, move |_, _, e: &lm_gui_plugins::PanelEvent, window, cx| match e {
                    lm_gui_plugins::PanelEvent::OpenBinding(target) => {
                        crate::ui::midi::open_binding(&workspace, target.clone(), window, cx)
                    }
                    lm_gui_plugins::PanelEvent::OpenModulator(target) => {
                        crate::ui::midi::open_modulator(&core, &workspace, target.clone(), window, cx)
                    }
                });
            Some((node, plugin, entity.into(), sub))
        });
    }

    /// The tab is kept in the interface document (no undo, saved with the project)
    fn remember_tab(&self, cx: &mut Context<Self>) {
        let v = lm_port::Value::text(format!("{:?}", self.tab));
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| pt.set_ui(vec![lm_port::PathStep::key("instrument"), lm_port::PathStep::key("tab")], v));
        });
    }

    /// Ends every live note this window started
    fn release_notes(&mut self, cx: &mut Context<Self>) {
        let core = self.scx.core.clone();
        self.live.release_all(&core, cx);
        self.held = None;
        cx.notify();
    }

    /// A live note on the window's track (the test keyboard, a kit piece)
    fn play(&mut self, key: u8, velocity: u8, cx: &mut Context<Self>) {
        if let Some(t) = self.track {
            let core = self.scx.core.clone();
            self.live.on(&core, t, key, velocity, cx);
        }
    }

    fn stop(&mut self, key: u8, cx: &mut Context<Self>) {
        if let Some(t) = self.track {
            let core = self.scx.core.clone();
            self.live.off(&core, t, key, cx);
        }
    }

    /// The computer keyboard as a piano (F-IJN-022, the legacy's layout), only while the window itself has the focus
    /// (a text field inside keeps its keys)
    fn key_down(&mut self, ev: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        // keymap-ok: the computer keyboard as a piano is a key down and up pair that sounds a note, not a command
        let m = ev.keystroke.modifiers;
        if ev.is_held || m.control || m.alt || m.platform || m.shift || !self.focus.is_focused(window) {
            return;
        }
        if let Some(k) = live::window_key(&ev.keystroke.key) {
            self.played.insert(ev.keystroke.key.clone(), k);
            self.play(k, live::DEFAULT_VELOCITY, cx);
            self.held = Some(k);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn key_up(&mut self, ev: &KeyUpEvent, cx: &mut Context<Self>) {
        // keymap-ok: the piano keys
        if let Some(k) = self.played.remove(&ev.keystroke.key) {
            self.stop(k, cx);
            if self.held == Some(k) {
                self.held = None;
            }
            cx.notify();
        }
    }

    fn set_track(&mut self, track: Option<NodeId>, cx: &mut Context<Self>) {
        if self.track != track {
            self.release_notes(cx);
        }
        self.track = track;
        if let Some(t) = track {
            cx.set_global(super::browser::ActiveTrack(t));
        }
        self.feed = track.map(|t| self.scx.core.update(cx, |core, _| core.port_mut().feed(Topic::Node(t))));
        cx.notify();
    }

    fn track_view(&self, cx: &App) -> Option<TrackView> {
        let id = self.track?;
        model::tracks(self.scx.core.read(cx).port().root()).into_iter().find(|t| t.id == id)
    }

    /// The previous or next instrument track (F-IJN-001)
    fn step_track(&mut self, d: i32, cx: &mut Context<Self>) {
        let list: Vec<NodeId> = model::tracks(self.scx.core.read(cx).port().root())
            .into_iter()
            .filter(|t| t.instrument.is_some())
            .map(|t| t.id)
            .collect();
        if list.is_empty() {
            return;
        }
        let at = self.track.and_then(|t| list.iter().position(|x| *x == t)).unwrap_or(0) as i32;
        let next = list[(at + d).rem_euclid(list.len() as i32) as usize];
        self.set_track(Some(next), cx);
    }

    /// Swaps the instrument without removing the track (design 05): a new node, whose id the document gives; the
    /// outputs follow its ports (I-3.9)
    fn swap(&mut self, plugin: &str, cx: &mut Context<Self>) {
        let Some(track) = self.track else { return };
        let node = Value::object([("plugin", Value::text(plugin)), ("state", Value::empty_object())]);
        let core = self.scx.core.clone();
        apply_ops(&core, "Swap the instrument", vec![EditOp::set(track, &["instrument"], node)], cx);
    }

    /// The presets of a plugin, from the browser service's Presets page (Waiting keeps the last list)
    fn refresh_presets(&mut self, plugin: &str, cx: &mut Context<Self>) {
        let r = self.scx.core.update(cx, |core, _| core.port_mut().browse(BrowsePage::Presets, None, false));
        if let Pending::Ready(Ok(entries)) = r {
            let mut v: Vec<PresetEntry> = entries
                .into_iter()
                .filter(|e| e.info.get("plugin").and_then(Value::as_text) == Some(plugin))
                .map(|e| (e.id, e.name.clone(), e.factory))
                .collect();
            v.sort_by_key(|(_, n, f)| (!f, n.to_lowercase()));
            self.presets = v;
        }
    }

    fn load_preset(&mut self, node: NodeId, entry: lm_port::EntryId, name: Arc<str>, cx: &mut Context<Self>) {
        let params = Value::object([("node", Value::text(node.to_string())), ("entry", Value::int(entry.0 as i64))]);
        let core = self.scx.core.clone();
        run_action(&core, "plugin.load_preset", params, cx);
        cx.default_global::<PresetNames>().0.insert(node, name);
    }

    fn step_preset(&mut self, d: i32, cx: &mut Context<Self>) {
        let Some((node, plugin)) = self.track_view(cx).and_then(|t| t.instrument) else { return };
        self.refresh_presets(&plugin, cx);
        if self.presets.is_empty() {
            return;
        }
        let cur = preset_name(node, cx);
        let at = cur
            .and_then(|c| self.presets.iter().position(|p| p.1 == c))
            .map_or(if d > 0 { -1 } else { 0 }, |k| k as i32);
        let (entry, name, _) = self.presets[(at + d).rem_euclid(self.presets.len() as i32) as usize].clone();
        self.load_preset(node, entry, name, cx);
    }

    fn save_preset(&mut self, cx: &mut Context<Self>) {
        let Some((node, plugin)) = self.track_view(cx).and_then(|t| t.instrument) else { return };
        self.refresh_presets(&plugin, cx);
        let base = preset_name(node, cx).map_or_else(|| tr("Preset").to_string(), |n| n.to_string());
        let base = base.trim_end_matches(|ch: char| ch.is_ascii_digit() || ch == ' ').to_string();
        let name =
            (1..).map(|k| format!("{base} {k}")).find(|n| !self.presets.iter().any(|p| &*p.1 == n)).unwrap_or(base);
        let params = Value::object([("node", Value::text(node.to_string())), ("name", Value::text(name.as_str()))]);
        let core = self.scx.core.clone();
        run_action(&core, "plugin.save_preset", params, cx);
        cx.default_global::<PresetNames>().0.insert(node, Arc::from(name.as_str()));
    }

    // --- header, tabs, footer ---------------------------------------------------------------------------------

    fn header(&self, t: &TrackView, plugins: &[PluginInfo], cx: &Context<Self>) -> impl IntoElement {
        let (mute, solo, tid) = (t.mute, t.solo, t.id);
        let color = crate::ui::track_color(t.index);
        let plugin = t.instrument.as_ref().map_or_else(|| Arc::from(""), |(_, p)| p.clone());
        let info = plugins.iter().find(|p| p.id == plugin);
        let preset = t
            .instrument
            .as_ref()
            .and_then(|(n, _)| preset_name(*n, cx))
            .map_or_else(|| tr("No preset").to_string(), |p| p.to_string());
        let line = || div().flex().items_center().gap(px(6.)).px(px(10.));
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .pt(px(8.))
            .pb(px(6.))
            .bg(c(THEME.bg2))
            .border_b_1()
            .border_color(c(THEME.line))
            .child(
                line()
                    .child(div().w(px(4.)).h(px(20.)).rounded(px(2.)).bg(c(color)))
                    .child(
                        div()
                            .id("iw-track")
                            .debug_selector(|| "iw-track".into())
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(15.))
                            .font_weight(FontWeight::BOLD)
                            .cursor_pointer()
                            .child(t.name.to_string())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|w, ev: &MouseDownEvent, _, cx| {
                                    w.editor.popup = Some((Popup::Own(IwPopup::Tracks), ev.position));
                                    cx.notify();
                                }),
                            ),
                    )
                    .child(button("iw-prev", "‹", false, 0).on_click(cx.listener(|w, _, _, cx| w.step_track(-1, cx))))
                    .child(button("iw-next", "›", false, 0).on_click(cx.listener(|w, _, _, cx| w.step_track(1, cx))))
                    .child(button("iw-mute", "M", mute, THEME.orange).on_click(cx.listener(move |w, _, _, cx| {
                        let core = w.scx.core.clone();
                        apply_ops(&core, "Mute", vec![EditOp::set(tid, &["mute"], Value::bool(!mute))], cx);
                    })))
                    .child(button("iw-solo", "S", solo, THEME.yellow).on_click(cx.listener(move |w, _, _, cx| {
                        let core = w.scx.core.clone();
                        apply_ops(&core, "Solo", vec![EditOp::set(tid, &["solo"], Value::bool(!solo))], cx);
                    })))
                    .child(button("iw-close", "×", false, 0).on_click(cx.listener(|w, _, _, cx| w.scx.close(cx)))),
            )
            .child(
                line()
                    .child(plugin_chip(info, &plugin))
                    .child(div().text_size(px(12.)).text_color(c(THEME.text2)).child(plugin_name(plugins, &plugin)))
                    .child(button("iw-swap", tr("Swap ▾"), false, 0).debug_selector(|| "iw-swap".into()).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|w, ev: &MouseDownEvent, _, cx| {
                            w.editor.popup = Some((Popup::Own(IwPopup::Swap), ev.position));
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ))
                    .child(div().flex_1())
                    .when(info.is_some_and(|i| i.third_party), |e| {
                        e.child(div().text_size(px(9.5)).text_color(c(THEME.purple)).child(tr("outside the program")))
                    })
                    // I-22: a third-party plugin opens its own window (the helper makes it)
                    .when_some(
                        t.instrument.as_ref().map(|(n, _)| *n).filter(|_| info.is_some_and(|i| i.third_party)),
                        |e, node| {
                            e.child(
                                button("iw-plugin-window", tr("Open the plugin's window"), false, 0)
                                    .debug_selector(|| "iw-plugin-window".into())
                                    .on_click(cx.listener(move |w, _, _, cx| {
                                        let core = w.scx.core.clone();
                                        let params = Value::object([("node", Value::text(node.to_string()))]);
                                        run_action(&core, "plugin.show_window", params, cx);
                                    })),
                            )
                        },
                    ),
            )
            .child(
                line()
                    .child(
                        button("iw-preset-prev", "◀", false, 0)
                            .debug_selector(|| "iw-preset-prev".into())
                            .on_click(cx.listener(|w, _, _, cx| w.step_preset(-1, cx))),
                    )
                    .child(
                        div()
                            .id("iw-preset")
                            .debug_selector(|| "iw-preset".into())
                            .flex_1()
                            .h(px(20.))
                            .px(px(6.))
                            .flex()
                            .items_center()
                            .rounded(px(4.))
                            .bg(c(THEME.lcd_bg))
                            .text_size(px(11.5))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .cursor_pointer()
                            .child(trf("Preset: {0}", &[&preset]))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                                    if let Some((_, p)) = w.track_view(cx).and_then(|t| t.instrument) {
                                        w.refresh_presets(&p, cx);
                                    }
                                    w.editor.popup = Some((Popup::Own(IwPopup::Presets), ev.position));
                                    cx.notify();
                                }),
                            ),
                    )
                    .child(
                        button("iw-preset-next", "▶", false, 0)
                            .debug_selector(|| "iw-preset-next".into())
                            .on_click(cx.listener(|w, _, _, cx| w.step_preset(1, cx))),
                    )
                    .child(
                        button("iw-save", tr("Save preset"), false, 0)
                            .debug_selector(|| "iw-save".into())
                            .on_click(cx.listener(|w, _, _, cx| w.save_preset(cx))),
                    ),
            )
    }

    fn tabs(&self, t: &TrackView, cx: &Context<Self>) -> impl IntoElement {
        let effects = t
            .outputs
            .first()
            .and_then(|o| model::channels(self.scx.core.read(cx).port().root()).into_iter().find(|c| c.id == o.to))
            .map_or(0, |c| c.inserts.len());
        div().flex().flex_wrap().px(px(4.)).bg(c(THEME.bg2)).border_b_1().border_color(c(THEME.line)).children(
            Tab::ALL.into_iter().map(|tb| {
                let label: SharedString =
                    if tb == Tab::Effects { trf("Effects ({0})", &[&effects]).into() } else { tr(tb.title()).into() };
                tab(SharedString::from(format!("iw-tab-{tb:?}")), label, self.tab == tb)
                    .debug_selector(move || format!("iw-tab-{tb:?}"))
                    .px(px(8.))
                    .text_size(px(12.))
                    .on_click(cx.listener(move |w, _, _, cx| {
                        w.tab = tb;
                        w.remember_tab(cx);
                        cx.notify();
                    }))
            }),
        )
    }

    fn footer(&self, t: &TrackView, ctls: &[Ctl], cx: &Context<Self>) -> impl IntoElement {
        let root = self.scx.core.read(cx).port().root().clone();
        let vol = Ctl {
            node: t.id,
            plugin: Arc::from(""),
            slot: Slot::TrackVolume,
            kind: CtlKind::Volume,
            unit: "dB",
            automatable: true,
            value: view::gain_to_db(t.volume).max(-60.0),
            choice: Arc::from(""),
            automated: lanes_of(&root, t.id).iter().any(|(_, _, k)| &**k == "volume"),
            enabled: true,
            shown: None,
        };
        let mut controls: Vec<AnyElement> = vec![control(self, vol, cx)];
        for k in TUNING_KEYS {
            if let Some(cc) = ctls.iter().find(|x| x.key() == k) {
                controls.push(control(self, cc.clone(), cx));
            }
        }
        let channels = model::channels(&root);
        let out_text: SharedString = match t.outputs.len() {
            0 => tr("No output").into(),
            1 => {
                let to =
                    channels.iter().find(|c| c.id == t.outputs[0].to).map_or(String::from("?"), |c| c.name.to_string());
                trf("→ {0}", &[&to]).into()
            }
            n => trf("{0} outputs", &[&n]).into(),
        };
        let multi = t.outputs.len() > 1;
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(10.))
            .py(px(6.))
            .bg(c(THEME.bg2))
            .border_t_1()
            .border_color(c(THEME.line))
            .child(div().flex().items_end().gap(px(4.)).children(controls))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        button("iw-out", out_text, false, 0)
                            .h(px(22.))
                            .debug_selector(|| "iw-out".into())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                                    if multi {
                                        w.tab = Tab::Instrument;
                                    } else {
                                        w.editor.popup = Some((Popup::Own(IwPopup::Output(0)), ev.position));
                                    }
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        button("iw-keyboard", tr("Keyboard"), self.keyboard, THEME.green)
                            .h(px(22.))
                            .debug_selector(|| "iw-keyboard".into())
                            .on_click(cx.listener(|w, _, _, cx| {
                                w.keyboard = !w.keyboard;
                                cx.notify();
                            })),
                    ),
            )
    }

    /// The test keyboard (F-IJN-021): a click marks the key, a drag slides
    fn keyboard_view(&self, cx: &Context<Self>) -> impl IntoElement {
        let held = self.held;
        let bounds = self.keys_bounds.clone();
        div()
            .id("iw-keys")
            .debug_selector(|| "iw-keys".into())
            .h(px(60.))
            .px(px(6.))
            .py(px(4.))
            .bg(c(THEME.bg1))
            .child(
                canvas(
                    |_, _, _| {},
                    move |b, _, window, _| {
                        bounds.set(Some(b));
                        paint_keys(b, held, window);
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|w, ev: &MouseDownEvent, window, cx| {
                    window.focus(&w.focus, cx);
                    if let Some(b) = w.keys_bounds.get() {
                        w.held = key_at(b, ev.position);
                        if let Some(k) = w.held {
                            w.play(k, key_velocity(b, ev.position), cx);
                        }
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|w, ev: &MouseMoveEvent, _, cx| {
                if w.held.is_some()
                    && ev.pressed_button == Some(MouseButton::Left)
                    && let Some(b) = w.keys_bounds.get()
                {
                    let k = key_at(b, ev.position);
                    if k != w.held {
                        // a glissando: the key left ends, the key entered starts
                        if let Some(old) = w.held {
                            w.stop(old, cx);
                        }
                        w.held = k;
                        if let Some(k) = k {
                            w.play(k, key_velocity(b, ev.position), cx);
                        }
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|w, _, _, cx| w.release_notes(cx)))
    }

    // --- the tabs' bodies -------------------------------------------------------------------------------------

    fn instrument_tab(
        &self,
        t: &TrackView,
        ctls: &[Ctl],
        plugins: &[PluginInfo],
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let Some((node, plugin)) = t.instrument.clone() else {
            return vec![caption(tr("No instrument")).into_any_element()];
        };
        let mut out = Vec::new();
        let port = self.scx.core.read(cx).port();
        // building: the loading in place of the controls, never half-built buttons (I-2.3); a missing file (H17)
        let target = Target::Node(node);
        let loading = port.loading(&target);
        let core_e = self.scx.core.clone();
        let cancel_on = target.clone();
        if let Some(l) =
            loading_component("iw-loading", &loading, move |_, cx| cancel_target(&core_e, cancel_on.clone(), cx))
        {
            out.push(
                div()
                    .h(px(32.))
                    .rounded(px(6.))
                    .overflow_hidden()
                    .debug_selector(|| "iw-loading".into())
                    .child(l)
                    .into_any_element(),
            );
            if matches!(loading, Loading::Visible { .. } | Loading::Failed(_)) {
                return out;
            }
        }
        if let Loading::Missing { .. } = loading {
            out.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().flex_1())
                    .child(button("iw-locate", tr("Open the browser"), false, 0).h(px(22.)).on_click(cx.listener(
                        |w, _, window, cx| {
                            w.scx.open(Screen::Browser, None, window, cx);
                        },
                    )))
                    .into_any_element(),
            );
        }
        if !plugins.iter().any(|p| p.id == plugin) {
            // a plugin the program lacks: silent, its state kept and saved back (F-INS-024)
            out.push(notice(trf("\"{0}\" is not installed; the track is silent", &[&plugin])));
            return out;
        }
        if t.kit {
            out.push(self.kit_view(t, &plugin, cx));
        }
        let own: Vec<Ctl> = ctls.iter().filter(|k| !is_function_key(k.key())).cloned().collect();
        match &self.panel {
            // the plugin's own look, faithful to the legacy (I-22); the instrument functions stay in their tabs
            Some((n, p, view, _)) if *n == node && *p == plugin => {
                let (w, h) = lm_gui_plugins::panel_size(&plugin).unwrap_or((250.0, 250.0));
                out.push(div().flex_none().w(px(w)).h(px(h)).child(view.clone()).into_any_element());
            }
            _ => out.extend(grouped(self, own, &plugin_name(plugins, &plugin), cx)),
        }
        // the outputs (I-3.9): one per port of the instrument, each to a desk channel
        let channels = model::channels(port.root());
        let rows: Vec<AnyElement> = t
            .outputs
            .iter()
            .enumerate()
            .map(|(k, o)| {
                let to = channels.iter().find(|c| c.id == o.to).map_or(String::from("?"), |c| c.name.to_string());
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .w_full()
                    .child(div().w(px(64.)).text_size(px(11.)).child(o.port.to_string()))
                    .child(div().w(px(42.)).text_size(px(9.5)).text_color(c(THEME.text3)).child(if o.channels == 1 {
                        tr("mono")
                    } else {
                        tr("stereo")
                    }))
                    .child(
                        button(SharedString::from(format!("iw-out-{k}")), format!("→ {to} ▾"), false, 0)
                            .h(px(20.))
                            .debug_selector(move || format!("iw-out-{k}"))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                                    w.editor.popup = Some((Popup::Own(IwPopup::Output(k)), ev.position));
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        out.push(group_card(tr("Outputs"), rows));
        out
    }

    /// The kit's pieces as pictured, each with the key it plays: AVL Drums by the groups of its outputs (when it has
    /// them), else the General MIDI drum map (DrumMachines, and the single-output AVL kit)
    fn kit_pieces(t: &TrackView, plugin: &str) -> Vec<(Arc<str>, u8)> {
        let groups: Vec<(Arc<str>, u8)> = t
            .outputs
            .iter()
            .enumerate()
            .map(|(k, o)| (o.port.clone(), live::group_key(&o.port).unwrap_or((36 + k).min(127) as u8)))
            .collect();
        if plugin != "drummachines" && groups.len() > 1 {
            groups
        } else {
            super::steps::GM_DRUMS.iter().map(|(k, n)| (Arc::from(tr(n).as_ref()), *k)).collect()
        }
    }

    /// The kit drawn by its pieces; a click on a piece plays it and lights it (F-INS-002), and the note ends when
    /// the button comes up
    fn kit_view(&self, t: &TrackView, plugin: &str, cx: &Context<Self>) -> AnyElement {
        let bounds = self.kit_bounds.clone();
        let lit = self.lit.filter(|(_, at)| at.elapsed() < Duration::from_millis(250)).map(|(k, _)| k);
        let all = Self::kit_pieces(t, plugin);
        let keys: Vec<u8> = all.iter().map(|(_, k)| *k).collect();
        let pieces: Vec<Arc<str>> = all.into_iter().map(|(p, _)| p).collect();
        let n = pieces.len().max(1);
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(8.))
            .rounded(px(6.))
            .bg(c(THEME.bg2))
            .child(
                div()
                    .id("iw-kit")
                    .debug_selector(|| "iw-kit".into())
                    .h(px(90.))
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |b, _, window, _| {
                                bounds.set(Some(b));
                                for k in 0..n {
                                    let (center, r) = piece_at(b, k, n);
                                    let bb = Bounds::new(point(center.x - r, center.y - r), size(r * 2.0, r * 2.0));
                                    let col = if lit == Some(k) {
                                        THEME.green
                                    } else if k == 0 {
                                        THEME.bg4
                                    } else {
                                        THEME.bg3
                                    };
                                    window.paint_quad(gpui::quad(
                                        bb,
                                        r,
                                        c(col),
                                        px(1.5),
                                        c(THEME.line2),
                                        Default::default(),
                                    ));
                                }
                            },
                        )
                        .size_full(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |w, ev: &MouseDownEvent, window, cx| {
                            window.focus(&w.focus, cx);
                            let Some(b) = w.kit_bounds.get() else { return };
                            let hit = (0..n).find(|k| {
                                let (center, r) = piece_at(b, *k, n);
                                let (dx, dy) =
                                    ((ev.position.x - center.x).as_f32(), (ev.position.y - center.y).as_f32());
                                dx * dx + dy * dy <= r.as_f32() * r.as_f32()
                            });
                            if let Some(k) = hit {
                                if let Some(key) = keys.get(k) {
                                    w.play(*key, live::DEFAULT_VELOCITY, cx);
                                }
                                w.lit = Some((k, Instant::now()));
                                window.request_animation_frame();
                                cx.notify();
                            }
                        }),
                    ),
            )
            .child(div().flex().flex_wrap().gap(px(8.)).children(pieces.into_iter().enumerate().map(|(k, p)| {
                div()
                    .text_size(px(9.5))
                    .text_color(c(if lit == Some(k) { THEME.green } else { THEME.text3 }))
                    .child(p.to_string())
            })))
            .into_any_element()
    }

    fn envelopes_tab(&self, ctls: &[Ctl], cx: &Context<Self>) -> Vec<AnyElement> {
        let t = ["vol", "cut", "res"][self.target];
        let find = |k: &str| ctls.iter().find(|x| x.key() == k).cloned();
        if find("volAttack").is_none() {
            return vec![notice(tr("Not available for this instrument"))];
        }
        let mut out = vec![
            div()
                .flex()
                .gap(px(4.))
                .children(["Volume", "Cutoff", "Resonance"].into_iter().enumerate().map(|(k, l)| {
                    button(SharedString::from(format!("iw-target-{k}")), tr(l), self.target == k, THEME.green)
                        .h(px(22.))
                        .px(px(10.))
                        .debug_selector(move || format!("iw-target-{k}"))
                        .on_click(cx.listener(move |w, _, _, cx| {
                            w.target = k;
                            cx.notify();
                        }))
                }))
                .into_any_element(),
        ];
        let env: Vec<Ctl> = ENV_KEYS[..7].iter().filter_map(|n| find(&format!("{t}{n}"))).collect();
        let lfo: Vec<Ctl> = ENV_KEYS[7..]
            .iter()
            .map(|n| format!("{t}{n}"))
            .chain([format!("{t}LfoShape"), format!("{t}LfoSync")])
            .filter_map(|k| find(&k))
            .collect();
        let v = |n: &str| find(&format!("{t}{n}")).map_or(0.0, |x| x.value);
        let pts = [v("Predelay"), v("Attack"), v("Hold"), v("Decay"), v("Sustain"), v("Release"), v("Amount")];
        let amount = find(&format!("{t}Amount"));
        out.push(
            div()
                .id("iw-env-graph")
                .debug_selector(|| "iw-env-graph".into())
                .h(px(64.))
                .rounded(px(6.))
                .bg(c(THEME.lcd_bg))
                .cursor_pointer()
                .child(canvas(|_, _, _| {}, move |b, _, window, _| paint_envelope(b, pts, window)).size_full())
                // a click on the graph switches the amount between 0 and full (the legacy's gesture)
                .on_click(cx.listener(move |w, _, _, cx| {
                    if let Some(a) = &amount {
                        let core = w.scx.core.clone();
                        apply_ops(&core, "Envelope amount", vec![a.op(if a.value == 0.0 { 1.0 } else { 0.0 })], cx);
                    }
                }))
                .into_any_element(),
        );
        out.push(group_card(tr("Envelope"), env.into_iter().map(|k| control(self, k, cx)).collect()));
        let shape = find(&format!("{t}LfoShape")).map_or(Arc::from("sine"), |x| x.choice);
        let (speed, x100, lamount) = (v("LfoSpeed"), v("LfoX100"), v("LfoAmount"));
        let speed = speed * if x100 > 0.5 { 100.0 } else { 1.0 };
        out.push(
            div()
                .h(px(40.))
                .rounded(px(6.))
                .bg(c(THEME.lcd_bg))
                .child(
                    canvas(|_, _, _| {}, move |b, _, window, _| paint_lfo(b, &shape, speed, lamount, window))
                        .size_full(),
                )
                .into_any_element(),
        );
        out.push(group_card(tr("LFO"), lfo.into_iter().map(|k| control(self, k, cx)).collect()));
        out.push(group_card(
            tr("Filter"),
            FILTER_KEYS.iter().filter_map(|k| find(k)).map(|k| control(self, k, cx)).collect(),
        ));
        out
    }

    fn effects_tab(&self, t: &TrackView, plugins: &[PluginInfo], cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(chid) = t.outputs.first().map(|o| o.to) else { return vec![] };
        let Some(ch) = model::channels(self.scx.core.read(cx).port().root()).into_iter().find(|c| c.id == chid) else {
            return vec![];
        };
        let mut out = vec![caption(trf("Channel \"{0}\"", &[&ch.name])).into_any_element()];
        let rows: Vec<AnyElement> = ch
            .inserts
            .iter()
            .map(|x| {
                let (id, on) = (x.id, !x.bypass);
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .w_full()
                    .child(
                        button(
                            SharedString::from(format!("iw-fx-{}", id.number)),
                            if on { tr("On") } else { tr("Off") },
                            on,
                            THEME.green,
                        )
                        .on_click(cx.listener(move |w, _, _, cx| {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Bypass", vec![EditOp::set(id, &["bypass"], Value::bool(on))], cx);
                        })),
                    )
                    .child(plugin_chip(plugins.iter().find(|p| p.id == x.plugin), &x.plugin))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.5))
                            .text_color(c(if on { THEME.text } else { THEME.text3 }))
                            .child(plugin_name(plugins, &x.plugin)),
                    )
                    .into_any_element()
            })
            .collect();
        let rows = if rows.is_empty() { vec![caption(tr("No effects")).into_any_element()] } else { rows };
        out.push(group_card(tr("Inserts"), rows));
        out.push(
            button("iw-open-effects", tr("Open in the effect window"), false, 0)
                .h(px(24.))
                .px(px(10.))
                .debug_selector(|| "iw-open-effects".into())
                .on_click(cx.listener(move |w, _, window, cx| w.scx.open(Screen::Effect, Some(chid), window, cx)))
                .into_any_element(),
        );
        out
    }

    /// The track's node, as it is in the tree
    fn track_node(&self, cx: &App) -> Value {
        let Some(id) = self.track else { return Value::NULL };
        model::list(self.scx.core.read(cx).port().root(), "tracks")
            .find(|x| model::id_of(x) == Some(id))
            .cloned()
            .unwrap_or_default()
    }

    /// Sets one of the track's optional objects whole (`midiInput`, `midiOutput`, `tuning`, `keys`); `None`
    /// removes it (absent keeps the old meaning, U29)
    fn set_track_object(&mut self, key: &str, v: Option<Value>, label: &'static str, cx: &mut Context<Self>) {
        let Some(track) = self.track else { return };
        let op = EditOp::set(track, &[key], v.unwrap_or(Value::NULL));
        let core = self.scx.core.clone();
        apply_ops(&core, label, vec![op], cx);
    }

    /// The machine's MIDI ports by alias (the preferences, I-21.1): (alias, input, output)
    fn midi_ports(&self, cx: &App) -> Vec<(Arc<str>, bool, bool)> {
        let prefs = self.scx.core.read(cx).port().preferences().clone();
        model::at(&prefs, &["midi", "ports"])
            .map(|l| {
                l.items()
                    .filter_map(|p| {
                        let alias =
                            p.get("alias").and_then(Value::as_shared_text).cloned().filter(|a| !a.is_empty())?;
                        let role = p.get("role").and_then(Value::as_text).unwrap_or("both");
                        let input = p.get("input").and_then(Value::as_bool).unwrap_or(true) && role != "controller";
                        let output = p.get("output").and_then(Value::as_bool).unwrap_or(false);
                        Some((alias, input, output))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The MIDI tab (F-IJN-017): the track's input (ports, channel, fixed velocity, follow the selection), its output
    /// to a device (port, channel, program, fixed velocity and note) and the base velocity
    fn midi_tab(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let node = self.track_node(cx);
        let input = node.get("midiInput").filter(|v| !v.is_null()).cloned();
        let output = node.get("midiOutput").filter(|v| !v.is_null()).cloned();
        let ports = self.midi_ports(cx);
        let inp = input.clone().unwrap_or_else(|| {
            Value::object([
                ("ports", Value::list([])),
                ("channel", Value::int(-1)),
                ("followSelection", Value::bool(false)),
            ])
        });
        let chosen: Vec<Arc<str>> = model::list(&inp, "ports").filter_map(|v| v.as_shared_text().cloned()).collect();
        let channel = model::int(&inp, "channel", -1);
        let fixed_in = inp.get("fixedVelocity").and_then(Value::as_int);
        let follow = model::flag(&inp, "followSelection");
        // input: the ports (none chosen = every instrument port)
        let mut in_rows: Vec<AnyElement> = vec![
            caption(if chosen.is_empty() { tr("Every instrument port") } else { tr("These ports") }).into_any_element(),
        ];
        for (alias, is_in, _) in ports.iter().filter(|p| p.1) {
            let _ = is_in;
            let on = chosen.contains(alias);
            let (a, base) = (alias.clone(), inp.clone());
            in_rows.push(
                button(
                    SharedString::from(format!("iw-midi-in-{alias}")),
                    format!("{} {}", if on { "✓" } else { "·" }, alias),
                    on,
                    THEME.blue,
                )
                .h(px(20.))
                .on_click(cx.listener(move |w, _, _, cx| {
                    let mut list: Vec<Value> =
                        model::list(&base, "ports").filter(|v| v.as_text() != Some(&*a)).cloned().collect();
                    if !on {
                        list.push(Value::text(a.clone()));
                    }
                    w.set_track_object("midiInput", Some(base.with("ports", Value::list(list))), "MIDI input", cx);
                }))
                .into_any_element(),
            );
        }
        let inp_c = inp.clone();
        in_rows.push(self.stepper(
            "iw-midi-in-channel",
            tr("Channel"),
            if channel < 0 { tr("All").to_string() } else { (channel + 1).to_string() },
            cx,
            move |w, d, cx| {
                let v = (model::int(&inp_c, "channel", -1) + d).clamp(-1, 15);
                w.set_track_object("midiInput", Some(inp_c.with("channel", Value::int(v))), "MIDI input", cx);
            },
        ));
        let inp_v = inp.clone();
        in_rows.push(self.stepper(
            "iw-midi-in-velocity",
            tr("Fixed velocity"),
            fixed_in.map_or_else(|| tr("As played").to_string(), |v| v.to_string()),
            cx,
            move |w, d, cx| {
                let next = match inp_v.get("fixedVelocity").and_then(Value::as_int) {
                    None if d > 0 => Some(1),
                    None => None,
                    Some(v) if v + d < 1 => None,
                    Some(v) => Some((v + d).min(127)),
                };
                let node = match next {
                    Some(v) => inp_v.with("fixedVelocity", Value::int(v)),
                    None => inp_v.without("fixedVelocity"),
                };
                w.set_track_object("midiInput", Some(node), "MIDI input", cx);
            },
        ));
        let inp_f = inp.clone();
        in_rows.push(
            button("iw-midi-follow", tr("Follow the selected track"), follow, THEME.green)
                .h(px(20.))
                .debug_selector(|| "iw-midi-follow".into())
                .on_click(cx.listener(move |w, _, _, cx| {
                    w.set_track_object(
                        "midiInput",
                        Some(inp_f.with("followSelection", Value::bool(!follow))),
                        "MIDI input",
                        cx,
                    );
                }))
                .into_any_element(),
        );
        // output (track.midi_output v2): absent = none; `on` switches it and keeps the settings, as the legacy's
        // checkbox; any number of ports
        let out_on = output.as_ref().is_some_and(|o| o.get("on").and_then(Value::as_bool).unwrap_or(true));
        let mut out_rows: Vec<AnyElement> = vec![
            button("iw-midi-out-on", tr("Send MIDI to a device"), out_on, THEME.green)
                .h(px(20.))
                .debug_selector(|| "iw-midi-out-on".into())
                .on_click(cx.listener({
                    let current = output.clone();
                    let first = ports.iter().find(|p| p.2).map(|p| p.0.clone());
                    move |w, _, _, cx| {
                        let v = match &current {
                            Some(o) => o.with("on", Value::bool(!out_on)),
                            None => Value::object([
                                ("on", Value::bool(true)),
                                ("ports", Value::list(first.iter().map(|a| Value::text(a.clone())))),
                                ("channel", Value::int(0)),
                            ]),
                        };
                        w.set_track_object("midiOutput", Some(v), "MIDI output", cx);
                    }
                }))
                .into_any_element(),
        ];
        if let Some(out) = output {
            let chosen: Vec<Arc<str>> =
                model::list(&out, "ports").filter_map(|v| v.as_shared_text().cloned()).collect();
            for (alias, _, _) in ports.iter().filter(|p| p.2) {
                let on = chosen.contains(alias);
                let (a, base) = (alias.clone(), out.clone());
                out_rows.push(
                    button(
                        SharedString::from(format!("iw-midi-out-{alias}")),
                        format!("{} {}", if on { "✓" } else { "·" }, alias),
                        on,
                        THEME.blue,
                    )
                    .h(px(20.))
                    .on_click(cx.listener(move |w, _, _, cx| {
                        let mut list: Vec<Value> =
                            model::list(&base, "ports").filter(|v| v.as_text() != Some(&*a)).cloned().collect();
                        if !on {
                            list.push(Value::text(a.clone()));
                        }
                        w.set_track_object(
                            "midiOutput",
                            Some(base.with("ports", Value::list(list))),
                            "MIDI output",
                            cx,
                        );
                    }))
                    .into_any_element(),
                );
            }
            let o = out.clone();
            out_rows.push(self.stepper(
                "iw-midi-out-channel",
                tr("Channel"),
                (model::int(&out, "channel", 0) + 1).to_string(),
                cx,
                move |w, d, cx| {
                    let v = (model::int(&o, "channel", 0) + d).clamp(0, 15);
                    w.set_track_object("midiOutput", Some(o.with("channel", Value::int(v))), "MIDI output", cx);
                },
            ));
            for (key, label, min, max) in [
                ("program", "Program", 0, 127),
                ("fixedVelocity", "Fixed velocity", 1, 127),
                ("fixedNote", "Fixed note", 0, 127),
            ] {
                let cur = out.get(key).and_then(Value::as_int);
                let o = out.clone();
                let shown = match (key, cur) {
                    (_, None) => tr("As played").to_string(),
                    ("program", Some(v)) => (v + 1).to_string(),
                    ("fixedNote", Some(v)) => note_name(v as f64),
                    (_, Some(v)) => v.to_string(),
                };
                out_rows.push(self.stepper(
                    intern(&format!("iw-midi-out-{key}")),
                    tr(label),
                    shown,
                    cx,
                    move |w, d, cx| {
                        let next = match o.get(key).and_then(Value::as_int) {
                            None if d > 0 => Some(min),
                            None => None,
                            Some(v) if v + d < min => None,
                            Some(v) => Some((v + d).min(max)),
                        };
                        let node = match next {
                            Some(v) => o.with(key, Value::int(v)),
                            None => o.without(key),
                        };
                        w.set_track_object("midiOutput", Some(node), "MIDI output", cx);
                    },
                ));
            }
        }
        let base = node.get("baseVelocity").and_then(Value::as_int).unwrap_or(63);
        let tid = self.track;
        let vel = self.stepper("iw-base-velocity", tr("Base velocity"), base.to_string(), cx, move |w, d, cx| {
            let Some(t) = tid else { return };
            let core = w.scx.core.clone();
            apply_ops(
                &core,
                "Base velocity",
                vec![EditOp::set(t, &["baseVelocity"], Value::int((base + d).clamp(1, 127)))],
                cx,
            );
        });
        // the CC rack (F-IJN-025, the legacy MidiCCRackView): its switch, and 128 controllers sent to the instrument
        let rack_on = node.get("ccRack").and_then(|r| r.get("on")).and_then(Value::as_bool).unwrap_or(false);
        let mut rack: Vec<AnyElement> = vec![
            button("iw-cc-rack-on", tr("Send the controllers to the instrument"), rack_on, THEME.green)
                .h(px(20.))
                .debug_selector(|| "iw-cc-rack-on".into())
                .on_click(cx.listener(move |w, _, _, cx| {
                    let Some(t) = w.track else { return };
                    let core = w.scx.core.clone();
                    let op = EditOp::set(t, &["ccRack"], Value::object([("on", Value::bool(!rack_on))]));
                    apply_ops(&core, "CC rack", vec![op], cx);
                }))
                .into_any_element(),
        ];
        if let Some(t) = self.track {
            let root = self.scx.core.read(cx).port().root().clone();
            let lanes = lanes_of(&root, t);
            let cc = node.get("cc").cloned().unwrap_or_default();
            rack.extend((0..128u8).map(|n| {
                let key = intern(&format!("cc.{n}"));
                let ctl = Ctl {
                    node: t,
                    plugin: Arc::from(""),
                    slot: Slot::Cc(n),
                    kind: CtlKind::Knob { min: 0.0, max: 127.0, def: 0.0, log: false },
                    unit: "",
                    automatable: true,
                    value: cc.get(&format!("c{n}")).and_then(Value::as_real).unwrap_or(0.0),
                    choice: Arc::from(""),
                    automated: lanes.iter().any(|(_, _, k)| &**k == key),
                    enabled: true,
                    shown: None,
                };
                control(self, ctl, cx)
            }));
        }
        vec![
            group_card(tr("MIDI input"), in_rows),
            group_card(tr("MIDI output"), out_rows),
            group_card(tr("Velocity"), vec![vel]),
            group_card(tr("Controllers (CC)"), rack),
        ]
    }

    /// A labelled − value + stepper of a track field (the click steps by one, Shift by ten)
    fn stepper(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        value: String,
        cx: &Context<Self>,
        step: impl Fn(&mut Self, i64, &mut Context<Self>) + Clone + 'static,
    ) -> AnyElement {
        let (s1, s2) = (step.clone(), step);
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w(px(80.))
            .child(div().text_size(px(9.5)).text_color(c(THEME.text2)).child(label.into()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .child(
                        button(SharedString::from(format!("{id}-minus")), "−", false, 0)
                            .debug_selector(move || format!("{id}-minus"))
                            .on_click(cx.listener(move |w, ev: &gpui::ClickEvent, _, cx| {
                                s1(
                                    w,
                                    if crate::keymap::holds("Instrument", "param.spin_coarse", &ev.modifiers()) {
                                        -10
                                    } else {
                                        -1
                                    },
                                    cx,
                                )
                            })),
                    )
                    .child(
                        div()
                            .id(id)
                            .debug_selector(move || id.to_string())
                            .min_w(px(44.))
                            .h(px(18.))
                            .px(px(4.))
                            .rounded(px(3.))
                            .bg(c(THEME.lcd_bg))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(10.))
                            .child(value),
                    )
                    .child(
                        button(SharedString::from(format!("{id}-plus")), "+", false, 0)
                            .debug_selector(move || format!("{id}-plus"))
                            .on_click(cx.listener(move |w, ev: &gpui::ClickEvent, _, cx| {
                                s2(
                                    w,
                                    if crate::keymap::holds("Instrument", "param.spin_coarse", &ev.modifiers()) {
                                        10
                                    } else {
                                        1
                                    },
                                    cx,
                                )
                            })),
                    ),
            )
            .into_any_element()
    }

    /// The Tuning tab (F-IJN-018..021): pitch, bend range, the global transposition; the tuning by file (on, the
    /// `.scl` and `.kbm` pickers, the keymap's ranges); the first, last and base key
    fn tuning_tab(&self, t: &TrackView, ctls: &[Ctl], cx: &Context<Self>) -> Vec<AnyElement> {
        let node = self.track_node(cx);
        let follows = node.get("transpose").and_then(Value::as_bool).unwrap_or(true);
        let tid = t.id;
        let mut controls: Vec<AnyElement> = TUNING_KEYS
            .iter()
            .filter_map(|k| ctls.iter().find(|x| x.key() == *k))
            .map(|k| control(self, k.clone(), cx))
            .collect();
        controls.push(
            button("iw-transpose", tr("Follow the global transposition"), follows, THEME.green)
                .h(px(20.))
                .debug_selector(|| "iw-transpose".into())
                .on_click(cx.listener(move |w, _, _, cx| {
                    let core = w.scx.core.clone();
                    apply_ops(
                        &core,
                        "Transposition",
                        vec![EditOp::set(tid, &["transpose"], Value::bool(!follows))],
                        cx,
                    );
                }))
                .into_any_element(),
        );
        let mut out = vec![group_card(tr("Transposition"), controls)];
        // the tuning by file: not for an instrument that plays its own voices
        let per_note = ctls.iter().any(|k| k.key() == "volAttack");
        let tuning = node
            .get("tuning")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| Value::object([("on", Value::bool(false)), ("importRanges", Value::bool(true))]));
        let on = model::flag(&tuning, "on");
        let mut mrows: Vec<AnyElement> = Vec::new();
        if !per_note {
            mrows.push(notice(tr("Not available for this instrument")));
        }
        let t1 = tuning.clone();
        mrows.push(
            button("iw-tuning-on", tr("Tuning"), on, THEME.green)
                .h(px(20.))
                .debug_selector(|| "iw-tuning-on".into())
                .on_click(cx.listener(move |w, _, _, cx| {
                    w.set_track_object("tuning", Some(t1.with("on", Value::bool(!on))), "Tuning", cx)
                }))
                .into_any_element(),
        );
        let has = |k: &str| tuning.get(k).is_some_and(|v| !v.is_null());
        mrows.push(
            div()
                .flex()
                .gap(px(6.))
                .child(
                    button("iw-tuning-scale", "Scale .scl", has("scale"), THEME.green)
                        .h(px(22.))
                        .px(px(8.))
                        .debug_selector(|| "iw-tuning-scale".into())
                        .on_click(cx.listener(|w, _, _, cx| w.pick_tuning(false, cx))),
                )
                .child(
                    button("iw-tuning-keymap", "Keymap .kbm", has("keymap"), THEME.green)
                        .h(px(22.))
                        .px(px(8.))
                        .debug_selector(|| "iw-tuning-keymap".into())
                        .on_click(cx.listener(|w, _, _, cx| w.pick_tuning(true, cx))),
                )
                .child(
                    button("iw-tuning-clear", tr("Clear"), false, 0)
                        .h(px(22.))
                        .px(px(8.))
                        .debug_selector(|| "iw-tuning-clear".into())
                        .on_click(cx.listener(|w, _, _, cx| w.clear_tuning(cx))),
                )
                .into_any_element(),
        );
        let imp = model::flag(&tuning, "importRanges") || tuning.get("importRanges").is_none();
        let t2 = tuning.clone();
        mrows.push(
            button("iw-tuning-ranges", tr("Note range from the keymap"), imp, THEME.green)
                .h(px(20.))
                .on_click(cx.listener(move |w, _, _, cx| {
                    w.set_track_object("tuning", Some(t2.with("importRanges", Value::bool(!imp))), "Tuning", cx)
                }))
                .into_any_element(),
        );
        out.push(group_card(tr("Tuning"), mrows));
        // the keys: the first and last the instrument plays, the base key
        let keys = node.get("keys").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| {
            Value::object([("first", Value::int(0)), ("last", Value::int(127)), ("base", Value::int(69))])
        });
        let mut krows = Vec::new();
        for (key, label, def) in [("first", "First key", 0), ("last", "Last key", 127), ("base", "Base key", 69)] {
            let k = keys.clone();
            let v = model::int(&keys, key, def);
            krows.push(self.stepper(
                intern(&format!("iw-keys-{key}")),
                tr(label),
                note_name(v as f64),
                cx,
                move |w, d, cx| {
                    let n = (model::int(&k, key, def) + d).clamp(0, 127);
                    let mut node = k.with(key, Value::int(n));
                    // first ≤ last
                    let (f, l) = (model::int(&node, "first", 0), model::int(&node, "last", 127));
                    if f > l {
                        node = node.with(if key == "first" { "last" } else { "first" }, Value::int(n));
                    }
                    w.set_track_object("keys", Some(node), "Keys", cx);
                },
            ));
        }
        out.push(group_card(tr("Keys"), krows));
        out
    }

    /// A Scala file (`.scl`, `.kbm`) for the track: the core copies it into the project (`tuning.set_scale`,
    /// `tuning.set_keymap`, one undo step)
    fn pick_tuning(&mut self, keymap: bool, cx: &mut Context<Self>) {
        let Some(track) = self.track else { return };
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |w, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.first().and_then(|p| p.to_str()).map(str::to_string) else { return };
            let _ = w.update(cx, |w, cx| {
                let params =
                    Value::object([("track", Value::text(track.to_string())), ("path", Value::text(path.as_str()))]);
                let core = w.scx.core.clone();
                run_action(&core, if keymap { "tuning.set_keymap" } else { "tuning.set_scale" }, params, cx);
            });
        })
        .detach();
    }

    fn clear_tuning(&mut self, cx: &mut Context<Self>) {
        let Some(track) = self.track else { return };
        let core = self.scx.core.clone();
        run_action(&core, "tuning.clear", Value::object([("track", Value::text(track.to_string()))]), cx);
    }

    /// A drop from the browser (F-IJN-004): an instrument swaps the instrument; a preset of this instrument loads
    fn drop_item(&mut self, d: &super::browser::Dragged, cx: &mut Context<Self>) {
        let Some(t) = self.track_view(cx) else { return };
        match d {
            super::browser::Dragged::Plugin(id) => {
                let is_instrument = self.scx.core.read(cx).port().plugins().iter().any(|p| p.id == *id && p.instrument);
                if is_instrument {
                    let id = id.clone();
                    self.swap(&id, cx);
                }
            }
            super::browser::Dragged::Entry(e) => {
                let Some((node, plugin)) = t.instrument else { return };
                if e.kind == lm_port::EntryKind::Preset
                    && e.info.get("plugin").and_then(Value::as_text) == Some(&*plugin)
                {
                    self.load_preset(node, e.id, e.name.clone(), cx);
                }
            }
        }
    }
}

/// Where a kit piece is drawn: a row of circles, the first (the kick) bigger
fn piece_at(b: Bounds<Pixels>, k: usize, n: usize) -> (Point<Pixels>, Pixels) {
    let w = b.size.width.as_f32() / n as f32;
    let r = if k == 0 { (w / 2.0 - 3.0).min(34.0) } else { (w / 2.0 - 4.0).min(20.0) };
    let cx_ = b.origin.x.as_f32() + w * (k as f32 + 0.5);
    let cy = b.origin.y.as_f32() + b.size.height.as_f32() / 2.0;
    (point(px(cx_), px(cy)), px(r.max(6.0)))
}

/// The keys drawn by the test keyboard: C2..C7
const LO: u8 = 36;
const HI: u8 = 96;

fn is_black(k: u8) -> bool {
    matches!(k % 12, 1 | 3 | 6 | 8 | 10)
}

fn white_index(k: u8) -> usize {
    (LO..k).filter(|x| !is_black(*x)).count()
}

/// The velocity a click on a key gives: its height on the key, as the legacy's (the top is soft, the bottom hard)
fn key_velocity(b: Bounds<Pixels>, p: Point<Pixels>) -> u8 {
    let y = ((p.y.as_f32() - b.origin.y.as_f32()) / b.size.height.as_f32().max(1.0)).clamp(0.0, 1.0);
    (30.0 + y * 97.0).round().clamp(1.0, 127.0) as u8
}

fn key_at(b: Bounds<Pixels>, p: Point<Pixels>) -> Option<u8> {
    if !b.contains(&p) {
        return None;
    }
    let whites = (LO..=HI).filter(|k| !is_black(*k)).count() as f32;
    let ww = b.size.width.as_f32() / whites;
    let x = p.x.as_f32() - b.origin.x.as_f32();
    let y = (p.y.as_f32() - b.origin.y.as_f32()) / b.size.height.as_f32();
    if y < 0.6
        && let Some(k) = (LO..=HI).find(|k| is_black(*k) && (x - white_index(*k) as f32 * ww).abs() < ww * 0.3)
    {
        return Some(k);
    }
    (LO..=HI).filter(|k| !is_black(*k)).nth((x / ww) as usize)
}

fn paint_keys(b: Bounds<Pixels>, held: Option<u8>, window: &mut Window) {
    let whites = (LO..=HI).filter(|k| !is_black(*k)).count() as f32;
    let ww = b.size.width.as_f32() / whites;
    let h = b.size.height.as_f32();
    for k in (LO..=HI).filter(|k| !is_black(*k)) {
        let x = b.origin.x.as_f32() + white_index(k) as f32 * ww;
        let col = if held == Some(k) { THEME.green } else { THEME.fader_cap };
        window.paint_quad(fill(Bounds::new(point(px(x + 0.5), b.origin.y), size(px(ww - 1.0), px(h))), c(col)));
    }
    for k in (LO..=HI).filter(|k| is_black(*k)) {
        let x = b.origin.x.as_f32() + white_index(k) as f32 * ww - ww * 0.3;
        let col = if held == Some(k) { THEME.green } else { THEME.lcd_bg };
        window.paint_quad(fill(Bounds::new(point(px(x), b.origin.y), size(px(ww * 0.6), px(h * 0.6))), c(col)));
    }
}

/// The DAHDSR envelope as one stroked path: [predelay, attack, hold, decay, sustain 0..1, release, amount -1..1]
fn paint_envelope(b: Bounds<Pixels>, e: [f64; 7], window: &mut Window) {
    let [pre, att, hold, dec, sus, rel, amount] = e;
    let total = pre + att + hold + dec + 0.4 + rel;
    let (x0, y0) = (b.origin.x.as_f32() + 6.0, b.origin.y.as_f32() + 6.0);
    let (w, h) = (b.size.width.as_f32() - 12.0, b.size.height.as_f32() - 12.0);
    let a = amount.abs().max(0.05) as f32;
    let x = |t: f64| x0 + (t / total.max(1e-6)) as f32 * w;
    let y = |v: f64| y0 + h - (v as f32) * h * a;
    let mut t = 0.0;
    let mut pb = PathBuilder::stroke(px(1.5));
    pb.move_to(point(px(x(0.0)), px(y(0.0))));
    for (dt, v) in [(pre, 0.0), (att, 1.0), (hold, 1.0), (dec, sus), (0.4, sus), (rel, 0.0)] {
        t += dt;
        pb.line_to(point(px(x(t)), px(y(v))));
    }
    if let Ok(p) = pb.build() {
        let col = if amount == 0.0 {
            THEME.text3
        } else if amount < 0.0 {
            THEME.orange
        } else {
            THEME.green
        };
        window.paint_path(p, c(col));
    }
}

/// The LFO's shape over the graph as one stroked path
fn paint_lfo(b: Bounds<Pixels>, shape: &str, speed: f64, amount: f64, window: &mut Window) {
    let (x0, mid) = (b.origin.x.as_f32() + 4.0, b.origin.y.as_f32() + b.size.height.as_f32() / 2.0);
    let w = b.size.width.as_f32() - 8.0;
    let amp = (b.size.height.as_f32() / 2.0 - 4.0) * amount.abs().max(0.05) as f32;
    let n = 160;
    let cycles = (speed * 20.0).clamp(0.5, 40.0);
    let mut pb = PathBuilder::stroke(px(1.2));
    let mut rnd = 0.37f64;
    for i in 0..=n {
        let ph = (i as f64 / n as f64 * cycles).fract();
        let v = match shape {
            "triangle" => 1.0 - 4.0 * (ph - 0.5).abs(),
            "saw" => 2.0 * ph - 1.0,
            "square" => {
                if ph < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            "random" => {
                if i % 8 == 0 {
                    rnd = (rnd * 7.13 + 0.31).fract();
                }
                rnd * 2.0 - 1.0
            }
            _ => (ph * std::f64::consts::TAU).sin(),
        };
        let p = point(px(x0 + w * i as f32 / n as f32), px(mid - v as f32 * amp));
        if i == 0 { pb.move_to(p) } else { pb.line_to(p) }
    }
    if let Ok(p) = pb.build() {
        window.paint_path(p, c(if amount == 0.0 { THEME.text3 } else { THEME.cyan }));
    }
}

impl Render for InstrumentWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // U38: the window follows the track by its feed, and closes when the track is gone
        if let Some(feed) = self.feed.as_mut() {
            let _ = self.scx.core.read(cx).port().drain(feed);
        }
        let Some(t) = self.track_view(cx) else {
            if self.track.is_some() {
                let scx = self.scx.clone();
                cx.defer(move |cx| scx.close(cx));
            }
            return div()
                .id("instrument-window")
                .size_full()
                .bg(c(THEME.bg1))
                .p(px(12.))
                .child(caption(tr("No instrument")));
        };
        if self.lit.is_some_and(|(_, at)| at.elapsed() < Duration::from_millis(260)) {
            window.request_animation_frame();
        }
        let plugins = self.scx.core.read(cx).port().plugins();
        let ctls: Vec<Ctl> = match t.instrument.clone() {
            Some((node, plugin)) => {
                let core = self.scx.core.clone();
                let schema = schema_of(&core, &plugin, cx);
                let port = core.read(cx).port();
                match (port.find(node), schema) {
                    (Some(v), Some(s)) => controls_of(node, &v, &s, port.root()),
                    _ => Vec::new(),
                }
            }
            None => Vec::new(),
        };
        self.update_panel(t.instrument.clone(), window, cx);
        let body: Vec<AnyElement> = match self.tab {
            Tab::Instrument => self.instrument_tab(&t, &ctls, &plugins, cx),
            Tab::Envelopes => self.envelopes_tab(&ctls, cx),
            Tab::NoteEffects => self.note_effects_tab(&plugins, cx),
            Tab::Effects => self.effects_tab(&t, &plugins, cx),
            Tab::Midi => self.midi_tab(cx),
            Tab::Tuning => self.tuning_tab(&t, &ctls, cx),
        };
        div()
            .id("instrument-window")
            .debug_selector(|| "instrument-window".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(c(THEME.bg1))
            .text_color(c(THEME.text))
            .on_mouse_move(cx.listener(|w, ev: &MouseMoveEvent, _, cx| drag_move(w, ev, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|w, _: &MouseUpEvent, _, cx| {
                    drag_end(w, cx);
                    // a note started here ends wherever the button comes up
                    w.release_notes(cx);
                }),
            )
            .track_focus(&self.focus)
            // keymap-ok: the piano keys
            .on_key_down(cx.listener(|w, ev: &KeyDownEvent, window, cx| w.key_down(ev, window, cx)))
            // keymap-ok: the piano keys
            .on_key_up(cx.listener(|w, ev: &KeyUpEvent, _, cx| w.key_up(ev, cx)))
            // an instrument or a preset dropped from the browser (F-IJN-004)
            .on_drop(cx.listener(|w, d: &super::browser::Dragged, _, cx| w.drop_item(d, cx)))
            .child(self.header(&t, &plugins, cx))
            .child(self.tabs(&t, cx))
            .child(
                div()
                    .id("iw-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .children(body),
            )
            .when(self.keyboard, |d| d.child(self.keyboard_view(cx)))
            .child(self.footer(&t, &ctls, cx))
            .children(popup_layer(self, cx))
    }
}
