//! The mixing page: the legacy desk (legado/src/gui: MixerView.cpp, MixerChannelView.cpp, MasterSectionView.cpp),
//! reproduced faithfully in both of its modes (user's decision of 20/11, decisoes-autonomas.md "A mesa de mixagem").
//!
//! - The channels in a row, then the ghost channel that adds one; the effect returns at the right, next to the master
//!   with its master section; on the far right the ANALOG / DIGITAL switch, the selected channel's settings and its
//!   effects rack.
//! - A strip, from the top: the send button or the receive arrow, the knob of the selected channel's send to it, the
//!   channel's number; in analog mode the strip of an analog desk (INPUT, FILTER, DYN, EQ), in digital mode the effects
//!   by name; the sends to the effect returns; the pan; the fader over its meters with the peak above it, and beside it
//!   M, S, Ø, ⇄ and, in analog mode, the effects as LEDs; the name.
//! - The digital desk is the plain strip: no analog controls, the effects by name. Switching keeps the strip's
//!   settings for when the user switches back (the document keeps the rack; the compiler leaves it out in digital).
//!
//! What differs from the legacy, and why, is written in decisoes-autonomas.md ("A mesa nova e a do legado").
//!
//! Knobs and faders: drag = one gesture (Shift = fine); double click or the middle button = default; the right
//! button opens the menu. Keys (registry, context "Mixing"): ←/→ select, Alt+←/→ move the channel, ↑/↓ the fader
//! 1 dB (Shift 3, Alt 0.1), Delete removes the channel, Enter or F2 renames it, Shift+Insert adds one.

use std::collections::{BTreeSet, HashMap, HashSet};

use gpui::{
    AnyElement, Bounds, ClickEvent, Context, Div, Entity, FocusHandle, FontWeight, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ScrollEvent, SharedString, Stateful, Subscription, WeakEntity,
    Window, canvas, div, prelude::*, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use lm_gui_plugins::Gesture;
use lm_port::{EditOp, Label, NodeId, PathStep, ROOT, Ref, Value};

use super::desk::{self, Light, Role, Slot, col, rgb};
use crate::actions::{
    AddChannel, FaderDown, FaderDownCoarse, FaderDownFine, FaderUp, FaderUpCoarse, FaderUpFine, MoveChannelLeft,
    MoveChannelRight, RemoveChannel, RenameChannel, SelectLeft, SelectRight,
};
use crate::core::Core;
use crate::keymap;
use crate::model::{self, ChannelKind, ChannelView};
use crate::screens::Screen;
use crate::ui::{THEME, c, midi, tr, trf};
use crate::view;
use crate::workspace::Workspace;

// --- the controls, as the legacy lays them out -----------------------------------------------------------------

/// How a control's travel maps to its value
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scale {
    Lin(f64, f64),
    /// frequencies
    Log(f64, f64),
    /// a fader or a send: linear gain, 0 dB at 75 % of the travel
    Gain,
    Pan,
    /// a stepped knob over a choice's keys
    Choice(&'static [&'static str]),
    Int(i64, i64),
}

/// One knob of a strip: the section's key in `rack`, or a key of the channel itself (`sec` empty)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knob {
    pub sec: &'static str,
    pub key: &'static str,
    pub label: &'static str,
    pub name: &'static str,
    pub cap: u32,
    pub scale: Scale,
    pub default: f64,
    pub unit: &'static str,
}

#[allow(clippy::too_many_arguments)]
const CTX: &str = "Mixing";

/// The mouse actions of the desk (`keymap/mouse_defs.rs`): the gestures are the user's, never read from the event
fn holds(id: &str, m: &gpui::Modifiers) -> bool {
    keymap::holds(CTX, id, m)
}

fn fires(id: &str, button: MouseButton, clicks: usize, m: &gpui::Modifiers) -> bool {
    keymap::fires(CTX, id, button, clicks, m)
}

const fn knob(
    sec: &'static str,
    key: &'static str,
    label: &'static str,
    name: &'static str,
    cap: u32,
    scale: Scale,
    default: f64,
    unit: &'static str,
) -> Knob {
    Knob { sec, key, label, name, cap, scale, default, unit }
}

pub const GAIN_MODELS: &[&str] =
    &["flat", "4558", "tl072", "5532", "5534", "12ax7", "12ax7-vintage", "12ax7-hot", "12at7", "12au7"];
pub const COMPRESSOR_MODELS: &[&str] =
    &["ssl", "api2500", "dbx160", "urei1176", "la2a", "fairchild670", "gates-sta-level", "altec436", "rca-ba6a"];
pub const COMPRESSOR_SPEEDS: &[&str] = &["fast", "norm", "slow"];
pub const GATE_MODES: &[&str] = &["std", "kick", "snare", "toms", "vox", "guitar", "bass", "noise", "expander"];
pub const BUS_RATIOS: &[&str] = &["2", "4", "10"];
pub const INSERT_POINTS: &[&str] = &["pre-gain", "post-gain", "post-filter", "post-dynamics", "post-eq", "pre-pan"];

use desk::col::{BLUE, BROWN, BUS, GREEN, GREY, ORANGE, RED, TEAL, YELLOW};

pub const GAIN: Knob = knob("tube", "drive", "GAIN", "Gain", RED, Scale::Lin(0.0, 1.0), 0.25, "gain");
pub const TRIM: Knob = knob("", "trim", "TRIM", "Input trim", GREY, Scale::Lin(-60.0, 24.0), 0.0, "dB");
pub const HPF: Knob = knob("eq", "highPass", "HPF", "High-pass", GREY, Scale::Log(20.0, 20000.0), 20.0, "Hz");
pub const LPF: Knob = knob("eq", "lowPass", "LPF", "Low-pass", GREY, Scale::Log(20.0, 20000.0), 20000.0, "Hz");
pub const BASS: Knob = knob("eq", "monoBass", "BASS", "Mono bass", GREY, Scale::Lin(0.0, 100.0), 0.0, "%");
pub const BASS_FREQ: Knob =
    knob("eq", "monoBassFrequency", "FREQ", "Mono bass frequency", GREY, Scale::Log(20.0, 500.0), 120.0, "Hz");
pub const COMP: Knob = knob("dynamics", "compAmount", "COMP", "Compressor", YELLOW, Scale::Lin(0.0, 1.0), 0.0, "");
pub const RATIO: Knob =
    knob("dynamics", "compRatio", "RATIO", "Compressor ratio", YELLOW, Scale::Lin(0.0, 1.0), 0.0, "");
pub const ATK: Knob =
    knob("dynamics", "compSpeed", "ATK", "Compressor attack", YELLOW, Scale::Choice(COMPRESSOR_SPEEDS), 1.0, "");
pub const REL: Knob =
    knob("dynamics", "compRelease", "REL", "Compressor release", YELLOW, Scale::Lin(0.0, 1.0), 0.5, "");
pub const GATE: Knob =
    knob("dynamics", "gateThreshold", "GATE", "Gate threshold", TEAL, Scale::Lin(-60.0, 20.0), -10.0, "dBu");
pub const RANGE: Knob = knob("dynamics", "gateRange", "RANGE", "Gate range", TEAL, Scale::Lin(0.0, 80.0), 0.0, "dB");
pub const HI: Knob = knob("eq", "high", "HI", "EQ high (12 kHz)", ORANGE, Scale::Lin(-15.0, 15.0), 0.0, "dB");
pub const HMF: Knob = knob("eq", "highMid", "HMF", "EQ high mid (2.5 kHz)", GREEN, Scale::Lin(-15.0, 15.0), 0.0, "dB");
pub const LMF: Knob = knob("eq", "lowMid", "LMF", "EQ low mid (400 Hz)", BLUE, Scale::Lin(-15.0, 15.0), 0.0, "dB");
pub const LO: Knob = knob("eq", "low", "LO", "EQ low (80 Hz)", BROWN, Scale::Lin(-15.0, 15.0), 0.0, "dB");
pub const PAN: Knob = knob("", "pan", "PAN", "Panning", desk::col::PAN, Scale::Pan, 0.0, "%");
/// An insert's wet/dry mix (EffectView's W/D): absent is all wet
pub const WET: Knob = knob("", "wet", "W/D", "Wet/dry", desk::col::GREY, Scale::Lin(0.0, 1.0), 1.0, "");

pub const WIDTH: Knob = knob("stereo", "width", "WIDTH", "Stereo width", GREY, Scale::Lin(0.0, 200.0), 100.0, "%");
pub const M_BASS: Knob = knob("stereo", "monoBass", "BASS", "Mono bass", GREY, Scale::Lin(0.0, 100.0), 0.0, "%");
pub const M_FREQ: Knob =
    knob("stereo", "bassFrequency", "FREQ", "Mono bass frequency", GREY, Scale::Log(20.0, 500.0), 120.0, "Hz");
pub const AIR: Knob = knob("stereo", "air", "AIR", "Air", GREY, Scale::Lin(-12.0, 12.0), 0.0, "dB");
pub const THR: Knob = knob("dynamics", "threshold", "THR", "Threshold", BUS, Scale::Lin(-15.0, 15.0), 5.0, "dB");
pub const B_RATIO: Knob = knob("dynamics", "ratio", "RATIO", "Ratio", BUS, Scale::Choice(BUS_RATIOS), 1.0, ":1");
pub const MAKE: Knob = knob("dynamics", "makeup", "MAKE", "Make-up gain", BUS, Scale::Lin(-5.0, 15.0), 0.0, "dB");
pub const B_ATK: Knob = knob("dynamics", "attack", "ATK", "Attack", BUS, Scale::Int(0, 5), 4.0, "");
pub const B_REL: Knob = knob("dynamics", "release", "REL", "Release", BUS, Scale::Int(0, 4), 4.0, "");

/// Every knob of a channel's analog strip, in the legacy's order (the tests check each key is in the document)
#[cfg(all(test, debug_assertions))]
pub const STRIP_KNOBS: &[Knob] =
    &[GAIN, TRIM, HPF, LPF, BASS, BASS_FREQ, COMP, RATIO, ATK, REL, GATE, RANGE, HI, HMF, LMF, LO, PAN];
/// Every knob of the master section
#[cfg(all(test, debug_assertions))]
pub const MASTER_KNOBS: &[Knob] = &[WIDTH, M_BASS, M_FREQ, AIR, THR, B_RATIO, MAKE, B_ATK, B_REL];

/// The bus compressor's steps (MasterSection::AttackTimes, ReleaseTimes)
const ATTACK_MS: [f64; 6] = [0.1, 0.3, 1.0, 3.0, 10.0, 30.0];
const RELEASE_S: [f64; 4] = [0.1, 0.3, 0.6, 1.2];

/// The heights of the strip's upper area, as tall in every strip so the faders line up (MixerView::alignStrips): the
/// tallest of a channel's and the master section's, per mode
const ANALOG_SECTION_H: f32 = 317.0;
const UPPER_ANALOG: f32 = 413.0;
const UPPER_DIGITAL: f32 = 368.0;
const NAME_H: f32 = 15.0;
const SLOT_H: f32 = 11.0;

// --- what a gesture moves ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Target {
    /// a knob of a node: the channel's rack (`sec`) or the channel itself
    Knob(NodeId, Knob),
    /// a fader: the channel's volume, or one of the master's PRE faders (`preLeft`, `preRight`)
    Fader(NodeId, &'static str),
    /// a send's amount
    Send(NodeId),
    /// the monitor's level (preferences, not the document)
    Monitor,
}

struct Drag {
    target: Target,
    start_y: f32,
    start_t: f64,
    travel: f32,
    gesture: Option<Gesture>,
    /// the linked faders and the other PRE fader, with their starting travels
    linked: Vec<(Target, f64)>,
}

enum Menu {
    Knob(Target),
    /// a switch's menu: only the MIDI items (its node and field path)
    Switch(NodeId, &'static [&'static str]),
    Strip(NodeId),
    Choice {
        id: NodeId,
        sec: &'static str,
        key: &'static str,
        choices: &'static [&'static str],
    },
    Effect(NodeId, NodeId),
}

/// What a mapping to this control points at (I-21.2): a parameter for the continuous controls, a field for the
/// stepped ones; the monitoring is the machine's and is not mapped here
fn map_target(t: &Target) -> Option<lm_port::MapTarget> {
    Some(match t {
        Target::Knob(id, k) if matches!(k.scale, Scale::Choice(_)) => midi::field(*id, &path_of(k)),
        // the insert's mix is the parameter "insert.wet" (lm-doc's param key)
        Target::Knob(id, k) if k.key == "wet" => midi::param(*id, "insert.wet"),
        Target::Knob(id, k) => midi::param(*id, &path_of(k).join(".")),
        Target::Fader(id, key) => midi::param(*id, key),
        Target::Send(sid) => midi::param(*sid, "amount"),
        Target::Monitor => return None,
    })
}

fn path_of(k: &Knob) -> Vec<&'static str> {
    if k.sec.is_empty() { vec![k.key] } else { vec!["rack", k.sec, k.key] }
}

fn to_travel(scale: Scale, v: &Value, default: f64) -> f64 {
    let num = v.as_real().or_else(|| v.as_int().map(|i| i as f64));
    let t = match scale {
        Scale::Lin(a, b) => (num.unwrap_or(default) - a) / (b - a),
        Scale::Log(a, b) => (num.unwrap_or(default).max(a) / a).ln() / (b / a).ln(),
        Scale::Gain => view::gain_to_travel(num.unwrap_or(default)),
        Scale::Pan => (num.unwrap_or(default) + 1.0) / 2.0,
        Scale::Choice(ch) => {
            let i = v.as_text().and_then(|t| ch.iter().position(|c| *c == t)).map_or(default, |i| i as f64);
            i / (ch.len() as f64 - 1.0)
        }
        Scale::Int(a, b) => (num.unwrap_or(default) - a as f64) / (b - a) as f64,
    };
    t.clamp(0.0, 1.0)
}

fn from_travel(scale: Scale, t: f64) -> Value {
    let t = t.clamp(0.0, 1.0);
    let real = |v: f64| Value::real(v).unwrap_or_default();
    match scale {
        Scale::Lin(a, b) => real(a + t * (b - a)),
        Scale::Log(a, b) => real(a * (b / a).powf(t)),
        Scale::Gain => real(view::travel_to_gain(t)),
        Scale::Pan => real(t * 2.0 - 1.0),
        Scale::Choice(ch) => Value::text(ch[(t * (ch.len() as f64 - 1.0)).round() as usize]),
        Scale::Int(a, b) => Value::int(a + (t * (b - a) as f64).round() as i64),
    }
}

