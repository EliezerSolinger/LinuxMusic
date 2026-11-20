//! Owner: agent C. The instrument window (approved design 05): a docked panel with the track's name and colour, the
//! instrument and "Swap", the presets on top (previous, next, the list, save), text tabs (Instrument, Envelopes and
//! filter, Chords and arpeggio, Effects (n), MIDI, Tuning), and a fixed footer (volume, pitch, bend range, the note
//! range, where the outputs go, the test keyboard). The ⇱ button detaches it into its own window.
//!
//! The first half of this file is THE GENERIC PLUGIN EDITOR, shared with the effect window: one control per
//! `ParamSpec` of the plugin's schema (a knob for reals, linear or log, a stepper for stepped numbers, a switch for
//! bools, a list for choices), with the unit, the automation and MIDI-learn marks, and the gestures of every control
//! (F-CTL-005/006): drag (Shift: 4x slower), middle click = default, double click = type a value, wheel = 100 steps
//! (Shift 10, Ctrl/Cmd 1000), Shift+drag copies the value onto another control, right click = the menu (default,
//! copy, paste, automate, MIDI learn, linear/log display). A drag is one gesture: one undo step (I-3.7).
//!
//! The window knows only node ids (U38): it reads the core's root through the `CorePort` in `render`, follows the track by a `Feed`, and closes
//! when the track is removed. Every change is data (`plugins::Op`), never a closure into the core (I-15).

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use fake_core::arrange::TrackKind;
use fake_core::plugins::{self, Descriptor, Host, Kind, ParamSpec, PluginNode, Role, Scale};
use fake_core::port::{Applied, EditOp, Pending, Ref};
use fake_core::{Doc, Feed, Field, GestureId, Loading, NodeId, Target, Topic, Value};
use gpui::{
    AnyElement, App, Bounds, Context, Entity, FontWeight, Global, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PathBuilder, Pixels, Point, Render, ScrollEvent, SharedString, Subscription, Window, WindowBounds,
    WindowOptions, anchored, canvas, deferred, div, fill, point, prelude::*, px, size,
};
use gpui_component::Sizable;
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};

use super::{Screen, ScreenCx};
use crate::core::Core;
use crate::ui::{THEME, button, c, caption, knob_face, loading_component, tab, tr, trf};

// =================================================================================================================
// The generic plugin editor (shared with screens/effect.rs)
// =================================================================================================================

/// The track's own volume, shown as a knob like a parameter (the footer)
pub static TRACK_VOLUME: ParamSpec = ParamSpec {
    key: "track.volume",
    label: "Volume",
    group: "",
    kind: Kind::Real { min: -60.0, max: 12.0, def: 0.0, scale: Scale::Linear },
    unit: "dB",
    automatable: true,
};

/// A knob drag in progress: one gesture from mouse down to mouse up
pub struct ParamDrag {
    node: NodeId,
    spec: &'static ParamSpec,
    start_y: f32,
    start_t: f64,
    gesture: GestureId,
}

/// A popup of a window: the shared ones (a choice's list, a control's menu) or the window's own
#[derive(Clone, Debug, PartialEq)]
pub enum Popup<C> {
    Choice { node: NodeId, key: &'static str },
    Param { node: NodeId, key: &'static str },
    Own(C),
}

/// What the generic editor keeps in a window
pub struct EditorState<C> {
    pub drag: Option<ParamDrag>,
    pub popup: Option<(Popup<C>, Point<Pixels>)>,
    /// typing a value (double click): the node, the parameter, the field
    pub typing: Option<(NodeId, &'static str, Entity<InputState>, Subscription)>,
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
    fn core(&self) -> &Entity<Core>;
    fn editor(&mut self) -> &mut EditorState<C>;
    fn editor_ref(&self) -> &EditorState<C>;
    /// the window's own popup items
    fn own_menu(&self, popup: &C, cx: &Context<Self>) -> Vec<MenuItem<Self>>;
}

/// The value clipboard and the display options of the controls (P-06: linear/log per control is a display option of
/// the interface document, not of the plugin's schema)
#[derive(Default)]
struct EditorGlobals {
    clipboard: Option<f64>,
    flipped: HashSet<(&'static str, &'static str)>,
}
impl Global for EditorGlobals {}

fn flipped(plugin: &'static str, key: &'static str, cx: &App) -> bool {
    cx.try_global::<EditorGlobals>().is_some_and(|g| g.flipped.contains(&(plugin, key)))
}

/// The travel mapping, with the display option applied: a log parameter shown linear, or a linear one shown log
fn to_t(spec: &ParamSpec, v: f64, flip: bool) -> f64 {
    match (spec.kind, flip) {
        (Kind::Real { min, max, scale: Scale::Log, .. }, true) => ((v - min) / (max - min)).clamp(0.0, 1.0),
        (Kind::Real { min, max, scale: Scale::Linear, .. }, true) if min > 0.0 => ((v / min).ln() / (max / min).ln()).clamp(0.0, 1.0),
        _ => spec.to_travel(v),
    }
}
fn from_t(spec: &ParamSpec, t: f64, flip: bool) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match (spec.kind, flip) {
        (Kind::Real { min, max, scale: Scale::Log, .. }, true) => spec.clamp(min + t * (max - min)),
        (Kind::Real { min, max, scale: Scale::Linear, .. }, true) if min > 0.0 => spec.clamp(min * (max / min).powf(t)),
        _ => spec.from_travel(t),
    }
}

/// The plugin of a node id, for the display options ("" for the track's own volume)
fn plugin_of(core: &Core, node: NodeId) -> &'static str {
    let p = &core.port().root().plugins;
    if node.kind == b't' {
        p.functions_of(node).map_or("", |n| n.plugin)
    } else {
        p.node(node).map_or("", |n| n.plugin)
    }
}

/// The current value of a control
pub fn value_of(core: &Core, node: NodeId, spec: &ParamSpec) -> f64 {
    if spec.key == TRACK_VOLUME.key {
        return core.port().root().tracks.iter().find(|t| t.id == node).map_or(0.0, |t| t.volume_db);
    }
    let root = core.port().root();
    if node.kind == b'm' {
        // a desk channel's rack (the effect window shows it with the same controls)
        return root.channels.iter().find(|c| c.id == node).and_then(|c| c.params.get(spec.key).copied()).unwrap_or(spec.default_value());
    }
    let p = &root.plugins;
    let n = if node.kind == b't' { p.functions_of(node) } else { p.node(node) };
    n.map_or(spec.default_value(), |n| n.real(spec.key))
}

/// The edit that sets a control
pub fn op_for(node: NodeId, spec: &ParamSpec, v: f64) -> EditOp {
    if spec.key == TRACK_VOLUME.key { EditOp::set(node, Field::Volume, Value::Real(v)) } else { ops::param(node, spec.key, v) }
}

/// The edits of the plugin windows, as data (I-15): what `plugins.rs` documents in `STATE_KEYS`
pub mod ops {
    use super::*;

    pub fn param(node: NodeId, key: &'static str, v: f64) -> EditOp {
        EditOp::set(node, Field::Param(key), Value::Real(v))
    }
    /// a choice by its permanent key (U30)
    pub fn choice(node: NodeId, key: &'static str, value: &str) -> EditOp {
        EditOp::set(node, Field::Param(key), Value::Text(value.to_string()))
    }
    /// the rest of a node's state ("preset", "resource", "automated/<param>"...)
    pub fn state(node: NodeId, key: &str, v: Value) -> EditOp {
        EditOp::set(node, Field::Plugin(plugins::key(key)), v)
    }
    pub fn text(node: NodeId, key: &str, t: &str) -> EditOp {
        state(node, key, Value::Text(t.to_string()))
    }
    /// a new instrument on the track (it replaces the one there): `Ref::Temp(1)` names it later in the same list
    pub fn swap(track: NodeId, plugin: &str) -> EditOp {
        EditOp::Insert { temp: 1, parent: Ref::Id(track), list: "instrument", index: 0, node: Value::Text(plugin.to_string()) }
    }
    /// a new insert at `index` of a desk channel's chain
    pub fn insert(channel: NodeId, index: usize, plugin: &str) -> EditOp {
        EditOp::Insert { temp: 1, parent: Ref::Id(channel), list: "inserts", index, node: Value::Text(plugin.to_string()) }
    }
    /// the node created earlier in the same list
    pub fn on_new(key: &str, v: Value) -> EditOp {
        EditOp::Set { id: Ref::Temp(1), field: Field::Plugin(plugins::key(key)), value: v }
    }
}

/// Applies ops as one undo step (the core refuses the whole list on any error)
/// Over the network the answer comes in a later pump (`Waiting`): the window shows the change when the root has it,
/// and a refusal comes as a notice; nothing here waits for it.
pub fn apply_ops<V: 'static>(core: &Entity<Core>, label: &'static str, ops: Vec<EditOp>, cx: &mut Context<V>) -> Option<Applied> {
    let r = core.update(cx, |core, _| core.port_mut().apply(label, ops));
    cx.notify();
    match r {
        Pending::Ready(Ok(a)) => Some(a),
        Pending::Ready(Err(_)) | Pending::Waiting(_) => None,
    }
}

fn find_spec(core: &Core, node: NodeId, key: &str) -> Option<&'static ParamSpec> {
    if key == TRACK_VOLUME.key {
        return Some(&TRACK_VOLUME);
    }
    if node.kind == b'm' {
        return super::effect::rack_spec(key);
    }
    plugins::spec(plugin_of(core, node), key)
}

pub fn begin_drag<C: Clone + 'static, V: ParamHost<C>>(
    v: &mut V,
    node: NodeId,
    spec: &'static ParamSpec,
    ev: &MouseDownEvent,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    cx.stop_propagation();
    let core = v.core().clone();
    let value = value_of(core.read(cx), node, spec);
    if ev.modifiers.shift {
        // Shift+drag: carry the value to another control (F-CTL-006)
        v.editor().carried = Some(value);
        return;
    }
    if ev.click_count == 2 {
        start_typing(v, node, spec, value, window, cx);
        return;
    }
    let flip = flipped(plugin_of(core.read(cx), node), spec.key, cx);
    let prev = v.editor().drag.take();
    let gesture = core.update(cx, |core, _| {
        if let Some(d) = prev {
            core.port_mut().gesture_commit(d.gesture);
        }
        core.port_mut().begin_gesture("Change a parameter")
    });
    v.editor().drag = Some(ParamDrag { node, spec, start_y: ev.position.y.as_f32(), start_t: to_t(spec, value, flip), gesture });
    cx.notify();
}

/// On the window's root: the drag's moves
pub fn drag_move<C: Clone + 'static, V: ParamHost<C>>(v: &mut V, ev: &MouseMoveEvent, cx: &mut Context<V>) {
    let core = v.core().clone();
    let Some(d) = &v.editor_ref().drag else { return };
    let (node, spec) = (d.node, d.spec);
    // 200 px for the whole travel; Shift: 4x slower (F-CTL-006)
    let slow = if ev.modifiers.shift { 4.0 } else { 1.0 };
    let t = d.start_t - ((ev.position.y.as_f32() - d.start_y) / (200.0 * slow)) as f64;
    let flip = flipped(plugin_of(core.read(cx), node), spec.key, cx);
    let value = from_t(spec, t, flip);
    core.update(cx, |core, _| {
        let g = &v.editor_ref().drag.as_ref().unwrap().gesture;
        core.port_mut().gesture_apply(g, vec![op_for(node, spec, value)]);
    });
    cx.notify();
}

