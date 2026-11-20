//! The mixing page: the whole desk, a page of its own (user's decision, 19/11). Not the legacy's layout: each strip
//! shows what is touched while mixing (the rack's sections as switches with the SAT and CLIP lights and the tube's
//! lamp, inserts with their point, sends to every return, pan, mute, solo, phase, the fader over the meter, the
//! output), and the selected channel's whole rack, or the master's section with its meters and the monitoring,
//! opens in the inspector on the right. Groups are channels others send to; Ctrl+click selects several strips and
//! "Link faders" makes them move together (P-18). Returns and the master stay pinned at the right edge.
//!
//! Knobs and faders, everywhere: drag = one gesture; Shift = fine; double click or middle button = default; the
//! right button opens the menu (default, copy and paste the value, MIDI learn, automate) (F-CTL-005/006).
//! Keys: ←/→ select, ↑/↓ move the selected fader 1 dB (Shift 3 dB, Alt 0.1 dB), Shift+Insert adds a channel.
//! Everything goes through the port (I-15): edits as `EditOp`s, the gesture by id, actions by id.

use std::collections::BTreeSet;

use fake_core::mixer::{self, ChannelKind, ParamSpec};
use fake_core::port::EditOp;
use fake_core::view;
use fake_core::{Channel, Field, GestureId, NodeId, ROOT, Value};
use gpui::{
    Bounds, Context, Entity, FocusHandle, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PathBuilder, Pixels, Point, Render, ScrollEvent, SharedString, WeakEntity, Window, actions, canvas, div, fill,
    point, prelude::*, px, size,
};

use crate::core::Core;
use crate::screens::Screen;
use crate::ui::{THEME, button, c, caption, fader_over_meter, knob_face, tr, trf, track_color};
use crate::workspace::Workspace;

actions!(mixing, [SelectLeft, SelectRight, FaderUp, FaderDown, FaderUpCoarse, FaderDownCoarse, FaderUpFine, FaderDownFine, AddChannel]);

pub const STRIP_W: f32 = 96.0;
const INSPECTOR_W: f32 = 330.0;
const FADER_H: f32 = 190.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Volume(usize),
    Pan(usize),
    Param(usize, &'static str),
    Send(usize, NodeId),
}

struct Drag {
    target: Target,
    start_y: f32,
    start_t: f64,
    /// pixels for the whole travel: a fader's height, or a knob's 200 px
    travel: f32,
    gesture: GestureId,
    /// the linked faders' ids and starting travels (relative group, P-18)
    linked: Vec<(NodeId, f64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MeterKind {
    Peak,
    Rms,
    Vu,
    K14,
}

impl MeterKind {
    fn label(self) -> &'static str {
        match self {
            MeterKind::Peak => "Peak",
            MeterKind::Rms => "RMS",
            MeterKind::Vu => "VU",
            MeterKind::K14 => "K-14",
        }
    }
    fn next(self) -> Self {
        match self {
            MeterKind::Peak => MeterKind::Rms,
            MeterKind::Rms => MeterKind::Vu,
            MeterKind::Vu => MeterKind::K14,
            MeterKind::K14 => MeterKind::Peak,
        }
    }
    /// The fake's reading of the same signal on each scale (the real ones come from the engine, P-22)
    fn shown(self, peak_fill: f32) -> f32 {
        let on = if peak_fill > 0.0 { 1.0 } else { 0.0 };
        match self {
            MeterKind::Peak => peak_fill,
            MeterKind::Rms => peak_fill * 0.82,
            MeterKind::Vu => (peak_fill * 0.82 + 0.06).min(1.0) * on,
            MeterKind::K14 => (peak_fill * 0.82 + 0.14).min(1.0) * on,
        }
    }
}

/// The monitoring of the master (F-MIX-067): never saved, undone or exported, so it is the interface's state
#[derive(Clone, Copy, Debug, Default)]
struct Monitoring {
    level_db: f64,
    dim: bool,
    cut: bool,
    mono: bool,
    side: bool,
}

pub struct MixingPage {
    core: Entity<Core>,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    scroll: f32,
    selected: usize,
    /// Ctrl+click: several strips, for "Link faders"
    multi: BTreeSet<usize>,
    drag: Option<Drag>,
    menu: Option<(Target, Point<Pixels>)>,
    output_picker: Option<usize>,
    clipboard: Option<f64>,
    learning: Option<Target>,
    meter_kind: MeterKind,
    monitor: Monitoring,
    strips_bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
}

fn chs(core: &Core) -> &[Channel] {
    &core.port().root().channels
}

impl MixingPage {
    pub fn new(core: Entity<Core>, workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        MixingPage {
            core,
            workspace,
            focus: cx.focus_handle(),
            scroll: 0.0,
            selected: 1,
            multi: BTreeSet::new(),
            drag: None,
            menu: None,
            output_picker: None,
            clipboard: None,
            learning: None,
            meter_kind: MeterKind::Peak,
            monitor: Monitoring::default(),
            strips_bounds: Default::default(),
        }
    }

    // --- values and edits ----------------------------------------------------------------------------------