fn default_travel(scale: Scale, default: f64) -> f64 {
    match scale {
        Scale::Choice(ch) => default / (ch.len() as f64 - 1.0),
        _ => to_travel(scale, &Value::real(default).unwrap_or_default(), default),
    }
}

/// A value as the legacy's hint shows it, in its unit
fn value_text(k: &Knob, v: &Value) -> String {
    let n = v.as_real().or_else(|| v.as_int().map(|i| i as f64)).unwrap_or(k.default);
    match (k.scale, k.unit) {
        (Scale::Choice(_), unit) => format!("{}{unit}", v.as_text().unwrap_or("")),
        (_, "gain") => view::format_db(if n <= 0.0 { f64::NEG_INFINITY } else { 20.0 * (n / 0.25).log10() }),
        (Scale::Pan, _) => format!("{:.0}%", n * 100.0),
        (Scale::Int(..), _) if k.label == "ATK" => format!("{} ms", ATTACK_MS[(n as usize).min(5)]),
        (Scale::Int(..), _) if k.label == "REL" => {
            if n >= 4.0 {
                tr("Auto").to_string()
            } else {
                format!("{} s", RELEASE_S[n as usize])
            }
        }
        (_, "Hz") if n >= 1000.0 => format!("{:.1} kHz", n / 1000.0),
        (_, "") => format!("{n:.2}"),
        (_, u) => format!("{n:.1} {u}"),
    }
}

// --- the page ---------------------------------------------------------------------------------------------------

/// A channel's live readings from the engine (`CorePort::display`, `trees::rack_display`), this frame
#[derive(Clone, Debug, Default)]
struct Reading {
    /// the gain stage's saturation, 0..1
    sat: f32,
    /// the CLIP LED (lit for a moment after a block clipped)
    clip: bool,
    /// the master's bus compressor gain reduction, dB
    bus_gr: f32,
    correlation: f32,
    /// (side, mid) points of the master's output
    gonio: Vec<(f32, f32)>,
    /// the master's loudness (BS.1770): momentary, short-term, integrated LUFS, true peak dBTP, range LU; NaN
    /// before anything passed
    loudness: [f32; 5],
}

/// How long the CLIP LED stays lit after a clipped block (SaturationLeds::ClipHold: 10 GUI updates)
const CLIP_HOLD: std::time::Duration = std::time::Duration::from_millis(400);

pub struct MixingPage {
    core: Entity<Core>,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    /// horizontal scroll of the channel area, in pixels
    pub scroll: f32,
    pub selected: Option<NodeId>,
    multi: BTreeSet<NodeId>,
    /// linked faders (P-18): the interface's state
    linked: BTreeSet<NodeId>,
    /// group channels whose members are hidden (F-MIX-011): the interface's state, as in the legacy
    collapsed: HashSet<NodeId>,
    drag: Option<Drag>,
    menu: Option<(Menu, Point<Pixels>)>,
    clipboard: Option<f64>,
    /// the highest peak since the last click, per channel, in dB (PeakIndicator)
    peaks: HashMap<NodeId, f32>,
    renaming: Option<(NodeId, Entity<InputState>, Subscription)>,
    /// A channel others use, waiting for "remove it?" (MixerChannelView::confirmRemoval), and "don't ask again"
    confirm_remove: Option<(NodeId, bool)>,
    /// the hint of the value being dragged (SimpleTextFloat)
    hint: Option<(String, Point<Pixels>)>,
    area: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    /// The engine's readings this frame, and each channel's clip count with when its LED goes off
    readings: HashMap<NodeId, Reading>,
    clips: HashMap<NodeId, (f32, std::time::Instant)>,
    /// the page's bounds in the window: the menus and the hint are placed in its coordinates
    page: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    fader_h: f32,
    /// the rows of send knobs, two returns to a row: the upper areas grow by the extra rows
    send_rows: usize,
}

fn channels(core: &Core) -> Vec<ChannelView> {
    model::channels(core.port().root())
}

fn is_analog(core: &Core) -> bool {
    model::song(core.port().root()).analog
}

impl MixingPage {
    pub fn new(core: Entity<Core>, workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        // the master is selected at the start, as in the legacy
        let master = channels(core.read(cx)).into_iter().find(|c| c.kind == ChannelKind::Master).map(|c| c.id);
        // the linked faders (P-18) are the interface's state: kept in the interface document (I-3.8)
        let linked: BTreeSet<NodeId> = core
            .read(cx)
            .port()
            .ui()
            .get("desk")
            .and_then(|d| d.get("linked"))
            .map(|l| l.items().filter_map(|v| v.as_text().and_then(|t| NodeId::parse(t).ok())).collect())
            .unwrap_or_default();
        MixingPage {
            core,
            workspace,
            focus: cx.focus_handle(),
            scroll: 0.0,
            selected: master,
            multi: BTreeSet::new(),
            linked,
            collapsed: HashSet::new(),
            drag: None,
            menu: None,
            clipboard: None,
            peaks: HashMap::new(),
            renaming: None,
            confirm_remove: None,
            hint: None,
            area: Default::default(),
            readings: HashMap::new(),
            clips: HashMap::new(),
            page: Default::default(),
            fader_h: 300.0,
            send_rows: 1,
        }
    }

    fn channel(&self, id: NodeId, cx: &Context<Self>) -> Option<ChannelView> {
        channels(self.core.read(cx)).into_iter().find(|c| c.id == id)
    }

    /// The strips in the order they are shown: the channels (minus the hidden members of collapsed groups), the
    /// effect returns, the master
    fn order(&self, chs: &[ChannelView]) -> Vec<NodeId> {
        let hidden = self.hidden(chs);
        let normal = chs
            .iter()
            .filter(|c| matches!(c.kind, ChannelKind::Normal | ChannelKind::Group) && !hidden.contains(&c.id));
        let returns = chs.iter().filter(|c| c.kind == ChannelKind::Return);
        let master = chs.iter().filter(|c| c.kind == ChannelKind::Master);
        normal.chain(returns).chain(master).map(|c| c.id).collect()
    }

    /// The channels hidden by collapsed groups: whose output goes to a collapsed group, or to a hidden one
    fn hidden(&self, chs: &[ChannelView]) -> HashSet<NodeId> {
        let mut hidden = HashSet::new();
        for c in chs {
            let mut at = c.output;
            for _ in 0..=chs.len() {
                let Some(o) = at else { break };
                if self.collapsed.contains(&o) {
                    hidden.insert(c.id);
                    break;
                }
                at = chs.iter().find(|x| x.id == o).and_then(|x| x.output);
            }
        }
        hidden
    }

    // --- values and edits ----------------------------------------------------------------------------------

    fn value_of(&self, t: &Target, cx: &Context<Self>) -> Value {
        let port = self.core.read(cx).port();
        match t {
            Target::Knob(id, k) => port.find(*id).and_then(|n| model::at(&n, &path_of(k)).cloned()).unwrap_or_default(),
            Target::Fader(id, key) => port.find(*id).and_then(|n| n.get(key).cloned()).unwrap_or_default(),
            Target::Send(sid) => port.find(*sid).and_then(|n| n.get("amount").cloned()).unwrap_or_default(),
            Target::Monitor => model::at(port.preferences(), &["monitor", "level"]).cloned().unwrap_or_default(),
        }
    }

    fn scale_of(t: &Target) -> (Scale, f64) {
        match t {
            Target::Knob(_, k) => (k.scale, k.default),
            Target::Fader(..) | Target::Send(_) => (Scale::Gain, 1.0),
            Target::Monitor => (Scale::Lin(-60.0, 12.0), 0.0),
        }
    }

    fn travel_of(&self, t: &Target, cx: &Context<Self>) -> f64 {
        let (scale, default) = Self::scale_of(t);
        to_travel(scale, &self.value_of(t, cx), default)
    }

    fn op_of(t: &Target, travel: f64) -> Option<EditOp> {
        let (scale, _) = Self::scale_of(t);
        let value = from_travel(scale, travel);
        Some(match t {
            Target::Knob(id, k) => EditOp::set(*id, &path_of(k), value),
            Target::Fader(id, key) => EditOp::set(*id, &[key], value),
            Target::Send(sid) => EditOp::set(*sid, &["amount"], value),
            Target::Monitor => return None,
        })
    }

    fn apply(&mut self, label: &'static str, ops: Vec<EditOp>, cx: &mut Context<Self>) {
        self.core.update(cx, |core, _| {
            core.report(|pt| pt.apply(Label::new(label), ops));
        });
        cx.notify();
    }

    fn set_travel(&mut self, t: &Target, travel: f64, label: &'static str, cx: &mut Context<Self>) {
        match Self::op_of(t, travel) {
            Some(op) => self.apply(label, vec![op], cx),
            None => self.set_monitor(&["level"], from_travel(Scale::Lin(-60.0, 12.0), travel), cx),
        }
    }

    fn set_monitor(&mut self, key: &[&str], v: Value, cx: &mut Context<Self>) {
        let mut path = vec![PathStep::key("monitor")];
        path.extend(key.iter().map(|k| PathStep::key(k)));
        self.core.update(cx, |core, _| {
            core.report(|pt| pt.set_preference(path, v));
        });
        cx.notify();
    }

    fn toggle(&mut self, id: NodeId, path: &[&str], now: bool, cx: &mut Context<Self>) {
        self.apply("Toggle", vec![EditOp::set(id, path, Value::bool(!now))], cx);
    }

    fn set_choice(&mut self, id: NodeId, sec: &str, key: &str, v: &str, cx: &mut Context<Self>) {
        self.apply("Choice", vec![EditOp::set(id, &["rack", sec, key], Value::text(v))], cx);
    }

