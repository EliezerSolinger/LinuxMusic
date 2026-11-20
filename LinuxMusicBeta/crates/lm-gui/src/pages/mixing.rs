//! The mixing page: the whole desk, a page of its own (user's decision, 19/11). Each strip shows what is touched while
//! mixing — the rack's sections as switches with the SAT and CLIP lights and the tube's lamp, inserts with their
//! point, a send per return, pan, mute, solo, the fader over the meter, the output — and the selected channel's whole
//! rack, or the master's section with its meters and the monitoring, opens in the inspector on the right. Groups are
//! channels others output to; returns, channels others send to (read from the routing). Ctrl+click selects several
//! strips and "Link faders" makes them move together, in one gesture (P-18, the interface's own state).
//!
//! Knobs and faders: drag = one gesture; Shift = fine; double click or middle button = default; the right button
//! opens the menu (default, copy and paste the value, MIDI learn, automate) (F-CTL-005/006). Keys: ←/→ select,
//! ↑/↓ move the selected fader 1 dB (Shift 3 dB, Alt 0.1 dB), Shift+Insert adds a channel. Through the port only.

use std::collections::BTreeSet;

use gpui::{
    AnyElement, Bounds, Context, Entity, FocusHandle, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PathBuilder, Pixels, Point, Render, ScrollEvent, SharedString, WeakEntity, Window, canvas, div, fill,
    point, prelude::*, px, size,
};
use lm_port::{EditOp, GestureId, Label, NodeId, PathStep, ROOT, Ref, Value};

use crate::actions::{
    AddChannel, FaderDown, FaderDownCoarse, FaderDownFine, FaderUp, FaderUpCoarse, FaderUpFine, SelectLeft, SelectRight,
};
use crate::core::Core;
use crate::model::{self, ChannelKind, ChannelView};
use crate::screens::Screen;
use crate::ui::{THEME, button, c, caption, fader_over_meter, knob_face, tr, track_color, trf};
use crate::view;
use crate::workspace::Workspace;

pub const STRIP_W: f32 = 96.0;
const INSPECTOR_W: f32 = 330.0;
const FADER_H: f32 = 190.0;

/// One parameter of a rack section: its key in the section's node, label, range and default; or the choices of a
/// selector (the value is then the choice's key text), or a switch
#[derive(Clone, Copy, Debug)]
pub enum Spec {
    Knob { key: &'static str, label: &'static str, min: f64, max: f64, default: f64, unit: &'static str },
    Choice { key: &'static str, label: &'static str, choices: &'static [&'static str], default: &'static str },
    Switch { key: &'static str, label: &'static str },
}

impl Spec {
    pub fn key(&self) -> &'static str {
        match self {
            Spec::Knob { key, .. } | Spec::Choice { key, .. } | Spec::Switch { key, .. } => key,
        }
    }
}

const fn k(key: &'static str, label: &'static str, min: f64, max: f64, default: f64, unit: &'static str) -> Spec {
    Spec::Knob { key, label, min, max, default, unit }
}

pub const GAIN_MODELS: &[&str] =
    &["flat", "4558", "tl072", "5532", "5534", "12ax7", "12ax7-vintage", "12ax7-hot", "12at7", "12au7"];
pub const COMPRESSOR_MODELS: &[&str] =
    &["ssl", "api2500", "dbx160", "urei1176", "la2a", "fairchild670", "gates-sta-level", "altec436", "rca-ba6a"];
pub const COMPRESSOR_SPEEDS: &[&str] = &["fast", "norm", "slow"];
pub const GATE_MODES: &[&str] = &["std", "kick", "snare", "toms", "vox", "guitar", "bass", "noise", "expander"];
pub const BUS_RATIOS: &[&str] = &["2", "4", "10"];
pub const INSERT_POINTS: &[&str] = &["pre-gain", "post-gain", "post-filter", "post-dynamics", "post-eq", "pre-pan"];

/// The channel's rack (lm-doc's schemas `rack.*`; F-MIX-040..055): (section key, label, parameters)
pub const RACK: &[(&str, &str, &[Spec])] = &[
    (
        "tube",
        "Gain stage",
        &[
            k("drive", "GAIN", 0.0, 1.0, 0.25, "gain"),
            Spec::Choice { key: "model", label: "Model", choices: GAIN_MODELS, default: "4558" },
            Spec::Switch { key: "realistic", label: "Realistic tubes" },
            Spec::Switch { key: "lowCut", label: "LCUT" },
        ],
    ),
    (
        "eq",
        "EQ and filters",
        &[
            k("highPass", "HPF", 20.0, 20000.0, 20.0, "Hz"),
            k("lowPass", "LPF", 20.0, 20000.0, 20000.0, "Hz"),
            k("monoBass", "Mono bass", 0.0, 100.0, 0.0, "%"),
            k("monoBassFrequency", "Freq", 20.0, 500.0, 120.0, "Hz"),
            k("high", "HI", -15.0, 15.0, 0.0, "dB"),
            k("highMid", "HMF", -15.0, 15.0, 0.0, "dB"),
            k("lowMid", "LMF", -15.0, 15.0, 0.0, "dB"),
            k("low", "LO", -15.0, 15.0, 0.0, "dB"),
        ],
    ),
    (
        "dynamics",
        "Dynamics",
        &[
            Spec::Choice { key: "compModel", label: "Model", choices: COMPRESSOR_MODELS, default: "ssl" },
            k("compAmount", "COMP", 0.0, 1.0, 0.0, ""),
            k("compRatio", "RATIO", 0.0, 1.0, 0.0, ""),
            Spec::Choice { key: "compSpeed", label: "ATK", choices: COMPRESSOR_SPEEDS, default: "norm" },
            k("compRelease", "REL", 0.0, 1.0, 0.5, ""),
            k("gateThreshold", "Gate", -60.0, 20.0, -10.0, "dB"),
            k("gateRange", "Range", 0.0, 80.0, 0.0, "dB"),
            Spec::Choice { key: "gateMode", label: "Mode", choices: GATE_MODES, default: "std" },
        ],
    ),
    ("transformer", "Transformer", &[]),
    ("tolerance", "Tolerance", &[]),
];