/// On the window's root: the end of a drag, or a carried value dropped nowhere
pub fn drag_end<C: Clone + 'static, V: ParamHost<C>>(v: &mut V, cx: &mut Context<V>) {
    let core = v.core().clone();
    if let Some(d) = v.editor().drag.take() {
        core.update(cx, |core, _| core.port_mut().gesture_commit(d.gesture));
        cx.notify();
    }
    v.editor().carried = None;
}

fn start_typing<C: Clone + 'static, V: ParamHost<C>>(
    v: &mut V,
    node: NodeId,
    spec: &'static ParamSpec,
    value: f64,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    let text = match spec.kind {
        Kind::Real { scale: Scale::Stepped, .. } => format!("{value:.0}"),
        _ => format!("{value:.2}"),
    };
    let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
    input.update(cx, |i, cx| i.focus(window, cx));
    let core = v.core().clone();
    let sub = cx.subscribe(&input, move |this: &mut V, input, ev: &InputEvent, cx| match ev {
        InputEvent::PressEnter { .. } => {
            let text = input.read(cx).value().to_string();
            // a number, with or without its unit; kHz and s are understood
            let t = text.trim().to_lowercase();
            let (num, mul) = if let Some(n) = t.strip_suffix("khz") {
                (n.to_string(), 1000.0)
            } else if let Some(n) = t.strip_suffix(" s") {
                (n.to_string(), 1000.0)
            } else {
                (t.clone(), 1.0)
            };
            let num: String = num.chars().filter(|ch| ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+' | ',')).collect();
            if let Ok(x) = num.replace(',', ".").parse::<f64>() {
                apply_ops(&core, "Type a value", vec![op_for(node, spec, spec.clamp(x * mul))], cx);
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
    v.editor().typing = Some((node, spec.key, input, sub));
    cx.notify();
}

/// One control as drawn: what the editor needs to know about it
#[derive(Clone)]
pub struct Ctl {
    pub node: NodeId,
    pub spec: &'static ParamSpec,
    pub value: f64,
    pub choice: &'static str,
    pub automated: bool,
    pub cc: Option<u8>,
    pub enabled: bool,
    /// a text in place of the value (a synced time shows the note)
    pub shown: Option<String>,
}

pub fn ctl(n: &PluginNode, spec: &'static ParamSpec) -> Ctl {
    Ctl {
        node: n.id,
        spec,
        value: n.real(spec.key),
        choice: n.choice(spec.key),
        automated: n.automated.contains(spec.key),
        cc: n.learned.get(spec.key).copied(),
        enabled: true,
        shown: None,
    }
}

fn ctl_id(node: NodeId, key: &str) -> SharedString {
    SharedString::from(format!("{}{}-{key}", node.kind as char, node.number))
}

/// The marks of a control: automation (an orange dot) and a MIDI controller (its CC)
fn marks(c_: &Ctl) -> impl IntoElement {
    div()
        .flex()
        .gap(px(2.))
        .h(px(9.))
        .when(c_.automated, |d| d.child(div().size(px(6.)).mt(px(1.)).rounded(px(3.)).bg(c(THEME.orange))))
        .when_some(c_.cc, |d, cc| d.child(div().text_size(px(7.5)).text_color(c(THEME.cyan)).child(format!("CC{cc}"))))
}

fn sel(id: &SharedString) -> impl Fn() -> String + 'static {
    let id = id.clone();
    move || format!("ctl-{id}")
}

/// One control by the kind of its parameter
pub fn control<C: Clone + 'static, V: ParamHost<C>>(this: &V, c_: Ctl, cx: &Context<V>) -> AnyElement {
    let spec = c_.spec;
    let node = c_.node;
    let id = ctl_id(node, spec.key);
    let label = tr(spec.label);
    let value_color = if c_.automated { THEME.orange } else { THEME.text };
    let typing = this.editor_ref().typing.as_ref().filter(|(n, k, _, _)| *n == node && *k == spec.key).map(|t| t.2.clone());
    let value_text: AnyElement = match typing {
        Some(input) => div().w(px(70.)).child(Input::new(&input).small()).into_any_element(),
        None => div()
            .text_size(px(10.))
            .text_color(c(value_color))
            .whitespace_nowrap()
            .child(c_.shown.clone().unwrap_or_else(|| spec.format(c_.value)))
            .into_any_element(),
    };
    let body = match spec.kind {
        Kind::Real { scale: Scale::Stepped, min, max, .. } => {
            let v0 = c_.value;
            let step = |d: f64, s: &'static str| {
                button(SharedString::from(format!("{id}-{s}")), s, false, 0).on_click(cx.listener(move |v: &mut V, _, _, cx| {
                    let core = v.core().clone();
                    apply_ops(&core, "Change a parameter", vec![op_for(node, spec, (v0 + d).clamp(min, max))], cx);
                }))
            };
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .min_w(px(64.))
                .child(div().flex().gap(px(4.)).child(div().text_size(px(9.5)).text_color(c(THEME.text2)).child(label)).child(marks(&c_)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(3.))
                        .child(step(-1.0, "−"))
                        .child(
                            div()
                                .id(SharedString::from(format!("{id}-v")))
                                .debug_selector(sel(&id))
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
                                .on_mouse_down(MouseButton::Left, cx.listener(move |v: &mut V, ev: &MouseDownEvent, w, cx| begin_drag(v, node, spec, ev, w, cx))),
                        )
                        .child(step(1.0, "+")),
                )
                .into_any_element()
        }
        Kind::Real { .. } => {
            let flip = flipped(plugin_of(this.core().read(cx), node), spec.key, cx);
            let t = to_t(spec, c_.value, flip);
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(1.))
                .w(px(66.))
                .child(marks(&c_))
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-k")))
                        .debug_selector(sel(&id))
                        .cursor_ns_resize()
                        .child(knob_face(t, 30.0))
                        .on_mouse_down(MouseButton::Left, cx.listener(move |v: &mut V, ev: &MouseDownEvent, w, cx| begin_drag(v, node, spec, ev, w, cx)))
                        .on_scroll(cx.listener(move |v: &mut V, ev: &ScrollEvent, _, cx| {
                            // wheel: 100 steps for the whole travel; Shift 10, Ctrl/Cmd 1000 (F-CTL-006)
                            let dy = ev.delta.pixel_delta(px(16.)).y.as_f32();
                            if dy == 0.0 {
                                return;
                            }
                            let steps = if ev.modifiers.shift { 10.0 } else if ev.modifiers.control || ev.modifiers.platform { 1000.0 } else { 100.0 };
                            let core = v.core().clone();
                            let flip = flipped(plugin_of(core.read(cx), node), spec.key, cx);
                            let now = value_of(core.read(cx), node, spec);
                            let t = to_t(spec, now, flip) + f64::from(dy.signum()) / steps;
                            apply_ops(&core, "Change a parameter", vec![op_for(node, spec, from_t(spec, t, flip))], cx);
                            cx.stop_propagation();
                        })),
                )
                .child(div().text_size(px(9.5)).text_color(c(THEME.text2)).max_w(px(64.)).overflow_hidden().whitespace_nowrap().child(label))
                .child(value_text)
                .into_any_element()
        }
        Kind::Bool { .. } => {
            let on = c_.value > 0.5;
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .min_w(px(64.))
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-b")))
                        .debug_selector(sel(&id))
                        .w(px(26.))
                        .h(px(14.))
                        .rounded(px(7.))
                        .bg(c(if on { THEME.green } else { THEME.bg4 }))
                        .cursor_pointer()
                        .child(div().size(px(10.)).mt(px(2.)).ml(px(if on { 14. } else { 2. })).rounded(px(5.)).bg(c(THEME.text)))
                        .on_click(cx.listener(move |v: &mut V, _, _, cx| {
                            let core = v.core().clone();
                            apply_ops(&core, "Switch", vec![op_for(node, spec, if on { 0.0 } else { 1.0 })], cx);
                        })),
                )
                .child(div().text_size(px(10.5)).text_color(c(if on { THEME.text } else { THEME.text2 })).child(label))
                .child(marks(&c_))
                .into_any_element()
        }
        Kind::Choice { .. } => div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w(px(90.))
            .child(div().flex().gap(px(4.)).child(div().text_size(px(9.5)).text_color(c(THEME.text2)).child(label)).child(marks(&c_)))
            .child(
                div()
                    .id(SharedString::from(format!("{id}-c")))
                    .debug_selector(sel(&id))
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
                    .text_color(c(value_color))
                    .cursor_pointer()
                    .child(tr(spec.choice_label(c_.choice)))
                    .child(div().text_color(c(THEME.text3)).child("▾"))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |v: &mut V, ev: &MouseDownEvent, _, cx| {
                        v.editor().popup = Some((Popup::Choice { node, key: spec.key }, ev.position));
                        cx.stop_propagation();
                        cx.notify();
                    })),
            )
            .into_any_element(),
    };
    // the gestures every control shares: the menu, middle click, a carried value dropped here
    let wrapped = div()
        .id(SharedString::from(format!("{id}-w")))
        .child(body)
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |v: &mut V, ev: &MouseDownEvent, _, cx| {
                v.editor().popup = Some((Popup::Param { node, key: spec.key }, ev.position));
                cx.stop_propagation();
                cx.notify();
            }),
        )
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |v: &mut V, _, _, cx| {
                let core = v.core().clone();
                apply_ops(&core, "Back to the default", vec![op_for(node, spec, spec.default_value())], cx);
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |v: &mut V, _: &MouseUpEvent, _, cx| {
                if let Some(x) = v.editor().carried.take() {
                    let core = v.core().clone();
                    apply_ops(&core, "Copy a value", vec![op_for(node, spec, spec.clamp(x))], cx);
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

/// The generic editor of a node: a card per group of the schema (only `groups` when given)
pub fn generic_editor<C: Clone + 'static, V: ParamHost<C>>(
    this: &V,
    n: &PluginNode,
    d: &'static Descriptor,
    groups: Option<&[&str]>,
    cx: &Context<V>,
) -> Vec<AnyElement> {
    d.groups()
        .into_iter()
        .filter(|g| groups.is_none_or(|gs| gs.contains(g)))
        .map(|g| {
            let controls = d.params.iter().filter(|p| p.group == g).map(|p| control(this, ctl(n, p), cx)).collect();
            group_card(tr(g), controls)
        })
        .collect()
}

/// A popup item
pub struct MenuItem<V> {
    pub label: SharedString,
    pub checked: bool,
    pub enabled: bool,
    pub action: Rc<dyn Fn(&mut V, &mut Window, &mut Context<V>)>,
}

impl<V> MenuItem<V> {
    pub fn new(label: impl Into<SharedString>, action: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static) -> Self {
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
    /// a non-clickable line (a heading)
    pub fn heading(label: impl Into<SharedString>) -> Self {
        MenuItem { label: label.into(), checked: false, enabled: false, action: Rc::new(|_, _, _| {}) }
    }
}

/// The items of the shared popups
fn shared_menu<C: Clone + 'static, V: ParamHost<C>>(this: &V, p: &Popup<C>, cx: &Context<V>) -> Vec<MenuItem<V>> {
    let core = this.core().read(cx);
    match p {
        Popup::Choice { node, key } => {
            let Some(spec) = find_spec(core, *node, key) else { return Vec::new() };
            let Kind::Choice { options, .. } = spec.kind else { return Vec::new() };
            let pl = &core.port().root().plugins;
            let cur = if node.kind == b't' { pl.functions_of(*node) } else { pl.node(*node) }.map_or("", |n| n.choice(key));
            let node = *node;
            let key = spec.key;
            options
                .iter()
                .map(|(k, l)| {
                    let k: &'static str = k;
                    MenuItem::new(tr(l), move |v: &mut V, _, cx| {
                        let core = v.core().clone();
                        apply_ops(&core, "Choose", vec![ops::choice(node, key, k)], cx);
                    })
                    .checked(k == cur)
                })
                .collect()
        }
        Popup::Param { node, key } => {
            let Some(spec) = find_spec(core, *node, key) else { return Vec::new() };
            let node = *node;
            let pl = &core.port().root().plugins;
            let n = if node.kind == b't' { pl.functions_of(node) } else { pl.node(node) };
            let automated = n.is_some_and(|n| n.automated.contains(spec.key));
            let learned = n.and_then(|n| n.learned.get(spec.key).copied());
            let plugin = plugin_of(core, node);
            let is_choice = matches!(spec.kind, Kind::Choice { .. });
            let def = spec.default_value();
            let clip = cx.try_global::<EditorGlobals>().and_then(|g| g.clipboard);
            let mut items = vec![
                MenuItem::new(trf("Back to the default ({0})", &[&spec.format(def)]), move |v: &mut V, _, cx| {
                    let core = v.core().clone();
                    apply_ops(&core, "Back to the default", vec![op_for(node, spec, def)], cx);
                })
                .enabled(!is_choice),
                MenuItem::new(tr("Copy the value"), move |v: &mut V, _, cx| {
                    let x = value_of(v.core().read(cx), node, spec);
                    cx.default_global::<EditorGlobals>().clipboard = Some(x);
                })
                .enabled(!is_choice),
                MenuItem::new(
                    match clip {
                        Some(x) => trf("Paste the value ({0})", &[&spec.format(spec.clamp(x))]),
                        None => tr("Paste the value").to_string(),
                    },
                    move |v: &mut V, _, cx| {
                        if let Some(x) = clip {
                            let core = v.core().clone();
                            apply_ops(&core, "Paste a value", vec![op_for(node, spec, spec.clamp(x))], cx);
                        }
                    },
                )
                .enabled(clip.is_some() && !is_choice),
            ];
            if spec.key != TRACK_VOLUME.key && node.kind != b'm' {
                items.push(
                    MenuItem::new(if automated { tr("Remove the automation lane") } else { tr("Automate (add a lane)") }, move |v: &mut V, _, cx| {
                        let core = v.core().clone();
                        apply_ops(&core, "Automate", vec![ops::state(node, &format!("automated/{}", spec.key), Value::Bool(!automated))], cx);
                    })
                    .enabled(spec.automatable),
                );
                items.push(
                    MenuItem::new(
                        match learned {
                            Some(cc) => trf("Forget MIDI CC {0}", &[&cc]),
                            None => tr("MIDI learn (the next controller moved)").to_string(),
                        },
                        move |v: &mut V, _, cx| {
                            let core = v.core().clone();
                            apply_ops(&core, "MIDI learn", vec![ops::state(node, &format!("learned/{}", spec.key), Value::Bool(learned.is_none()))], cx);
                        },
                    )
                    .enabled(spec.automatable),
                );
            }
            if let Kind::Real { min, scale, .. } = spec.kind {
                if scale == Scale::Log || (scale == Scale::Linear && min > 0.0) {
                    let on = flipped(plugin, spec.key, cx);
                    let shown_log = (scale == Scale::Log) != on;
                    items.push(
                        MenuItem::new(tr("Logarithmic scale"), move |_: &mut V, _, cx| {
                            let g = cx.default_global::<EditorGlobals>();
                            if !g.flipped.remove(&(plugin, spec.key)) {
                                g.flipped.insert((plugin, spec.key));
                            }
                            cx.notify();
                        })
                        .checked(shown_log),
                    );
                }
            }
            items
        }
        Popup::Own(o) => this.own_menu(o, cx),
    }
}

/// The popup layer of a window (drawn over everything, at the mouse; a click outside closes it)
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
                    .min_w(px(180.))
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
                            .py(px(4.))
                            .flex()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .text_color(c(if enabled { THEME.text } else { THEME.text3 }))
                            .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(c(THEME.bg4))))
                            .child(div().w(px(10.)).text_color(c(THEME.green)).child(if it.checked { "✓" } else { "" }))
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