    /// A desk switch's click: the next choice (ChoiceButton, DriveButton)
    fn next_choice(
        &mut self,
        id: NodeId,
        sec: &str,
        key: &str,
        choices: &'static [&'static str],
        now: &str,
        cx: &mut Context<Self>,
    ) {
        let i = choices.iter().position(|c| *c == now).map_or(0, |i| (i + 1) % choices.len());
        self.set_choice(id, sec, key, choices[i], cx);
    }

    // --- gestures ------------------------------------------------------------------------------------------

    fn press(
        &mut self,
        t: Target,
        travel: f32,
        channel: NodeId,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.select(channel, false, cx);
        cx.stop_propagation();
        let (scale, default) = Self::scale_of(&t);
        match ev.button {
            MouseButton::Right => {
                self.menu = Some((Menu::Knob(t), ev.position));
                return cx.notify();
            }
            MouseButton::Middle => return self.set_travel(&t, default_travel(scale, default), "Default", cx),
            _ => {}
        }
        if fires("mix.knob.reset", ev.button, ev.click_count, &ev.modifiers) {
            return self.set_travel(&t, default_travel(scale, default), "Default", cx);
        }
        let start_t = self.travel_of(&t, cx);
        let mut linked = Vec::new();
        match &t {
            Target::Fader(id, "volume") if self.linked.contains(id) => {
                for c in channels(self.core.read(cx)) {
                    if c.id != *id && self.linked.contains(&c.id) {
                        linked.push((Target::Fader(c.id, "volume"), view::gain_to_travel(c.volume)));
                    }
                }
            }
            Target::Fader(id, side @ ("preLeft" | "preRight")) if self.channel(*id, cx).is_some_and(|c| c.pre.2) => {
                let other = if *side == "preLeft" { "preRight" } else { "preLeft" };
                let o = Target::Fader(*id, other);
                let ot = self.travel_of(&o, cx);
                linked.push((o, ot));
            }
            _ => {}
        }
        let prev = self.drag.take();
        let monitor = t == Target::Monitor;
        let gesture = self.core.update(cx, |core, _| {
            if let Some(g) = prev.and_then(|d| d.gesture) {
                core.gesture_commit(g);
            }
            (!monitor).then(|| core.begin_gesture(Label::new("Mix")))
        });
        self.drag = Some(Drag { target: t, start_y: ev.position.y.as_f32(), start_t, travel, gesture, linked });
        cx.notify();
    }

    fn drag_to(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = &self.drag else { return };
        let fine = if holds("mix.fader.fine", &ev.modifiers) { 10.0 } else { 1.0 };
        let dt = -((ev.position.y.as_f32() - d.start_y) / (d.travel * fine)) as f64;
        let t = (d.start_t + dt).clamp(0.0, 1.0);
        let target = d.target.clone();
        let mut ops: Vec<EditOp> = Self::op_of(&target, t).into_iter().collect();
        for (o, start) in &d.linked {
            ops.extend(Self::op_of(o, start + dt));
        }
        if let Target::Fader(id, "volume") = target {
            // the fader's value reaches the audio at once (I-6); the edit follows in the gesture
            let g = view::travel_to_gain(t) as f32;
            self.core.update(cx, |core, _| core.port_mut().param_write(id, "volume", g));
        }
        if d.gesture.is_some() {
            let drag = self.drag.as_ref();
            self.core.update(cx, |core, _| {
                if let Some(g) = drag.and_then(|d| d.gesture.as_ref()) {
                    core.gesture_apply(*g, ops);
                }
            });
        } else {
            self.set_monitor(&["level"], from_travel(Scale::Lin(-60.0, 12.0), t), cx);
        }
        // the hint: the value, beside the pointer
        let text = self.hint_text(&target, cx);
        self.hint = Some((text, ev.position));
        cx.notify();
    }

    fn hint_text(&self, t: &Target, cx: &Context<Self>) -> String {
        let v = self.value_of(t, cx);
        match t {
            Target::Knob(_, k) => format!("{}: {}", tr(k.name), value_text(k, &v)),
            Target::Fader(..) => format!("{} {}", tr("Volume:"), view::format_gain(v.as_real().unwrap_or(1.0))),
            Target::Send(_) => format!("{} {}", tr("Send:"), view::format_gain(v.as_real().unwrap_or(0.0))),
            Target::Monitor => format!("{} {:.1} dB", tr("Monitor level:"), v.as_real().unwrap_or(0.0)),
        }
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.drag.take() {
            if let Some(g) = d.gesture {
                self.core.update(cx, |core, _| core.gesture_commit(g));
            }
            self.hint = None;
            cx.notify();
        }
    }

    // --- selection, keys, channel commands -----------------------------------------------------------------

    fn select(&mut self, id: NodeId, ctrl: bool, cx: &mut Context<Self>) {
        if ctrl {
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
        self.menu = None;
        cx.notify();
    }

    /// Whether the focus is in this page (its keys work)
    pub fn has_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.focus.contains_focused(window, cx)
    }

    /// Shows a channel (the FX button of a track's header, P-28)
    pub fn select_channel(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let chs = channels(self.core.read(cx));
        if !chs.iter().any(|c| c.id == id) {
            return;
        }
        self.selected = Some(id);
        self.multi.clear();
        let order = self.order(&chs);
        let normal =
            chs.iter().filter(|c| order.contains(&c.id) && matches!(c.kind, ChannelKind::Normal | ChannelKind::Group));
        if let Some(n) = normal.map(|c| c.id).position(|c| c == id) {
            self.scroll = (n as f32 * desk::STRIP_W - 2.0 * desk::STRIP_W).max(0.0);
        }
        cx.notify();
    }

    fn link_selected(&mut self, cx: &mut Context<Self>) {
        let mut set = self.multi.clone();
        set.extend(self.selected);
        if set.len() > 1 && !set.iter().all(|id| self.linked.contains(id)) {
            self.linked.extend(set);
        } else {
            for id in set {
                self.linked.remove(&id);
            }
        }
        // kept in the interface document, so the link survives saving and opening the project
        let list = Value::list(self.linked.iter().map(|id| Value::text(id.to_string())));
        self.core.update(cx, |core, _| {
            core.report(|pt| pt.set_ui(vec![PathStep::key("desk")], Value::object([("linked", list)])));
        });
        cx.notify();
    }

    /// The fader and the wheel: the legacy's steps (Fader::determineAdjustmentDelta): 1 dB, Shift 3 dB, Ctrl (Cmd on
    /// macOS) 0.1 dB, for one channel
    fn nudge_channel(&mut self, id: NodeId, db: f64, cx: &mut Context<Self>) {
        // linked faders (P-18) move together, each by the same number of dB (one undo step)
        let mut ids = vec![id];
        if self.linked.contains(&id) {
            ids.extend(self.linked.iter().copied().filter(|c| *c != id));
        }
        let mut ops = Vec::new();
        for c in ids {
            let Some(ch) = self.channel(c, cx) else { continue };
            let v = (view::gain_to_db(ch.volume).max(-60.0) + db).clamp(-60.0, 12.0);
            ops.push(EditOp::set(c, &["volume"], Value::real(view::db_to_gain(v)).unwrap_or_default()));
        }
        if !ops.is_empty() {
            self.apply("Fader", ops, cx);
        }
    }

    fn wheel_fader(&mut self, id: NodeId, ev: &ScrollEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let dy = ev.delta.pixel_delta(px(16.0)).y.as_f32();
        if dy == 0.0 {
            return;
        }
        let ids = ["mix.fader.wheel_coarse", "mix.fader.wheel_fine", "mix.fader.wheel"];
        let step = match keymap::pick_wheel(CTX, &ids, &ev.modifiers) {
            Some("mix.fader.wheel_coarse") => 3.0,
            Some("mix.fader.wheel_fine") => 0.1,
            Some(_) => 1.0,
            None => return,
        };
        self.nudge_channel(id, if dy > 0.0 { step } else { -step }, cx);
    }

    /// The wheel on a knob (Knob::wheelEvent): a step of its travel each notch, Shift finer
    fn wheel_knob(&mut self, t: Target, ev: &ScrollEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let dy = ev.delta.pixel_delta(px(16.0)).y.as_f32();
        if dy == 0.0 {
            return;
        }
        let step = match keymap::pick_wheel(CTX, &["mix.knob.wheel_fine", "mix.knob.wheel"], &ev.modifiers) {
            Some("mix.knob.wheel_fine") => 0.001,
            Some(_) => 0.01,
            None => return,
        };
        let travel = self.travel_of(&t, cx) + if dy > 0.0 { step } else { -step };
        self.set_travel(&t, travel, "Knob", cx);
    }

    fn nudge(&mut self, db: f64, cx: &mut Context<Self>) {
        if let Some(id) = self.selected {
            self.nudge_channel(id, db, cx);
        }
    }

    fn move_selection(&mut self, by: isize, cx: &mut Context<Self>) {
        let chs = channels(self.core.read(cx));
        let order = self.order(&chs);
        if order.is_empty() {
            return;
        }
        let i = self.selected.and_then(|s| order.iter().position(|c| *c == s)).unwrap_or(0) as isize;
        let j = (i + by).clamp(0, order.len() as isize - 1) as usize;
        self.selected = order.get(j).copied();
        self.multi.clear();
        cx.notify();
    }

    /// Alt+←/→ and the menu's Move left/right: the channel's place in the document's list
    fn move_channel(&mut self, id: NodeId, by: isize, cx: &mut Context<Self>) {
        let list: Vec<NodeId> = model::at(self.core.read(cx).port().root(), &["mixer", "channels"])
            .map(|l| l.items().filter_map(model::id_of).collect())
            .unwrap_or_default();
        let Some(i) = list.iter().position(|c| *c == id) else { return };
        let j = i as isize + by;
        if j < 0 || j as usize >= list.len() {
            return;
        }
        let op =
            EditOp::Move { id: Ref::Id(id), parent: Ref::Id(ROOT), list: "mixer.channels".into(), index: j as usize };
        self.apply("Move channel", vec![op], cx);
    }

    /// A new channel after the last one (the ghost channel, Shift+Insert)
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
        let created = self.core.update(cx, |core, _| core.report(|pt| pt.apply(Label::new("Add channel"), vec![op])));
        if let Some(id) = created.and_then(|a| a.created.first().map(|(_, id)| *id)) {
            self.selected = Some(id);
            self.scroll = 1e9;
        }
        cx.notify();
    }

    /// Remove: a channel something feeds asks first, unless the user said not to ask again
    /// (MixerChannelView::confirmRemoval, the preference `interface.confirmChannelRemoval`)
    fn remove_channel(&mut self, id: NodeId, cx: &mut Context<Self>) {
        if !self.channel(id, cx).is_some_and(|c| c.kind != ChannelKind::Master) {
            return;
        }
        let ask = model::at(self.core.read(cx).port().preferences(), &["interface", "confirmChannelRemoval"])
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if ask && self.in_use(cx).contains(&id) {
            self.confirm_remove = Some((id, false));
            cx.notify();
            return;
        }
        self.remove_now(id, cx);
    }

    fn remove_now(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.apply("Remove channel", vec![EditOp::Remove { id: Ref::Id(id) }], cx);
        self.move_selection(0, cx);
    }

    fn answer_remove(&mut self, yes: bool, cx: &mut Context<Self>) {
        let Some((id, dont_ask)) = self.confirm_remove.take() else { return };
        if dont_ask {
            self.core.update(cx, |core, _| {
                let path = vec![PathStep::key("interface"), PathStep::key("confirmChannelRemoval")];
                core.report(|pt| pt.set_preference(path, Value::bool(false)));
            });
        }
        if yes {
            self.remove_now(id, cx);
        }
        cx.notify();
    }

    /// The channels something feeds: a track's output, another channel's output or send
    fn in_use(&self, cx: &Context<Self>) -> HashSet<NodeId> {
        let root = self.core.read(cx).port().root().clone();
        let mut used: HashSet<NodeId> =
            model::tracks(&root).iter().flat_map(|t| t.outputs.iter().map(|o| o.to)).collect();
        for c in model::channels(&root) {
            used.extend(c.output);
            used.extend(c.sends.iter().map(|s| s.to));
        }
        used
    }

    /// "Arrange channels like the tracks" (Mixer::arrangeChannelsByTracks): the channels the tracks feed, in the
    /// tracks' order, then the others as they were; one undo step
    fn arrange_like_tracks(&mut self, cx: &mut Context<Self>) {
        let root = self.core.read(cx).port().root().clone();
        let mut current: Vec<NodeId> = model::at(&root, &["mixer", "channels"])
            .map(|l| l.items().filter_map(model::id_of).collect())
            .unwrap_or_default();
        let mut wanted: Vec<NodeId> = Vec::new();
        for t in model::tracks(&root) {
            for o in &t.outputs {
                if current.contains(&o.to) && !wanted.contains(&o.to) {
                    wanted.push(o.to);
                }
            }
        }
        wanted.extend(current.iter().copied().filter(|c| !wanted.contains(c)).collect::<Vec<_>>());
        let mut ops = Vec::new();
        for (index, id) in wanted.iter().enumerate() {
            if current.get(index) != Some(id) {
                current.retain(|c| c != id);
                current.insert(index, *id);
                ops.push(EditOp::Move {
                    id: Ref::Id(*id),
                    parent: Ref::Id(ROOT),
                    list: "mixer.channels".into(),
                    index,
                });
            }
        }
        if !ops.is_empty() {
            self.apply("Arrange channels like the tracks", ops, cx);
        }
    }

    /// The selected channels into a new group channel (P-18): the group made before the first of them, their
    /// outputs into it, in one step (the core's `mixer.group`)
    fn group_selected(&mut self, cx: &mut Context<Self>) {
        let mut set: Vec<NodeId> = self.multi.iter().copied().collect();
        set.extend(self.selected.filter(|s| !set.contains(s)));
        let chs = channels(self.core.read(cx));
        set.retain(|id| chs.iter().any(|c| c.id == *id && c.kind != ChannelKind::Master));
        if set.is_empty() {
            return;
        }
        let params = Value::object([("channels", Value::list(set.iter().map(|id| Value::text(id.to_string()))))]);
        self.core.update(cx, |core, _| {
            core.report(|pt| pt.run_action("mixer.group".into(), params));
        });
        self.multi.clear();
        cx.notify();
    }

    /// The first insert of a channel whose plugin has a sidechain input
    fn sidechain_insert(&self, ch: &ChannelView, cx: &Context<Self>) -> Option<NodeId> {
        let plugins = self.core.read(cx).port().plugins();
        ch.inserts.iter().find(|i| plugins.iter().any(|p| p.id == i.plugin && p.sidechain)).map(|i| i.id)
    }

    /// The sidechain send from a source channel to one insert, made or removed (a send with `port: "sidechain"`,
    /// as the effect window's source). A loop is refused here and by the document (`doc.routing.cycle`)
    fn toggle_sidechain(&mut self, from: NodeId, target: NodeId, cx: &mut Context<Self>) {
        let chs = channels(self.core.read(cx));
        let Some(src) = chs.iter().find(|c| c.id == from) else { return };
        let existing = src.sends.iter().find(|s| s.to == target).map(|s| s.id);
        let op = match existing {
            Some(sid) => EditOp::Remove { id: Ref::Id(sid) },
            None => {
                let Some(owner) = chs.iter().find(|c| c.inserts.iter().any(|i| i.id == target)).map(|c| c.id) else {
                    return;
                };
                if owner == from || leads_to(&chs, owner, from) {
                    return;
                }
                EditOp::Insert {
                    temp: 0,
                    parent: Ref::Id(from),
                    list: "sends".into(),
                    index: src.sends.len(),
                    node: Value::object([("to", Value::text(target.to_string())), ("port", Value::text("sidechain"))]),
                }
            }
        };
        self.apply("Sidechain", vec![op], cx);
    }

    /// Remove unused channels: nothing outputs or sends to them, and no track's output goes to them
    fn remove_unused(&mut self, cx: &mut Context<Self>) {
        let root = self.core.read(cx).port().root().clone();
        let chs = model::channels(&root);
        let mut used: HashSet<NodeId> =
            model::tracks(&root).iter().flat_map(|t| t.outputs.iter().map(|o| o.to)).collect();
        for c in &chs {
            used.extend(c.output);
            used.extend(c.sends.iter().map(|s| s.to));
        }
        let ops: Vec<EditOp> = chs
            .iter()
            .filter(|c| c.kind != ChannelKind::Master && !used.contains(&c.id))
            .map(|c| EditOp::Remove { id: Ref::Id(c.id) })
            .collect();
        if !ops.is_empty() {
            self.apply("Remove unused channels", ops, cx);
        }
    }

    fn start_rename(&mut self, id: NodeId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ch) = self.channel(id, cx) else { return };
        let state = cx.new(|cx| InputState::new(window, cx).default_value(ch.name.to_string()));
        state.update(cx, |s, cx| s.focus(window, cx));
        let sub = cx.subscribe_in(&state, window, move |p: &mut MixingPage, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                p.finish_rename(cx);
            }
        });
        self.renaming = Some((id, state, sub));
        self.menu = None;
        cx.notify();
    }

    /// The new name, when the edit ends (Enter, or the field loses the focus: MixerChannelView::renameFinished)
    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some((id, state, _)) = self.renaming.take() else { return };
        let name = state.read(cx).value().trim().to_string();
        if !name.is_empty() && self.channel(id, cx).is_some_and(|c| *c.name != name) {
            self.apply("Rename channel", vec![EditOp::set(id, &["name"], Value::text(name))], cx);
        }
        cx.notify();
    }

    /// The send button: the selected channel's send to this one, made or removed (Mixer::toggledSend)
    fn toggle_send(&mut self, to: NodeId, cx: &mut Context<Self>) {
        let Some(from) = self.selected.and_then(|s| self.channel(s, cx)) else { return };
        match from.sends.iter().find(|s| s.to == to) {
            Some(s) => self.apply("Send", vec![EditOp::Remove { id: Ref::Id(s.id) }], cx),
            None => {
                let node = Value::object([("to", Value::text(to.to_string()))]);
                let op = EditOp::Insert {
                    temp: 0,
                    parent: Ref::Id(from.id),
                    list: "sends".into(),
                    index: from.sends.len(),
                    node,
                };
                self.apply("Send", vec![op], cx);
            }
        }
    }

    /// An FX send knob pressed with no send yet: the send is made at 0 first, then the knob moves it
    fn send_to_return(
        &mut self,
        ch: NodeId,
        ret: NodeId,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = self.channel(ch, cx).and_then(|c| c.sends.iter().find(|s| s.to == ret).map(|s| s.id));
        let sid = match existing {
            Some(s) => s,
            None if ev.button == MouseButton::Left && ev.click_count == 1 => {
                let n = self.channel(ch, cx).map_or(0, |c| c.sends.len());
                let node = Value::object([
                    ("to", Value::text(ret.to_string())),
                    ("amount", Value::real(0.0).unwrap_or_default()),
                ]);
                let op = EditOp::Insert { temp: 0, parent: Ref::Id(ch), list: "sends".into(), index: n, node };
                let made = self.core.update(cx, |core, _| core.report(|pt| pt.apply(Label::new("Send"), vec![op])));
                match made.and_then(|a| a.created.first().map(|(_, id)| *id)) {
                    Some(id) => id,
                    None => return,
                }
            }
            None => return,
        };
        self.press(Target::Send(sid), 160.0, ch, ev, window, cx);
    }

    fn open_effect(&mut self, insert: NodeId, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.workspace.update(cx, |ws, cx| ws.open(Screen::Effect, Some(insert), window, cx));
    }

    /// "Add effect": the browser's effects, to add to this channel (EffectRackView::addEffect)
    fn add_effect(&mut self, channel: NodeId, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.workspace.update(cx, |ws, cx| ws.open(Screen::Browser, Some(channel), window, cx));
    }

    fn move_effect(&mut self, channel: NodeId, insert: NodeId, by: isize, cx: &mut Context<Self>) {
        let Some(ch) = self.channel(channel, cx) else { return };
        let Some(i) = ch.inserts.iter().position(|p| p.id == insert) else { return };
        let j = i as isize + by;
        if j >= 0 && (j as usize) < ch.inserts.len() {
            let op = EditOp::Move {
                id: Ref::Id(insert),
                parent: Ref::Id(channel),
                list: "inserts".into(),
                index: j as usize,
            };
            self.apply("Move effect", vec![op], cx);
        }
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let delta = if d.x.as_f32().abs() > d.y.as_f32().abs() { d.x } else { d.y }.as_f32();
        self.scroll -= delta;
        cx.notify();
    }
}