/// The master's rack (`rack.master`; F-MIX-060..066)
pub const MASTER_RACK: &[(&str, &str, &[Spec])] = &[
    (
        "tube",
        "DRIVE",
        &[
            k("drive", "DRIVE", 0.0, 1.0, 0.25, "gain"),
            Spec::Choice { key: "model", label: "Model", choices: GAIN_MODELS, default: "4558" },
        ],
    ),
    (
        "stereo",
        "Stereo",
        &[
            k("width", "WIDTH", 0.0, 200.0, 100.0, "%"),
            k("monoBass", "BASS", 0.0, 100.0, 0.0, "%"),
            k("bassFrequency", "FREQ", 20.0, 500.0, 120.0, "Hz"),
            k("air", "AIR", -12.0, 12.0, 0.0, "dB"),
        ],
    ),
    (
        "dynamics",
        "BUS COMP",
        &[
            k("threshold", "THR", -15.0, 15.0, 5.0, ""),
            Spec::Choice { key: "ratio", label: "RATIO", choices: BUS_RATIOS, default: "4" },
            k("attack", "ATK", 0.0, 5.0, 4.0, "step"),
            k("release", "REL", 0.0, 4.0, 4.0, "step"),
            k("makeup", "MAKE", -5.0, 15.0, 0.0, "dB"),
        ],
    ),
    ("transformer", "XFMR", &[]),
];