/// The missing-file state (H17), in place of the plugin's controls: "locate file" and "locate folder".
/// TODO(lm-gui): this belongs in the shared loading component once `Loading::Missing` is in the core's surface.
pub fn missing_panel<C: Clone + 'static, V: ParamHost<C>>(node: NodeId, file: &str, cx: &Context<V>) -> AnyElement {
    let name = file.rsplit('/').next().unwrap_or(file).to_string();
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(c(THEME.orange))
        .bg(c(THEME.bg2))
        .child(div().text_size(px(12.)).font_weight(FontWeight::BOLD).text_color(c(THEME.orange)).child(tr("File not found")))
        .child(div().text_size(px(11.)).text_color(c(THEME.text2)).child(trf("The project expects it at {0}", &[&file])))
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(button("locate-file", tr("Locate file…"), false, 0).h(px(22.)).px(px(8.)).on_click(cx.listener(move |v: &mut V, _, _, cx| {
                    let found = format!("/home/ana/Amostras/{name}");
                    // checking the file's hash is an operation the core starts (H17)
                    let core = v.core().clone();
                    apply_ops(&core, "Locate the file", vec![ops::text(node, "resource", &found)], cx);
                })))
                .child(button("locate-folder", tr("Locate folder…"), false, 0).h(px(22.)).px(px(8.)).on_click(cx.listener(|v: &mut V, _, _, cx| {
                    // every missing file of the library that moved, in one step
                    let core = v.core().clone();
                    let edits: Vec<EditOp> = core
                        .read(cx)
                        .port()
                        .root()
                        .plugins
                        .nodes
                        .values()
                        .filter(|n| n.missing)
                        .map(|n| {
                            let f = n.resource.clone().unwrap_or_default();
                            ops::text(n.id, "resource", &format!("/home/ana/Amostras/{}", f.rsplit('/').next().unwrap_or(&f)))
                        })
                        .collect();
                    apply_ops(&core, "Locate the folder", edits, cx);
                }))),
        )
        .into_any_element()
}

/// The chip of a plugin: its category's initials, and a colour for third-party plugins (they run outside)
pub fn plugin_chip(d: Option<&'static Descriptor>) -> impl IntoElement {
    let (short, color) = match d {
        Some(d) if d.host != Host::Internal => (d.short, THEME.purple),
        Some(d) if d.role == Role::Instrument => (d.short, THEME.blue),
        Some(d) => (d.short, THEME.cyan),
        None => ("?", THEME.red),
    };
    div()
        .px(px(4.))
        .h(px(16.))
        .flex()
        .items_center()
        .rounded(px(3.))
        .bg(c(color))
        .text_color(c(THEME.on_accent))
        .text_size(px(9.))
        .font_weight(FontWeight::BOLD)
        .child(short)
}

/// A note's name (MIDI numbering, C4 = 60)
pub fn note_name(k: f64) -> String {
    const N: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let k = k.round() as i32;
    format!("{}{}", N[k.rem_euclid(12) as usize], k.div_euclid(12) - 1)
}

/// The loading component's "Cancel": the operations the target's loading waits on
pub fn cancel_target(core: &Entity<Core>, target: Target, cx: &mut App) {
    core.update(cx, |core, _| {
        let ops: Vec<u64> = core.port().activity().iter().filter(|a| a.target == target).map(|a| a.op).collect();
        for op in ops {
            core.port_mut().cancel(op);
        }
    });
}

/// A message in a window (an instrument the tab does not apply to)
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

/// Which track the instrument window shows: set by whoever opens it (a track header, the browser, the palette)
pub struct InstrumentFocus(pub NodeId);
impl Global for InstrumentFocus {}

/// Opens the instrument window on a track (or moves it there if already open)
pub fn show_track(scx: &ScreenCx, track: NodeId, window: &mut Window, cx: &mut App) {
    cx.set_global(InstrumentFocus(track));
    scx.open(Screen::Instrument, window, cx);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Instrument,
    Envelopes,
    Chords,
    Effects,
    Midi,
    Tuning,
}