// -----------------------------------------------------------------------------------------------------------------
// A strip

fn meter_sides(core: &Core, id: NodeId) -> [f32; 2] {
    let shown = core.meter(id);
    let Some(m) = core.port().telemetry().meters.get(&id) else { return [0.0; 2] };
    let last = (m.channels as usize).max(1) - 1;
    let f = |i: usize| view::meter_fill(m.peak[i.min(last)]);
    let (l, r) = (f(0), f(1));
    let max = l.max(r);
    if max <= 0.0 { [0.0; 2] } else { [shown * l / max, shown * r / max] }
}

fn peak_db(core: &Core, id: NodeId) -> f32 {
    let p = core.port().telemetry().peak(id);
    if p <= 0.0 { -100.0 } else { 20.0 * p.log10() }
}

impl MixingPage {
    /// A switch's right button: its menu, with the MIDI items
    fn switch_menu(
        &self,
        id: NodeId,
        path: &'static [&'static str],
        cx: &Context<Self>,
    ) -> impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static {
        cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            p.menu = Some((Menu::Switch(id, path), ev.position));
            cx.notify();
        })
    }

    /// A desk knob of `k` on node `id`, interactive
    fn desk_knob(&self, id: NodeId, channel: NodeId, k: Knob, enabled: bool, cx: &Context<Self>) -> Stateful<Div> {
        let t = Target::Knob(id, k);
        let travel = self.travel_of(&t, cx);
        let sel = format!("rk-{}-{}-{}", id.number, if k.sec.is_empty() { "ch" } else { k.sec }, k.key);
        let s2 = sel.clone();
        let (t1, t2, t3) = (t.clone(), t.clone(), t);
        div()
            .id(SharedString::from(sel))
            .debug_selector(move || s2.clone())
            .role(gpui::Role::Slider)
            .aria_label(SharedString::from(tr(k.name)))
            .aria_numeric_value(travel)
            .aria_min_numeric_value(0.0)
            .aria_max_numeric_value(1.0)
            .cursor_ns_resize()
            .child(desk::desk_knob(k.cap, tr(k.label), travel, enabled))
            .on_scroll({
                let t = Target::Knob(id, k);
                cx.listener(move |p, ev: &ScrollEvent, _, cx| p.wheel_knob(t.clone(), ev, cx))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |p, ev, w, cx| p.press(t1.clone(), 160.0, channel, ev, w, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |p, ev, w, cx| p.press(t2.clone(), 160.0, channel, ev, w, cx)),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |p, ev, w, cx| p.press(t3.clone(), 160.0, channel, ev, w, cx)),
            )
    }

    fn knob_row(&self, ch: &ChannelView, a: Knob, b: Knob, cx: &Context<Self>) -> Div {
        let enabled = |k: &Knob| {
            // the release does not apply to the models whose release is program dependent (ChannelCompressor)
            k.key != "compRelease"
                || !matches!(
                    model::at(&ch.rack, &["dynamics", "compModel"]).and_then(Value::as_text),
                    Some("la2a" | "fairchild670" | "gates-sta-level" | "altec436" | "rca-ba6a")
                )
        };
        div()
            .w(px(desk::NAME_W))
            .flex()
            .justify_center()
            .child(self.desk_knob(ch.id, ch.id, a, enabled(&a), cx))
            .child(self.desk_knob(ch.id, ch.id, b, enabled(&b), cx))
    }

    /// A desk switch choosing among a few options: a click goes to the next one, the right button lists them
    #[allow(clippy::too_many_arguments)]
    fn choice_button(
        &self,
        ch: NodeId,
        sec: &'static str,
        key: &'static str,
        choices: &'static [&'static str],
        now: String,
        light: Light,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let shown = choice_name(key, &now);
        desk::desk_button(
            SharedString::from(format!("ch-{}-{sec}-{key}", ch.number)),
            shown,
            light != Light::Plain,
            light,
            44.0,
            11.0,
            7.0,
        )
        .debug_selector(move || format!("choice-{}-{sec}-{key}", ch.number))
        .on_click(cx.listener(move |p, _, _, cx| p.next_choice(ch, sec, key, choices, &now, cx)))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                p.menu = Some((Menu::Choice { id: ch, sec, key, choices }, ev.position));
                cx.notify();
            }),
        )
    }

    /// The strip of an analog desk (analog mode, not the master): INPUT, FILTER, DYN, EQ, in signal order
    fn analog_section(&self, ch: &ChannelView, sat: f32, clip: bool, cx: &Context<Self>) -> Div {
        let id = ch.id;
        let w = desk::NAME_W;
        let text = |sec: &str, key: &str, def: &str| {
            model::at(&ch.rack, &[sec, key]).and_then(Value::as_text).unwrap_or(def).to_string()
        };
        let flag = |sec: &str, key: &str| model::at(&ch.rack, &[sec, key]).and_then(Value::as_bool).unwrap_or(false);
        let gain_model = text("tube", "model", "4558");
        let tube = gain_model.starts_with("12a");
        let realistic = flag("tube", "realistic");
        let comp_model = text("dynamics", "compModel", "ssl");
        let comp_light = match comp_model.as_str() {
            "urei1176" => Light::Fet,
            "la2a" => Light::Optical,
            "fairchild670" | "gates-sta-level" | "altec436" | "rca-ba6a" => Light::Tube,
            _ => Light::OpAmp,
        };
        let drive_light = if tube {
            Light::Tube
        } else if gain_model == "flat" {
            Light::Plain
        } else {
            Light::OpAmp
        };
        let (mono, low_cut) = (ch.mono, flag("tube", "lowCut"));
        div()
            .w(px(w))
            .h(px(ANALOG_SECTION_H))
            .pt(px(2.))
            .pb(px(3.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(1.))
            .rounded(px(3.))
            .bg(rgb(col::ANALOG_STRIP))
            .child(desk::section_title(tr("INPUT"), w))
            .child(self.knob_row(ch, GAIN, TRIM, cx))
            .child(self.choice_button(id, "tube", "model", GAIN_MODELS, gain_model, drive_light, cx))
            .child(
                div()
                    .id(SharedString::from(format!("sat-{}", id.number)))
                    .debug_selector(move || format!("sat-{}", id.number))
                    .child(desk::saturation_leds(sat, clip, tube, realistic))
                    // a click on the lamp switches between the light and the realistic tube
                    .when(tube, |d| {
                        d.cursor_pointer().on_click(
                            cx.listener(move |p, _, _, cx| p.toggle(id, &["rack", "tube", "realistic"], realistic, cx)),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(w))
                    .flex()
                    .justify_around()
                    .child(
                        desk::desk_button(
                            SharedString::from(format!("mono-{}", id.number)),
                            tr("MONO"),
                            mono,
                            Light::Glow,
                            23.0,
                            11.0,
                            7.0,
                        )
                        .debug_selector(move || format!("mono-{}", id.number))
                        .on_mouse_down(MouseButton::Right, self.switch_menu(id, &["mono"], cx))
                        .on_click(cx.listener(move |p, _, _, cx| p.toggle(id, &["mono"], mono, cx))),
                    )
                    .child(
                        desk::desk_button(
                            SharedString::from(format!("lcut-{}", id.number)),
                            tr("LCUT"),
                            low_cut,
                            Light::Amber,
                            23.0,
                            11.0,
                            7.0,
                        )
                        .debug_selector(move || format!("lcut-{}", id.number))
                        .on_mouse_down(MouseButton::Right, self.switch_menu(id, &["rack", "tube", "lowCut"], cx))
                        .on_click(
                            cx.listener(move |p, _, _, cx| p.toggle(id, &["rack", "tube", "lowCut"], low_cut, cx)),
                        ),
                    ),
            )
            .child(desk::section_title(tr("FILTER"), w))
            .child(self.knob_row(ch, HPF, LPF, cx))
            .child(self.knob_row(ch, BASS, BASS_FREQ, cx))
            .child(desk::section_title(tr("DYN"), w))
            .child(self.knob_row(ch, COMP, RATIO, cx))
            .child(self.choice_button(id, "dynamics", "compModel", COMPRESSOR_MODELS, comp_model, comp_light, cx))
            .child(self.knob_row(ch, ATK, REL, cx))
            .child(self.knob_row(ch, GATE, RANGE, cx))
            .child(self.choice_button(
                id,
                "dynamics",
                "gateMode",
                GATE_MODES,
                text("dynamics", "gateMode", "std"),
                Light::Plain,
                cx,
            ))
            .child(desk::section_title(tr("EQ"), w))
            .child(self.knob_row(ch, HI, HMF, cx))
            .child(self.knob_row(ch, LMF, LO, cx))
    }

    /// The effects by name, in digital mode (EffectSlotList): the LED turns one on or off, a double click opens it
    /// (or adds one in an empty slot), the right button opens its menu
    fn slot_list(&self, ch: &ChannelView, cx: &Context<Self>) -> Div {
        let id = ch.id;
        let count = ch.inserts.len();
        div().w(px(desk::NAME_W)).h(px(desk::SLOTS as f32 * SLOT_H)).flex().flex_col().children((0..desk::SLOTS).map(
            |k| {
                let row = div()
                    .id(SharedString::from(format!("slot-{}-{k}", id.number)))
                    .debug_selector(move || format!("slot-{}-{k}", id.number))
                    .h(px(SLOT_H - 1.0))
                    .mb(px(1.))
                    .flex()
                    .items_center()
                    .bg(gpui::rgba(0xffffff10))
                    // the slot's own clicks, not the strip's (a double click on the strip renames it)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
                if k >= count {
                    return row.on_click(cx.listener(move |p, ev: &ClickEvent, w, cx| {
                        if ev.click_count() == 2 {
                            p.add_effect(id, w, cx);
                        }
                    }));
                }
                if k == desk::SLOTS - 1 && count > desk::SLOTS {
                    return row
                        .justify_center()
                        .text_size(px(8.))
                        .text_color(rgb(col::TEXT))
                        .child(trf("+{0} more", &[&(count - desk::SLOTS + 1)]));
                }
                let ins = &ch.inserts[k];
                let (iid, bypass) = (ins.id, ins.bypass);
                row.child(
                    div()
                        .id(SharedString::from(format!("led-{}", iid.number)))
                        .debug_selector(move || format!("slot-led-{}", iid.number))
                        .w(px(9.))
                        .h_full()
                        .flex()
                        .items_center()
                        .pl(px(2.))
                        .child(div().w(px(4.)).h(px(4.)).bg(rgb(if bypass {
                            col::SLOT_LED_OFF
                        } else {
                            col::GREEN_LED
                        })))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            cx.stop_propagation();
                            p.toggle(iid, &["bypass"], bypass, cx)
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .text_size(px(8.))
                        .line_height(px(SLOT_H - 1.0))
                        .text_color(rgb(if bypass { 0x6e7680 } else { col::TEXT }))
                        .child(effect_name(&ins.plugin)),
                )
                .on_click(cx.listener(move |p, ev: &ClickEvent, w, cx| {
                    p.select(id, false, cx);
                    if ev.click_count() == 2 {
                        p.open_effect(iid, w, cx);
                    }
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        p.menu = Some((Menu::Effect(id, iid), ev.position));
                        cx.notify();
                    }),
                )
            },
        ))
    }

    /// The engine's readings of every channel this frame (`CorePort::display`, the rack layout); none in the
    /// digital desk. The CLIP LED lights when the clip count moves and stays lit a moment
    fn read_engine(&mut self, chs: &[ChannelView], cx: &Context<Self>) {
        use lm_port::trees::rack_display as d;
        let now = std::time::Instant::now();
        let port = self.core.read(cx).port();
        let num = |f: &[f32], i: usize| f.get(i).copied().filter(|v| v.is_finite()).unwrap_or(0.0);
        self.readings.clear();
        for c in chs {
            let Some(view) = port.display(c.id).filter(|v| &*v.kind == d::KIND) else { continue };
            let f = &view.frame;
            let count = num(f, d::CLIP);
            let (last, until) = self.clips.get(&c.id).copied().unwrap_or((count, now));
            let until = if count != last { now + CLIP_HOLD } else { until };
            self.clips.insert(c.id, (count, until));
            let gonio = (0..d::POINTS)
                .filter_map(|i| {
                    let (l, r) = (f.get(d::GONIO + 2 * i)?, f.get(d::GONIO + 2 * i + 1)?);
                    (l.is_finite() && r.is_finite())
                        .then(|| ((l - r) * std::f32::consts::FRAC_1_SQRT_2, (l + r) * std::f32::consts::FRAC_1_SQRT_2))
                })
                .collect();
            self.readings.insert(
                c.id,
                Reading {
                    sat: num(f, d::SAT).clamp(0.0, 1.0),
                    clip: now < until,
                    bus_gr: num(f, d::BUS_GR),
                    correlation: num(f, d::CORRELATION),
                    gonio,
                    loudness: [d::LUFS_M, d::LUFS_S, d::LUFS_I, d::TRUE_PEAK, d::LRA]
                        .map(|i| f.get(i).copied().filter(|v| v.is_finite()).unwrap_or(f32::NAN)),
                },
            );
        }
    }

    /// The upper area of a channel's strip: send button or receive arrow, send knob, number, the analog section or
    /// the effect list, the FX sends
    fn upper(
        &self,
        ch: &ChannelView,
        number: Option<usize>,
        chs: &[ChannelView],
        analog: bool,
        cx: &Context<Self>,
    ) -> Div {
        let id = ch.id;
        let selected = self.selected.and_then(|s| chs.iter().find(|c| c.id == s)).filter(|s| s.id != id);
        let r = self.readings.get(&id).cloned().unwrap_or_default();
        let (sat, clip) = (r.sat, r.clip);
        // the selected channel's routes to and from this one (MixerView::updateMixerChannel)
        let send_from_selected = selected.and_then(|s| s.sends.iter().find(|x| x.to == id));
        let receives = selected.is_some_and(|s| ch.output == Some(s.id) || ch.sends.iter().any(|x| x.to == s.id));
        let loops = selected.is_none_or(|s| leads_to(chs, id, s.id));
        // a send knob per return (P-19), two to a row as the legacy's FX1 and FX2
        let returns: Vec<&ChannelView> = chs.iter().filter(|c| c.kind == ChannelKind::Return).collect();
        let is_return = ch.kind == ChannelKind::Return;
        let from = selected.map_or(id, |s| s.id);
        let mut d = div().w(px(desk::NAME_W)).h(px(self.upper_h(analog))).flex().flex_col().items_center().gap(px(2.));
        d = d.child(div().h(px(14.)).child(if receives {
            desk::receive_arrow().into_any_element()
        } else if loops {
            div().into_any_element()
        } else {
            desk::send_button(SharedString::from(format!("send-{}", id.number)), send_from_selected.is_some())
                .debug_selector(move || format!("send-{}", id.number))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |p, _, _, cx| p.toggle_send(id, cx)))
                .into_any_element()
        }));
        // the knob of the selected channel's send to this one, and the arrow into the strip
        d = d.child(div().h(px(26.)).children(send_from_selected.map(|s| {
            let t = Target::Send(s.id);
            let travel = self.travel_of(&t, cx);
            let (t1, t2) = (t.clone(), t);
            div()
                .id(SharedString::from(format!("sendknob-{}", id.number)))
                .debug_selector(move || format!("sendknob-{}", id.number))
                .cursor_ns_resize()
                .child(desk::plain_knob(travel, 26.0))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |p, ev, w, cx| p.press(t1.clone(), 160.0, from, ev, w, cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |p, ev, w, cx| p.press(t2.clone(), 160.0, from, ev, w, cx)),
                )
        })));
        d = d.child(div().h(px(8.)).when(send_from_selected.is_some(), |d| d.child(desk::send_arrow())));
        // the number (an effect return has none: the channels are numbered without them)
        d = d.child(div().h(px(13.)).children(number.filter(|_| !is_return).map(desk::lcd)));
        if analog {
            d = d.child(self.analog_section(ch, sat, clip, cx));
        }
        // the sends to the effect returns (not on the returns themselves)
        let rows = self.send_rows as f32;
        d = d.child(
            div()
                .w(px(desk::NAME_W))
                .h(px((desk::KNOB_D + desk::KNOB_LABEL_H) * rows))
                .flex()
                .flex_wrap()
                .justify_center()
                .when(!is_return, |d| {
                    d.children(returns.iter().enumerate().map(|(i, r)| {
                        let rid = r.id;
                        let travel =
                            ch.sends.iter().find(|s| s.to == rid).map_or(0.0, |s| view::gain_to_travel(s.amount));
                        let sel = format!("fx-{}-{}", id.number, i + 1);
                        let s2 = sel.clone();
                        div()
                            .id(SharedString::from(sel))
                            .debug_selector(move || s2.clone())
                            .cursor_ns_resize()
                            .child(desk::desk_knob(col::FX_SEND, format!("FX{}", i + 1), travel, true))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |p, ev: &MouseDownEvent, w, cx| p.send_to_return(id, rid, ev, w, cx)),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |p, ev: &MouseDownEvent, w, cx| p.send_to_return(id, rid, ev, w, cx)),
                            )
                    }))
                }),
        );
        if !analog {
            d = d.child(self.slot_list(ch, cx));
        }
        d
    }

    /// The fader row: the peak above the fader, and beside it M, S, Ø, ⇄ and (analog) the effect LEDs
    fn fader_row(&self, ch: &ChannelView, analog: bool, cx: &Context<Self>) -> Div {
        let id = ch.id;
        let levels = meter_sides(self.core.read(cx), id);
        let peak = self.peaks.get(&id).copied().unwrap_or(-100.0);
        let role = match ch.kind {
            ChannelKind::Master => Role::Post,
            ChannelKind::Return => Role::Return,
            _ => Role::Channel,
        };
        let master = ch.kind == ChannelKind::Master;
        let fader_w = if master { desk::POST_FADER_W } else { desk::FADER_W };
        let travel = view::gain_to_travel(ch.volume);
        let fader_h = self.fader_h;
        let grab = fader_h - desk::HANDLE_H;
        let fader = div()
            .id(SharedString::from(format!("fader-{}", id.number)))
            .debug_selector(move || format!("fader-{}", id.number))
            .role(gpui::Role::Slider)
            .aria_label(SharedString::from(format!("{} {}", tr("Volume"), ch.name)))
            .aria_numeric_value(view::gain_to_db(ch.volume).max(-60.0))
            .h(px(fader_h))
            .cursor_ns_resize()
            .child(desk::fader(travel, levels, role, fader_w))
            .on_scroll(cx.listener(move |p, ev: &ScrollEvent, _, cx| p.wheel_fader(id, ev, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |p, ev, w, cx| p.press(Target::Fader(id, "volume"), grab, id, ev, w, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |p, ev, w, cx| p.press(Target::Fader(id, "volume"), grab, id, ev, w, cx)),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |p, ev, w, cx| p.press(Target::Fader(id, "volume"), grab, id, ev, w, cx)),
            );
        let fader_column = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .child(
                desk::peak_indicator(SharedString::from(format!("peak-{}", id.number)), peak)
                    .debug_selector(move || format!("peak-{}", id.number))
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.peaks.insert(id, -100.0);
                        cx.notify();
                    })),
            )
            .child(fader)
            .when(master, |d| d.child(fader_label("POST")));
        let letter = |key: &'static str, text: &'static str, on: bool, l: Light| {
            let field: &'static [&'static str] = match key {
                "mute" => &["mute"],
                "solo" => &["solo"],
                "phase" => &["phaseInvert"],
                _ => &["swap"],
            };
            desk::desk_button(SharedString::from(format!("{key}-{}", id.number)), text, on, l, 13.0, 13.0, 9.0)
                .debug_selector(move || format!("{key}-{}", id.number))
                .on_mouse_down(MouseButton::Right, self.switch_menu(id, field, cx))
                .on_click(cx.listener(move |p, _, _, cx| {
                    let path: &[&str] = match key {
                        "mute" => &["mute"],
                        "solo" => &["solo"],
                        "phase" => &["phaseInvert"],
                        _ => &["swap"],
                    };
                    p.toggle(id, path, on, cx)
                }))
        };
        // an effect the engine silenced (it panicked or made NaN, I-9.3) is red, as the legacy's corrupted audio
        let slots: Vec<Slot> = {
            let meters = &self.core.read(cx).port().telemetry().meters;
            ch.inserts
                .iter()
                .map(|i| {
                    if meters.get(&i.id).is_some_and(|m| m.dead) {
                        Slot::Corrupted
                    } else if i.bypass {
                        Slot::Off
                    } else {
                        Slot::On
                    }
                })
                .collect()
        };
        let inserts: Vec<NodeId> = ch.inserts.iter().map(|i| i.id).collect();
        let buttons = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .pt(px(14.))
            .child(letter("mute", "M", ch.mute, Light::Mute))
            // the master has no solo, phase or swap in the model
            .when(!master, |d| {
                d.child(letter("solo", "S", ch.solo, Light::Solo))
                    .child(letter("phase", "Ø", ch.phase_invert, Light::Amber))
                    .child(letter("swap", "⇄", ch.swap, Light::Amber))
            })
            .when(analog, |d| d.child(div().h(px(2.))).child(self.effect_leds(ch, slots, inserts, cx)));
        let mut row = div().flex().flex_row().justify_center().gap(px(2.));
        if master {
            row = row.child(self.pre_faders(ch, cx)).child(div().pt(px(14.)).child(desk::db_scale(fader_h)));
        }
        row.child(fader_column).child(buttons)
    }

    /// The effects as LEDs beside the fader, in analog mode (EffectLeds): five slots, the last standing for the
    /// rest; a double click opens the slot's effect (or adds one in an empty slot), the right button its menu
    fn effect_leds(&self, ch: &ChannelView, slots: Vec<Slot>, inserts: Vec<NodeId>, cx: &Context<Self>) -> Div {
        let id = ch.id;
        let n = inserts.len();
        div().w(px(desk::LED_W)).flex().flex_col().children((0..desk::SLOTS).map(|k| {
            let mut state = slots.get(k).copied();
            if k == desk::SLOTS - 1 && slots.iter().skip(k + 1).any(|s| *s == Slot::On) {
                state = Some(Slot::On);
            }
            let insert = inserts.get(k).copied().filter(|_| !(k == desk::SLOTS - 1 && n > desk::SLOTS));
            let led = div()
                .id(SharedString::from(format!("led-{}-{k}", id.number)))
                .debug_selector(move || format!("led-{}-{k}", id.number))
                .w(px(desk::LED_W))
                .h(px(desk::LED_H))
                .mb(px(desk::LED_PITCH - desk::LED_H))
                .p(px(1.))
                .bg(rgb(0x0c0d0f))
                .child(div().size_full().bg(rgb(desk::slot_color(state))))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |p, ev: &MouseDownEvent, w, cx| {
                        cx.stop_propagation();
                        p.select(id, false, cx);
                        if ev.click_count == 2 {
                            match insert {
                                Some(i) => p.open_effect(i, w, cx),
                                None if k >= n => p.add_effect(id, w, cx),
                                None => {}
                            }
                        }
                    }),
                );
            match insert {
                Some(i) => led.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        p.menu = Some((Menu::Effect(id, i), ev.position));
                        cx.notify();
                    }),
                ),
                None => led,
            }
        }))
    }

    /// The master's PRE faders, one per side, which LINK moves together (MixerChannelView, master)
    fn pre_faders(&self, ch: &ChannelView, cx: &Context<Self>) -> Div {
        let id = ch.id;
        let (l, r, link) = ch.pre;
        let fader_h = self.fader_h;
        let grab = fader_h - desk::HANDLE_H;
        let one = |key: &'static str, gain: f64| {
            div()
                .id(SharedString::from(format!("{key}-{}", id.number)))
                .debug_selector(move || key.to_string())
                .h(px(fader_h))
                .cursor_ns_resize()
                .child(desk::fader(view::gain_to_travel(gain), [0.0, 0.0], Role::Pre, desk::PRE_FADER_W))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |p, ev, w, cx| p.press(Target::Fader(id, key), grab, id, ev, w, cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |p, ev, w, cx| p.press(Target::Fader(id, key), grab, id, ev, w, cx)),
                )
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |p, ev, w, cx| p.press(Target::Fader(id, key), grab, id, ev, w, cx)),
                )
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .child(
                desk::desk_button("pre-link", tr("LINK"), link, Light::Amber, 30.0, 12.0, 7.0)
                    .debug_selector(|| "pre-link".into())
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.apply("Link", vec![EditOp::set(id, &["preLink"], Value::bool(!link))], cx)
                    })),
            )
            .child(div().flex().gap(px(2.)).child(one("preLeft", l)).child(one("preRight", r)))
            .child(fader_label("PRE  L  R"))
    }

    fn strip(
        &self,
        ch: &ChannelView,
        number: Option<usize>,
        chs: &[ChannelView],
        analog: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let id = ch.id;
        let active = self.selected == Some(id) || self.multi.contains(&id);
        let master = ch.kind == ChannelKind::Master;
        let group = ch.output.and_then(|o| chs.iter().find(|c| c.id == o && c.kind == ChannelKind::Group));
        let renaming = self.renaming.as_ref().filter(|(r, ..)| *r == id).map(|(_, s, _)| s.clone());
        let pan = div()
            .w(px(desk::NAME_W))
            .h(px(if analog { desk::KNOB_D + desk::KNOB_LABEL_H + 4.0 } else { 21.0 }))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .when(!master, |d| {
                d.bg(rgb(col::PAN_BACKGROUND)).child(if analog {
                    self.desk_knob(id, id, PAN, true, cx).into_any_element()
                } else {
                    let t = Target::Knob(id, PAN);
                    let travel = self.travel_of(&t, cx);
                    let (t1, t2, t3) = (t.clone(), t.clone(), t);
                    div()
                        .id(SharedString::from(format!("rk-{}-ch-pan", id.number)))
                        .debug_selector(move || format!("rk-{}-ch-pan", id.number))
                        .cursor_ns_resize()
                        .child(desk::plain_knob(travel, 17.0))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |p, ev, w, cx| p.press(t1.clone(), 160.0, id, ev, w, cx)),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |p, ev, w, cx| p.press(t2.clone(), 160.0, id, ev, w, cx)),
                        )
                        .on_mouse_down(
                            MouseButton::Middle,
                            cx.listener(move |p, ev, w, cx| p.press(t3.clone(), 160.0, id, ev, w, cx)),
                        )
                        .into_any_element()
                })
            });
        let upper = if master {
            self.master_section(ch, analog, cx).into_any_element()
        } else {
            self.upper(ch, number, chs, analog, cx).into_any_element()
        };
        let name: AnyElement = match renaming {
            Some(state) => div()
                .w(px(desk::NAME_W))
                .text_size(px(9.))
                .child(crate::ui::text_entry(Input::new(&state)))
                .into_any_element(),
            None => div()
                .id(SharedString::from(format!("name-{}", id.number)))
                .debug_selector(move || format!("name-{}", id.number))
                .w(px(desk::NAME_W))
                .h(px(NAME_H))
                .px(px(2.))
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .rounded(px(2.))
                .bg(rgb(col::NAME_BACKGROUND))
                .border_1()
                .border_color(rgb(col::STROKE_INNER))
                .text_size(px(9.))
                .text_color(rgb(col::TEXT))
                .child(elide(&ch.name, 9))
                .into_any_element(),
        };
        let collapsed = self.collapsed.contains(&id);
        div()
            .id(SharedString::from(format!("strip-{}", id.number)))
            .debug_selector(move || format!("strip-{}", id.number))
            .relative()
            .flex_none()
            .w(px(if master { desk::MASTER_W } else { desk::STRIP_W }))
            .h_full()
            .p(px(4.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .bg(desk::strip_bg(ch.color, ch.mute, active))
            .child(desk::strip_borders())
            .children(group.map(|g| desk::member_line(g.color.map_or(rgb(col::TEXT), rgb))))
            .when(ch.kind == ChannelKind::Group, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!("group-{}", id.number)))
                        .debug_selector(move || format!("group-{}", id.number))
                        .absolute()
                        .left(px(3.))
                        .top(px(3.))
                        .child(desk::group_marker(collapsed))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            if !p.collapsed.remove(&id) {
                                p.collapsed.insert(id);
                            }
                            cx.notify();
                        })),
                )
            })
            .child(upper)
            .child(pan)
            .child(div().flex_1().child(self.fader_row(ch, analog, cx)))
            .child(name)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |p, ev: &MouseDownEvent, w, cx| {
                    w.focus(&p.focus, cx);
                    if fires("mix.channel.rename", ev.button, ev.click_count, &ev.modifiers) {
                        return p.start_rename(id, w, cx);
                    }
                    p.select(id, holds("mix.channel.multi", &ev.modifiers), cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                    p.select(id, false, cx);
                    p.menu = Some((Menu::Strip(id), ev.position));
                    cx.notify();
                }),
            )
    }
}