#[derive(Clone, Debug, PartialEq)]
enum Target {
    Volume(NodeId),
    Pan(NodeId),
    /// a rack parameter: the channel, the section and the key
    Rack(NodeId, &'static str, Spec),
    Send(NodeId),
}

impl PartialEq for Spec {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

struct Drag {
    target: Target,
    start_y: f32,
    start_t: f64,
    travel: f32,
    gesture: GestureId,
    /// the linked faders' ids and starting travels (P-18)
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
    /// The same signal on each scale (the real readings come from the engine's meters, P-22)
    fn shown(self, fill_v: f32) -> f32 {
        let on = if fill_v > 0.0 { 1.0 } else { 0.0 };
        match self {
            MeterKind::Peak => fill_v,
            MeterKind::Rms => fill_v * 0.82,
            MeterKind::Vu => (fill_v * 0.82 + 0.06).min(1.0) * on,
            MeterKind::K14 => (fill_v * 0.82 + 0.14).min(1.0) * on,
        }
    }
}

/// The monitoring of the master (F-MIX-067): never saved, undone or exported, so it is the interface's state
#[derive(Clone, Copy, Debug, Default)]
struct Monitoring {
    dim: bool,
    cut: bool,
    mono: bool,
    side: bool,
}

pub struct MixingPage {
    core: Entity<Core>,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    pub scroll: f32,
    pub selected: Option<NodeId>,
    multi: BTreeSet<NodeId>,
    /// linked faders (P-18): the interface's state until the document has a group for it
    linked: BTreeSet<NodeId>,
    drag: Option<Drag>,
    menu: Option<(Target, Point<Pixels>)>,
    output_picker: Option<NodeId>,
    clipboard: Option<f64>,
    learning: Option<Target>,
    meter_kind: MeterKind,
    monitor: Monitoring,
    strips_bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
}

fn channels(core: &Core) -> Vec<ChannelView> {
    model::channels(core.port().root())
}

fn num(v: &Value) -> Option<f64> {
    v.as_real().or_else(|| v.as_int().map(|i| i as f64))
}

impl MixingPage {
    pub fn new(core: Entity<Core>, workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        let first = channels(core.read(cx)).first().map(|c| c.id);
        MixingPage {
            core,
            workspace,
            focus: cx.focus_handle(),
            scroll: 0.0,
            selected: first,
            multi: BTreeSet::new(),
            linked: BTreeSet::new(),
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

    fn channel(&self, id: NodeId, cx: &Context<Self>) -> Option<ChannelView> {
        channels(self.core.read(cx)).into_iter().find(|c| c.id == id)
    }

    // --- values and edits ----------------------------------------------------------------------------------

    /// A target's value as 0..1 of its control's travel
    fn travel_of(&self, t: &Target, cx: &Context<Self>) -> f64 {
        let chs = channels(self.core.read(cx));
        let ch = |id: NodeId| chs.iter().find(|c| c.id == id);
        match t {
            Target::Volume(id) => ch(*id).map_or(0.75, |c| view::gain_to_travel(c.volume)),
            Target::Pan(id) => ch(*id).map_or(0.5, |c| (c.pan + 1.0) / 2.0),
            Target::Rack(id, sec, Spec::Knob { key, min, max, default, .. }) => {
                let v = ch(*id).and_then(|c| model::at(&c.rack, &[sec, key]).and_then(num)).unwrap_or(*default);
                (v - min) / (max - min)
            }
            Target::Rack(..) => 0.0,
            Target::Send(sid) => chs
                .iter()
                .flat_map(|c| c.sends.iter())
                .find(|s| s.id == *sid)
                .map_or(0.0, |s| view::gain_to_travel(s.amount)),
        }
    }

    fn op_of(t: &Target, travel: f64) -> EditOp {
        let travel = travel.clamp(0.0, 1.0);
        let real = |v: f64| Value::real(v).unwrap_or_default();
        match t {
            Target::Volume(id) => EditOp::set(*id, &["volume"], real(view::travel_to_gain(travel))),
            Target::Pan(id) => EditOp::set(*id, &["pan"], real(travel * 2.0 - 1.0)),
            Target::Rack(id, sec, Spec::Knob { key, min, max, unit, .. }) => {
                let v = min + travel * (max - min);
                let value = if *unit == "step" { Value::int(v.round() as i64) } else { real(v) };
                EditOp::set(*id, &["rack", sec, key], value)
            }
            Target::Rack(id, sec, s) => EditOp::set(*id, &["rack", sec, s.key()], Value::NULL),
            Target::Send(sid) => EditOp::set(*sid, &["amount"], real(view::travel_to_gain(travel))),
        }
    }

    fn default_travel(t: &Target) -> f64 {
        match t {
            Target::Volume(_) => view::db_to_travel(0.0),
            Target::Send(_) => view::db_to_travel(0.0),
            Target::Pan(_) => 0.5,
            Target::Rack(_, _, Spec::Knob { min, max, default, .. }) => (default - min) / (max - min),
            Target::Rack(..) => 0.0,
        }
    }

    fn apply(&mut self, label: &'static str, ops: Vec<EditOp>, cx: &mut Context<Self>) {
        self.core.update(cx, |core, _| {
            let _ = core.port_mut().apply(Label::new(label), ops);
        });
        cx.notify();
    }

    fn set(&mut self, t: &Target, travel: f64, label: &'static str, cx: &mut Context<Self>) {
        self.apply(label, vec![Self::op_of(t, travel)], cx);
    }

    // --- gestures ------------------------------------------------------------------------------------------

    fn press(&mut self, t: Target, travel: f32, channel: NodeId, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        self.selected = Some(channel);
        self.menu = None;
        cx.stop_propagation();
        match ev.button {
            MouseButton::Right => {
                self.menu = Some((t, ev.position));
                cx.notify();
                return;
            }
            MouseButton::Middle => return self.set(&t, Self::default_travel(&t), "Default", cx),
            _ => {}
        }
        if ev.click_count == 2 {
            return self.set(&t, Self::default_travel(&t), "Default", cx);
        }
        if self.learning.as_ref() == Some(&t) {
            self.learning = None; // learned: the next controller move would be bound (no MIDI input here yet)
        }
        let start_t = self.travel_of(&t, cx);
        let linked: Vec<(NodeId, f64)> = match &t {
            Target::Volume(id) if self.linked.contains(id) => channels(self.core.read(cx))
                .iter()
                .filter(|c| c.id != *id && self.linked.contains(&c.id))
                .map(|c| (c.id, view::gain_to_travel(c.volume)))
                .collect(),
            _ => Vec::new(),
        };
        let prev = self.drag.take();
        let gesture = self.core.update(cx, |core, _| {
            if let Some(d) = prev {
                core.port_mut().gesture_commit(d.gesture);
            }
            core.port_mut().begin_gesture(Label::new("Mix"))
        });
        self.drag = Some(Drag { target: t, start_y: ev.position.y.as_f32(), start_t, travel, gesture, linked });
        cx.notify();
    }

    fn drag_to(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = &self.drag else { return };
        let fine = if ev.modifiers.shift { 10.0 } else { 1.0 };
        let dt = -((ev.position.y.as_f32() - d.start_y) / (d.travel * fine)) as f64;
        let mut ops = vec![Self::op_of(&d.target, d.start_t + dt)];
        // the linked faders move by the same travel, in the same gesture (one undo step)
        for (id, start) in &d.linked {
            ops.push(Self::op_of(&Target::Volume(*id), start + dt));
        }
        // the fader's value reaches the audio at once (I-6), the edit follows in the gesture
        if let Target::Volume(id) = d.target {
            let db = view::travel_to_db((d.start_t + dt).clamp(0.0, 1.0));
            self.core.update(cx, |core, _| core.port_mut().param_write(id, "volume", view::db_to_gain(db) as f32));
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

    fn target_name(&self, t: &Target, cx: &Context<Self>) -> String {
        let chs = channels(self.core.read(cx));
        let (id, what) = match t {
            Target::Volume(id) => (Some(*id), tr("Volume").to_string()),
            Target::Pan(id) => (Some(*id), tr("Pan").to_string()),
            Target::Rack(id, _, s) => (
                Some(*id),
                tr(match s {
                    Spec::Knob { label, .. } | Spec::Choice { label, .. } | Spec::Switch { label, .. } => label,
                })
                .to_string(),
            ),
            Target::Send(sid) => {
                (chs.iter().find(|c| c.sends.iter().any(|s| s.id == *sid)).map(|c| c.id), tr("Send").to_string())
            }
        };
        let name = id.and_then(|id| chs.iter().find(|c| c.id == id)).map_or_else(String::new, |c| c.name.to_string());
        format!("{name} · {what}")
    }

    fn toggle(&mut self, id: NodeId, path: &[&str], now: bool, cx: &mut Context<Self>) {
        self.apply("Toggle", vec![EditOp::set(id, path, Value::bool(!now))], cx);
    }

    /// A selector's next choice (click)
    fn cycle(
        &mut self,
        id: NodeId,
        sec: &str,
        key: &str,
        choices: &'static [&'static str],
        now: &str,
        cx: &mut Context<Self>,
    ) {
        let i = choices.iter().position(|c| *c == now).map_or(0, |i| (i + 1) % choices.len());
        self.apply("Choice", vec![EditOp::set(id, &["rack", sec, key], Value::text(choices[i]))], cx);
    }

    fn select(&mut self, id: NodeId, ctrl: bool, cx: &mut Context<Self>) {
        if ctrl {
            // the strip selected before joins the selection
            if let Some(prev) = self.selected {
                self.multi.insert(prev);
            }
            if !self.multi.remove(&id) {
                self.multi.insert(id);
            }
        } else {
            self.multi.clear();
        }
        self.selected = Some(id);
        self.output_picker = None;
        self.menu = None;
        cx.notify();
    }

    /// Shows a channel (the FX button of a track's header, P-28)
    pub fn select_channel(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let chs = channels(self.core.read(cx));
        let normal: Vec<NodeId> = chs
            .iter()
            .filter(|c| c.kind != ChannelKind::Master && c.kind != ChannelKind::Return)
            .map(|c| c.id)
            .collect();
        if chs.iter().any(|c| c.id == id) {
            self.selected = Some(id);
            self.multi.clear();
            let n = normal.iter().position(|c| *c == id).unwrap_or(0) as f32;
            self.scroll = (n * STRIP_W - 2.0 * STRIP_W).max(0.0);
            cx.notify();
        }
    }

    fn link_selected(&mut self, cx: &mut Context<Self>) {
        let mut set = self.multi.clone();
        if let Some(s) = self.selected {
            set.insert(s);
        }
        if set.len() > 1 {
            self.linked.extend(set);
        } else {
            for id in set {
                self.linked.remove(&id);
            }
        }
        cx.notify();
    }

    fn nudge(&mut self, db: f64, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        let Some(ch) = self.channel(id, cx) else { return };
        let v = (view::gain_to_db(ch.volume).max(-60.0) + db).clamp(-60.0, 12.0);
        self.apply(
            "Fader",
            vec![EditOp::set(id, &["volume"], Value::real(view::db_to_gain(v)).unwrap_or_default())],
            cx,
        );
    }

    fn move_selection(&mut self, by: isize, cx: &mut Context<Self>) {
        let chs = channels(self.core.read(cx));
        let i = self.selected.and_then(|s| chs.iter().position(|c| c.id == s)).unwrap_or(0) as isize;
        let j = (i + by).clamp(0, chs.len() as isize - 1) as usize;
        self.selected = chs.get(j).map(|c| c.id);
        cx.notify();
    }

    /// A new channel after the last one (the ghost strip, Shift+Insert, F-MIX-004)
    fn add_channel(&mut self, cx: &mut Context<Self>) {
        let n = model::at(self.core.read(cx).port().root(), &["mixer", "channels"]).map_or(0, |l| l.len());
        let name = trf("Channel {0}", &[&(n + 1)]);
        let op = EditOp::Insert {
            temp: 0,
            parent: Ref::Id(ROOT),
            list: "mixer.channels".into(),
            index: n,
            node: Value::object([("name", Value::text(name))]),
        };
        let created = self.core.update(cx, |core, _| core.port_mut().apply(Label::new("Add channel"), vec![op]).ok());
        if let Some(id) = created.and_then(|a| a.created.first().map(|(_, id)| *id)) {
            self.selected = Some(id);
        }
        cx.notify();
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let delta = if d.x.as_f32().abs() > d.y.as_f32().abs() { d.x } else { d.y }.as_f32();
        let normal = channels(self.core.read(cx))
            .iter()
            .filter(|c| c.kind != ChannelKind::Master && c.kind != ChannelKind::Return)
            .count()
            + 1;
        let visible = self.strips_bounds.get().size.width.as_f32();
        self.scroll = (self.scroll - delta).clamp(0.0, (normal as f32 * STRIP_W - visible).max(0.0));
        cx.notify();
    }
}

// -----------------------------------------------------------------------------------------------------------------
// Pieces

impl MixingPage {
    fn knob(&self, id: SharedString, t: Target, channel: NodeId, d: f32, cx: &Context<Self>) -> impl IntoElement {
        let travel = self.travel_of(&t, cx);
        let learning = self.learning.as_ref() == Some(&t);
        let name = self.target_name(&t, cx);
        let (t1, t2, t3) = (t.clone(), t.clone(), t);
        div()
            .id(id.clone())
            .debug_selector(move || id.to_string())
            .role(gpui::Role::Slider)
            .aria_label(SharedString::from(name))
            .aria_numeric_value(travel)
            .aria_min_numeric_value(0.0)
            .aria_max_numeric_value(1.0)
            .cursor_ns_resize()
            .rounded(px(d))
            .when(learning, |d| d.border_2().border_color(c(THEME.yellow)))
            .child(knob_face(travel, d))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.press(t1.clone(), 200.0, channel, ev, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.press(t2.clone(), 200.0, channel, ev, cx)),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |p, ev: &MouseDownEvent, _, cx| p.press(t3.clone(), 200.0, channel, ev, cx)),
            )
    }

    /// A rack parameter: a knob with its label and value, a selector (click = next choice), or a switch
    fn param(&self, ch: &ChannelView, sec: &'static str, spec: Spec, cx: &Context<Self>) -> AnyElement {
        let id = ch.id;
        let v = model::at(&ch.rack, &[sec, spec.key()]).cloned().unwrap_or_default();
        match spec {
            Spec::Choice { key, label, choices, default } => {
                let now = v.as_text().unwrap_or(default).to_string();
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        button(
                            SharedString::from(format!("ch{}{sec}{key}", id.number)),
                            tr(Box::leak(now.clone().into_boxed_str())),
                            false,
                            0,
                        )
                        .min_w(px(52.))
                        .on_click(cx.listener(move |p, _, _, cx| p.cycle(id, sec, key, choices, &now, cx))),
                    )
                    .child(div().text_size(px(9.)).text_color(c(THEME.text3)).child(tr(label)))
                    .into_any_element()
            }
            Spec::Switch { key, label } => {
                let on = v.as_bool().unwrap_or(false);
                button(SharedString::from(format!("sw{}{sec}{key}", id.number)), tr(label), on, THEME.orange)
                    .on_click(cx.listener(move |p, _, _, cx| p.toggle(id, &["rack", sec, key], on, cx)))
                    .into_any_element()
            }
            Spec::Knob { key, label, default, unit, .. } => {
                let value = num(&v).unwrap_or(default);
                let shown = match unit {
                    "gain" => view::format_db(if value <= 0.0 {
                        f64::NEG_INFINITY
                    } else if value <= 0.25 {
                        60.0 * (value / 0.25).log10()
                    } else {
                        (value - 0.25) / 0.75 * 60.0
                    }),
                    "" | "step" => format!("{value:.2}"),
                    u => format!("{value:.1} {u}"),
                };
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(2.))
                    .child(self.knob(
                        SharedString::from(format!("rk-{}-{sec}-{key}", id.number)),
                        Target::Rack(id, sec, spec),
                        id,
                        26.0,
                        cx,
                    ))
                    .child(div().text_size(px(9.)).text_color(c(THEME.text2)).child(tr(label)))
                    .child(div().text_size(px(9.)).child(shown))
                    .into_any_element()
            }
        }
    }