impl Tab {
    pub const ALL: [Tab; 6] = [Tab::Instrument, Tab::Envelopes, Tab::Chords, Tab::Effects, Tab::Midi, Tab::Tuning];
    fn title(self) -> &'static str {
        match self {
            Tab::Instrument => "Instrument",
            Tab::Envelopes => "Envelopes and filter",
            Tab::Chords => "Chords and arpeggio",
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
    /// the destination of a track output, by its index
    Output(usize),
}

/// The editor of a scale or keymap slot (F-IJN-020)
struct TuningEditor {
    keymap: bool,
    slot: usize,
    description: Entity<InputState>,
    text: Entity<TextareaState>,
    /// the keymap's numbers: first, last, middle, base key, base frequency
    numbers: [f64; 5],
    error: Option<String>,
}

pub struct InstrumentWindow {
    scx: ScreenCx,
    track: Option<NodeId>,
    feed: Option<Feed>,
    tab: Tab,
    /// the envelope's target: volume, cutoff, resonance
    target: usize,
    keyboard: bool,
    /// the test keyboard's key held, and its velocity
    held: Option<(u8, f32)>,
    keys_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    kit_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// a kit piece lit by a click, and when
    lit: Option<(usize, Instant)>,
    /// the third-party plugin's own window is open (it runs outside, P-05)
    external_open: bool,
    detached: bool,
    tuning: Option<TuningEditor>,
    editor: EditorState<IwPopup>,
    _subs: Vec<Subscription>,
}

impl ParamHost<IwPopup> for InstrumentWindow {
    fn core(&self) -> &Entity<Core> {
        &self.scx.core
    }
    fn editor(&mut self) -> &mut EditorState<IwPopup> {
        &mut self.editor
    }
    fn editor_ref(&self) -> &EditorState<IwPopup> {
        &self.editor
    }
    fn own_menu(&self, p: &IwPopup, cx: &Context<Self>) -> Vec<MenuItem<Self>> {
        let s = self.scx.core.read(cx).port().root();
        let Some(track) = self.track else { return Vec::new() };
        match p {
            IwPopup::Tracks => s
                .tracks
                .iter()
                .filter(|t| s.plugins.instrument_of(t.id).is_some())
                .map(|t| {
                    let id = t.id;
                    MenuItem::new(t.name.clone(), move |w: &mut Self, _, cx| w.set_track(Some(id), cx)).checked(id == track)
                })
                .collect(),
            IwPopup::Swap => {
                let cur = s.plugins.instrument_of(track).map_or("", |n| n.plugin);
                let mut items = Vec::new();
                let mut last = "";
                let mut list: Vec<&Descriptor> = plugins::all().filter(|d| d.role == Role::Instrument).collect();
                list.sort_by_key(|d| (d.host != Host::Internal, d.category));
                for d in list {
                    if d.category != last {
                        items.push(MenuItem::heading(tr(d.category)));
                        last = d.category;
                    }
                    let id = d.id;
                    items.push(
                        MenuItem::new(tr(d.id), move |w: &mut Self, _, cx| {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Swap the instrument", vec![ops::swap(track, id)], cx);
                        })
                        .checked(id == cur),
                    );
                }
                items
            }
            IwPopup::Presets => {
                let Some(n) = s.plugins.instrument_of(track) else { return Vec::new() };
                let node = n.id;
                let cur = n.preset.clone();
                let factory: Vec<&str> = n.descriptor().map(|d| d.presets.iter().map(|p| p.name).collect()).unwrap_or_default();
                let mut items = vec![MenuItem::heading(tr("Factory presets"))];
                let mut user_heading = false;
                for name in plugins::presets_of(n.plugin) {
                    if !factory.contains(&name.as_str()) && !user_heading {
                        items.push(MenuItem::heading(tr("Your presets")));
                        user_heading = true;
                    }
                    let checked = cur.as_deref() == Some(name.as_str());
                    let nm = name.clone();
                    items.push(
                        MenuItem::new(name, move |w: &mut Self, _, cx| {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Apply a preset", vec![ops::text(node, "preset", &nm)], cx);
                        })
                        .checked(checked),
                    );
                }
                items
            }
            IwPopup::Output(k) => {
                let Some(t) = s.tracks.iter().find(|t| t.id == track) else { return Vec::new() };
                let Some(o) = t.outputs.get(*k) else { return Vec::new() };
                let output = o.id;
                let to = o.to;
                s.channels
                    .iter()
                    .map(|ch| {
                        let channel = ch.id;
                        MenuItem::new(ch.name.clone(), move |w: &mut Self, _, cx| {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Route an output", vec![EditOp::set(track, Field::OutputTo(output), Value::Id(channel))], cx);
                        })
                        .checked(channel == to)
                    })
                    .collect()
            }
        }
    }
}

impl InstrumentWindow {
    pub fn new(scx: ScreenCx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&scx.core, |_, _, cx| cx.notify());
        let focus = cx.observe_global::<InstrumentFocus>(|w: &mut Self, cx| {
            let t = cx.global::<InstrumentFocus>().0;
            w.set_track(Some(t), cx);
        });
        let mut w = InstrumentWindow {
            scx,
            track: None,
            feed: None,
            tab: Tab::Instrument,
            target: 0,
            keyboard: false,
            held: None,
            keys_bounds: Rc::new(Cell::new(None)),
            kit_bounds: Rc::new(Cell::new(None)),
            lit: None,
            external_open: false,
            detached: false,
            tuning: None,
            editor: EditorState::default(),
            _subs: vec![observe, focus],
        };
        let first = cx.try_global::<InstrumentFocus>().map(|f| f.0).or_else(|| {
            let s = w.scx.core.read(cx).port().root();
            s.tracks.iter().find(|t| t.kind == TrackKind::Instrument && s.plugins.instrument_of(t.id).is_some()).map(|t| t.id)
        });
        w.set_track(first, cx);
        w
    }

    fn set_track(&mut self, track: Option<NodeId>, cx: &mut Context<Self>) {
        self.track = track;
        self.tuning = None;
        self.feed = track.map(|t| self.scx.core.update(cx, |core, _| core.port_mut().feed(Topic::Node(t))));
        cx.notify();
    }

    /// The previous or next instrument track (F-IJN-001)
    fn step_track(&mut self, d: i32, cx: &mut Context<Self>) {
        let s = self.scx.core.read(cx).port().root();
        let list: Vec<NodeId> = s.tracks.iter().filter(|t| s.plugins.instrument_of(t.id).is_some()).map(|t| t.id).collect();
        if list.is_empty() {
            return;
        }
        let at = self.track.and_then(|t| list.iter().position(|x| *x == t)).unwrap_or(0) as i32;
        let next = list[(at + d).rem_euclid(list.len() as i32) as usize];
        self.set_track(Some(next), cx);
    }

    fn step_preset(&mut self, d: i32, cx: &mut Context<Self>) {
        let Some(track) = self.track else { return };
        let s = self.scx.core.read(cx).port().root();
        let Some(n) = s.plugins.instrument_of(track) else { return };
        let list = plugins::presets_of(n.plugin);
        if list.is_empty() {
            return;
        }
        let at = n.preset.as_ref().and_then(|p| list.iter().position(|x| x == p)).map_or(if d > 0 { -1 } else { 0 }, |k| k as i32);
        let name = list[(at + d).rem_euclid(list.len() as i32) as usize].clone();
        let node = n.id;
        let core = self.scx.core.clone();
        apply_ops(&core, "Apply a preset", vec![ops::text(node, "preset", &name)], cx);
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.detached {
            window.remove_window();
        } else {
            self.scx.close(cx);
        }
    }