fn fader_label(text: &'static str) -> Div {
    div().h(px(9.)).text_size(px(7.)).line_height(px(9.)).text_color(rgb(col::DIM)).child(tr(text))
}

/// Whether `from`'s routes lead to `to` (Mixer::isInfiniteLoop): outputs and sends
fn leads_to(chs: &[ChannelView], from: NodeId, to: NodeId) -> bool {
    // a send to an insert's sidechain input leads to the insert's channel
    let owner = |n: NodeId| chs.iter().find(|c| c.inserts.iter().any(|i| i.id == n)).map_or(n, |c| c.id);
    let mut seen = HashSet::new();
    let mut stack = vec![from];
    while let Some(n) = stack.pop() {
        if n == to {
            return true;
        }
        if !seen.insert(n) {
            continue;
        }
        if let Some(c) = chs.iter().find(|c| c.id == n) {
            stack.extend(c.output);
            stack.extend(c.sends.iter().map(|s| owner(s.to)));
        }
    }
    false
}

fn elide(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max - 1).collect::<String>()) }
}

/// A plugin's name as the effect slots show it
fn effect_name(plugin: &str) -> String {
    let mut c = plugin.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// A choice's name on its desk switch (DriveButton::name, ChannelCompressor::name, ChannelGate::modeName)
fn choice_name(key: &str, v: &str) -> String {
    match (key, v) {
        ("model", "flat") => tr("FLAT").to_string(),
        ("model", "12ax7-vintage") => "12AX7 V".into(),
        ("model", "12ax7-hot") => "12AX7 H".into(),
        ("model", m) => m.to_uppercase(),
        ("compModel", m) => match m {
            "ssl" => "SSL",
            "api2500" => "API 2500",
            "dbx160" => "dbx 160",
            "urei1176" => "1176",
            "la2a" => "LA-2A",
            "fairchild670" => "670",
            "gates-sta-level" => "STA-LVL",
            "altec436" => "436",
            "rca-ba6a" => "BA-6A",
            other => other,
        }
        .into(),
        ("gateMode", m) => tr(match m {
            "std" => "GATE",
            "kick" => "KICK",
            "snare" => "SNARE",
            "toms" => "TOMS",
            "vox" => "VOX",
            "guitar" => "GTR",
            "bass" => "BASS",
            "noise" => "NOISE",
            _ => "EXP",
        })
        .to_string(),
        (_, v) => v.to_string(),
    }
}

// -----------------------------------------------------------------------------------------------------------------
// The master section

impl MixingPage {
    /// The upper area's height: the legacy's, plus a row for each two returns past the first two (P-19)
    fn upper_h(&self, analog: bool) -> f32 {
        let base = if analog { UPPER_ANALOG } else { UPPER_DIGITAL };
        base + (self.send_rows.saturating_sub(1)) as f32 * (desk::KNOB_D + desk::KNOB_LABEL_H)
    }

    /// The master's loudness (BS.1770-4, EBU R 128) from the engine: M, S and I in LUFS, the true peak and the range;
    /// a click resets the integrated value, the range and the peak hold (the core's `loudness.reset`)
    fn loudness_readout(&self, r: &Reading, w: f32, cx: &Context<Self>) -> impl IntoElement {
        let show = |v: f32, unit: &str| if v.is_finite() { format!("{v:.1}{unit}") } else { "—".to_owned() };
        let [m, st, i, tp, lra] = r.loudness;
        let cell = |label: &'static str, text: String, red: bool| {
            div()
                .flex()
                .flex_col()
                .items_center()
                .child(div().text_size(px(6.)).text_color(rgb(col::DIM)).child(label))
                .child(
                    div()
                        .text_size(px(8.))
                        .font_family("Menlo")
                        .text_color(rgb(if red { col::LED_RED } else { col::GREEN_LED }))
                        .child(text),
                )
        };
        div()
            .id("loudness")
            .debug_selector(|| "loudness".into())
            .w(px(w))
            .h(px(24.))
            .px(px(2.))
            .flex()
            .justify_between()
            .items_center()
            .rounded(px(2.))
            .bg(rgb(0x0c0d0f))
            .cursor_pointer()
            .child(cell("M", show(m, ""), false))
            .child(cell("S", show(st, ""), false))
            .child(cell("I LUFS", show(i, ""), false))
            .child(cell("TP", show(tp, ""), tp.is_finite() && tp > -1.0))
            .child(cell("LRA", show(lra, ""), false))
            .on_click(cx.listener(|p, _, _, cx| {
                p.core.update(cx, |core, _| {
                    core.report(|pt| pt.run_action("loudness.reset".into(), Value::empty_object()));
                });
                cx.notify();
            }))
    }

    fn master_section(&self, ch: &ChannelView, analog: bool, cx: &Context<Self>) -> Div {
        let id = ch.id;
        let core = self.core.read(cx);
        let [l, r] = meter_sides(core, id);
        let prefs = core.port().preferences();
        let mon = |k: &str| model::at(prefs, &["monitor", k]).and_then(Value::as_bool).unwrap_or(false);
        let (dim, cut, mono, side) = (mon("dim"), mon("cut"), mon("mono"), mon("side"));
        let rack = &ch.rack;
        let flag = |sec: &str, key: &str| model::at(rack, &[sec, key]).and_then(Value::as_bool).unwrap_or(false);
        let gain_model = model::at(rack, &["tube", "model"]).and_then(Value::as_text).unwrap_or("4558").to_string();
        let tube = gain_model.starts_with("12a");
        let (xfmr, comp_in, realistic) = (flag("transformer", "on"), flag("dynamics", "on"), flag("tube", "realistic"));
        let w = desk::MASTER_SECTION_W - 8.0;
        // the meter's fill as dBFS, against a reference of -18 dBFS = 0 VU
        let vu = |level: f32| if level <= 0.0 { 0.0 } else { desk::vu_position(-60.0 + 60.0 * level + 18.0).min(1.08) };
        let master_button = |key: &'static str, text: &'static str, on: bool| {
            desk::desk_button(SharedString::from(format!("master-{key}")), tr(text), on, Light::Amber, 26.0, 13.0, 7.0)
                .debug_selector(move || format!("master-{key}"))
        };
        let knob = |k: Knob| self.desk_knob(id, id, k, true, cx);
        let title = |t: &'static str| desk::section_title(tr(t), w);
        let reading = self.readings.get(&id).cloned().unwrap_or_default();
        let monitor_t = self.travel_of(&Target::Monitor, cx);
        div()
            .w(px(desk::MASTER_SECTION_W))
            .h(px(self.upper_h(analog)))
            .p(px(4.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(3.))
            .bg(rgb(col::MASTER_SECTION))
            .border_l_1()
            .border_r_1()
            .border_color(rgb(col::STROKE_INNER))
            .child(title("METERS"))
            .child(div().flex().gap(px(4.)).child(desk::vu_meter("L", vu(l))).child(desk::vu_meter("R", vu(r))))
            .child(desk::goniometer(reading.gonio.clone(), w))
            .child(desk::correlation(reading.correlation, w))
            .child(self.loudness_readout(&reading, w, cx))
            .when(analog, |d| {
                d.child(title("DRIVE"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(self.choice_button(
                                id,
                                "tube",
                                "model",
                                GAIN_MODELS,
                                gain_model.clone(),
                                if tube { Light::Tube } else { Light::OpAmp },
                                cx,
                            ))
                            .child(
                                div()
                                    .id("master-sat")
                                    .child(desk::saturation_leds(reading.sat, reading.clip, tube, realistic))
                                    .when(tube, |d| {
                                        d.cursor_pointer().on_click(cx.listener(move |p, _, _, cx| {
                                            p.toggle(id, &["rack", "tube", "realistic"], realistic, cx)
                                        }))
                                    }),
                            ),
                    )
                    .child(master_button("xfmr", "XFMR", xfmr).on_click(
                        cx.listener(move |p, _, _, cx| p.toggle(id, &["rack", "transformer", "on"], xfmr, cx)),
                    ))
            })
            .child(title("STEREO"))
            .child(div().flex().child(knob(WIDTH)).child(knob(M_BASS)).child(knob(M_FREQ)).child(knob(AIR)))
            .child(title("BUS COMP"))
            .child(
                div()
                    .w(px(w))
                    .p(px(3.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .rounded(px(3.))
                    .bg(rgb(0x2a2e34))
                    .child(
                        div()
                            .flex()
                            .gap(px(4.))
                            .child(master_button("comp-in", "IN", comp_in).on_click(
                                cx.listener(move |p, _, _, cx| p.toggle(id, &["rack", "dynamics", "on"], comp_in, cx)),
                            ))
                            .child(desk::gain_reduction_meter(reading.bus_gr, w - 40.0)),
                    )
                    .child(div().flex().child(knob(THR)).child(knob(B_RATIO)).child(knob(MAKE)))
                    .child(div().flex().child(knob(B_ATK)).child(knob(B_REL))),
            )
            .child(title("MONITOR"))
            .child(
                div()
                    .flex()
                    .gap(px(2.))
                    .child(
                        div()
                            .id("monitor-level")
                            .debug_selector(|| "monitor-level".into())
                            .cursor_ns_resize()
                            .child(desk::desk_knob(GREY, tr("LEVEL"), monitor_t, true))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |p, ev, w, cx| p.press(Target::Monitor, 160.0, id, ev, w, cx)),
                            )
                            .on_mouse_down(
                                MouseButton::Middle,
                                cx.listener(move |p, ev, w, cx| p.press(Target::Monitor, 160.0, id, ev, w, cx)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .w(px(56.))
                            .gap(px(2.))
                            .child(master_button("dim", "DIM", dim).on_click(
                                cx.listener(move |p, _, _, cx| p.set_monitor(&["dim"], Value::bool(!dim), cx)),
                            ))
                            .child(master_button("cut", "CUT", cut).on_click(
                                cx.listener(move |p, _, _, cx| p.set_monitor(&["cut"], Value::bool(!cut), cx)),
                            ))
                            .child(master_button("mono", "MONO", mono).on_click(cx.listener(move |p, _, _, cx| {
                                p.set_monitor(&["mono"], Value::bool(!mono), cx);
                                if !mono {
                                    p.set_monitor(&["side"], Value::bool(false), cx);
                                }
                            })))
                            .child(master_button("side", "SIDE", side).on_click(cx.listener(move |p, _, _, cx| {
                                p.set_monitor(&["side"], Value::bool(!side), cx);
                                if !side {
                                    p.set_monitor(&["mono"], Value::bool(false), cx);
                                }
                            }))),
                    ),
            )
    }
}

// -----------------------------------------------------------------------------------------------------------------
// The right column: the mode, the selected channel's settings, its effects rack

impl MixingPage {
    fn right_column(&self, chs: &[ChannelView], analog: bool, cx: &Context<Self>) -> Div {
        let mode_button = |key: &'static str, text: &'static str, on: bool, to: &'static str| {
            div()
                .id(key)
                .debug_selector(move || key.into())
                .px(px(6.))
                .py(px(2.))
                .border_1()
                .border_color(rgb(col::STROKE_INNER))
                .bg(rgb(if on { col::GREEN_LED } else { 0x262b30 }))
                .text_size(px(9.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(if on { 0x14161a } else { col::TEXT }))
                .cursor_pointer()
                .child(tr(text))
                .on_click(cx.listener(move |p, _, _, cx| {
                    let op = EditOp::Set {
                        id: Ref::Id(ROOT),
                        path: vec![PathStep::key("mixer"), PathStep::key("mode")],
                        value: Value::text(to),
                    };
                    p.apply("Desk mode", vec![op], cx);
                }))
        };
        let column = div()
            .w(px(desk::RACK_W))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(c(THEME.bg1))
            .border_l_1()
            .border_color(rgb(col::STROKE_INNER))
            .child(
                div()
                    .flex()
                    .px(px(6.))
                    .pt(px(4.))
                    .pb(px(2.))
                    .child(mode_button("mode-analog", "ANALOG", analog, "analog"))
                    .child(mode_button("mode-digital", "DIGITAL", !analog, "digital")),
            );
        let Some(ch) = self.selected.and_then(|s| chs.iter().find(|c| c.id == s)).or_else(|| chs.last()).cloned()
        else {
            return column;
        };
        let id = ch.id;
        let master = ch.kind == ChannelKind::Master;
        let mono = ch.mono;
        let flag = |sec: &str, def: bool| model::at(&ch.rack, &[sec, "on"]).and_then(Value::as_bool).unwrap_or(def);
        let (xfmr, tol) = (flag("transformer", false), flag("tolerance", true));
        let linked = self.linked.contains(&id);
        let led_box = |key: &'static str, text: &'static str, on: bool| {
            div()
                .id(key)
                .debug_selector(move || key.into())
                .flex()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .child(div().size(px(9.)).rounded(px(2.)).bg(rgb(if on { col::GREEN_LED } else { col::LED_OFF })))
                .child(div().text_size(px(10.)).text_color(c(THEME.text)).child(tr(text)))
        };
        // the channel's settings: Mono and the pan (MixerView), and the strip's switches the legacy did not show
        // (the transformer and the tolerances, analog) and the faders' link (P-18)
        let settings = div()
            .px(px(6.))
            .pt(px(2.))
            .pb(px(4.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .when(!master, |d| {
                let travel = self.travel_of(&Target::Knob(id, PAN), cx);
                d.child(
                    led_box("settings-mono", "Mono", mono)
                        .on_click(cx.listener(move |p, _, _, cx| p.toggle(id, &["mono"], mono, cx))),
                )
                .child(
                    div()
                        .id("settings-pan")
                        .flex()
                        .flex_col()
                        .items_center()
                        .cursor_ns_resize()
                        .child(desk::plain_knob(travel, 26.0))
                        .child(div().text_size(px(8.)).text_color(c(THEME.text2)).child(tr("PAN")))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |p, ev, w, cx| p.press(Target::Knob(id, PAN), 160.0, id, ev, w, cx)),
                        ),
                )
            })
            .when(analog && !master, |d| {
                d.child(
                    led_box("settings-xfmr", "XFMR", xfmr).on_click(
                        cx.listener(move |p, _, _, cx| p.toggle(id, &["rack", "transformer", "on"], xfmr, cx)),
                    ),
                )
                .child(
                    led_box("settings-tol", "TOL", tol)
                        .on_click(cx.listener(move |p, _, _, cx| p.toggle(id, &["rack", "tolerance", "on"], tol, cx))),
                )
            })
            .when(!master, |d| {
                d.child(led_box("link", "Link", linked).on_click(cx.listener(|p, _, _, cx| p.link_selected(cx))))
            });
        // the effects rack (EffectRackView): each effect with its LED, its name, its point in the strip (analog; in
        // digital every point collapses into the plain chain), its controls; "Add effect" at the end
        let rows: Vec<AnyElement> = ch
            .inserts
            .iter()
            .map(|ins| {
                let (iid, bypass, point) = (ins.id, ins.bypass, ins.point.to_string());
                div()
                    .id(SharedString::from(format!("rack-{}", iid.number)))
                    .debug_selector(move || format!("rack-{}", iid.number))
                    .mx(px(6.))
                    .mb(px(4.))
                    .p(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(3.))
                    .bg(c(THEME.bg3))
                    .border_1()
                    .border_color(c(THEME.line))
                    .child(
                        div()
                            .id(SharedString::from(format!("rack-led-{}", iid.number)))
                            .debug_selector(move || format!("rack-led-{}", iid.number))
                            .size(px(10.))
                            .rounded(px(5.))
                            .bg(rgb(if bypass { col::SLOT_LED_OFF } else { col::GREEN_LED }))
                            .cursor_pointer()
                            .on_click(cx.listener(move |p, _, _, cx| p.toggle(iid, &["bypass"], bypass, cx))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_size(px(11.))
                            .text_color(c(if bypass { THEME.text3 } else { THEME.text }))
                            .child(effect_name(&ins.plugin)),
                    )
                    .when(analog && !master, |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("point-{}", iid.number)))
                                .debug_selector(move || format!("point-{}", iid.number))
                                .w(px(80.))
                                .px(px(4.))
                                .rounded(px(2.))
                                .bg(c(THEME.bg1))
                                .text_size(px(9.))
                                .text_color(c(THEME.text2))
                                .cursor_pointer()
                                .child(tr(point_name(&point)))
                                .on_click(cx.listener(move |p, _, _, cx| {
                                    let i = INSERT_POINTS
                                        .iter()
                                        .position(|x| *x == point)
                                        .map_or(0, |i| (i + 1) % INSERT_POINTS.len());
                                    p.apply(
                                        "Insert point",
                                        vec![EditOp::set(iid, &["point"], Value::text(INSERT_POINTS[i]))],
                                        cx,
                                    );
                                })),
                        )
                    })
                    .child({
                        // W/D: the insert's wet/dry mix
                        let t = Target::Knob(iid, WET);
                        let travel = self.travel_of(&t, cx);
                        let (t1, t2, t3) = (t.clone(), t.clone(), t);
                        div()
                            .id(SharedString::from(format!("wet-{}", iid.number)))
                            .debug_selector(move || format!("wet-{}", iid.number))
                            .flex()
                            .flex_col()
                            .items_center()
                            .cursor_ns_resize()
                            .child(desk::plain_knob(travel, 17.0))
                            .child(div().text_size(px(7.)).text_color(c(THEME.text3)).child(tr("W/D")))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |p, ev, w, cx| p.press(t1.clone(), 160.0, id, ev, w, cx)),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |p, ev, w, cx| p.press(t2.clone(), 160.0, id, ev, w, cx)),
                            )
                            .on_mouse_down(
                                MouseButton::Middle,
                                cx.listener(move |p, ev, w, cx| p.press(t3.clone(), 160.0, id, ev, w, cx)),
                            )
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("controls-{}", iid.number)))
                            .debug_selector(move || format!("controls-{}", iid.number))
                            .px(px(4.))
                            .rounded(px(2.))
                            .bg(c(THEME.bg4))
                            .text_size(px(9.))
                            .cursor_pointer()
                            .child(tr("Controls"))
                            .on_click(cx.listener(move |p, _, w, cx| p.open_effect(iid, w, cx))),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |p, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            p.menu = Some((Menu::Effect(id, iid), ev.position));
                            cx.notify();
                        }),
                    )
                    .into_any_element()
            })
            .collect();
        column.child(settings).child(
            div().id("rack").flex_1().overflow_y_scroll().pt(px(4.)).children(rows).child(
                div()
                    .id("add-effect")
                    .debug_selector(|| "add-effect".into())
                    .mx(px(6.))
                    .py(px(4.))
                    .flex()
                    .justify_center()
                    .rounded(px(3.))
                    .border_1()
                    .border_dashed()
                    .border_color(c(THEME.line2))
                    .text_size(px(11.))
                    .text_color(c(THEME.text2))
                    .cursor_pointer()
                    .child(tr("Add effect"))
                    .on_click(cx.listener(move |p, _, w, cx| p.add_effect(id, w, cx))),
            ),
        )
    }
}