    fn strip(
        &self,
        ch: &ChannelView,
        index: usize,
        returns: &[(NodeId, String)],
        all: &[ChannelView],
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let core = self.core.read(cx);
        let analog = model::song(core.port().root()).analog;
        let id = ch.id;
        let peak = core.meter(id);
        let level = self.meter_kind.shown(peak);
        let color = match ch.kind {
            ChannelKind::Normal => track_color(index),
            ChannelKind::Group => THEME.purple,
            ChannelKind::Return => THEME.cyan,
            ChannelKind::Master => THEME.green,
        };
        let out_name: SharedString = match ch.output {
            None => tr("→ Device").into(),
            Some(o) => format!("→ {}", all.iter().find(|x| x.id == o).map_or("?", |x| &x.name)).into(),
        };
        let selected = self.selected == Some(id) || self.multi.contains(&id);
        let fader_t = view::gain_to_travel(ch.volume);
        let model_key = model::at(&ch.rack, &["tube", "model"]).and_then(Value::as_text).unwrap_or("4558").to_string();
        let tube = analog && model_key.starts_with("12a") && ch.kind != ChannelKind::Master;
        let realistic = model::at(&ch.rack, &["tube", "realistic"]).and_then(Value::as_bool).unwrap_or(false);
        let (mute, solo, volume) = (ch.mute, ch.solo, ch.volume);
        let linked = self.linked.contains(&id);
        div()
            .id(("strip", index))
            .debug_selector(move || format!("strip-{}", id.number))
            .w(px(STRIP_W - 2.0))
            .h_full()
            .flex()
            .flex_col()
            .gap(px(3.))
            .pb(px(6.))
            .bg(c(if selected { THEME.bg3 } else { THEME.bg2 }))
            .border_r_1()
            .border_color(c(THEME.line))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                    p.select(id, ev.modifiers.platform || ev.modifiers.control, cx)
                }),
            )
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
                    .child(if ch.kind == ChannelKind::Master { String::new() } else { format!("{}", index + 1) })
                    .child(div().flex_1().overflow_hidden().child(ch.name.to_string()))
                    .when(linked, |d| d.child("🔗")),
            )
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .px(px(4.))
                    .h(px(10.))
                    .items_center()
                    .when(analog, |d| {
                        d.child(led(tr("SAT"), peak > 0.78, THEME.yellow)).child(led(
                            tr("CLIP"),
                            peak > 0.95,
                            THEME.red,
                        ))
                    })
                    .when(tube, |d| {
                        d.child(led(
                            if realistic { "RL" } else { "LT" },
                            true,
                            if realistic { THEME.red } else { THEME.yellow },
                        ))
                    })
                    .when(!analog, |d| {
                        d.child(div().text_size(px(8.)).text_color(c(THEME.text3)).child(tr("digital")))
                    }),
            )
            // the rack's sections as switches; the knobs are in the inspector
            .when(ch.kind != ChannelKind::Master, |d| {
                d.child(div().flex().flex_wrap().gap(px(2.)).px(px(3.)).children(
                    RACK.iter().filter(|(s, _, _)| *s != "tolerance").map(|(sec, label, _)| {
                        let on = analog
                            && model::at(&ch.rack, &[sec, "on"])
                                .and_then(Value::as_bool)
                                .unwrap_or(*sec != "transformer");
                        let short: String = tr(label).chars().take(3).collect();
                        let sec = *sec;
                        button(SharedString::from(format!("rk{}{sec}", id.number)), short, on, THEME.orange)
                            .debug_selector(move || format!("section-{}-{sec}", id.number))
                            .on_click(cx.listener(move |p, _, _, cx| p.toggle(id, &["rack", sec, "on"], on, cx)))
                    }),
                ))
            })
            // inserts with their point (P-20); a click opens the effect's window, Alt+click bypasses
            .child(div().px(px(3.)).child(caption(tr("Inserts"))))
            .children(ch.inserts.iter().map(|ins| {
                let (iid, bypass) = (ins.id, ins.bypass);
                div()
                    .id(SharedString::from(format!("ins{}", iid.number)))
                    .mx(px(3.))
                    .px(px(4.))
                    .h(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .rounded(px(3.))
                    .bg(c(THEME.bg1))
                    .text_size(px(10.))
                    .text_color(c(if bypass { THEME.text3 } else { THEME.text }))
                    .overflow_hidden()
                    .cursor_pointer()
                    .child(div().flex_1().overflow_hidden().child(ins.plugin.to_string()))
                    .child(div().text_size(px(8.)).text_color(c(THEME.text3)).child(
                        tr(Box::leak(ins.point.to_string().into_boxed_str())).chars().take(5).collect::<String>(),
                    ))
                    .on_click(cx.listener(move |p, ev: &gpui::ClickEvent, window, cx| {
                        if ev.modifiers().alt {
                            p.toggle(iid, &["bypass"], bypass, cx);
                        } else {
                            let _ = p.workspace.update(cx, |ws, cx| ws.open(Screen::Effect, Some(iid), window, cx));
                        }
                    }))
            }))
            // sends: one per return (P-19)
            .when(ch.kind == ChannelKind::Normal || ch.kind == ChannelKind::Group, |d| {
                d.child(div().px(px(3.)).child(caption(tr("Sends")))).children(
                    returns.iter().filter(|(r, _)| *r != id).map(|(rid, name)| {
                        let snd = ch.sends.iter().find(|x| x.to == *rid);
                        div()
                            .flex()
                            .items_center()
                            .gap(px(3.))
                            .px(px(3.))
                            .children(snd.map(|s| {
                                self.knob(
                                    SharedString::from(format!("snd-{}", s.id.number)),
                                    Target::Send(s.id),
                                    id,
                                    16.0,
                                    cx,
                                )
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(9.5))
                                    .text_color(c(if snd.is_some() { THEME.text } else { THEME.text3 }))
                                    .overflow_hidden()
                                    .child(name.clone()),
                            )
                            .when(snd.is_some_and(|x| x.pre), |d| {
                                d.child(div().text_size(px(8.5)).text_color(c(THEME.yellow)).child(tr("PRE")))
                            })
                    }),
                )
            })
            .child(div().flex_1())
            .when(ch.kind != ChannelKind::Master, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(3.))
                        .child(self.knob(
                            SharedString::from(format!("pan-{}", id.number)),
                            Target::Pan(id),
                            id,
                            22.0,
                            cx,
                        ))
                        .child(
                            button(SharedString::from(format!("mm{}", id.number)), "M", mute, THEME.orange)
                                .debug_selector(move || format!("mute-{}", id.number))
                                .on_click(cx.listener(move |p, _, _, cx| p.toggle(id, &["mute"], mute, cx))),
                        )
                        .child(
                            button(SharedString::from(format!("ms{}", id.number)), "S", solo, THEME.yellow)
                                .debug_selector(move || format!("solo-{}", id.number))
                                .on_click(cx.listener(move |p, _, _, cx| p.toggle(id, &["solo"], solo, cx))),
                        ),
                )
            })
            // the peak readout, the fader over the meter (user's decision), the value
            .child(
                div()
                    .mx_auto()
                    .text_size(px(9.))
                    .text_color(c(if peak > 0.95 { THEME.red } else { THEME.text3 }))
                    .child(if peak > 0.0 { format!("{:.1}", peak * 60.0 - 60.0) } else { "-inf".into() }),
            )
            .child(
                div()
                    .id(SharedString::from(format!("fader-{}", id.number)))
                    .debug_selector(move || format!("fader-{}", id.number))
                    .role(gpui::Role::Slider)
                    .aria_label(SharedString::from(format!("{} {}", tr("Volume"), ch.name)))
                    .aria_numeric_value(view::gain_to_db(volume).max(-60.0))
                    .aria_min_numeric_value(-60.0)
                    .aria_max_numeric_value(12.0)
                    .mx_auto()
                    .cursor_ns_resize()
                    .child(fader_over_meter(fader_t, vec![level, (level * 0.92).min(1.0)], 44.0, FADER_H))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                            p.press(Target::Volume(id), FADER_H - 12.0, id, ev, cx)
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                            p.press(Target::Volume(id), FADER_H - 12.0, id, ev, cx)
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                            p.press(Target::Volume(id), FADER_H - 12.0, id, ev, cx)
                        }),
                    ),
            )
            .child(div().text_size(px(10.)).text_color(c(THEME.text2)).mx_auto().child(view::format_gain(volume)))
            .child(
                div()
                    .id(SharedString::from(format!("out-{}", id.number)))
                    .debug_selector(move || format!("out-{}", id.number))
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
                            p.output_picker = if p.output_picker == Some(id) { None } else { Some(id) };
                            cx.notify();
                        }))
                    }),
            )
    }

    fn ghost(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("ghost")
            .debug_selector(|| "ghost".into())
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
// The inspector

impl MixingPage {
    fn inspector(&self, cx: &Context<Self>) -> impl IntoElement {
        let core = self.core.read(cx);
        let chs = channels(core);
        let analog = model::song(core.port().root()).analog;
        let Some(ch) = self.selected.and_then(|s| chs.iter().find(|c| c.id == s)).or(chs.first()).cloned() else {
            return div().into_any_element();
        };
        let body = if ch.kind == ChannelKind::Master {
            self.master_section(&ch, cx).into_any_element()
        } else {
            self.rack(&ch, &chs, cx).into_any_element()
        };
        let is_linked = self.linked.contains(&ch.id);
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
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .child(
                        button(
                            "mode",
                            if analog { tr("Analog desk") } else { tr("Digital desk") },
                            analog,
                            THEME.orange,
                        )
                        .debug_selector(|| "desk-mode".into())
                        .on_click(cx.listener(move |p, _, _, cx| {
                            let op = EditOp::Set {
                                id: Ref::Id(ROOT),
                                path: vec![PathStep::key("mixer"), PathStep::key("mode")],
                                value: Value::text(if analog { "digital" } else { "analog" }),
                            };
                            p.apply("Desk mode", vec![op], cx);
                        })),
                    )
                    .child(button("meters", trf("Meters: {0}", &[&tr(self.meter_kind.label())]), false, 0).on_click(
                        cx.listener(|p, _, _, cx| {
                            p.meter_kind = p.meter_kind.next();
                            cx.notify();
                        }),
                    ))
                    .child(
                        button("link", trf("Link faders ({0})", &[&(self.multi.len() + 1)]), is_linked, THEME.blue)
                            .debug_selector(|| "link".into())
                            .on_click(cx.listener(|p, _, _, cx| p.link_selected(cx))),
                    ),
            )
            .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).child(ch.name.to_string()))
            .child(body)
            .into_any_element()
    }

    fn section(title: &'static str, toggle: Option<(bool, NodeId, &'static str)>, cx: &Context<Self>) -> gpui::Div {
        let head = div()
            .flex()
            .items_center()
            .child(div().flex_1().font_weight(FontWeight::BOLD).text_size(px(12.)).child(tr(title)));
        let head = match toggle {
            Some((on, id, sec)) => head.child(
                button(
                    SharedString::from(format!("ion{}{sec}", id.number)),
                    if on { tr("On") } else { tr("Off") },
                    on,
                    THEME.orange,
                )
                .on_click(cx.listener(move |p, _, _, cx| p.toggle(id, &["rack", sec, "on"], on, cx))),
            ),
            None => head,
        };
        div().flex().flex_col().gap(px(6.)).p(px(8.)).rounded(px(6.)).bg(c(THEME.bg2)).child(head)
    }

    fn rack(&self, ch: &ChannelView, all: &[ChannelView], cx: &Context<Self>) -> impl IntoElement {
        let id = ch.id;
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(caption(tr("Analog desk rack")))
            .children(RACK.iter().map(|(sec, label, params)| {
                let on = model::at(&ch.rack, &[sec, "on"]).and_then(Value::as_bool).unwrap_or(*sec != "transformer");
                let sect = Self::section(label, Some((on, id, sec)), cx);
                if *sec == "tolerance" {
                    // fixed deviations per channel, seeded by its id (P-21), made by the DSP
                    return sect.child(
                        div()
                            .text_size(px(10.))
                            .text_color(c(THEME.text2))
                            .child(tr("Component tolerances, fixed per channel")),
                    );
                }
                sect.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .children(params.iter().map(|spec| self.param(ch, sec, *spec, cx))),
                )
            }))
            // inserts: the point among the sections (click = next point), bypass
            .child(Self::section("Inserts", None, cx).children(ch.inserts.iter().map(|ins| {
                let (iid, point, bypass) = (ins.id, ins.point.to_string(), ins.bypass);
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().flex_1().text_size(px(11.)).child(ins.plugin.to_string()))
                    .child(
                        button(
                            SharedString::from(format!("pt{}", iid.number)),
                            tr(Box::leak(point.clone().into_boxed_str())),
                            false,
                            0,
                        )
                        .on_click(cx.listener(move |p, _, _, cx| {
                            let i = INSERT_POINTS
                                .iter()
                                .position(|x| *x == point)
                                .map_or(1, |i| (i + 1) % INSERT_POINTS.len());
                            p.apply(
                                "Insert point",
                                vec![EditOp::set(iid, &["point"], Value::text(INSERT_POINTS[i]))],
                                cx,
                            );
                        })),
                    )
                    .child(
                        button(SharedString::from(format!("by{}", iid.number)), tr("Bypass"), bypass, THEME.orange)
                            .on_click(cx.listener(move |p, _, _, cx| p.toggle(iid, &["bypass"], bypass, cx))),
                    )
            })))
            .child(Self::section("Sends", None, cx).children(ch.sends.iter().map(|s| {
                let to = all.iter().find(|c| c.id == s.to).map_or_else(|| "?".to_string(), |c| c.name.to_string());
                let (sid, pre) = (s.id, s.pre);
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(self.knob(
                        SharedString::from(format!("isnd-{}", sid.number)),
                        Target::Send(sid),
                        id,
                        22.0,
                        cx,
                    ))
                    .child(div().flex_1().text_size(px(11.)).child(to))
                    .child(div().text_size(px(10.)).child(view::format_gain(s.amount)))
                    .child(
                        button(
                            SharedString::from(format!("pre{}", sid.number)),
                            if pre { tr("PRE") } else { tr("POST") },
                            pre,
                            THEME.yellow,
                        )
                        .on_click(cx.listener(move |p, _, _, cx| p.toggle(sid, &["pre"], pre, cx))),
                    )
            })))
    }

    fn master_section(&self, ch: &ChannelView, cx: &Context<Self>) -> impl IntoElement {
        let level = self.core.read(cx).meter(ch.id);
        let bus_on = model::at(&ch.rack, &["dynamics", "on"]).and_then(Value::as_bool).unwrap_or(false);
        let gr = if bus_on { (level * 6.0).min(10.0) } else { 0.0 };
        let m = self.monitor;
        let id = ch.id;
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(MASTER_RACK.iter().map(|(sec, label, params)| {
                // a section without a switch in the document (the stereo section) has none here
                let toggle = model::at(&ch.rack, &[sec, "on"]).and_then(Value::as_bool).map(|on| (on, id, *sec));
                Self::section(label, toggle, cx)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .children(params.iter().map(|spec| self.param(ch, sec, *spec, cx))),
                    )
                    .when(*sec == "dynamics", |d| {
                        d.child(
                            div()
                                .text_size(px(10.))
                                .text_color(c(THEME.text2))
                                .child(trf("Gain reduction {0} dB", &[&format!("{gr:.1}")])),
                        )
                    })
            }))
            .child(
                Self::section("Meters", None, cx)
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(vu_needle(level))
                            .child(vu_needle((level * 0.95).min(1.0)))
                            .child(goniometer(level)),
                    )
                    .child(correlation(0.6 + 0.3 * level)),
            )
            .child(
                Self::section("Monitoring", None, cx)
                    .child(div().text_size(px(9.5)).text_color(c(THEME.text3)).child(tr("Not saved, not exported")))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(4.))
                            .child(button("dim", tr("DIM"), m.dim, THEME.yellow).on_click(cx.listener(
                                |p, _, _, cx| {
                                    p.monitor.dim = !p.monitor.dim;
                                    cx.notify();
                                },
                            )))
                            .child(button("cut", tr("CUT"), m.cut, THEME.red).on_click(cx.listener(|p, _, _, cx| {
                                p.monitor.cut = !p.monitor.cut;
                                cx.notify();
                            })))
                            .child(button("mono", tr("MONO"), m.mono, THEME.blue).on_click(cx.listener(
                                |p, _, _, cx| {
                                    p.monitor.mono = !p.monitor.mono;
                                    p.monitor.side = false;
                                    cx.notify();
                                },
                            )))
                            .child(button("side", tr("SIDE"), m.side, THEME.blue).on_click(cx.listener(
                                |p, _, _, cx| {
                                    p.monitor.side = !p.monitor.side;
                                    p.monitor.mono = false;
                                    cx.notify();
                                },
                            ))),
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
            for k in 0..=10 {
                let a = (-50.0 + k as f32 * 10.0).to_radians();
                let col = if k >= 8 { THEME.red } else { THEME.text3 };
                window.paint_quad(fill(
                    Bounds::new(point(base.x + px(a.sin() * r), base.y - px(a.cos() * r)), size(px(2.0), px(2.0))),
                    c(col),
                ));
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
            let t0 =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f32());
            let mut pb = PathBuilder::stroke(px(1.0));
            for k in 0..160 {
                let t = t0 * 3.0 + k as f32 * 0.21;
                let l = (t * 1.7).sin() * level;
                let rr = (t * 1.7 + 0.35).sin() * level * 0.9;
                let p = point(
                    center.x + px((l - rr) * std::f32::consts::FRAC_1_SQRT_2 * r),
                    center.y - px((l + rr) * std::f32::consts::FRAC_1_SQRT_2 * r),
                );
                if k == 0 { pb.move_to(p) } else { pb.line_to(p) }
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
            window.paint_quad(fill(
                Bounds::new(point(x - px(2.0), b.origin.y), size(px(4.0), b.size.height)),
                c(if v < 0.0 { THEME.red } else { THEME.green }),
            ));
        },
    )
    .w_full()
    .h(px(10.))
}