    /// ⇱: the window floats on its own (design 05); its ⇲ docks it again
    fn detach(&mut self, cx: &mut Context<Self>) {
        let scx = self.scx.clone();
        let track = self.track;
        let tab = self.tab;
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(200.), px(120.)), size(px(460.), px(760.))))),
                ..Default::default()
            },
            move |window, cx| {
                let v = cx.new(|cx| {
                    let mut w = InstrumentWindow::new(scx.clone(), window, cx);
                    w.detached = true;
                    w.tab = tab;
                    w.set_track(track, cx);
                    w
                });
                cx.new(|cx| gpui_component::Root::new(v, window, cx))
            },
        );
        if opened.is_ok() {
            self.scx.close(cx);
        }
    }

    fn dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.track {
            cx.set_global(InstrumentFocus(t));
        }
        let scx = self.scx.clone();
        scx.open(Screen::Instrument, window, cx);
        window.remove_window();
    }

    // --- header, tabs, footer ---------------------------------------------------------------------------------

    fn header(&self, cx: &Context<Self>) -> impl IntoElement {
        let s = self.scx.core.read(cx).port().root();
        let track = self.track.and_then(|id| s.tracks.iter().find(|t| t.id == id));
        let (name, color, mute, solo) = track.map_or((String::new(), THEME.text3, false, false), |t| (t.name.clone(), t.color, t.mute, t.solo));
        let tid = self.track;
        let n = tid.and_then(|t| s.plugins.instrument_of(t));
        let d = n.and_then(|n| n.descriptor());
        let plugin_name = n.map_or("", |n| n.plugin);
        let preset: String = match n.and_then(|n| n.preset.clone()) {
            Some(p) => format!("{}{}", p, if n.is_some_and(|n| n.modified) { " *" } else { "" }),
            None => tr("No preset").to_string(),
        };
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
                            .child(name)
                            .on_mouse_down(MouseButton::Left, cx.listener(|w, ev: &MouseDownEvent, _, cx| {
                                w.editor.popup = Some((Popup::Own(IwPopup::Tracks), ev.position));
                                cx.notify();
                            })),
                    )
                    .child(button("iw-prev", "‹", false, 0).on_click(cx.listener(|w, _, _, cx| w.step_track(-1, cx))))
                    .child(button("iw-next", "›", false, 0).on_click(cx.listener(|w, _, _, cx| w.step_track(1, cx))))
                    .child(button("iw-mute", "M", mute, THEME.orange).on_click(cx.listener(move |w, _, _, cx| {
                        if let Some(track) = tid {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Mute", vec![EditOp::set(track, Field::Mute, Value::Bool(!mute))], cx);
                        }
                    })))
                    .child(button("iw-solo", "S", solo, THEME.yellow).on_click(cx.listener(move |w, _, _, cx| {
                        if let Some(track) = tid {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Solo", vec![EditOp::set(track, Field::Solo, Value::Bool(!solo))], cx);
                        }
                    })))
                    .child(if self.detached {
                        button("iw-dock", "⇲", false, 0).on_click(cx.listener(|w, _, window, cx| w.dock(window, cx)))
                    } else {
                        button("iw-detach", "⇱", false, 0).on_click(cx.listener(|w, _, _, cx| w.detach(cx)))
                    })
                    .child(button("iw-close", "×", false, 0).on_click(cx.listener(|w, _, window, cx| w.close(window, cx)))),
            )
            .child(
                line()
                    .child(plugin_chip(d))
                    .child(div().text_size(px(12.)).text_color(c(THEME.text2)).child(tr(plugin_name)))
                    .child(
                        button("iw-swap", tr("Swap ▾"), false, 0)
                            .debug_selector(|| "iw-swap".into())
                            .on_mouse_down(MouseButton::Left, cx.listener(|w, ev: &MouseDownEvent, _, cx| {
                                w.editor.popup = Some((Popup::Own(IwPopup::Swap), ev.position));
                                cx.stop_propagation();
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .when(d.is_some_and(|d| d.host != Host::Internal), |e| {
                        e.child(div().text_size(px(9.5)).text_color(c(THEME.purple)).child(tr("outside the program")))
                    }),
            )
            .child(
                line()
                    .child(button("iw-preset-prev", "◀", false, 0).on_click(cx.listener(|w, _, _, cx| w.step_preset(-1, cx))))
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
                            .on_mouse_down(MouseButton::Left, cx.listener(|w, ev: &MouseDownEvent, _, cx| {
                                w.editor.popup = Some((Popup::Own(IwPopup::Presets), ev.position));
                                cx.notify();
                            })),
                    )
                    .child(button("iw-preset-next", "▶", false, 0).on_click(cx.listener(|w, _, _, cx| w.step_preset(1, cx))))
                    .child(button("iw-save", tr("Save preset"), false, 0).on_click(cx.listener(|w, _, _, cx| {
                        let Some(track) = w.track else { return };
                        let saved = {
                            let s = w.scx.core.read(cx).port().root();
                            s.plugins.instrument_of(track).map(|n| (n.id, plugins::new_preset_name(n)))
                        };
                        if let Some((node, name)) = saved {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "Save a preset", vec![ops::text(node, "preset.save", &name)], cx);
                        }
                    }))),
            )
    }

    fn tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        let effects = self.output_channel(cx).map_or(0, |ch| {
            let s = self.scx.core.read(cx).port().root();
            s.channels.iter().find(|c| c.id == ch).map_or(0, |c| c.inserts.len())
        });
        div()
            .flex()
            .flex_wrap()
            .px(px(4.))
            .bg(c(THEME.bg2))
            .border_b_1()
            .border_color(c(THEME.line))
            .children(Tab::ALL.into_iter().map(|t| {
                let label: SharedString = if t == Tab::Effects { trf("Effects ({0})", &[&effects]).into() } else { tr(t.title()).into() };
                tab(SharedString::from(format!("iw-tab-{t:?}")), label, self.tab == t)
                    .debug_selector(move || format!("iw-tab-{t:?}"))
                    .px(px(8.))
                    .text_size(px(12.))
                    .on_click(cx.listener(move |w, _, _, cx| {
                        w.tab = t;
                        cx.notify();
                    }))
            }))
    }

    /// The desk channel of the track's main output
    fn output_channel(&self, cx: &Context<Self>) -> Option<NodeId> {
        let s = self.scx.core.read(cx).port().root();
        s.tracks.iter().find(|t| Some(t.id) == self.track).and_then(|t| t.outputs.first()).map(|o| o.to)
    }

    fn footer(&self, cx: &Context<Self>) -> impl IntoElement {
        let s = self.scx.core.read(cx).port().root();
        let Some(track) = self.track else { return div() };
        let Some(t) = s.tracks.iter().find(|t| t.id == track) else { return div() };
        let f = s.plugins.functions_of(track);
        let fd = plugins::descriptor(plugins::FUNCTIONS).unwrap();
        let vol = Ctl {
            node: track,
            spec: &TRACK_VOLUME,
            value: t.volume_db,
            choice: "",
            automated: t.lanes.iter().any(|l| l.param == "Volume"),
            cc: None,
            enabled: true,
            shown: None,
        };
        let mut controls: Vec<AnyElement> = vec![control(self, vol, cx)];
        if let Some(f) = f {
            for k in ["tune.pitch", "tune.range"] {
                controls.push(control(self, ctl(f, fd.param(k).unwrap()), cx));
            }
        }
        let range = f.map_or(String::new(), |f| format!("{}–{}", note_name(f.real("keys.first")), note_name(f.real("keys.last"))));
        let out_text: SharedString = match t.outputs.len() {
            0 => tr("No output").into(),
            1 => trf("→ {0}", &[&s.channels.iter().find(|c| c.id == t.outputs[0].to).map_or("?", |c| c.name.as_str())]).into(),
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
                    .child(button("iw-range", trf("Notes: {0}", &[&range]), false, 0).h(px(22.)).on_click(cx.listener(|w, _, _, cx| {
                        w.tab = Tab::Tuning;
                        cx.notify();
                    })))
                    .child(
                        button("iw-out", out_text, false, 0)
                            .h(px(22.))
                            .debug_selector(|| "iw-out".into())
                            .on_mouse_down(MouseButton::Left, cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                                if multi {
                                    w.tab = Tab::Instrument;
                                } else {
                                    w.editor.popup = Some((Popup::Own(IwPopup::Output(0)), ev.position));
                                }
                                cx.stop_propagation();
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(button("iw-keyboard", tr("Keyboard"), self.keyboard, THEME.green).h(px(22.)).on_click(cx.listener(|w, _, _, cx| {
                        w.keyboard = !w.keyboard;
                        cx.notify();
                    }))),
            )
    }

    // --- the test keyboard (F-IJN-021): click plays (velocity by height), the range shaded, the base key marked --

    fn keyboard_view(&self, cx: &Context<Self>) -> impl IntoElement {
        let s = self.scx.core.read(cx).port().root();
        let f = self.track.and_then(|t| s.plugins.functions_of(t));
        let (first, last, base) = f.map_or((0.0, 127.0, 69.0), |f| (f.real("keys.first"), f.real("keys.last"), f.real("keys.base")));
        let held = self.held;
        let bounds = self.keys_bounds.clone();
        div()
            .id("iw-keys")
            .debug_selector(|| "iw-keys".into())
            .h(px(64.))
            .px(px(6.))
            .py(px(4.))
            .bg(c(THEME.bg1))
            .child(
                canvas(
                    |_, _, _| {},
                    move |b, _, window, _| {
                        bounds.set(Some(b));
                        paint_keys(b, first as u8, last as u8, base as u8, held.map(|h| h.0), window);
                    },
                )
                .size_full(),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(|w, ev: &MouseDownEvent, _, cx| {
                if let Some(b) = w.keys_bounds.get() {
                    w.held = key_at(b, ev.position);
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|w, ev: &MouseMoveEvent, _, cx| {
                // a drag slides from key to key (glissando)
                if w.held.is_some() && ev.pressed_button == Some(MouseButton::Left) {
                    if let Some(b) = w.keys_bounds.get() {
                        let k = key_at(b, ev.position);
                        if k.map(|k| k.0) != w.held.map(|h| h.0) {
                            w.held = k;
                            cx.notify();
                        }
                    }
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|w, _, _, cx| {
                w.held = None;
                cx.notify();
            }))
    }

    // --- the tabs' bodies -------------------------------------------------------------------------------------

    fn instrument_tab(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let core = self.scx.core.read(cx);
        let s = core.port().root();
        let Some(track) = self.track else { return vec![] };
        let Some(n) = s.plugins.instrument_of(track).cloned() else {
            return vec![caption(tr("This track has no instrument.")).into_any_element()];
        };
        let mut out = Vec::new();
        // building: the loading in place of the controls, never half-built buttons (I-2.3)
        let target = Target::Node(n.id);
        let loading = core.port().loading(&target);
        let core_e = self.scx.core.clone();
        if let Some(l) = loading_component("iw-loading", &loading, move |_, cx| cancel_target(&core_e, target, cx)) {
            out.push(div().h(px(36.)).rounded(px(6.)).overflow_hidden().debug_selector(|| "iw-loading".into()).child(l).into_any_element());
            if matches!(loading, Loading::Visible { .. }) {
                return out;
            }
        }
        let Some(d) = n.descriptor() else {
            // a plugin the program lacks: silent, its state kept and saved back (F-INS-024)
            out.push(group_card(
                tr("Plugin not available"),
                vec![
                    caption(trf("\"{0}\" is not installed. The track is silent; its settings are kept and saved with the project.", &[&n.plugin]))
                        .into_any_element(),
                ],
            ));
            return out;
        };
        if let Some(r) = d.resource {
            let file = n.resource.clone().unwrap_or_default();
            if n.missing {
                out.push(missing_panel(n.id, &file, cx));
            } else {
                out.push(
                    div()
                        .id("iw-resource")
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .p(px(8.))
                        .rounded(px(6.))
                        .bg(c(THEME.bg2))
                        .child(caption(tr(r)))
                        .child(div().flex_1().text_size(px(11.5)).overflow_hidden().whitespace_nowrap().child(file.rsplit('/').next().unwrap_or("").to_string()))
                        .child(button("iw-resource-change", tr("Change…"), false, 0).on_click(cx.listener(|w, _, window, cx| {
                            super::browser::show_page(&w.scx, fake_core::browser::Page::Samples, window, cx);
                        })))
                        .into_any_element(),
                );
            }
        }
        if n.plugin == "AVL Drums" {
            out.push(self.kit_view(&n, cx));
        }
        if d.host != Host::Internal {
            let open = self.external_open;
            out.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().flex_1().child(caption(tr("Its own window runs outside the program, held to this track."))))
                    .child(button("iw-ext", if open { tr("Hide its window") } else { tr("Show its window") }, open, THEME.purple).on_click(cx.listener(|w, _, _, cx| {
                        w.external_open = !w.external_open;
                        cx.notify();
                    })))
                    .into_any_element(),
            );
        }
        out.extend(generic_editor(self, &n, d, None, cx));
        // the outputs (I-3.9): one per port of the instrument, each to a desk channel
        if let Some(t) = s.tracks.iter().find(|t| t.id == track) {
            let rows: Vec<AnyElement> = t
                .outputs
                .iter()
                .enumerate()
                .map(|(k, o)| {
                    let to = s.channels.iter().find(|c| c.id == o.to).map_or("?", |c| c.name.as_str()).to_string();
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .w_full()
                        .child(div().w(px(70.)).text_size(px(11.)).child(o.port))
                        .child(div().w(px(46.)).text_size(px(9.5)).text_color(c(THEME.text3)).child(if o.channels == 1 { tr("mono") } else { tr("stereo") }))
                        .child(
                            button(SharedString::from(format!("iw-out-{k}")), format!("→ {to} ▾"), false, 0)
                                .h(px(20.))
                                .on_mouse_down(MouseButton::Left, cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                                    w.editor.popup = Some((Popup::Own(IwPopup::Output(k)), ev.position));
                                    cx.stop_propagation();
                                    cx.notify();
                                })),
                        )
                        .into_any_element()
                })
                .collect();
            out.push(group_card(tr("Outputs"), rows));
        }
        out.push(caption(tr(d.description)).into_any_element());
        out
    }

    /// AVL Drums: the kit's picture (drawn), clicking a piece plays it, lit with the velocity (F-INS-002)
    fn kit_view(&self, n: &PluginNode, cx: &Context<Self>) -> AnyElement {
        let bounds = self.kit_bounds.clone();
        let lit = self.lit.filter(|(_, at)| at.elapsed() < Duration::from_millis(250)).map(|(k, _)| k);
        let kit = tr(plugins::KITS.iter().find(|(k, _)| *k == n.choice("kit")).map_or("", |(_, l)| l));
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(8.))
            .rounded(px(6.))
            .bg(c(THEME.bg2))
            .child(caption(trf("{0} · click a piece to hear it", &[&kit])))
            .child(
                div()
                    .id("iw-kit")
                    .debug_selector(|| "iw-kit".into())
                    .h(px(120.))
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |b, _, window, _| {
                                bounds.set(Some(b));
                                for (k, (x, y, r, _)) in KIT_PIECES.iter().enumerate() {
                                    let center = point(b.origin.x + b.size.width * *x, b.origin.y + b.size.height * *y);
                                    let rr = px(*r);
                                    let bb = Bounds::new(point(center.x - rr, center.y - rr), size(rr * 2.0, rr * 2.0));
                                    let col = if lit == Some(k) { THEME.green } else if k == 0 { THEME.bg4 } else { THEME.bg3 };
                                    window.paint_quad(gpui::quad(bb, rr, c(col), px(1.5), c(THEME.line2), Default::default()));
                                }
                            },
                        )
                        .size_full(),
                    )
                    .on_mouse_down(MouseButton::Left, cx.listener(|w, ev: &MouseDownEvent, window, cx| {
                        let Some(b) = w.kit_bounds.get() else { return };
                        // the smallest piece under the mouse wins (the cymbals over the kick)
                        let hit = KIT_PIECES
                            .iter()
                            .enumerate()
                            .filter(|(_, (x, y, r, _))| {
                                let cx_ = b.origin.x.as_f32() + b.size.width.as_f32() * x;
                                let cy = b.origin.y.as_f32() + b.size.height.as_f32() * y;
                                let (dx, dy) = (ev.position.x.as_f32() - cx_, ev.position.y.as_f32() - cy);
                                dx * dx + dy * dy <= r * r
                            })
                            .min_by(|a, b| a.1.2.total_cmp(&b.1.2))
                            .map(|(k, _)| k);
                        if let Some(k) = hit {
                            w.lit = Some((k, Instant::now()));
                            window.request_animation_frame();
                            cx.notify();
                        }
                    })),
            )
            .child(div().flex().flex_wrap().gap(px(8.)).children(KIT_PIECES.iter().enumerate().map(|(k, p)| {
                div().text_size(px(9.5)).text_color(c(if lit == Some(k) { THEME.green } else { THEME.text3 })).child(tr(p.3))
            })))
            .into_any_element()
    }

    fn envelopes_tab(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let s = self.scx.core.read(cx).port().root();
        let Some(track) = self.track else { return vec![] };
        let Some(f) = s.plugins.functions_of(track).cloned() else { return vec![] };
        let fd = plugins::descriptor(plugins::FUNCTIONS).unwrap();
        let per_note = s.plugins.instrument_of(track).and_then(|n| n.descriptor()).is_none_or(|d| d.per_note);
        let mut out = Vec::new();
        if !per_note {
            out.push(notice(tr("Envelopes, LFOs and the filter do not apply to this instrument: it plays its own voices.")));
        }
        let t = ["vol", "cut", "res"][self.target];
        out.push(
            div()
                .flex()
                .gap(px(4.))
                .children(["Volume", "Cutoff", "Resonance"].into_iter().enumerate().map(|(k, l)| {
                    button(SharedString::from(format!("iw-target-{k}")), tr(l), self.target == k, THEME.green).h(px(22.)).px(px(10.)).on_click(cx.listener(move |w, _, _, cx| {
                        w.target = k;
                        cx.notify();
                    }))
                }))
                .into_any_element(),
        );
        let env: Vec<&'static ParamSpec> = fd.params.iter().filter(|p| p.key.starts_with(&format!("env.{t}."))).collect();
        let lfo: Vec<&'static ParamSpec> = fd.params.iter().filter(|p| p.key.starts_with(&format!("lfo.{t}."))).collect();
        let r = |k: &str| f.real(&format!("env.{t}.{k}"));
        let env_pts = [r("predelay"), r("attack"), r("hold"), r("decay"), r("sustain") / 100.0, r("release"), r("amount") / 100.0];
        let amount_key: &'static str = env.iter().find(|p| p.key.ends_with(".amount")).map_or("", |p| p.key);
        let amount = r("amount");
        let node = f.id;
        out.push(
            div()
                .id("iw-env-graph")
                .debug_selector(|| "iw-env-graph".into())
                .h(px(70.))
                .rounded(px(6.))
                .bg(c(THEME.lcd_bg))
                .cursor_pointer()
                .child(canvas(|_, _, _| {}, move |b, _, window, _| paint_envelope(b, env_pts, window)).size_full())
                // a click on the graph toggles the amount between 0 and full (the legacy's gesture)
                .on_click(cx.listener(move |w, _, _, cx| {
                    let core = w.scx.core.clone();
                    apply_ops(&core, "Envelope amount", vec![ops::param(node, amount_key, if amount == 0.0 { 100.0 } else { 0.0 })], cx);
                }))
                .into_any_element(),
        );
        out.push(group_card(tr("Envelope"), env.iter().map(|p| control(self, Ctl { enabled: per_note, ..ctl(&f, p) }, cx)).collect()));
        let shape = f.choice(&format!("lfo.{t}.shape"));
        let l = |k: &str| f.real(&format!("lfo.{t}.{k}"));
        let lfo_pts = (l("speed") * if l("x100") > 0.5 { 100.0 } else { 1.0 }, l("amount") / 100.0);
        out.push(
            div()
                .h(px(44.))
                .rounded(px(6.))
                .bg(c(THEME.lcd_bg))
                .child(canvas(|_, _, _| {}, move |b, _, window, _| paint_lfo(b, shape, lfo_pts.0, lfo_pts.1, window)).size_full())
                .into_any_element(),
        );
        out.push(group_card(tr("LFO"), lfo.iter().map(|p| control(self, Ctl { enabled: per_note, ..ctl(&f, p) }, cx)).collect()));
        out.push(group_card(
            tr("Filter"),
            fd.params.iter().filter(|p| p.group == "Filter").map(|p| control(self, Ctl { enabled: per_note, ..ctl(&f, p) }, cx)).collect(),
        ));
        out
    }

    fn chords_tab(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let s = self.scx.core.read(cx).port().root();
        let Some(f) = self.track.and_then(|t| s.plugins.functions_of(t)).cloned() else { return vec![] };
        let fd = plugins::descriptor(plugins::FUNCTIONS).unwrap();
        let synced = f.choice("arp.sync");
        let mut out = Vec::new();
        for g in ["Chords", "Arpeggio", "Voices"] {
            let controls = fd
                .params
                .iter()
                .filter(|p| p.group == g)
                .map(|p| {
                    let mut k = ctl(&f, p);
                    if p.key == "arp.time" && synced != "free" {
                        // a synced time shows the note (F-CTL-005)
                        k.shown = Some(tr(plugins::SYNC_NOTES.iter().find(|(x, _)| *x == synced).map_or("", |(_, l)| l)).to_string());
                        k.enabled = false;
                    }
                    if p.key == "voice.legato" || p.key == "voice.glide" {
                        k.enabled = f.real("voice.mono") > 0.5;
                    }
                    control(self, k, cx)
                })
                .collect();
            out.push(group_card(tr(g), controls));
        }
        out
    }

    fn effects_tab(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let s = self.scx.core.read(cx).port().root();
        let Some(chid) = self.output_channel(cx) else { return vec![] };
        let Some(ch) = s.channels.iter().find(|c| c.id == chid) else { return vec![] };
        let mut out = vec![caption(trf("The effects are on the desk channel \"{0}\", where the main output goes.", &[&ch.name])).into_any_element()];
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
                    .child(button(SharedString::from(format!("iw-fx-{}", id.number)), if on { tr("On") } else { tr("Off") }, on, THEME.green).on_click(cx.listener(move |w, _, _, cx| {
                        let core = w.scx.core.clone();
                        apply_ops(&core, "Bypass", vec![EditOp::set(chid, Field::Bypass(id), Value::Bool(on))], cx);
                    })))
                    .child(plugin_chip(plugins::descriptor(x.plugin)))
                    .child(div().flex_1().text_size(px(11.5)).text_color(c(if on { THEME.text } else { THEME.text3 })).child(tr(x.plugin)))
                    .into_any_element()
            })
            .collect();
        out.push(group_card(tr("Inserts"), if rows.is_empty() { vec![caption(tr("No effects yet.")).into_any_element()] } else { rows }));
        out.push(
            button("iw-open-effects", tr("Open in the effect window"), false, 0)
                .h(px(24.))
                .px(px(10.))
                .debug_selector(|| "iw-open-effects".into())
                .on_click(cx.listener(move |w, _, window, cx| super::effect::show_channel(&w.scx, chid, window, cx)))
                .into_any_element(),
        );
        out
    }

    fn midi_tab(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let s = self.scx.core.read(cx).port().root();
        let Some(track) = self.track else { return vec![] };
        let Some(f) = s.plugins.functions_of(track).cloned() else { return vec![] };
        let fd = plugins::descriptor(plugins::FUNCTIONS).unwrap();
        let mut out = Vec::new();
        for (g, output) in [("MIDI input", false), ("MIDI output", true)] {
            let mut controls: Vec<AnyElement> = fd.params.iter().filter(|p| p.group == g).map(|p| control(self, ctl(&f, p), cx)).collect();
            let set = if output { &f.devices_out } else { &f.devices_in };
            controls.push(caption(if output { tr("Send to") } else { tr("Receive from") }).into_any_element());
            for dev in plugins::MIDI_DEVICES {
                let on = set.contains(dev);
                controls.push(
                    button(SharedString::from(format!("iw-dev-{output}-{dev}")), format!("{} {}", if on { "✓" } else { "·" }, dev), on, THEME.blue)
                        .h(px(20.))
                        .on_click(cx.listener(move |w, _, _, cx| {
                            let core = w.scx.core.clone();
                            apply_ops(&core, "MIDI device", vec![ops::state(track, &format!("device.{}/{dev}", if output { "out" } else { "in" }), Value::Bool(!on))], cx);
                        }))
                        .into_any_element(),
                );
            }
            out.push(group_card(tr(g), controls));
        }
        out.extend(generic_editor(self, &f, fd, Some(&["Velocity"]), cx));
        out.push(caption(tr("MIDI notes at the base velocity play at 100 %.")).into_any_element());
        out
    }

    fn tuning_tab(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let s = self.scx.core.read(cx).port().root();
        let Some(track) = self.track else { return vec![] };
        let Some(f) = s.plugins.functions_of(track).cloned() else { return vec![] };
        let fd = plugins::descriptor(plugins::FUNCTIONS).unwrap();
        let per_note = s.plugins.instrument_of(track).and_then(|n| n.descriptor()).is_none_or(|d| d.per_note);
        let mut out = generic_editor(self, &f, fd, Some(&["Transposition"]), cx);
        if !per_note {
            out.push(notice(tr("The microtuner does not apply to this instrument: it plays its own voices.")));
        }
        let micro: Vec<AnyElement> = fd
            .params
            .iter()
            .filter(|p| p.group == "Microtuner")
            .map(|p| {
                let mut k = ctl(&f, p);
                k.enabled = per_note && (p.key == "micro.on" || f.real("micro.on") > 0.5);
                if p.key == "micro.scale" || p.key == "micro.keymap" {
                    let slot = (f.real(p.key) as usize).saturating_sub(1);
                    let pl = &s.plugins;
                    let desc = if p.key == "micro.scale" { pl.scales.get(slot).map(|x| x.description.clone()) } else { pl.keymaps.get(slot).map(|x| x.description.clone()) };
                    k.shown = Some(format!("{} · {}", slot + 1, desc.filter(|d| !d.is_empty()).unwrap_or_else(|| tr("(empty)").to_string())));
                }
                control(self, k, cx)
            })
            .collect();
        out.push(group_card(tr("Microtuner"), micro));
        out.push(
            div()
                .flex()
                .gap(px(6.))
                .child(button("iw-edit-scale", tr("Edit scales…"), false, 0).h(px(22.)).px(px(8.)).debug_selector(|| "iw-edit-scale".into()).on_click(cx.listener(|w, _, window, cx| w.open_tuning(false, window, cx))))
                .child(button("iw-edit-keymap", tr("Edit keymaps…"), false, 0).h(px(22.)).px(px(8.)).on_click(cx.listener(|w, _, window, cx| w.open_tuning(true, window, cx))))
                .into_any_element(),
        );
        if let Some(e) = &self.tuning {
            out.push(self.tuning_editor(e, cx));
        }
        let keys: Vec<AnyElement> = fd
            .params
            .iter()
            .filter(|p| p.group == "Keys")
            .map(|p| {
                let mut k = ctl(&f, p);
                k.shown = Some(note_name(k.value));
                control(self, k, cx)
            })
            .collect();
        out.push(group_card(tr("Keys"), keys));
        out
    }

    fn open_tuning(&mut self, keymap: bool, window: &mut Window, cx: &mut Context<Self>) {
        let slot = {
            let s = self.scx.core.read(cx).port().root();
            let f = self.track.and_then(|t| s.plugins.functions_of(t));
            f.map_or(0, |f| (f.real(if keymap { "micro.keymap" } else { "micro.scale" }) as usize).saturating_sub(1))
        };
        self.load_tuning(keymap, slot, window, cx);
    }

    fn load_tuning(&mut self, keymap: bool, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (desc, text, numbers) = {
            let p = &self.scx.core.read(cx).port().root().plugins;
            if keymap {
                let k = &p.keymaps[slot];
                (k.description.clone(), k.degrees.clone(), [f64::from(k.first), f64::from(k.last), f64::from(k.middle), f64::from(k.base_key), k.base_freq])
            } else {
                let k = &p.scales[slot];
                (k.description.clone(), k.intervals.clone(), [0.0; 5])
            }
        };
        let description = cx.new(|cx| InputState::new(window, cx).default_value(desc));
        let text = cx.new(|cx| TextareaState::new(window, cx).rows(6).default_value(text));
        self.tuning = Some(TuningEditor { keymap, slot, description, text, numbers, error: None });
        cx.notify();
    }

    fn apply_tuning(&mut self, cx: &mut Context<Self>) {
        let Some(e) = &mut self.tuning else { return };
        let description = e.description.read(cx).value().to_string();
        let text = e.text.read(cx).value().to_string();
        // the slot's fields on the project's root, one undo step; the core refuses the whole list on a bad line
        let r = fake_core::ROOT;
        let edits = if e.keymap {
            let n = e.numbers;
            let k = e.slot;
            let mut v = vec![ops::text(r, &format!("keymap.{k}.description"), &description), ops::text(r, &format!("keymap.{k}.degrees"), &text)];
            for (f, x) in ["first", "last", "middle", "base_key", "base_freq"].iter().zip(n) {
                v.push(ops::state(r, &format!("keymap.{k}.{f}"), Value::Real(x)));
            }
            v
        } else {
            if let Err(line) = plugins::parse_intervals(&text) {
                e.error = Some(trf("Line {0} is not an interval (cents with a point, or a ratio like 3/2).", &[&line]));
                cx.notify();
                return;
            }
            let k = e.slot;
            vec![ops::text(r, &format!("scale.{k}.description"), &description), ops::text(r, &format!("scale.{k}.intervals"), &text)]
        };
        let core = self.scx.core.clone();
        let ok = apply_ops(&core, "Edit the tuning", edits, cx).is_some();
        if let Some(e) = &mut self.tuning {
            e.error = if ok { None } else { Some(tr("Not applied: check the keys and the base frequency (0.1–9999.999 Hz).").to_string()) };
        }
        cx.notify();
    }

    fn tuning_editor(&self, e: &TuningEditor, cx: &Context<Self>) -> AnyElement {
        let keymap = e.keymap;
        let slots: Vec<AnyElement> = (0..plugins::TUNING_SLOTS)
            .map(|k| {
                button(SharedString::from(format!("iw-slot-{k}")), format!("{}", k + 1), e.slot == k, THEME.green)
                    .on_click(cx.listener(move |w, _, window, cx| w.load_tuning(keymap, k, window, cx)))
                    .into_any_element()
            })
            .collect();
        let mut body = vec![
            div().flex().gap(px(3.)).children(slots).into_any_element(),
            caption(tr("Description")).into_any_element(),
            div().w_full().child(Input::new(&e.description).small()).into_any_element(),
            caption(if keymap { tr("Scale degree of each key (x = silent)") } else { tr("Intervals, one per line: cents (701.955) or ratios (3/2)") }).into_any_element(),
            div().w_full().child(Textarea::new(&e.text)).into_any_element(),
        ];
        if keymap {
            let labels = ["First key", "Last key", "Middle key", "Base key", "Base frequency"];
            let row: Vec<AnyElement> = labels
                .iter()
                .enumerate()
                .map(|(k, l)| {
                    let v = e.numbers[k];
                    let shown = if k == 4 { format!("{v:.3} Hz") } else { note_name(v) };
                    let stepper = |d: f64, s: &'static str| {
                        button(SharedString::from(format!("iw-km-{k}-{s}")), s, false, 0).on_click(cx.listener(move |w, _, _, cx| {
                            if let Some(e) = &mut w.tuning {
                                e.numbers[k] = if k == 4 { (e.numbers[k] + d).clamp(0.1, 9999.999) } else { (e.numbers[k] + d).clamp(0.0, 127.0) };
                                cx.notify();
                            }
                        }))
                    };
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(caption(tr(l)))
                        .child(div().flex().items_center().gap(px(2.)).child(stepper(-1.0, "−")).child(div().text_size(px(10.5)).min_w(px(52.)).child(shown)).child(stepper(1.0, "+")))
                        .into_any_element()
                })
                .collect();
            body.push(div().flex().flex_wrap().gap(px(8.)).children(row).into_any_element());
        }
        if let Some(err) = &e.error {
            body.push(div().text_size(px(11.)).text_color(c(THEME.red)).child(err.clone()).into_any_element());
        }
        body.push(
            div()
                .flex()
                .gap(px(6.))
                .child(button("iw-tuning-apply", tr("Apply"), false, 0).h(px(22.)).px(px(10.)).debug_selector(|| "iw-tuning-apply".into()).on_click(cx.listener(|w, _, _, cx| w.apply_tuning(cx))))
                .child(button("iw-tuning-close", tr("Close"), false, 0).h(px(22.)).px(px(10.)).on_click(cx.listener(|w, _, _, cx| {
                    w.tuning = None;
                    cx.notify();
                })))
                .into_any_element(),
        );
        group_card(if keymap { trf("Keymap {0}", &[&(e.slot + 1)]) } else { trf("Scale {0}", &[&(e.slot + 1)]) }, body)
    }

    fn drop_item(&mut self, d: &super::browser::Dragged, cx: &mut Context<Self>) {
        let Some(track) = self.track else { return };
        let core = self.scx.core.clone();
        let edits = drop_edits(core.read(cx).port().root(), track, d);
        if let Some((label, edits)) = edits {
            apply_ops(&core, label, edits, cx);
        }
    }
}