    fn spec_of(k: &'static str) -> &'static ParamSpec {
        mixer::spec(k).expect("a rack key")
    }

    /// A target's value as 0..1 of its control's travel
    fn travel_of(&self, t: Target, cx: &Context<Self>) -> f64 {
        let chs = chs(self.core.read(cx));
        match t {
            Target::Volume(i) => view::db_to_knob(chs[i].volume_db),
            Target::Pan(i) => (chs[i].pan + 1.0) / 2.0,
            Target::Param(i, k) => {
                let p = Self::spec_of(k);
                (chs[i].params.get(k).copied().unwrap_or(p.default) - p.min) / (p.max - p.min)
            }
            Target::Send(i, s) => view::db_to_knob(chs[i].sends.iter().find(|x| x.id == s).map_or(-60.0, |x| x.level_db)),
        }
    }

    fn edit_of(t: Target, travel: f64) -> (Field, Value) {
        match t {
            Target::Volume(_) => (Field::Volume, Value::Real(view::knob_to_db(travel).max(-60.0))),
            Target::Pan(_) => (Field::Pan, Value::Real(travel * 2.0 - 1.0)),
            Target::Param(_, k) => {
                let p = Self::spec_of(k);
                let mut v = p.min + travel * (p.max - p.min);
                if !p.choices.is_empty() {
                    v = v.round();
                }
                (Field::Param(k), Value::Real(v))
            }
            Target::Send(_, s) => (Field::SendLevel(s), Value::Real(view::knob_to_db(travel).max(-60.0))),
        }
    }

    fn default_travel(t: Target) -> f64 {
        match t {
            Target::Volume(_) => view::db_to_knob(0.0),
            Target::Send(..) => 0.0,
            Target::Pan(_) => 0.5,
            Target::Param(_, k) => {
                let p = Self::spec_of(k);
                (p.default - p.min) / (p.max - p.min)
            }
        }
    }

    fn channel_of(t: Target) -> usize {
        match t {
            Target::Volume(i) | Target::Pan(i) | Target::Param(i, _) | Target::Send(i, _) => i,
        }
    }

    fn id_of(&self, i: usize, cx: &Context<Self>) -> NodeId {
        chs(self.core.read(cx))[i].id
    }

    /// One edit, one undo step
    fn apply(&mut self, label: &'static str, ops: Vec<EditOp>, cx: &mut Context<Self>) {
        self.core.update(cx, |core, _| {
            // Pending: in the fake always ready; over the network the result comes in a later pump, and the feed
            // tells the view
            let _ = core.port_mut().apply(label, ops);
        });
        cx.notify();
    }

    fn set(&mut self, t: Target, travel: f64, label: &'static str, cx: &mut Context<Self>) {
        let (f, v) = Self::edit_of(t, travel.clamp(0.0, 1.0));
        let id = self.id_of(Self::channel_of(t), cx);
        self.apply(label, vec![EditOp::set(id, f, v)], cx);
    }

    // --- gestures ------------------------------------------------------------------------------------------

    fn press(&mut self, t: Target, travel: f32, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let i = Self::channel_of(t);
        self.selected = i;
        self.menu = None;
        cx.stop_propagation();
        match ev.button {
            MouseButton::Right => {
                self.menu = Some((t, ev.position));
                cx.notify();
                return;
            }
            MouseButton::Middle => return self.set(t, Self::default_travel(t), "Default", cx),
            _ => {}
        }
        if ev.click_count == 2 {
            return self.set(t, Self::default_travel(t), "Default", cx);
        }
        if self.learning == Some(t) {
            self.learning = None; // learned: the next controller move would be bound (the fake has no MIDI in)
        }
        let start_t = self.travel_of(t, cx);
        let linked: Vec<(NodeId, f64)> = match t {
            Target::Volume(i) if chs(self.core.read(cx))[i].linked => chs(self.core.read(cx))
                .iter()
                .enumerate()
                .filter(|(k, c)| *k != i && c.linked)
                .map(|(_, c)| (c.id, view::db_to_knob(c.volume_db)))
                .collect(),
            _ => Vec::new(),
        };
        let prev = self.drag.take();
        let gesture = self.core.update(cx, |core, _| {
            if let Some(d) = prev {
                core.port_mut().gesture_commit(d.gesture);
            }
            core.port_mut().begin_gesture("Mix")
        });
        self.drag = Some(Drag { target: t, start_y: ev.position.y.as_f32(), start_t, travel, gesture, linked });
        cx.notify();
    }

    fn drag_to(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = &self.drag else { return };
        let fine = if ev.modifiers.shift { 10.0 } else { 1.0 };
        let dt = -((ev.position.y.as_f32() - d.start_y) / (d.travel * fine)) as f64;
        let (f, v) = Self::edit_of(d.target, (d.start_t + dt).clamp(0.0, 1.0));
        let id = self.id_of(Self::channel_of(d.target), cx);
        let mut ops = vec![EditOp::set(id, f, v)];
        // the linked faders move by the same travel, in the same gesture (one undo step)
        for (lid, start) in &d.linked {
            ops.push(EditOp::set(*lid, Field::Volume, Value::Real(view::knob_to_db((start + dt).clamp(0.0, 1.0)).max(-60.0))));
        }
        self.core.update(cx, |core, _| core.port_mut().gesture_apply(&self.drag.as_ref().unwrap().gesture, ops));
        cx.notify();
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.drag.take() {
            self.core.update(cx, |core, _| core.port_mut().gesture_commit(d.gesture));
            cx.notify();
        }
    }

    fn target_name(&self, t: Target, cx: &Context<Self>) -> String {
        let ch = &chs(self.core.read(cx))[Self::channel_of(t)];
        let what = match t {
            Target::Volume(_) => tr("Volume").to_string(),
            Target::Pan(_) => tr("Pan").to_string(),
            Target::Param(_, k) => tr(Self::spec_of(k).label).to_string(),
            Target::Send(..) => tr("Send").to_string(),
        };
        format!("{} · {what}", ch.name)
    }

    fn toggle(&mut self, i: usize, f: Field, cx: &mut Context<Self>) {
        let ch = chs(self.core.read(cx))[i].clone();
        let v = match f {
            Field::Mute => Value::Bool(!ch.mute),
            Field::Solo => Value::Bool(!ch.solo),
            Field::FaderLink => Value::Bool(!ch.linked),
            Field::Param(k) => Value::Real(if ch.params.get(k).copied().unwrap_or(0.0) > 0.5 { 0.0 } else { 1.0 }),
            Field::Bypass(x) => Value::Bool(!ch.inserts.iter().find(|n| n.id == x).is_some_and(|n| n.bypass)),
            _ => return,
        };
        self.apply("Toggle", vec![EditOp::set(ch.id, f, v)], cx);
    }

    /// A selector's next choice (click)
    fn cycle(&mut self, i: usize, k: &'static str, cx: &mut Context<Self>) {
        let p = Self::spec_of(k);
        let ch = &chs(self.core.read(cx))[i];
        let v = ch.params.get(k).copied().unwrap_or(p.default);
        let next = if v + 1.0 > p.max { p.min } else { v + 1.0 };
        let id = ch.id;
        self.apply("Choice", vec![EditOp::set(id, Field::Param(k), Value::Real(next))], cx);
    }

    fn select(&mut self, i: usize, ctrl: bool, cx: &mut Context<Self>) {
        if ctrl {
            if !self.multi.remove(&i) {
                self.multi.insert(i);
            }
        } else {
            self.multi.clear();
        }
        self.selected = i;
        self.output_picker = None;
        self.menu = None;
        cx.notify();
    }

    fn link_selected(&mut self, cx: &mut Context<Self>) {
        let mut set = self.multi.clone();
        set.insert(self.selected);
        let on = set.len() > 1;
        let ops = set.iter().map(|&i| EditOp::set(self.id_of(i, cx), Field::FaderLink, Value::Bool(on))).collect();
        self.apply("Link faders", ops, cx);
    }

    fn nudge(&mut self, db: f64, cx: &mut Context<Self>) {
        let ch = &chs(self.core.read(cx))[self.selected];
        let (id, v) = (ch.id, (ch.volume_db + db).clamp(-60.0, 6.0));
        self.apply("Fader", vec![EditOp::set(id, Field::Volume, Value::Real(v))], cx);
    }

    fn add_channel(&mut self, cx: &mut Context<Self>) {
        self.core.update(cx, |core, _| {
            let _ = core.port_mut().run_action("mixer.add_channel", Value::Bool(true));
        });
        cx.notify();
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let delta = if d.x.as_f32().abs() > d.y.as_f32().abs() { d.x } else { d.y }.as_f32();
        let normal = self.normal(cx).len() + 1;
        let visible = self.strips_bounds.get().size.width.as_f32();
        let max = (normal as f32 * STRIP_W - visible).max(0.0);
        self.scroll = (self.scroll - delta).clamp(0.0, max);
        cx.notify();
    }

    /// Shows a channel (the FX button of a track's header, P-28)
    pub fn select_channel(&mut self, id: NodeId, cx: &mut Context<Self>) {
        if let Some(i) = chs(self.core.read(cx)).iter().position(|c| c.id == id) {
            self.selected = i;
            self.multi.clear();
            let n = self.normal(cx).iter().position(|&k| k == i).unwrap_or(0) as f32;
            self.scroll = (n * STRIP_W - 2.0 * STRIP_W).max(0.0);
            cx.notify();
        }
    }

    fn normal(&self, cx: &Context<Self>) -> Vec<usize> {
        let chs = chs(self.core.read(cx));
        (0..chs.len()).filter(|&k| matches!(chs[k].kind, ChannelKind::Normal | ChannelKind::Group)).collect()
    }
}

// -----------------------------------------------------------------------------------------------------------------
// Pieces

impl MixingPage {
    /// A knob bound to a target, with the gesture and the menu wired
    fn knob(&self, id: SharedString, t: Target, d: f32, cx: &Context<Self>) -> impl IntoElement {
        let travel = self.travel_of(t, cx);
        let learning = self.learning == Some(t);
        div()
            .id(id)
            .cursor_ns_resize()
            .rounded(px(d))
            .when(learning, |d| d.border_2().border_color(c(THEME.yellow)))
            .child(knob_face(travel, d))
            .on_mouse_down(MouseButton::Left, cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.press(t, 200.0, ev, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.press(t, 200.0, ev, cx)))
            .on_mouse_down(MouseButton::Middle, cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.press(t, 200.0, ev, cx)))
    }