impl MixingPage {
    /// "This channel is being used" (MixerChannelView::confirmRemoval): Cancel by default, "Don't ask again"
    fn render_confirm_remove(&self, id: NodeId, dont_ask: bool, cx: &Context<Self>) -> AnyElement {
        let name = self.channel(id, cx).map_or_else(String::new, |c| c.name.to_string());
        div()
            .absolute()
            .left(px(0.))
            .top(px(0.))
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x00000080))
            .occlude()
            .child(
                div()
                    .id("confirm-remove")
                    .w(px(380.))
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .rounded(px(8.))
                    .bg(c(THEME.bg2))
                    .border_1()
                    .border_color(c(THEME.line2))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::BOLD).child(tr("Confirm removal")))
                    .child(
                        div()
                            .text_size(px(12.))
                            .child(trf("The channel {0} is being used. Are you sure you want to remove it?", &[&name])),
                    )
                    .child(
                        div()
                            .id("confirm-dont-ask")
                            .debug_selector(|| "confirm-dont-ask".into())
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .cursor_pointer()
                            .child(div().size(px(10.)).rounded(px(2.)).bg(rgb(if dont_ask {
                                col::GREEN_LED
                            } else {
                                col::LED_OFF
                            })))
                            .child(div().text_size(px(11.)).child(tr("Don't ask again")))
                            .on_click(cx.listener(|p, _, _, cx| {
                                if let Some((_, d)) = p.confirm_remove.as_mut() {
                                    *d = !*d;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                crate::ui::button("confirm-cancel", tr("Cancel"), false, 0)
                                    .debug_selector(|| "confirm-cancel".into())
                                    .on_click(cx.listener(|p, _, _, cx| p.answer_remove(false, cx))),
                            )
                            .child(
                                crate::ui::button("confirm-ok", tr("OK"), true, THEME.red)
                                    .debug_selector(|| "confirm-ok".into())
                                    .on_click(cx.listener(|p, _, _, cx| p.answer_remove(true, cx))),
                            ),
                    ),
            )
            .into_any_element()
    }
}