/// What a drop from the browser (or its "send to the instrument") does on an instrument track (F-IJN-004,
/// F-BRW-008): an instrument swaps the instrument; a preset applies (swapping to its instrument if needed); a sample
/// loads into the sampler and a SoundFont into the player, swapping to them if needed. One list, one undo step: the
/// new instrument is `Ref::Temp(1)`, so it works the same over the network.
pub fn drop_edits(root: &Doc, track: NodeId, d: &super::browser::Dragged) -> Option<(&'static str, Vec<EditOp>)> {
    use fake_core::browser::Kind as F;
    let (inst, plugin) = root.plugins.instrument_of(track).map_or((None, ""), |n| (Some(n.id), n.plugin));
    let swap_and = |to: &str, key: &str, t: &str| vec![ops::swap(track, to), ops::on_new(key, Value::Text(t.to_string()))];
    match d {
        super::browser::Dragged::Plugin(id) => {
            plugins::descriptor(id).filter(|d| d.role == Role::Instrument).map(|_| ("Swap the instrument", vec![ops::swap(track, id)]))
        }
        super::browser::Dragged::Entry(e) => {
            let e = fake_core::browser::entry(*e)?;
            match (e.kind, inst) {
                (F::Preset, Some(node)) if e.plugin == Some(plugin) => Some(("Apply a preset", vec![ops::text(node, "preset", &e.name)])),
                (F::Preset, _) => {
                    let p = e.plugin.filter(|p| plugins::descriptor(p).is_some_and(|d| d.role == Role::Instrument))?;
                    Some(("Apply a preset", swap_and(p, "preset", &e.name)))
                }
                (F::Audio, Some(node)) if plugin == "AudioFileProcessor" => Some(("Load a sample", vec![ops::text(node, "resource", &e.path)])),
                (F::Audio, _) => Some(("Load a sample", swap_and("AudioFileProcessor", "resource", &e.path))),
                (F::SoundFont, Some(node)) if plugin == "Sf2 Player" => Some(("Load a SoundFont", vec![ops::text(node, "resource", &e.path)])),
                (F::SoundFont, _) => Some(("Load a SoundFont", swap_and("Sf2 Player", "resource", &e.path))),
                _ => None,
            }
        }
    }
}