    /// A parameter of the rack: a knob with its label and value, or a selector (click = next choice)
    fn param(&self, i: usize, ps: &'static ParamSpec, v: f64, cx: &Context<Self>) -> gpui::AnyElement {
        let k = ps.key;
        let label = tr(ps.label);
        if !ps.choices.is_empty() {
            let text = ps.choices.get(v as usize).copied().unwrap_or("?");
            let switch = ps.choices.len() == 2;
            return div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(2.))
                .child(
                    button(SharedString::from(format!("ch{i}{k}")), if switch { label.to_string() } else { tr(text).to_string() }, switch && v > 0.5, THEME.orange)
                        .min_w(px(44.))
                        .on_click(cx.listener(move |p, _, _, cx| p.cycle(i, k, cx))),
                )
                .when(!switch, |d| d.child(div().text_size(px(9.)).text_color(c(THEME.text3)).child(label)))
                .into_any_element();
        }
        let shown = match k {
            "rack.gain.gain" | "master.drive" => view::format_db(mixer::gain_db(v)),
            _ if ps.unit.is_empty() => format!("{v:.2}"),
            _ => format!("{v:.1} {}", ps.unit),
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .child(self.knob(SharedString::from(format!("pk{i}{k}")), Target::Param(i, k), 26.0, cx))
            .child(div().text_size(px(9.)).text_color(c(THEME.text2)).child(label))
            .child(div().text_size(px(9.)).child(shown))
            .into_any_element()
    }