impl Render for MixingPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let chs = channels(self.core.read(cx));
        if self.selected.is_none_or(|s| !chs.iter().any(|c| c.id == s)) {
            self.selected = chs.first().map(|c| c.id);
        }
        let returns: Vec<(NodeId, String)> =
            chs.iter().filter(|c| c.kind == ChannelKind::Return).map(|c| (c.id, c.name.to_string())).collect();
        let strips: Vec<(usize, &ChannelView)> = chs
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind != ChannelKind::Master && c.kind != ChannelKind::Return)
            .collect();
        let pinned: Vec<(usize, &ChannelView)> = chs
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind == ChannelKind::Master || c.kind == ChannelKind::Return)
            .collect();
        let visible_w = self.strips_bounds.get().size.width.as_f32();
        if visible_w > 0.0 {
            self.scroll = self.scroll.clamp(0.0, ((strips.len() + 1) as f32 * STRIP_W - visible_w).max(0.0));
        }
        let first = (self.scroll / STRIP_W).floor() as usize;
        // a strip is built only when visible (an upper bound of 24 strips on any screen width here)
        let visible: Vec<(usize, &ChannelView)> = strips.iter().copied().skip(first).take(24).collect();
        let show_ghost = first + visible.len() >= strips.len();
        let offset = -(self.scroll - first as f32 * STRIP_W);
        let bounds_cell = self.strips_bounds.clone();
        // the menu stays inside the window
        let menu = self
            .menu
            .clone()
            .map(|(t, p)| (t, point(p.x.min(viewport.width - px(250.)), p.y.min(viewport.height - px(160.)))));
        let picker = self.output_picker;
        let strip_els: Vec<_> =
            visible.iter().map(|(i, ch)| self.strip(ch, *i, &returns, &chs, cx).into_any_element()).collect();
        let pinned_els: Vec<_> =
            pinned.iter().map(|(i, ch)| self.strip(ch, *i, &returns, &chs, cx).into_any_element()).collect();
        div()
            .id("mixing")
            .size_full()
            .relative()
            .flex()
            .flex_row()
            .bg(c(THEME.bg1))
            .key_context("Mixing")
            .track_focus(&self.focus)
            .on_action(cx.listener(|p, _: &SelectLeft, _, cx| p.move_selection(-1, cx)))
            .on_action(cx.listener(|p, _: &SelectRight, _, cx| p.move_selection(1, cx)))
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
                            .children(strip_els)
                            .when(show_ghost, |d| d.child(self.ghost(cx))),
                    ),
            )
            .child(div().h_full().flex().flex_row().border_l_2().border_color(c(THEME.line)).children(pinned_els))
            .child(self.inspector(cx))
            .children(menu.map(|(t, pos)| self.context_menu(t, pos, cx)))
            .children(picker.map(|id| self.output_list(id, &chs, cx)))
    }
}