/// The kit's pieces as drawn: x, y (0..1 of the picture), radius in px, label key
const KIT_PIECES: [(f32, f32, f32, &str); 7] = [
    (0.5, 0.62, 34.0, "Kick"),
    (0.3, 0.58, 15.0, "Snare"),
    (0.14, 0.42, 13.0, "Hi-hat"),
    (0.42, 0.28, 12.0, "Tom"),
    (0.72, 0.6, 16.0, "Floor tom"),
    (0.24, 0.16, 16.0, "Crash"),
    (0.82, 0.22, 18.0, "Ride"),
];

/// The keys drawn by the keyboard: C2..C7
const LO: u8 = 36;
const HI: u8 = 96;

fn is_black(k: u8) -> bool {
    matches!(k % 12, 1 | 3 | 6 | 8 | 10)
}

fn white_index(k: u8) -> usize {
    (LO..k).filter(|x| !is_black(*x)).count()
}

fn key_at(b: Bounds<Pixels>, p: Point<Pixels>) -> Option<(u8, f32)> {
    if !b.contains(&p) {
        return None;
    }
    let whites = (LO..=HI).filter(|k| !is_black(*k)).count() as f32;
    let ww = b.size.width.as_f32() / whites;
    let x = p.x.as_f32() - b.origin.x.as_f32();
    let y = (p.y.as_f32() - b.origin.y.as_f32()) / b.size.height.as_f32();
    let vel = (0.2 + 0.8 * y).clamp(0.0, 1.0);
    if y < 0.6 {
        for k in LO..=HI {
            if is_black(k) {
                let cx_ = white_index(k) as f32 * ww;
                if (x - cx_).abs() < ww * 0.3 {
                    return Some((k, vel));
                }
            }
        }
    }
    let w = (x / ww) as usize;
    (LO..=HI).filter(|k| !is_black(*k)).nth(w).map(|k| (k, vel))
}