    fn strip(&self, i: usize, cx: &Context<Self>) -> impl IntoElement {
        let core = self.core.read(cx);
        let port = core.port();
        let root = port.root();
        let analog = root.song.desk_analog;
        let ch = &root.channels[i];
        let peak = core.shown[port.telemetry().channel_meter(i)];
        let level = self.meter_kind.shown(peak);
        let color = match ch.kind {
            ChannelKind::Normal => track_color(i),
            ChannelKind::Group => THEME.purple,
            ChannelKind::Return => THEME.cyan,
            ChannelKind::Master => THEME.green,
        };
        let out_name: SharedString = match ch.output {
            None => tr("→ Device").into(),
            Some(o) => format!("→ {}", root.channels.iter().find(|x| x.id == o).map_or("?", |x| x.name.as_str())).into(),
        };
        let selected = i == self.selected || self.multi.contains(&i);
        let fader_t = view::db_to_knob(ch.volume_db);
        // the lights: SAT and CLIP from the level, the tube's lamp when the gain stage is a tube (F-MIX-042/043)
        let model = ch.params.get("rack.gain.model").copied().unwrap_or(0.0) as usize;
        let tube = analog && model >= 5 && ch.kind != ChannelKind::Master;
        let realistic = ch.params.get("rack.gain.tube").copied().unwrap_or(0.0) > 0.5;
        let returns: Vec<(NodeId, String)> =
            root.channels.iter().filter(|c| c.kind == ChannelKind::Return).map(|c| (c.id, c.name.clone())).collect();
        let phase = ch.params.get("rack.input.phase").copied().unwrap_or(0.0) > 0.5;
        div()
            .id(("strip", i))
            .w(px(STRIP_W - 2.0))
            .h_full()
            .flex()
            .flex_col()
            .gap(px(3.))
            .pb(px(6.))
            .bg(c(if selected { THEME.bg3 } else { THEME.bg2 }))
            .border_r_1()
            .border_color(c(THEME.line))
            .on_mouse_down(MouseButton::Left, cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.select(i, ev.modifiers.platform || ev.modifiers.control, cx)))
            // number and name in the channel's color; 🔗 when the fader is linked
            .child(
                div()
                    .h(px(20.))
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .bg(c(color))
                    .text_color(c(THEME.on_accent))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::BOLD)
                    .overflow_hidden()
                    .child(format!("{}", i + 1))
                    .child(div().flex_1().overflow_hidden().child(ch.name.clone()))
                    .when(ch.linked, |d| d.child("🔗")),
            )
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .px(px(4.))
                    .h(px(10.))
                    .items_center()
                    .when(analog, |d| d.child(led(tr("SAT"), peak > 0.78, THEME.yellow)).child(led(tr("CLIP"), peak > 0.95, THEME.red)))
                    .when(tube, |d| d.child(led(if realistic { "RL" } else { "LT" }, true, if realistic { THEME.red } else { THEME.yellow })))
                    .when(!analog, |d| d.child(div().text_size(px(8.)).text_color(c(THEME.text3)).child(tr("digital")))),
            )
            // the rack's sections as switches; the knobs are in the inspector
            .when(ch.kind != ChannelKind::Master, |d| {
                d.child(div().flex().flex_wrap().gap(px(2.)).px(px(3.)).children(mixer::RACK.iter().filter(|(s, _, _)| *s != "tolerance").map(|(sec, label, _)| {
                    let key = mixer::section_key(sec);
                    let on = analog && ch.params.get(key).copied().unwrap_or(0.0) > 0.5;
                    let short: String = tr(label).chars().take(3).collect();
                    button(SharedString::from(format!("rk{i}{sec}")), short, on, THEME.orange)
                        .on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Param(key), cx)))
                })))
            })
            // inserts with their point (P-20); a click opens the effect's window, Alt+click bypasses
            .child(div().px(px(3.)).child(caption(tr("Inserts"))))
            .children(ch.inserts.iter().map(|ins| {
                let point = mixer::INSERT_POINTS[ins.point as usize];
                let id = ins.id;
                div()
                    .id(SharedString::from(format!("ins{}", ins.id.number)))
                    .mx(px(3.))
                    .px(px(4.))
                    .h(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .rounded(px(3.))
                    .bg(c(THEME.bg1))
                    .text_size(px(10.))
                    .text_color(c(if ins.bypass { THEME.text3 } else { THEME.text }))
                    .overflow_hidden()
                    .cursor_pointer()
                    .child(div().flex_1().overflow_hidden().child(tr(ins.plugin)))
                    .child(div().text_size(px(8.)).text_color(c(THEME.text3)).child(tr(point).chars().take(4).collect::<String>()))
                    .on_click(cx.listener(move |p, ev: &gpui::ClickEvent, window, cx| {
                        if ev.modifiers().alt {
                            p.toggle(i, Field::Bypass(id), cx);
                        } else {
                            let _ = p.workspace.update(cx, |ws, cx| ws.open(Screen::Effect, window, cx));
                        }
                    }))
            }))
            // sends: one per return (P-19)
            .when(ch.kind == ChannelKind::Normal, |d| {
                d.child(div().px(px(3.)).child(caption(tr("Sends")))).children(returns.iter().map(|(rid, name)| {
                    let snd = ch.sends.iter().find(|x| x.to == *rid);
                    let on = snd.is_some_and(|x| x.level_db > -60.0);
                    div()
                        .flex()
                        .items_center()
                        .gap(px(3.))
                        .px(px(3.))
                        .children(snd.map(|snd| self.knob(SharedString::from(format!("snd{i}-{}", snd.id.number)), Target::Send(i, snd.id), 16.0, cx)))
                        .child(div().flex_1().text_size(px(9.5)).text_color(c(if on { THEME.text } else { THEME.text3 })).overflow_hidden().child(name.clone()))
                        .when(snd.is_some_and(|x| x.pre_fader), |d| d.child(div().text_size(px(8.5)).text_color(c(THEME.yellow)).child(tr("PRE"))))
                }))
            })
            .child(div().flex_1())
            // pan, mute, solo, phase
            .when(ch.kind != ChannelKind::Master, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(3.))
                        .child(self.knob(SharedString::from(format!("pan{i}")), Target::Pan(i), 22.0, cx))
                        .child(button(SharedString::from(format!("mm{i}")), "M", ch.mute, THEME.orange).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Mute, cx))))
                        .child(button(SharedString::from(format!("ms{i}")), "S", ch.solo, THEME.yellow).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Solo, cx))))
                        .child(button(SharedString::from(format!("mp{i}")), "Ø", phase, THEME.blue).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Param("rack.input.phase"), cx)))),
                )
            })
            // the peak readout, the fader over the meter (user's decision), the value
            .child(div().mx_auto().text_size(px(9.)).text_color(c(if peak > 0.95 { THEME.red } else { THEME.text3 })).child(if peak > 0.0 { format!("{:.1}", peak * 60.0 - 60.0) } else { "-inf".into() }))
            .child(
                div()
                    .id(("fader", i))
                    .debug_selector(move || format!("fader-{i}"))
                    .mx_auto()
                    .cursor_ns_resize()
                    .child(fader_over_meter(fader_t, vec![level, (level * 0.92).min(1.0)], 44.0, FADER_H))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.press(Target::Volume(i), FADER_H - 12.0, ev, cx)))
                    .on_mouse_down(MouseButton::Right, cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.press(Target::Volume(i), FADER_H - 12.0, ev, cx)))
                    .on_mouse_down(MouseButton::Middle, cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.press(Target::Volume(i), FADER_H - 12.0, ev, cx))),
            )
            .child(div().text_size(px(10.)).text_color(c(THEME.text2)).mx_auto().child(view::format_db(ch.volume_db)))
            // the output: free routing; a click lists the channels it can go to (no loops)
            .child(
                div()
                    .id(("out", i))
                    .mx(px(3.))
                    .px(px(4.))
                    .rounded(px(3.))
                    .bg(c(THEME.bg1))
                    .text_size(px(9.5))
                    .text_color(c(THEME.text2))
                    .overflow_hidden()
                    .cursor_pointer()
                    .child(out_name)
                    .when(ch.kind != ChannelKind::Master, |d| {
                        d.on_click(cx.listener(move |p, _, _, cx| {
                            p.output_picker = if p.output_picker == Some(i) { None } else { Some(i) };
                            cx.notify();
                        }))
                    }),
            )
    }

    /// The ghost strip: a click adds a channel (F-MIX-004)
    fn ghost(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("ghost")
            .w(px(STRIP_W - 2.0))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .border_2()
            .border_dashed()
            .border_color(c(THEME.line2))
            .text_color(c(THEME.text3))
            .cursor_pointer()
            .child(tr("+ Channel"))
            .on_click(cx.listener(|p, _, _, cx| p.add_channel(cx)))
    }
}