impl MixingPage {
    /// The knob's and fader's menu (F-CTL-006)
    fn context_menu(&self, t: Target, pos: Point<Pixels>, cx: &Context<Self>) -> impl IntoElement {
        let item = |id: &'static str, label: String| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .px(px(10.))
                .py(px(4.))
                .text_size(px(12.))
                .cursor_pointer()
                .hover(|d| d.bg(c(THEME.bg4)))
                .child(label)
        };
        let title = self.target_name(&t, cx);
        let paste = self.clipboard;
        let (t1, t2, t3, t4) = (t.clone(), t.clone(), t.clone(), t);
        div()
            .absolute()
            .left(pos.x)
            .top(pos.y - px(crate::workspace::CHROME_H))
            .w(px(240.))
            .occlude()
            .py(px(4.))
            .rounded(px(6.))
            .bg(c(THEME.bg3))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(div().px(px(10.)).py(px(4.)).text_size(px(10.)).text_color(c(THEME.text3)).child(title))
            .child(item("menu-default", tr("Default value").into()).on_click(cx.listener(move |p, _, _, cx| {
                p.menu = None;
                p.set(&t1, Self::default_travel(&t1), "Default", cx);
            })))
            .child(item("menu-copy", tr("Copy value").into()).on_click(cx.listener(move |p, _, _, cx| {
                p.clipboard = Some(p.travel_of(&t2, cx));
                p.menu = None;
                cx.notify();
            })))
            .child(
                item(
                    "menu-paste",
                    if paste.is_some() { tr("Paste value").into() } else { format!("{} —", tr("Paste value")) },
                )
                .on_click(cx.listener(move |p, _, _, cx| {
                    p.menu = None;
                    match p.clipboard {
                        Some(v) => p.set(&t3, v, "Paste value", cx),
                        None => cx.notify(),
                    }
                })),
            )
            .child(item("menu-learn", tr("MIDI learn").into()).on_click(cx.listener(move |p, _, _, cx| {
                p.learning = Some(t4.clone());
                p.menu = None;
                cx.notify();
            })))
    }

    /// Where a channel's output can go: every channel that would not make a loop (the document refuses loops; the list
    /// does not even offer them), and the master
    fn output_list(&self, id: NodeId, chs: &[ChannelView], cx: &Context<Self>) -> impl IntoElement {
        let leads_to = |mut k: NodeId| {
            for _ in 0..=chs.len() {
                if k == id {
                    return true;
                }
                match chs.iter().find(|c| c.id == k).and_then(|c| c.output) {
                    Some(n) => k = n,
                    None => return false,
                }
            }
            true
        };
        let options: Vec<(NodeId, String)> =
            chs.iter().filter(|c| c.id != id && !leads_to(c.id)).map(|c| (c.id, c.name.to_string())).collect();
        let name = chs.iter().find(|c| c.id == id).map_or_else(String::new, |c| c.name.to_string());
        div()
            .id("outputs")
            .absolute()
            .left(px(8.))
            .bottom(px(30.))
            .w(px(220.))
            .occlude()
            .max_h(px(360.))
            .overflow_y_scroll()
            .py(px(4.))
            .rounded(px(6.))
            .bg(c(THEME.bg3))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(
                div()
                    .px(px(10.))
                    .py(px(4.))
                    .text_size(px(10.))
                    .text_color(c(THEME.text3))
                    .child(trf("Output of {0}", &[&name])),
            )
            .children(options.into_iter().map(|(to, name)| {
                div()
                    .id(SharedString::from(format!("opt-{}", to.number)))
                    .debug_selector(move || format!("opt-{}", to.number))
                    .px(px(10.))
                    .py(px(3.))
                    .text_size(px(11.))
                    .cursor_pointer()
                    .hover(|d| d.bg(c(THEME.bg4)))
                    .child(name)
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.output_picker = None;
                        p.apply("Output", vec![EditOp::set(id, &["output"], Value::text(to.to_string()))], cx);
                    }))
            }))
    }
}