fn paint_keys(b: Bounds<Pixels>, first: u8, last: u8, base: u8, held: Option<u8>, window: &mut Window) {
    let whites = (LO..=HI).filter(|k| !is_black(*k)).count() as f32;
    let ww = b.size.width.as_f32() / whites;
    let h = b.size.height.as_f32();
    for k in (LO..=HI).filter(|k| !is_black(*k)) {
        let x = b.origin.x.as_f32() + white_index(k) as f32 * ww;
        let out = k < first || k > last;
        let col = if held == Some(k) { THEME.green } else if out { THEME.text3 } else { THEME.fader_cap };
        window.paint_quad(fill(Bounds::new(point(px(x + 0.5), b.origin.y), size(px(ww - 1.0), px(h))), c(col)));
        if k == base {
            window.paint_quad(fill(Bounds::new(point(px(x + 1.5), b.origin.y + px(h - 6.0)), size(px(ww - 3.0), px(4.0))), c(THEME.orange)));
        }
    }
    for k in (LO..=HI).filter(|k| is_black(*k)) {
        let x = b.origin.x.as_f32() + white_index(k) as f32 * ww - ww * 0.3;
        let col = if held == Some(k) { THEME.green } else if k < first || k > last { THEME.bg4 } else { THEME.lcd_bg };
        window.paint_quad(fill(Bounds::new(point(px(x), b.origin.y), size(px(ww * 0.6), px(h * 0.6))), c(col)));
    }
}

/// The DAHDSR envelope as one stroked path: [predelay, attack, hold, decay, sustain 0..1, release, amount -1..1]
fn paint_envelope(b: Bounds<Pixels>, e: [f64; 7], window: &mut Window) {
    let [pre, att, hold, dec, sus, rel, amount] = e;
    let total = pre + att + hold + dec + 400.0 + rel;
    let (x0, y0) = (b.origin.x.as_f32() + 6.0, b.origin.y.as_f32() + 6.0);
    let (w, h) = (b.size.width.as_f32() - 12.0, b.size.height.as_f32() - 12.0);
    let a = amount.abs().max(0.05) as f32;
    let x = |t: f64| x0 + (t / total) as f32 * w;
    let y = |v: f64| y0 + h - (v as f32) * h * a;
    let mut t = 0.0;
    let mut pb = PathBuilder::stroke(px(1.5));
    pb.move_to(point(px(x(0.0)), px(y(0.0))));
    for (dt, v) in [(pre, 0.0), (att, 1.0), (hold, 1.0), (dec, sus), (400.0, sus), (rel, 0.0)] {
        t += dt;
        pb.line_to(point(px(x(t)), px(y(v))));
    }
    if let Ok(p) = pb.build() {
        window.paint_path(p, c(if amount == 0.0 { THEME.text3 } else if amount < 0.0 { THEME.orange } else { THEME.green }));
    }
}

/// Two seconds of the LFO's shape as one stroked path
fn paint_lfo(b: Bounds<Pixels>, shape: &str, hz: f64, amount: f64, window: &mut Window) {
    let (x0, mid) = (b.origin.x.as_f32() + 4.0, b.origin.y.as_f32() + b.size.height.as_f32() / 2.0);
    let w = b.size.width.as_f32() - 8.0;
    let amp = (b.size.height.as_f32() / 2.0 - 4.0) * amount.abs().max(0.05) as f32;
    let n = 160;
    let cycles = (hz * 2.0).clamp(0.5, 40.0);
    let mut pb = PathBuilder::stroke(px(1.2));
    let mut rnd = 0.37f64;
    for i in 0..=n {
        let ph = (i as f64 / n as f64 * cycles).fract();
        let v = match shape {
            "triangle" => 1.0 - 4.0 * (ph - 0.5).abs(),
            "saw" => 2.0 * ph - 1.0,
            "square" => {
                if ph < 0.5 { 1.0 } else { -1.0 }
            }
            "random" => {
                if i % 8 == 0 {
                    rnd = (rnd * 7.13 + 0.31).fract();
                }
                rnd * 2.0 - 1.0
            }
            "user" => (ph * std::f64::consts::TAU).sin() * 0.6 + (ph * 3.0 * std::f64::consts::TAU).sin() * 0.4,
            _ => (ph * std::f64::consts::TAU).sin(),
        };
        let p = point(px(x0 + w * i as f32 / n as f32), px(mid - v as f32 * amp));
        if i == 0 {
            pb.move_to(p)
        } else {
            pb.line_to(p)
        }
    }
    if let Ok(p) = pb.build() {
        window.paint_path(p, c(if amount == 0.0 { THEME.text3 } else { THEME.cyan }));
    }
}

impl Render for InstrumentWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // U38: the window follows the track by its feed, and closes when the track is gone
        if let (Some(t), Some(feed)) = (self.track, self.feed.as_mut()) {
            let core = self.scx.core.read(cx);
            let _ = core.port().drain(feed);
            if !core.port().root().tracks.iter().any(|x| x.id == t) {
                self.track = None;
                self.feed = None;
                if self.detached {
                    window.remove_window();
                } else {
                    let scx = self.scx.clone();
                    cx.defer(move |cx| scx.close(cx));
                }
            }
        }
        if self.lit.is_some_and(|(_, at)| at.elapsed() < Duration::from_millis(260)) {
            window.request_animation_frame();
        }
        let body: Vec<AnyElement> = match self.tab {
            Tab::Instrument => self.instrument_tab(cx),
            Tab::Envelopes => self.envelopes_tab(cx),
            Tab::Chords => self.chords_tab(cx),
            Tab::Effects => self.effects_tab(cx),
            Tab::Midi => self.midi_tab(cx),
            Tab::Tuning => self.tuning_tab(cx),
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
            .on_mouse_up(MouseButton::Left, cx.listener(|w, _: &MouseUpEvent, _, cx| drag_end(w, cx)))
            // a dropped preset, instrument, sample or SoundFont from the browser (F-IJN-004)
            .on_drop(cx.listener(|w, d: &super::browser::Dragged, _, cx| w.drop_item(d, cx)))
            .child(self.header(cx))
            .child(self.tabs(cx))
            .child(div().id("iw-body").flex_1().overflow_y_scroll().p(px(10.)).flex().flex_col().gap(px(8.)).children(body))
            .when(self.keyboard, |d| d.child(self.keyboard_view(cx)))
            .child(self.footer(cx))
            .children(popup_layer(self, cx))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;

    use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px};

    use super::*;
    use crate::workspace::{self, Workspace};

    /// Opens the workspace with a screen open
    pub(crate) fn open(cx: &mut TestAppContext, s: Screen) -> (Entity<Workspace>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            workspace::bind_keys(cx);
        });
        let (ws, cx) = cx.add_window_view(|window, cx| Workspace::new(window, cx));
        ws.update_in(cx, |ws, window, cx| ws.open(s, window, cx));
        cx.run_until_parked();
        (ws, cx)
    }

    pub(crate) fn click(cx: &mut VisualTestContext, sel: &str) {
        let s: &'static str = Box::leak(sel.to_string().into_boxed_str());
        let p = cx.debug_bounds(s).unwrap_or_else(|| panic!("{sel} is drawn")).center();
        cx.simulate_click(p, Modifiers::none());
        cx.run_until_parked();
    }

    fn the_track(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> (Entity<Core>, NodeId, NodeId) {
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        let (t, p) = core.read_with(cx, |core, _| {
            let s = core.port().root();
            let t = s.tracks.iter().find(|t| t.kind == TrackKind::Instrument && s.plugins.instrument_of(t.id).is_some()).unwrap().id;
            (t, s.plugins.instrument.get(&t).copied().unwrap())
        });
        (core, t, p)
    }

    /// A knob of the generic editor: a drag is one gesture and one undo step (I-3.7)
    #[gpui::test]
    fn generic_knob_drag_is_one_undo_step(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx, Screen::Instrument);
        let (core, track, _) = the_track(&ws, cx);
        let pitch = |core: &Entity<Core>, cx: &mut VisualTestContext| core.read_with(cx, |core, _| core.port().root().plugins.functions_of(track).unwrap().real("tune.pitch"));
        let before = pitch(&core, cx);
        let sel: &'static str = Box::leak(format!("ctl-t{}-tune.pitch", track.number).into_boxed_str());
        let at = cx.debug_bounds(sel).unwrap_or_else(|| panic!("{sel} drawn")).center();
        cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
        for k in 1..=10 {
            cx.simulate_mouse_move(point(at.x, at.y - px(5.0 * k as f32)), MouseButton::Left, Modifiers::none());
        }
        cx.simulate_mouse_up(point(at.x, at.y - px(50.)), MouseButton::Left, Modifiers::none());
        let after = pitch(&core, cx);
        assert!((after - before - 50.0).abs() < 1e-6, "50 px of 200 is a quarter of -100..100 cents: {after}");
        core.update(cx, |core, _| {
            let _ = core.port_mut().undo();
        });
        assert_eq!(pitch(&core, cx), before, "the whole drag is one undo step");
    }

    /// A choice opens its list and sets the choice by its key; swapping the instrument shows the loading past 200 ms
    #[gpui::test]
    fn choice_list_and_swap_with_loading(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx, Screen::Instrument);
        let (core, track, p) = the_track(&ws, cx);
        assert_eq!(core.read_with(cx, |core, _| core.port().root().plugins.node(p).unwrap().plugin), "AVL Drums");
        click(cx, &format!("ctl-u{}-kit", p.number));
        assert!(cx.debug_bounds("popup").is_some(), "the list opened");
        // the kits in order: the third is Blonde Bop
        click(cx, "popup-item-2");
        let kit = core.read_with(cx, |core, _| core.port().root().plugins.node(p).unwrap().choice("kit"));
        assert_eq!(kit, "blonde_bop");
        // Swap ▾ lists the instruments by category; choose the SoundFont player (headings are items too)
        click(cx, "iw-swap");
        let item = core.read_with(cx, |_, _| {
            let mut list: Vec<&Descriptor> = plugins::all().filter(|d| d.role == Role::Instrument).collect();
            list.sort_by_key(|d| (d.host != Host::Internal, d.category));
            let mut k = 0;
            let mut last = "";
            for d in list {
                if d.category != last {
                    k += 1;
                    last = d.category;
                }
                if d.id == "Sf2 Player" {
                    return k;
                }
                k += 1;
            }
            unreachable!()
        });
        click(cx, &format!("popup-item-{item}"));
        let now = core.read_with(cx, |core, _| core.port().root().plugins.instrument_of(track).unwrap().plugin);
        assert_eq!(now, "Sf2 Player");
        // the build starts on the next pump, on a worker with real time: past 200 ms the loading shows
        cx.executor().advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        assert!(cx.debug_bounds("iw-loading").is_none(), "nothing before 200 ms");
        std::thread::sleep(Duration::from_millis(260));
        cx.executor().advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        assert!(cx.debug_bounds("iw-loading").is_some(), "the SoundFont's loading is drawn in place of the controls");
    }

    /// Every tab draws; the scale editor applies; removing the track closes the window (U38)
    #[gpui::test]
    fn tabs_draw_and_the_window_closes_with_its_track(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx, Screen::Instrument);
        for t in Tab::ALL {
            click(cx, &format!("iw-tab-{t:?}"));
        }
        click(cx, "iw-edit-scale");
        click(cx, "iw-tuning-apply");
        let (core, track, _) = the_track(&ws, cx);
        core.update(cx, |core, _| {
            let _ = core.port_mut().apply("Remove a track", vec![EditOp::Remove { id: Ref::Id(track) }]);
        });
        cx.executor().advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        assert!(!ws.read_with(cx, |ws, _| ws.is_open(Screen::Instrument)), "closed with its track");
    }
}