fn led(label: impl Into<SharedString>, on: bool, color: u32) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .child(div().size(px(6.)).rounded(px(3.)).bg(c(if on { color } else { THEME.bg4 })))
        .child(div().text_size(px(7.5)).text_color(c(THEME.text3)).child(label.into()))
}

// -----------------------------------------------------------------------------------------------------------------
// The inspector: the channel's whole rack, or the master's section

type Toggle = Option<(bool, SharedString, Box<dyn Fn(&mut MixingPage, &mut Context<MixingPage>)>)>;

impl MixingPage {
    fn inspector(&self, cx: &Context<Self>) -> impl IntoElement {
        let core = self.core.read(cx);
        let root = core.port().root();
        let i = self.selected.min(root.channels.len() - 1);
        let ch = &root.channels[i];
        let analog = root.song.desk_analog;
        let body = if ch.kind == ChannelKind::Master { self.master_section(i, cx).into_any_element() } else { self.rack(i, cx).into_any_element() };
        div()
            .id("inspector")
            .w(px(INSPECTOR_W))
            .h_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(12.))
            .bg(c(THEME.bg1))
            .border_l_1()
            .border_color(c(THEME.line))
            .overflow_y_scroll()
            // the desk's switches: the mode (P-17), the meters (P-22), linking the selected faders (P-18)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .child(button("mode", if analog { tr("Analog desk") } else { tr("Digital desk") }, analog, THEME.orange).on_click(cx.listener(move |p, _, _, cx| {
                        p.apply("Desk mode", vec![EditOp::set(ROOT, Field::DeskAnalog, Value::Bool(!analog))], cx);
                    })))
                    .child(button("meters", trf("Meters: {0}", &[&tr(self.meter_kind.label())]), false, 0).on_click(cx.listener(|p, _, _, cx| {
                        p.meter_kind = p.meter_kind.next();
                        cx.notify();
                    })))
                    .child(button("link", trf("Link faders ({0})", &[&(self.multi.len() + 1)]), ch.linked, THEME.blue).on_click(cx.listener(|p, _, _, cx| p.link_selected(cx)))),
            )
            .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).child(format!("{} · {}", i + 1, ch.name)))
            .child(body)
    }

    fn section(title: &'static str, on: Toggle, cx: &Context<Self>) -> gpui::Div {
        let head = div().flex().items_center().child(div().flex_1().font_weight(FontWeight::BOLD).text_size(px(12.)).child(tr(title)));
        let head = match on {
            Some((on, id, f)) => head.child(button(id, if on { tr("On") } else { tr("Off") }, on, THEME.orange).on_click(cx.listener(move |p, _, _, cx| f(p, cx)))),
            None => head,
        };
        div().flex().flex_col().gap(px(6.)).p(px(8.)).rounded(px(6.)).bg(c(THEME.bg2)).child(head)
    }

    fn rack(&self, i: usize, cx: &Context<Self>) -> impl IntoElement {
        let core = self.core.read(cx);
        let root = core.port().root();
        let ch = &root.channels[i];
        let tol = mixer::tolerance(ch.id);
        let returns: Vec<(NodeId, String)> =
            root.channels.iter().filter(|c| c.kind == ChannelKind::Return).map(|c| (c.id, c.name.clone())).collect();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(caption(tr("Analog desk rack")))
            .children(mixer::RACK.iter().map(|(sec, label, params)| {
                let key = mixer::section_key(sec);
                let on = ch.params.get(key).copied().unwrap_or(0.0) > 0.5;
                let toggle: Toggle = if *sec == "tolerance" {
                    None
                } else {
                    Some((on, SharedString::from(format!("ion{sec}")), Box::new(move |p: &mut Self, cx: &mut Context<Self>| p.toggle(i, Field::Param(key), cx))))
                };
                let sect = Self::section(label, toggle, cx);
                if *sec == "tolerance" {
                    // always on; the deviations are fixed per channel, seeded by its id (P-21)
                    return sect.child(div().text_size(px(10.)).text_color(c(THEME.text2)).child(trf(
                        "gain {0} dB · frequencies {1} % · EQ {2} % · saturation {3} %",
                        &[&format!("{:+.2}", tol[0]), &format!("{:+.2}", tol[1]), &format!("{:+.2}", tol[2]), &format!("{:+.2}", tol[3])],
                    )));
                }
                sect.child(div().flex().flex_wrap().gap(px(8.)).children(params.iter().map(|ps| {
                    let v = ch.params.get(ps.key).copied().unwrap_or(ps.default);
                    self.param(i, ps, v, cx)
                })))
            }))
            // inserts: the point among the sections (click = next point), bypass
            .child(Self::section("Inserts", None, cx).children(ch.inserts.iter().map(|ins| {
                let (id, chid, point) = (ins.id, ch.id, ins.point);
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().flex_1().text_size(px(11.)).child(tr(ins.plugin)))
                    .child(button(SharedString::from(format!("pt{}", id.number)), tr(mixer::INSERT_POINTS[point as usize]), false, 0).on_click(cx.listener(move |p, _, _, cx| {
                        p.apply("Insert point", vec![EditOp::set(chid, Field::InsertPoint(id), Value::Real(((point + 1) % 6) as f64))], cx);
                    })))
                    .child(button(SharedString::from(format!("by{}", id.number)), tr("Bypass"), ins.bypass, THEME.orange).on_click(cx.listener(move |p, _, _, cx| p.toggle(i, Field::Bypass(id), cx))))
            })))
            .child(Self::section("Sends", None, cx).children(ch.sends.iter().map(|snd| {
                let to = returns.iter().find(|r| r.0 == snd.to).map_or("?".to_string(), |r| r.1.clone());
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(self.knob(SharedString::from(format!("isnd{}", snd.id.number)), Target::Send(i, snd.id), 22.0, cx))
                    .child(div().flex_1().text_size(px(11.)).child(to))
                    .child(div().text_size(px(10.)).child(view::format_db(snd.level_db.max(-60.0))))
                    .child(div().text_size(px(9.)).text_color(c(THEME.yellow)).child(if snd.pre_fader { tr("PRE") } else { tr("POST") }))
            })))
    }

    fn master_section(&self, i: usize, cx: &Context<Self>) -> impl IntoElement {
        let core = self.core.read(cx);
        let ch = &core.port().root().channels[i];
        let level = core.shown[core.port().telemetry().channel_meter(i)];
        let gr = if ch.params.get("master.bus.in").copied().unwrap_or(0.0) > 0.5 { (level * 6.0).min(10.0) } else { 0.0 };
        let m = self.monitor;
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(mixer::MASTER.iter().map(|(_, label, params)| {
                Self::section(label, None, cx)
                    .child(div().flex().flex_wrap().gap(px(8.)).children(params.iter().map(|ps| {
                        let v = ch.params.get(ps.key).copied().unwrap_or(ps.default);
                        self.param(i, ps, v, cx)
                    })))
                    .when(*label == "BUS COMP", |d| d.child(div().text_size(px(10.)).text_color(c(THEME.text2)).child(trf("Gain reduction {0} dB", &[&format!("{gr:.1}")]))))
            }))
            // the meters of the master (F-MIX-068): two needle VUs, the correlation, the goniometer
            .child(
                Self::section("Meters", None, cx)
                    .child(div().flex().gap(px(8.)).child(vu_needle(level)).child(vu_needle((level * 0.95).min(1.0))).child(goniometer(level)))
                    .child(correlation(0.6 + 0.3 * level)),
            )
            // the monitoring: never saved, undone or exported (F-MIX-067)
            .child(
                Self::section("Monitoring", None, cx)
                    .child(div().text_size(px(9.5)).text_color(c(THEME.text3)).child(tr("Not saved, not exported")))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(4.))
                            .child(button("dim", tr("DIM"), m.dim, THEME.yellow).on_click(cx.listener(|p, _, _, cx| {
                                p.monitor.dim = !p.monitor.dim;
                                cx.notify();
                            })))
                            .child(button("cut", tr("CUT"), m.cut, THEME.red).on_click(cx.listener(|p, _, _, cx| {
                                p.monitor.cut = !p.monitor.cut;
                                cx.notify();
                            })))
                            .child(button("mono", tr("MONO"), m.mono, THEME.blue).on_click(cx.listener(|p, _, _, cx| {
                                p.monitor.mono = !p.monitor.mono;
                                p.monitor.side = false;
                                cx.notify();
                            })))
                            .child(button("side", tr("SIDE"), m.side, THEME.blue).on_click(cx.listener(|p, _, _, cx| {
                                p.monitor.side = !p.monitor.side;
                                p.monitor.mono = false;
                                cx.notify();
                            })))
                            .child(div().text_size(px(10.)).child(trf("LEVEL {0}", &[&view::format_db(m.level_db)]))),
                    ),
            )
    }
}