/// An insert point's name (the effects' insert point combo of MixerView)
fn point_name(p: &str) -> &'static str {
    match p {
        "pre-gain" => "Pre gain",
        "post-gain" => "Post gain",
        "post-filter" => "Post filter",
        "post-dynamics" => "Post dynamics",
        "post-eq" => "Post EQ",
        _ => "Pre pan",
    }
}

// -----------------------------------------------------------------------------------------------------------------
// Menus

fn menu_item(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Stateful<Div> {
    let id: SharedString = id.into();
    let sel = id.to_string();
    div()
        .id(id)
        .debug_selector(move || sel.clone())
        .px(px(10.))
        .py(px(3.))
        .text_size(px(12.))
        .cursor_pointer()
        .hover(|d| d.bg(c(THEME.bg4)))
        .child(label.into())
}

fn separator() -> AnyElement {
    div().my(px(3.)).h(px(1.)).bg(c(THEME.line)).into_any_element()
}

fn caption_row(text: &'static str) -> AnyElement {
    div().px(px(10.)).text_size(px(10.)).text_color(c(THEME.text3)).child(tr(text)).into_any_element()
}

impl MixingPage {
    fn render_menu(&self, menu: &Menu, chs: &[ChannelView], cx: &Context<Self>) -> Vec<AnyElement> {
        let mut items: Vec<AnyElement> = Vec::new();
        match menu {
            Menu::Knob(t) => {
                // a send: before or after the fader (the model's `pre`)
                if let Target::Send(sid) = t {
                    let sid = *sid;
                    let pre = chs.iter().flat_map(|c| c.sends.iter()).find(|s| s.id == sid).is_some_and(|s| s.pre);
                    items.push(
                        menu_item(
                            "menu-send-pre",
                            format!("{}{}", if pre { "✓ " } else { "" }, tr("Before the fader")),
                        )
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.menu = None;
                            p.toggle(sid, &["pre"], pre, cx);
                        }))
                        .into_any_element(),
                    );
                    items.push(separator());
                }
                let (t1, t2, t3) = (t.clone(), t.clone(), t.clone());
                items.push(
                    menu_item("menu-default", tr("Reset"))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.menu = None;
                            let (scale, default) = Self::scale_of(&t1);
                            p.set_travel(&t1, default_travel(scale, default), "Default", cx);
                        }))
                        .into_any_element(),
                );
                items.push(
                    menu_item("menu-copy", tr("Copy value"))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.menu = None;
                            p.clipboard = Some(p.travel_of(&t2, cx));
                            cx.notify();
                        }))
                        .into_any_element(),
                );
                items.push(
                    menu_item("menu-paste", tr("Paste value"))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.menu = None;
                            match p.clipboard {
                                Some(v) => p.set_travel(&t3, v, "Paste value", cx),
                                None => cx.notify(),
                            }
                        }))
                        .into_any_element(),
                );
                if let Some(mt) = map_target(t) {
                    items.extend(self.midi_items(mt, cx));
                }
            }
            Menu::Choice { id, sec, key, choices } => {
                let (id, sec, key) = (*id, *sec, *key);
                for v in choices.iter().copied() {
                    items.push(
                        menu_item(format!("menu-choice-{v}"), choice_name(key, v))
                            .on_click(cx.listener(move |p, _, _, cx| {
                                p.menu = None;
                                p.set_choice(id, sec, key, v, cx);
                            }))
                            .into_any_element(),
                    );
                }
                items.extend(self.midi_items(midi::field(id, &["rack", sec, key]), cx));
            }
            Menu::Switch(id, path) => items.extend(self.midi_items(midi::field(*id, path), cx)),
            Menu::Effect(channel, insert) => {
                let (ch, ins) = (*channel, *insert);
                items.push(
                    menu_item("menu-effect-up", tr("Move up"))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.menu = None;
                            p.move_effect(ch, ins, -1, cx);
                        }))
                        .into_any_element(),
                );
                items.push(
                    menu_item("menu-effect-down", tr("Move down"))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.menu = None;
                            p.move_effect(ch, ins, 1, cx);
                        }))
                        .into_any_element(),
                );
                // The sidechain source of this insert (per insert): every channel that makes no loop
                let plugins = self.core.read(cx).port().plugins();
                let owner = chs.iter().find(|c| c.id == ch);
                let has_port = owner
                    .and_then(|c| c.inserts.iter().find(|i| i.id == ins))
                    .is_some_and(|i| plugins.iter().any(|p| p.id == i.plugin && p.sidechain));
                if has_port {
                    items.push(separator());
                    items.push(caption_row("Sidechain input from"));
                    for src in
                        chs.iter().filter(|c| c.id != ch && c.kind != ChannelKind::Master && !leads_to(chs, ch, c.id))
                    {
                        let sid = src.id;
                        let on = src.sends.iter().any(|s| s.to == ins && s.port.as_deref() == Some("sidechain"));
                        items.push(
                            menu_item(
                                format!("menu-effect-sc-{}", sid.number),
                                format!("{}{}", if on { "✓ " } else { "" }, src.name),
                            )
                            .on_click(cx.listener(move |p, _, _, cx| {
                                p.menu = None;
                                p.toggle_sidechain(sid, ins, cx);
                            }))
                            .into_any_element(),
                        );
                    }
                    items.push(separator());
                }
                items.push(
                    menu_item("menu-effect-remove", tr("Remove this plugin"))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.menu = None;
                            p.apply("Remove effect", vec![EditOp::Remove { id: Ref::Id(ins) }], cx);
                        }))
                        .into_any_element(),
                );
            }
            Menu::Strip(id) => self.strip_menu(*id, chs, &mut items, cx),
        }
        items
    }

    /// The MIDI items of a control's menu (I-21.2): learn, remove the mapping, the binding window
    fn midi_items(&self, target: lm_port::MapTarget, cx: &Context<Self>) -> Vec<AnyElement> {
        let me = cx.weak_entity();
        let close = move |cx: &mut gpui::App| {
            let _ = me.update(cx, |p, cx| {
                p.menu = None;
                cx.notify();
            });
        };
        midi::menu_items(&self.core, &self.workspace, target, close, cx)
    }

    /// The strip's menu (MixerChannelView::contextMenuEvent), with the output (free routing) in place of the
    /// legacy's sidechain menus
    fn strip_menu(&self, id: NodeId, chs: &[ChannelView], items: &mut Vec<AnyElement>, cx: &Context<Self>) {
        let Some(chv) = chs.iter().find(|c| c.id == id) else { return };
        let master = chv.kind == ChannelKind::Master;
        if !master {
            items.push(
                menu_item("menu-left", tr("Move left"))
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.menu = None;
                        p.move_channel(id, -1, cx);
                    }))
                    .into_any_element(),
            );
            items.push(
                menu_item("menu-right", tr("Move right"))
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.menu = None;
                        p.move_channel(id, 1, cx);
                    }))
                    .into_any_element(),
            );
        }
        items.push(
            menu_item("menu-rename", tr("Rename channel"))
                .on_click(cx.listener(move |p, _, w, cx| p.start_rename(id, w, cx)))
                .into_any_element(),
        );
        items.push(separator());
        if !master {
            items.push(
                menu_item("menu-remove", tr("Remove channel"))
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.menu = None;
                        p.remove_channel(id, cx);
                    }))
                    .into_any_element(),
            );
            items.push(separator());
        }
        items.push(
            menu_item("menu-remove-unused", tr("Remove unused channels"))
                .on_click(cx.listener(|p, _, _, cx| {
                    p.menu = None;
                    p.remove_unused(cx);
                }))
                .into_any_element(),
        );
        items.push(separator());
        items.push(
            menu_item("menu-arrange", tr("Arrange channels like the tracks"))
                .on_click(cx.listener(|p, _, _, cx| {
                    p.menu = None;
                    p.arrange_like_tracks(cx);
                }))
                .into_any_element(),
        );
        items.push(separator());
        // Color (ColorChooser's mixer palette: 8 hues in 6 shades), reset, a random one of it
        items.push(caption_row("Color"));
        items.push(
            menu_item("menu-color-reset", tr("Reset"))
                .on_click(cx.listener(move |p, _, _, cx| {
                    p.menu = None;
                    p.apply("Color", vec![EditOp::set(id, &["color"], Value::NULL)], cx);
                }))
                .into_any_element(),
        );
        items.push(
            menu_item("menu-color-random", tr("Pick random"))
                .on_click(cx.listener(move |p, _, _, cx| {
                    p.menu = None;
                    let n = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.subsec_nanos()) as usize;
                    let color = desk::mixer_palette()[n % 48];
                    p.apply("Color", vec![EditOp::set(id, &["color"], Value::text(format!("#{color:06x}")))], cx);
                }))
                .into_any_element(),
        );
        items.push(
            div()
                .px(px(10.))
                .py(px(3.))
                .grid()
                .grid_cols(8)
                .gap(px(2.))
                .children(desk::mixer_palette().into_iter().enumerate().map(|(n, color)| {
                    div()
                        .id(SharedString::from(format!("menu-color-{n}")))
                        .debug_selector(move || format!("menu-color-{n}"))
                        .size(px(14.))
                        .rounded(px(2.))
                        .bg(rgb(color))
                        .cursor_pointer()
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.menu = None;
                            p.apply(
                                "Color",
                                vec![EditOp::set(id, &["color"], Value::text(format!("#{color:06x}")))],
                                cx,
                            );
                        }))
                }))
                .into_any_element(),
        );
        if !master && (self.multi.len() + 1) > 1 {
            items.push(separator());
            items.push(
                menu_item("menu-group-selected", tr("Group the selected channels"))
                    .on_click(cx.listener(|p, _, _, cx| {
                        p.menu = None;
                        p.group_selected(cx);
                    }))
                    .into_any_element(),
            );
        }
        // Sidechain, either way (the legacy's "Sidechain input from" and "Send as sidechain to"): a send to an
        // insert's sidechain input; the channels that would make a loop are left out
        if !master {
            let plugins = self.core.read(cx).port().plugins();
            let has_sidechain =
                |c: &ChannelView| c.inserts.iter().any(|i| plugins.iter().any(|p| p.id == i.plugin && p.sidechain));
            let feeds = |from: &ChannelView, to: &ChannelView| {
                from.sends
                    .iter()
                    .any(|s| s.port.as_deref() == Some("sidechain") && to.inserts.iter().any(|i| i.id == s.to))
            };
            if has_sidechain(chv) {
                items.push(separator());
                items.push(caption_row("Sidechain input from"));
                for src in
                    chs.iter().filter(|c| c.id != id && c.kind != ChannelKind::Master && !leads_to(chs, id, c.id))
                {
                    let sid = src.id;
                    let Some(target) = self.sidechain_insert(chv, cx) else { continue };
                    let on = feeds(src, chv);
                    items.push(
                        menu_item(
                            format!("menu-sc-from-{}", sid.number),
                            format!("{}{}", if on { "✓ " } else { "" }, src.name),
                        )
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.menu = None;
                            p.toggle_sidechain(sid, target, cx);
                        }))
                        .into_any_element(),
                    );
                }
            }
            let targets: Vec<&ChannelView> =
                chs.iter().filter(|c| c.id != id && has_sidechain(c) && !leads_to(chs, c.id, id)).collect();
            if !targets.is_empty() {
                items.push(separator());
                items.push(caption_row("Send as sidechain to"));
                for to in targets {
                    let tid = to.id;
                    let Some(target) = self.sidechain_insert(to, cx) else { continue };
                    let on = feeds(chv, to);
                    items.push(
                        menu_item(
                            format!("menu-sc-to-{}", tid.number),
                            format!("{}{}", if on { "✓ " } else { "" }, to.name),
                        )
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.menu = None;
                            p.toggle_sidechain(id, target, cx);
                        }))
                        .into_any_element(),
                    );
                }
            }
        }
        if master {
            return;
        }
        // Track channels: the main pair and the auxiliary pairs
        items.push(separator());
        items.push(caption_row("Track channels"));
        items.push(
            div()
                .px(px(10.))
                .py(px(2.))
                .flex()
                .gap(px(3.))
                .children([2i64, 4, 6, 8, 12, 16].into_iter().map(|n| {
                    let on = chv.channels == n;
                    div()
                        .id(SharedString::from(format!("menu-channels-{n}")))
                        .debug_selector(move || format!("menu-channels-{n}"))
                        .px(px(4.))
                        .rounded(px(2.))
                        .bg(c(if on { THEME.blue } else { THEME.bg4 }))
                        .text_size(px(10.))
                        .cursor_pointer()
                        .child(n.to_string())
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.menu = None;
                            p.apply("Track channels", vec![EditOp::set(id, &["channels"], Value::int(n))], cx);
                        }))
                }))
                .into_any_element(),
        );
        // Output (free routing): every channel that makes no loop, and the master
        items.push(separator());
        items.push(caption_row("Output"));
        for to in chs.iter().filter(|c| c.id != id && !leads_to(chs, c.id, id)) {
            let tid = to.id;
            let on = chv.output == Some(tid);
            items.push(
                menu_item(format!("menu-output-{}", tid.number), format!("{}{}", if on { "✓ " } else { "" }, to.name))
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.menu = None;
                        p.apply("Output", vec![EditOp::set(id, &["output"], Value::text(tid.to_string()))], cx);
                    }))
                    .into_any_element(),
            );
        }
        if chv.kind == ChannelKind::Group {
            let collapsed = self.collapsed.contains(&id);
            items.push(separator());
            let label = if collapsed { tr("Show the group's channels") } else { tr("Hide the group's channels") };
            items.push(
                menu_item("menu-group", label)
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.menu = None;
                        if !p.collapsed.remove(&id) {
                            p.collapsed.insert(id);
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
    }
}

// -----------------------------------------------------------------------------------------------------------------
// The page

impl Render for MixingPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let chs = channels(self.core.read(cx));
        let analog = is_analog(self.core.read(cx));
        if self.selected.is_none_or(|s| !chs.iter().any(|c| c.id == s)) {
            self.selected = chs.iter().find(|c| c.kind == ChannelKind::Master).map(|c| c.id);
        }
        // the peaks since the last click
        {
            let core = self.core.read(cx);
            for c in &chs {
                let db = peak_db(core, c.id);
                let e = self.peaks.entry(c.id).or_insert(-100.0);
                *e = e.max(db);
            }
        }
        self.read_engine(&chs, cx);
        // the fader's height: what is left under the upper area, the pan, the peak and the name
        let page_h = viewport.height.as_f32() - crate::workspace::CHROME_H - 4.0;
        let pan_h = if analog { desk::KNOB_D + desk::KNOB_LABEL_H + 4.0 } else { 21.0 };
        self.send_rows = chs.iter().filter(|c| c.kind == ChannelKind::Return).count().div_ceil(2).max(1);
        let upper = self.upper_h(analog);
        let fixed = 8.0 + upper + pan_h + NAME_H + 12.0 + 9.0 + 2.0 * 6.0;
        let fader_room = page_h - fixed;
        let min_h = fixed + 116.0;
        self.fader_h = fader_room.max(116.0);
        let hidden = self.hidden(&chs);
        let mut number = 0;
        let normal: Vec<(Option<usize>, &ChannelView)> = chs
            .iter()
            .filter(|c| matches!(c.kind, ChannelKind::Normal | ChannelKind::Group))
            .map(|c| {
                number += 1;
                (Some(number), c)
            })
            .filter(|(_, c)| !hidden.contains(&c.id))
            .collect();
        // the horizontal scroll, kept within the strips; only the strips in view are built
        let area_w = self.area.get().size.width.as_f32();
        let total = (normal.len() + 1) as f32 * desk::STRIP_W;
        if area_w > 0.0 {
            self.scroll = self.scroll.clamp(0.0, (total - area_w).max(0.0));
        } else {
            self.scroll = self.scroll.clamp(0.0, total);
        }
        let first = (self.scroll / desk::STRIP_W).floor() as usize;
        let fit = (area_w.max(viewport.width.as_f32()) / desk::STRIP_W).ceil() as usize + 1;
        let offset = -(self.scroll - first as f32 * desk::STRIP_W);
        let strips: Vec<AnyElement> = normal
            .iter()
            .skip(first)
            .take(fit)
            .map(|(n, c)| self.strip(c, *n, &chs, analog, cx).into_any_element())
            .collect();
        let show_ghost = first + fit > normal.len();
        let returns: Vec<AnyElement> = chs
            .iter()
            .filter(|c| c.kind == ChannelKind::Return)
            .map(|c| self.strip(c, None, &chs, analog, cx).into_any_element())
            .collect();
        let master: Vec<AnyElement> = chs
            .iter()
            .filter(|c| c.kind == ChannelKind::Master)
            .map(|c| self.strip(c, None, &chs, analog, cx).into_any_element())
            .collect();
        let area = self.area.clone();
        let page = self.page.clone();
        let origin = page.get().origin;
        let size = page.get().size;
        let menu = self.menu.as_ref().map(|(m, p)| (self.render_menu(m, &chs, cx), *p));
        let hint = self.hint.clone();
        let right = self.right_column(&chs, analog, cx);
        // below the legacy's minimum height (MixerView: the tallest strip) the page scrolls, it never squeezes
        let content_h = page_h.max(min_h);
        div().id("mixing-page").size_full().overflow_y_scroll().child(
            div()
                .id("mixing")
                .w_full()
                .h(px(content_h))
                .relative()
                .flex()
                .flex_row()
                .pt(px(4.))
                .bg(desk::page_bg())
                .child(canvas(move |b, _, _| page.set(b), |_, _, _, _| {}).absolute().size_full())
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
                .on_action(cx.listener(|p, _: &MoveChannelLeft, _, cx| {
                    if let Some(id) = p.selected {
                        p.move_channel(id, -1, cx)
                    }
                }))
                .on_action(cx.listener(|p, _: &MoveChannelRight, _, cx| {
                    if let Some(id) = p.selected {
                        p.move_channel(id, 1, cx)
                    }
                }))
                .on_action(cx.listener(|p, _: &RemoveChannel, _, cx| {
                    if let Some(id) = p.selected {
                        p.remove_channel(id, cx)
                    }
                }))
                .on_action(cx.listener(|p, _: &RenameChannel, w, cx| {
                    // Enter or F2: renames the selected channel, or ends the renaming (MixerChannelView::keyPressEvent)
                    if p.renaming.is_some() {
                        p.finish_rename(cx)
                    } else if let Some(id) = p.selected {
                        p.start_rename(id, w, cx)
                    }
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|p, _, window, cx| {
                        window.focus(&p.focus, cx);
                        if p.menu.take().is_some() {
                            cx.notify();
                        }
                    }),
                )
                .on_mouse_move(cx.listener(|p, ev: &MouseMoveEvent, _, cx| p.drag_to(ev, cx)))
                .on_mouse_up(MouseButton::Left, cx.listener(|p, _: &MouseUpEvent, _, cx| p.release(cx)))
                .child(
                    div()
                        .id("channels")
                        .flex_1()
                        .h_full()
                        .relative()
                        .overflow_hidden()
                        .on_scroll(cx.listener(Self::on_scroll))
                        .child(canvas(move |b, _, _| area.set(b), |_, _, _, _| {}).absolute().size_full())
                        .child(
                            div()
                                .absolute()
                                .left(px(offset))
                                .top(px(0.))
                                .h_full()
                                .flex()
                                .flex_row()
                                .children(strips)
                                .when(show_ghost, |d| {
                                    d.child(
                                        desk::ghost("ghost")
                                            .debug_selector(|| "ghost".into())
                                            .on_click(cx.listener(|p, _, _, cx| p.add_channel(cx))),
                                    )
                                }),
                        ),
                )
                .child(div().h_full().flex().flex_row().children(returns))
                .child(div().h_full().flex().flex_row().children(master))
                .child(right)
                .children(menu.map(|(items, pos)| {
                    let x = (pos.x - origin.x).min(size.width - px(250.)).max(px(0.));
                    let y = (pos.y - origin.y).min(size.height - px(420.)).max(px(0.));
                    div()
                        .id("menu")
                        .absolute()
                        .left(x)
                        .top(y)
                        .w(px(240.))
                        .max_h(px(420.))
                        .overflow_y_scroll()
                        .occlude()
                        .py(px(4.))
                        .rounded(px(6.))
                        .bg(c(THEME.bg3))
                        .border_1()
                        .border_color(c(THEME.line2))
                        .shadow_lg()
                        .text_color(c(THEME.text))
                        .children(items)
                }))
                .children(self.confirm_remove.map(|(id, dont_ask)| self.render_confirm_remove(id, dont_ask, cx)))
                .children(hint.map(|(text, pos)| {
                    div()
                        .absolute()
                        .left(pos.x - origin.x + px(14.))
                        .top(pos.y - origin.y - px(8.))
                        .px(px(4.))
                        .py(px(1.))
                        .bg(rgb(0x0c0d0f))
                        .border_1()
                        .border_color(rgb(col::DIM))
                        .text_size(px(10.))
                        .text_color(rgb(col::TEXT))
                        .child(text)
                })),
        )
    }
}