fn vu_needle(level: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            window.paint_quad(gpui::quad(b, px(4.0), c(THEME.lcd_bg), px(1.0), c(THEME.line2), Default::default()));
            let base = point(b.origin.x + b.size.width / 2.0, b.origin.y + b.size.height - px(4.0));
            let r = b.size.height.as_f32() - 10.0;
            // the scale: ticks on an arc, red from 0 VU
            for k in 0..=10 {
                let a = (-50.0 + k as f32 * 10.0).to_radians();
                let col = if k >= 8 { THEME.red } else { THEME.text3 };
                let p = point(base.x + px(a.sin() * r), base.y - px(a.cos() * r));
                window.paint_quad(fill(Bounds::new(p, size(px(2.0), px(2.0))), c(col)));
            }
            let a = (-50.0 + 100.0 * level).to_radians();
            let mut pb = PathBuilder::stroke(px(1.5));
            pb.move_to(base);
            pb.line_to(point(base.x + px(a.sin() * r), base.y - px(a.cos() * r)));
            if let Ok(p) = pb.build() {
                window.paint_path(p, c(THEME.text));
            }
        },
    )
    .w(px(90.))
    .h(px(56.))
}

fn goniometer(level: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            window.paint_quad(gpui::quad(b, px(4.0), c(THEME.lcd_bg), px(1.0), c(THEME.line2), Default::default()));
            let center = b.center();
            let r = b.size.height.as_f32() / 2.0 - 4.0;
            // a Lissajous figure of a mostly-mono signal, as one path (one primitive)
            let t0 = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f32());
            let mut pb = PathBuilder::stroke(px(1.0));
            for k in 0..160 {
                let t = t0 * 3.0 + k as f32 * 0.21;
                let l = (t * 1.7).sin() * level;
                let rr = (t * 1.7 + 0.35).sin() * level * 0.9;
                let (x, y) = ((l - rr) * 0.7071, (l + rr) * 0.7071);
                let p = point(center.x + px(x * r), center.y - px(y * r));
                if k == 0 {
                    pb.move_to(p)
                } else {
                    pb.line_to(p)
                }
            }
            if let Ok(p) = pb.build() {
                window.paint_path(p, c(THEME.green));
            }
        },
    )
    .size(px(56.))
}

fn correlation(v: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            window.paint_quad(gpui::quad(b, px(3.0), c(THEME.lcd_bg), px(1.0), c(THEME.line2), Default::default()));
            let x = b.origin.x + b.size.width * ((v + 1.0) / 2.0).clamp(0.0, 1.0);
            window.paint_quad(fill(Bounds::new(point(x - px(2.0), b.origin.y), size(px(4.0), b.size.height)), c(if v < 0.0 { THEME.red } else { THEME.green })));
        },
    )
    .w_full()
    .h(px(10.))
}

// -----------------------------------------------------------------------------------------------------------------

impl Render for MixingPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let n_ch = chs(self.core.read(cx)).len();
        if self.selected >= n_ch {
            self.selected = n_ch - 1;
        }
        let normal = self.normal(cx);
        let pinned: Vec<usize> = (0..n_ch).filter(|k| !normal.contains(k)).collect();
        let first = (self.scroll / STRIP_W).floor() as usize;
        // a strip is built only when visible (an upper bound of 24 strips on any screen width here)
        let visible: Vec<usize> = normal.iter().copied().skip(first).take(24).collect();
        let show_ghost = first + visible.len() >= normal.len();
        let offset = -(self.scroll - first as f32 * STRIP_W);
        let bounds_cell = self.strips_bounds.clone();
        let menu = self.menu;
        let picker = self.output_picker;
        div()
            .id("mixing")
            .size_full()
            .relative()
            .flex()
            .flex_row()
            .bg(c(THEME.bg1))
            .key_context("Mixing")
            .track_focus(&self.focus)
            .on_action(cx.listener(|p, _: &SelectLeft, _, cx| {
                p.selected = p.selected.saturating_sub(1);
                cx.notify();
            }))
            .on_action(cx.listener(|p, _: &SelectRight, _, cx| {
                let n = chs(p.core.read(cx)).len();
                p.selected = (p.selected + 1).min(n - 1);
                cx.notify();
            }))
            .on_action(cx.listener(|p, _: &FaderUp, _, cx| p.nudge(1.0, cx)))
            .on_action(cx.listener(|p, _: &FaderDown, _, cx| p.nudge(-1.0, cx)))
            .on_action(cx.listener(|p, _: &FaderUpCoarse, _, cx| p.nudge(3.0, cx)))
            .on_action(cx.listener(|p, _: &FaderDownCoarse, _, cx| p.nudge(-3.0, cx)))
            .on_action(cx.listener(|p, _: &FaderUpFine, _, cx| p.nudge(0.1, cx)))
            .on_action(cx.listener(|p, _: &FaderDownFine, _, cx| p.nudge(-0.1, cx)))
            .on_action(cx.listener(|p, _: &AddChannel, _, cx| p.add_channel(cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(|p, _, window, cx| window.focus(&p.focus, cx)))
            .on_mouse_move(cx.listener(|p, ev: &MouseMoveEvent, _, cx| p.drag_to(ev, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|p, _: &MouseUpEvent, _, cx| p.release(cx)))
            .child(
                div()
                    .id("strips")
                    .flex_1()
                    .h_full()
                    .relative()
                    .overflow_hidden()
                    .on_scroll(cx.listener(Self::on_scroll))
                    .child(canvas(move |b, _, _| bounds_cell.set(b), |_, _, _, _| {}).absolute().size_full())
                    .child(
                        div()
                            .absolute()
                            .left(px(offset))
                            .top(px(0.))
                            .h_full()
                            .flex()
                            .flex_row()
                            .children(visible.into_iter().map(|i| self.strip(i, cx)))
                            .when(show_ghost, |d| d.child(self.ghost(cx))),
                    ),
            )
            .child(div().h_full().flex().flex_row().border_l_2().border_color(c(THEME.line)).children(pinned.into_iter().map(|i| self.strip(i, cx))))
            .child(self.inspector(cx))
            .children(menu.map(|(t, pos)| self.context_menu(t, pos, cx)))
            .children(picker.map(|i| self.output_list(i, cx)))
    }
}

impl MixingPage {
    /// The knob's and fader's menu (F-CTL-006)
    fn context_menu(&self, t: Target, pos: Point<Pixels>, cx: &Context<Self>) -> impl IntoElement {
        let item = |id: &'static str, label: String| {
            div().id(id).px(px(10.)).py(px(4.)).text_size(px(12.)).cursor_pointer().hover(|d| d.bg(c(THEME.bg4))).child(label)
        };
        let title = self.target_name(t, cx);
        let paste = self.clipboard;
        div()
            .absolute()
            .left(pos.x)
            .top(pos.y - px(crate::workspace::CHROME_H))
            .w(px(240.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(c(THEME.bg3))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(div().px(px(10.)).py(px(4.)).text_size(px(10.)).text_color(c(THEME.text3)).child(title))
            .child(item("m-default", tr("Default value").into()).on_click(cx.listener(move |p, _, _, cx| {
                p.menu = None;
                p.set(t, Self::default_travel(t), "Default", cx);
            })))
            .child(item("m-copy", tr("Copy value").into()).on_click(cx.listener(move |p, _, _, cx| {
                p.clipboard = Some(p.travel_of(t, cx));
                p.menu = None;
                cx.notify();
            })))
            .child(item("m-paste", if paste.is_some() { tr("Paste value").into() } else { format!("{} —", tr("Paste value")) }).on_click(cx.listener(move |p, _, _, cx| {
                p.menu = None;
                match p.clipboard {
                    Some(v) => p.set(t, v, "Paste value", cx),
                    None => cx.notify(),
                }
            })))
            .child(item("m-learn", tr("MIDI learn").into()).on_click(cx.listener(move |p, _, _, cx| {
                p.learning = Some(t);
                p.menu = None;
                cx.notify();
            })))
            .child(item("m-auto", tr("Automate (show the lane)").into()).on_click(cx.listener(|p, _, _, cx| {
                p.menu = None;
                cx.notify();
            })))
    }

    /// Where a channel's output can go: every channel that would not make a loop (I-3.5 refuses loops; the list does
    /// not even offer them)
    fn output_list(&self, i: usize, cx: &Context<Self>) -> impl IntoElement {
        let chs = chs(self.core.read(cx));
        let me = chs[i].id;
        let leads_to_me = |mut k: usize| {
            for _ in 0..=chs.len() {
                if chs[k].id == me {
                    return true;
                }
                match chs[k].output.and_then(|o| chs.iter().position(|c| c.id == o)) {
                    Some(n) => k = n,
                    None => return false,
                }
            }
            true
        };
        let options: Vec<(usize, NodeId, String)> =
            (0..chs.len()).filter(|&k| k != i && !leads_to_me(k)).map(|k| (k, chs[k].id, chs[k].name.clone())).collect();
        let x = (8.0 + self.normal(cx).iter().position(|&k| k == i).unwrap_or(0) as f32 * STRIP_W - self.scroll).clamp(0.0, 900.0);
        div()
            .id("outputs")
            .absolute()
            .left(px(x))
            .bottom(px(30.))
            .w(px(220.))
            .max_h(px(360.))
            .overflow_y_scroll()
            .py(px(4.))
            .rounded(px(6.))
            .bg(c(THEME.bg3))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(div().px(px(10.)).py(px(4.)).text_size(px(10.)).text_color(c(THEME.text3)).child(trf("Output of {0}", &[&chs[i].name])))
            .children(options.into_iter().map(|(k, id, name)| {
                div()
                    .id(("opt", k))
                    .px(px(10.))
                    .py(px(3.))
                    .text_size(px(11.))
                    .cursor_pointer()
                    .hover(|d| d.bg(c(THEME.bg4)))
                    .child(format!("{} · {}", k + 1, name))
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.output_picker = None;
                        p.apply("Output", vec![EditOp::set(me, Field::ChannelOutput, Value::Id(id))], cx);
                    }))
            }))
    }
}
