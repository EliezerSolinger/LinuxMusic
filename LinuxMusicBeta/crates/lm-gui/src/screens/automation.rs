//! Owner: agent B. The automation editor: the curve of one automation lane (F-AUT-001..011, F-CLP-081/082, P-27,
//! I-21.5).
//!
//! The legacy's screen (legado/src/gui/editors/AutomationEditor.cpp), kept: one toolbar row (play and stop; the
//! modes draw, erase and draw out-values; flip vertically and horizontally; the progression discrete, linear or
//! cubic with its tension; the zoom, quantization and ghost combos), the value scale on the left, the grid and the
//! curve. What the new model changes is in decisoes-autonomas.md (B-AE-*): the tangents are the points' `curve`
//! (a handle in the middle of a segment), the lane's mode and "re-arm" (I-21.5), the free pencil, selection,
//! scaling and thinning.
//!
//! The lane comes as the screen's target (`a:`), else the first lane of the project. Its points are a list in the
//! tree; a point's `shape`, `curve` and `tension` describe the segment that ENDS at it, `out` is the value leaving
//! it (P-27). Every edit sets the lane's whole `points` list (one transaction or one gesture).

use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Bounds, Context, CursorStyle, Div, FocusHandle, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PathBuilder, PinchEvent, Pixels, Point, Render, ScrollEvent, SharedString, Stateful, Window, canvas, div, fill,
    point, prelude::*, px, size,
};
use lm_gui_plugins::Gesture;
use lm_port::{CorePort, EditOp, Feed, HostTime, Label, NodeId, ParamRange, RangeScale, Topic, Value};

use super::ScreenCx;
use super::piano_roll::kit::{
    self, MenuItem, ValueEntry, dropdown, dropdown_icon, icon_button, label, paint_text, popup, sep, tip, with_alpha,
};
use super::piano_roll::music::{self, BAR, BEAT, Clip, GRIDS, NoteMap};
use crate::actions::{
    AuCopy, AuCut, AuDelete, AuHome, AuLeft, AuPaste, AuPlay, AuRight, AuSelectAll, AuSelectNone, AuToolDraw,
    AuToolErase, AuToolOut, AuToolPencil, AuToolSelect,
};
use crate::keymap;
use crate::model;
use crate::ui::{THEME, button, c, tr, track_color, trf};
use crate::view;

const CTX: &str = "Automation";

/// The mouse actions of this screen (`keymap/mouse_defs.rs`): the gestures are the user's, never read from the event
fn holds(id: &str, m: &gpui::Modifiers) -> bool {
    keymap::holds(CTX, id, m)
}

fn fires(id: &str, button: MouseButton, clicks: usize, m: &gpui::Modifiers) -> bool {
    keymap::fires(CTX, id, button, clicks, m)
}

fn pick_wheel(ids: &[&'static str], m: &gpui::Modifiers) -> Option<&'static str> {
    keymap::pick_wheel(CTX, ids, m)
}

const ROW_H: f32 = 26.0;
const TOP_MARGIN: f32 = 16.0;
const VALUES_W: f32 = 64.0;
const POINT_R: f32 = 4.0;
const TPP_100: f64 = 30.0;
const ZOOM_X: [f64; 7] = [0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
/// The legacy's vertical zoom: automatic (fits the points), then fixed steps of the range
const ZOOM_Y: [f64; 6] = [0.0, 0.25, 0.5, 1.0, 2.0, 4.0];

// ---------------------------------------------------------------------------------------------------------------
// The lane as the editor sees it

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Step,
    Curve,
    Smooth,
}

impl Shape {
    fn key(self) -> &'static str {
        match self {
            Shape::Step => "step",
            Shape::Curve => "curve",
            Shape::Smooth => "smooth",
        }
    }
    fn parse(s: Option<&str>) -> Shape {
        match s {
            Some("step") => Shape::Step,
            Some("smooth") => Shape::Smooth,
            _ => Shape::Curve,
        }
    }
}

/// A point: its fields of the segment that ENDS here (`shape`, `curve`, `tension`), and the value leaving it
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    pub tick: i64,
    pub value: f64,
    pub shape: Shape,
    pub curve: f64,
    pub tension: f64,
    pub out: Option<f64>,
}

impl Pt {
    pub fn new(tick: i64, value: f64, shape: Shape) -> Pt {
        Pt { tick, value, shape, curve: 0.0, tension: 0.5, out: None }
    }
    pub fn leaving(&self) -> f64 {
        self.out.unwrap_or(self.value)
    }
    fn read(v: &Value) -> Pt {
        Pt {
            tick: model::int(v, "tick", 0),
            value: model::real(v, "value", 0.0),
            shape: Shape::parse(v.get("shape").and_then(Value::as_text)),
            curve: model::real(v, "curve", 0.0),
            tension: model::real(v, "tension", 0.5),
            out: v.get("out").and_then(Value::as_real),
        }
    }
    fn to_value(self) -> Value {
        let r = |x: f64| Value::real(x).unwrap_or(Value::NULL);
        let mut f = vec![("tick", Value::int(self.tick.max(0))), ("value", r(self.value))];
        if self.curve != 0.0 {
            f.push(("curve", r(self.curve.clamp(-1.0, 1.0))));
        }
        if self.shape != Shape::Curve {
            f.push(("shape", Value::text(self.shape.key())));
        }
        if self.shape == Shape::Smooth && self.tension != 0.5 {
            f.push(("tension", r(self.tension.clamp(0.0, 1.0))));
        }
        if let Some(o) = self.out {
            f.push(("out", r(o)));
        }
        Value::object(f)
    }
}

/// How the values sit on the height: the parameter's own law, as the core serves it (`CorePort::param_range`):
/// the same law the compiler uses for mapped MIDI and automation, so the curve drawn is the curve played
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range(pub ParamRange);

impl Range {
    pub fn norm(self, v: f64) -> f64 {
        self.0.normalize(v as f32) as f64
    }
    pub fn denorm(self, t: f64) -> f64 {
        self.0.to_units(t.clamp(0.0, 1.0) as f32) as f64
    }
    fn format(self, v: f64) -> String {
        match self.0.scale {
            RangeScale::Gain => view::format_gain(v),
            RangeScale::Stepped => format!("{v:.0}"),
            _ => format!("{v:.2}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Lane {
    pub id: NodeId,
    pub owner: String,
    pub param: String,
    /// the target's node: with `param`, how the telemetry names the lane (`hand_won`)
    pub target: Option<NodeId>,
    pub color: u32,
    pub mode: String,
    pub points: Arc<Vec<Pt>>,
    /// the parameter's law; `None` when the core has none for it (the editor says so and does not edit)
    pub range: Option<Range>,
}

impl Lane {
    /// Every lane of the project: the tracks', the desk channels' and the master's, each with its parameter's law
    pub fn all(port: &dyn CorePort) -> Vec<Lane> {
        let root = port.root();
        let mut out = Vec::new();
        let mut owners: Vec<(&Value, u32)> =
            model::list(root, "tracks").enumerate().map(|(i, t)| (t, track_color(i))).collect();
        owners.extend(
            model::at(root, &["mixer", "channels"]).into_iter().flat_map(Value::items).map(|c| (c, THEME.cyan)),
        );
        owners.extend(model::at(root, &["mixer", "master"]).map(|m| (m, THEME.yellow)));
        for (o, color) in owners {
            for a in model::list(o, "automation") {
                let Some(id) = model::id_of(a) else { continue };
                let param = a.get("target").map(|t| model::text(t, "param").to_string()).unwrap_or_default();
                let mut points: Vec<Pt> = model::list(a, "points").map(Pt::read).collect();
                points.sort_by_key(|p| p.tick);
                let target = a.get("target").and_then(|t| model::ref_of(t, "node"));
                let range = target.and_then(|n| port.param_range(n, &param)).map(Range);
                out.push(Lane {
                    id,
                    owner: model::text(o, "name").to_string(),
                    target,
                    param,
                    color,
                    mode: a.get("mode").and_then(Value::as_text).unwrap_or("read").to_string(),
                    points: Arc::new(points),
                    range,
                });
            }
        }
        out
    }

    pub fn read(port: &dyn CorePort, id: NodeId) -> Option<Lane> {
        Lane::all(port).into_iter().find(|l| l.id == id)
    }

    /// The ids of the lanes (no laws needed): the first one opens when no lane is asked for
    pub fn ids(root: &Value) -> Vec<NodeId> {
        let tracks = model::list(root, "tracks");
        let channels = model::at(root, &["mixer", "channels"]).into_iter().flat_map(Value::items);
        let master = model::at(root, &["mixer", "master"]).into_iter();
        tracks
            .chain(channels)
            .chain(master)
            .flat_map(|o| model::list(o, "automation"))
            .filter_map(model::id_of)
            .collect()
    }

    /// The law to draw and edit with (a lane without one is not drawn: see `range`)
    pub fn law(&self) -> Range {
        self.range.unwrap_or(Range(ParamRange::linear(0.0, 1.0)))
    }

    /// The value at `tick` (the compiler's rule: a segment's shape is the one of the point it ends at)
    pub fn value_at(&self, tick: f64) -> f64 {
        value_at(&self.points, tick, self.law())
    }
}

/// The bend of a curve segment: t 0..1 -> 0..1; curve > 0 rises fast then slows, < 0 the opposite
pub fn bend(t: f64, curve: f64) -> f64 {
    if curve.abs() < 1e-6 { t } else { t.powf(4f64.powf(-curve * 1.5)) }
}

pub fn value_at(pts: &[Pt], tick: f64, range: Range) -> f64 {
    let Some(first) = pts.first() else { return 0.0 };
    if tick < first.tick as f64 {
        return first.value;
    }
    for i in 0..pts.len() {
        let a = pts[i];
        let Some(&b) = pts.get(i + 1) else { return a.leaving() };
        if tick < b.tick as f64 {
            let t = (tick - a.tick as f64) / (b.tick - a.tick).max(1) as f64;
            let (va, vb) = (range.norm(a.leaving()), range.norm(b.value));
            let v = match b.shape {
                Shape::Step => va,
                Shape::Curve => va + (vb - va) * bend(t, b.curve),
                Shape::Smooth => {
                    // a cardinal spline through the neighbours, with the segment's tension
                    let span = (b.tick - a.tick).max(1) as f64;
                    let k = 1.0 - b.tension;
                    let prev = if i > 0 { range.norm(pts[i - 1].leaving()) } else { va };
                    let next = pts.get(i + 2).map_or(vb, |p| range.norm(p.value));
                    let tp = if i > 0 { pts[i - 1].tick as f64 } else { a.tick as f64 - span };
                    let tn = pts.get(i + 2).map_or(b.tick as f64 + span, |p| p.tick as f64);
                    let ma = k * (vb - prev) / (b.tick as f64 - tp).max(1.0) * span;
                    let mb = k * (next - va) / (tn - a.tick as f64).max(1.0) * span;
                    let (t2, t3) = (t * t, t * t * t);
                    (2.0 * t3 - 3.0 * t2 + 1.0) * va
                        + (t3 - 2.0 * t2 + t) * ma
                        + (-2.0 * t3 + 3.0 * t2) * vb
                        + (t3 - t2) * mb
                }
            };
            return range.denorm(v.clamp(0.0, 1.0));
        }
    }
    pts[pts.len() - 1].leaving()
}

// ---------------------------------------------------------------------------------------------------------------
// The editor

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Draw,
    Erase,
    /// the legacy's "draw outValues": the jump of a point (P-27)
    Out,
    /// the free pencil (I-21.5)
    Pencil,
    Select,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Lane,
    Mode,
    ZoomX,
    ZoomY,
    Grid,
    Ghost,
    Tools,
}

const MODES: [(&str, &str); 5] =
    [("off", "Off"), ("read", "Read"), ("touch", "Touch"), ("latch", "Latch"), ("write", "Write")];

enum Drag {
    /// the selection moving (a new point is a selection of one); `uids` are the points' places before the drag
    Move {
        g: Option<Gesture>,
        origin: Arc<Vec<Pt>>,
        from: (f64, f64),
    },
    Out {
        g: Option<Gesture>,
        i: usize,
        origin: Arc<Vec<Pt>>,
    },
    Curve {
        g: Option<Gesture>,
        i: usize,
        origin: Arc<Vec<Pt>>,
        from_y: f32,
        rising: bool,
    },
    Rect {
        from: Point<Pixels>,
        to: Point<Pixels>,
    },
    Erase {
        gone: BTreeSet<usize>,
    },
    Pencil {
        stroke: Vec<(i64, f64)>,
    },
    Tension {
        g: Option<Gesture>,
        origin: Arc<Vec<Pt>>,
        from_y: f32,
        start: f64,
    },
}

pub struct AutomationEditor {
    scx: ScreenCx,
    focus: FocusHandle,
    feed: Feed,
    pub lane: Option<NodeId>,
    ghost: Option<NodeId>,
    pub tool: Tool,
    /// the progression of new points (the legacy's discrete, linear, cubic)
    shape: Shape,
    tension: f64,
    grid: usize,
    zoom_x: usize,
    zoom_y: usize,
    pub scroll: f64,
    pub tpp: f64,
    pub sel: BTreeSet<usize>,
    drag: Option<Drag>,
    menu: Option<(Menu, Point<Pixels>)>,
    entry: Option<(usize, ValueEntry)>,
    clipboard: Vec<Pt>,
    cursor: i64,
    last_added: Option<i64>,
    /// the last lane edit the core refused (shown in the toolbar until the next one)
    error: Option<String>,
    playing: Option<(HostTime, i64)>,
    ticks_per_sec: f64,
    curve_bounds: Rc<Cell<Bounds<Pixels>>>,
    ruler_bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl AutomationEditor {
    pub fn new(scx: ScreenCx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (feed, lane, tps) = scx.core.update(cx, |core, _| {
            let root = core.port().root();
            let lane = scx.target.filter(|t| t.kind == b'a').or_else(|| Lane::ids(root).first().copied());
            let map = model::tempo_map(root);
            let tps = map.tick_at(map.sample_rate() as i64) as f64;
            (core.port_mut().feed(Topic::All), lane, tps.max(1.0))
        });
        AutomationEditor {
            focus: cx.focus_handle(),
            feed,
            lane,
            ghost: None,
            tool: Tool::Draw,
            shape: Shape::Curve,
            tension: 0.5,
            grid: 4,
            zoom_x: 3,
            zoom_y: 0,
            scroll: 0.0,
            tpp: TPP_100,
            sel: BTreeSet::new(),
            drag: None,
            menu: None,
            entry: None,
            clipboard: Vec::new(),
            cursor: 0,
            last_added: None,
            error: None,
            playing: None,
            ticks_per_sec: tps,
            curve_bounds: Rc::new(Cell::new(Bounds::default())),
            ruler_bounds: Rc::new(Cell::new(Bounds::default())),
            scx,
        }
    }

    pub fn lane(&self, cx: &gpui::App) -> Option<Lane> {
        self.lane.and_then(|id| Lane::read(self.scx.core.read(cx).port(), id))
    }

    fn grid_ticks(&self) -> i64 {
        GRIDS[self.grid].1
    }

    /// The visible part of the normalized range: automatic (the points' span) or by the zoom
    fn v_range(&self, lane: &Lane) -> (f64, f64) {
        if self.zoom_y == 0 {
            let vals: Vec<f64> =
                lane.points.iter().flat_map(|p| [lane.law().norm(p.value), lane.law().norm(p.leaving())]).collect();
            if vals.is_empty() {
                return (0.0, 1.0);
            }
            let (lo, hi) = vals.iter().fold((1.0f64, 0.0f64), |(a, b), v| (a.min(*v), b.max(*v)));
            let pad = ((hi - lo) * 0.15).max(0.05);
            ((lo - pad).max(0.0), (hi + pad).min(1.0))
        } else {
            let span = (1.0 / ZOOM_Y[self.zoom_y]).min(1.0);
            (0.0, span)
        }
    }

    fn h(&self) -> f32 {
        self.curve_bounds.get().size.height.as_f32().max(1.0)
    }
    pub fn tick_to_x(&self, t: f64) -> f32 {
        ((t - self.scroll) / self.tpp) as f32
    }
    fn x_to_tick(&self, x: f32) -> f64 {
        self.scroll + x as f64 * self.tpp
    }
    fn y_to_norm(&self, y: f32, vr: (f64, f64)) -> f64 {
        let t = (1.0 - (y - 4.0) / (self.h() - 8.0)) as f64;
        vr.0 + t * (vr.1 - vr.0)
    }
    pub fn norm_to_y(&self, v: f64, vr: (f64, f64)) -> f32 {
        4.0 + (1.0 - ((v - vr.0) / (vr.1 - vr.0).max(1e-9)) as f32) * (self.h() - 8.0)
    }

    fn local(b: &Rc<Cell<Bounds<Pixels>>>, p: Point<Pixels>) -> (f32, f32) {
        let o = b.get().origin;
        ((p.x - o.x).as_f32(), (p.y - o.y).as_f32())
    }

    /// The point under (x, y), and whether the hit was on its out value
    fn point_at(&self, lane: &Lane, x: f32, y: f32) -> Option<(usize, bool)> {
        let vr = self.v_range(lane);
        let mut best: Option<(usize, bool, f32)> = None;
        for (i, p) in lane.points.iter().enumerate() {
            let px_ = self.tick_to_x(p.tick as f64);
            if (px_ - x).abs() > 7.0 {
                continue;
            }
            for (out, v) in [(false, p.value), (true, p.leaving())] {
                if out && p.out.is_none() {
                    continue;
                }
                let d = (self.norm_to_y(lane.law().norm(v), vr) - y).abs() + (px_ - x).abs();
                if d < 10.0 && best.is_none_or(|b| d < b.2) {
                    best = Some((i, out, d));
                }
            }
        }
        best.map(|b| (b.0, b.1))
    }

    /// The curve segment whose middle handle is under (x, y): the index of the point it ends at
    fn handle_at(&self, lane: &Lane, x: f32, y: f32) -> Option<(usize, bool)> {
        let vr = self.v_range(lane);
        for i in 1..lane.points.len() {
            let (a, b) = (lane.points[i - 1], lane.points[i]);
            if b.shape != Shape::Curve {
                continue;
            }
            let mid = (a.tick + b.tick) as f64 / 2.0;
            let (hx, hy) = (self.tick_to_x(mid), self.norm_to_y(lane.law().norm(lane.value_at(mid)), vr));
            if (hx - x).abs() < 6.0 && (hy - y).abs() < 6.0 {
                return Some((i, b.value >= a.leaving()));
            }
        }
        None
    }

    // --- changing, through the port ----------------------------------------------------------------------------

    fn points_op(&self, pts: &[Pt]) -> Option<EditOp> {
        let id = self.lane?;
        Some(EditOp::set(id, &["points"], Value::list(pts.iter().map(|p| p.to_value()))))
    }

    /// Sorts the new list by tick; the selection follows its points (by their place in `pts`)
    fn sorted(pts: Vec<Pt>, sel: &BTreeSet<usize>) -> (Vec<Pt>, BTreeSet<usize>) {
        let mut v: Vec<(usize, Pt)> = pts.into_iter().enumerate().collect();
        v.sort_by_key(|(i, p)| (p.tick, *i));
        let new_sel = v.iter().enumerate().filter(|(_, (i, _))| sel.contains(i)).map(|(k, _)| k).collect();
        (v.into_iter().map(|(_, p)| p).collect(), new_sel)
    }

    fn transact(&mut self, label: &'static str, pts: Vec<Pt>, cx: &mut Context<Self>) {
        let (pts, sel) = Self::sorted(pts, &self.sel);
        let Some(op) = self.points_op(&pts) else { return };
        self.sel = sel;
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| pt.apply(Label::new(label), vec![op]));
        });
        cx.notify();
    }

    fn gesture(&mut self, g: &mut Option<Gesture>, label: &'static str, pts: Vec<Pt>, cx: &mut Context<Self>) {
        let (pts, sel) = Self::sorted(pts, &self.sel);
        let Some(op) = self.points_op(&pts) else { return };
        self.sel = sel;
        self.scx.core.update(cx, |core, _| {
            // a gesture closed under the drag (the release was lost) refuses the edits: the drag writes nothing more
            let open = *g.get_or_insert_with(|| core.begin_gesture(Label::new(label)));
            core.gesture_apply(open, vec![op]);
        });
    }

    fn commit(&self, g: Option<Gesture>, cx: &mut Context<Self>) {
        if let Some(g) = g {
            self.scx.core.update(cx, |core, _| core.gesture_commit(g));
        }
    }

    fn snap_tick(&self, t: f64, free: bool) -> i64 {
        if free { t.round().max(0.0) as i64 } else { music::snap(t, self.grid_ticks()).max(0) }
    }

    pub fn open_lane(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.lane = Some(id);
        self.sel.clear();
        self.last_added = None;
        cx.notify();
    }

    // --- the mouse ---------------------------------------------------------------------------------------------

    fn curve_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.menu = None;
        self.entry = None;
        let Some(lane) = self.lane(cx) else { return };
        let vr = self.v_range(&lane);
        let (x, y) = Self::local(&self.curve_bounds, ev.position);
        let tick = self.x_to_tick(x);
        let hit = self.point_at(&lane, x, y);
        let mut pts = (*lane.points).clone();
        if fires("au.erase", ev.button, ev.click_count, &ev.modifiers) {
            // the right button: an out value goes back to the arrival value; else erase (dragging erases more)
            if let Some((i, true)) = hit {
                pts[i].out = None;
                self.transact("Remove the jump", pts, cx);
            } else {
                self.drag = Some(Drag::Erase { gone: hit.map(|h| h.0).into_iter().collect() });
            }
            cx.notify();
            return;
        }
        if fires("au.point.entry", ev.button, ev.click_count, &ev.modifiers)
            && let Some((i, out)) = hit
        {
            // a double click asks the value (F-AUT-005)
            let v = if out { pts[i].leaving() } else { pts[i].value };
            self.sel = [i].into_iter().collect();
            self.entry =
                Some((i + if out { 1 << 20 } else { 0 }, ValueEntry::new(format!("{v:.3}"), ev.position, window, cx)));
            cx.notify();
            return;
        }
        let tool = if holds("au.select_override", &ev.modifiers) { Tool::Select } else { self.tool };
        match tool {
            Tool::Erase => self.drag = Some(Drag::Erase { gone: hit.map(|h| h.0).into_iter().collect() }),
            Tool::Pencil => {
                let v = self.y_to_norm(y, vr);
                self.drag = Some(Drag::Pencil { stroke: vec![(tick.round().max(0.0) as i64, v)] });
            }
            Tool::Out => {
                if let Some((i, _)) = hit {
                    self.drag = Some(Drag::Out { g: None, i, origin: lane.points.clone() });
                }
            }
            Tool::Draw | Tool::Select => {
                if let Some((i, out)) = hit {
                    if out {
                        self.drag = Some(Drag::Out { g: None, i, origin: lane.points.clone() });
                    } else {
                        if !self.sel.contains(&i) {
                            if !holds("au.extend", &ev.modifiers) {
                                self.sel.clear();
                            }
                            self.sel.insert(i);
                        }
                        self.drag = Some(Drag::Move {
                            g: None,
                            origin: lane.points.clone(),
                            from: (tick, self.y_to_norm(y, vr)),
                        });
                    }
                } else if let Some((i, rising)) = self.handle_at(&lane, x, y) {
                    self.drag = Some(Drag::Curve { g: None, i, origin: lane.points.clone(), from_y: y, rising });
                } else if tool == Tool::Select {
                    let p = point(px(x), px(y));
                    if !holds("au.extend", &ev.modifiers) {
                        self.sel.clear();
                    }
                    self.drag = Some(Drag::Rect { from: p, to: p });
                } else {
                    let t = self.snap_tick(tick, holds("au.free", &ev.modifiers));
                    let v = lane.law().denorm(self.y_to_norm(y, vr));
                    let mut p = Pt::new(t, v, self.shape);
                    p.tension = self.tension;
                    match (
                        holds("au.draw.line", &ev.modifiers),
                        self.last_added.and_then(|lt| pts.iter().find(|q| q.tick == lt).copied()),
                    ) {
                        (true, Some(a)) => {
                            // Shift+click: a line of points from the last one, one per grid step (F-AUT-001)
                            let step = self.grid_ticks();
                            let (t0, t1) = (a.tick.min(t), a.tick.max(t));
                            let (va, vb) = if a.tick <= t { (a.value, v) } else { (v, a.value) };
                            pts.retain(|q| q.tick <= t0 || q.tick > t1);
                            let mut k = t0 + step;
                            while k <= t1 {
                                let f = (k - t0) as f64 / (t1 - t0).max(1) as f64;
                                pts.push(Pt::new(
                                    k,
                                    lane.law()
                                        .denorm(lane.law().norm(va) + (lane.law().norm(vb) - lane.law().norm(va)) * f),
                                    Shape::Curve,
                                ));
                                k += step;
                            }
                            self.sel.clear();
                            self.transact("Draw a line", pts, cx);
                            self.last_added = Some(t);
                        }
                        _ => {
                            // a new point (a point at the same tick is replaced), then dragged: one transaction, one
                            // gesture
                            pts.retain(|q| q.tick != t);
                            pts.push(p);
                            self.sel = [pts.len() - 1].into_iter().collect();
                            self.transact("Add a point", pts, cx);
                            self.last_added = Some(t);
                            let origin = self.lane(cx).map(|l| l.points).unwrap_or_default();
                            self.drag = Some(Drag::Move { g: None, origin, from: (t as f64, lane.law().norm(v)) });
                        }
                    }
                }
            }
        }
        cx.notify();
    }

    fn drag_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut drag) = self.drag.take() else { return };
        let Some(lane) = self.lane(cx) else { return };
        let vr = self.v_range(&lane);
        let (x, y) = Self::local(&self.curve_bounds, ev.position);
        let tick = self.x_to_tick(x);
        let r = lane.law();
        match &mut drag {
            Drag::Move { g, origin, from } => {
                let sel = self.sel.clone();
                let Some(anchor) = sel.iter().next().and_then(|i| origin.get(*i)).copied() else {
                    self.drag = Some(drag);
                    return;
                };
                let free = holds("au.free", &ev.modifiers);
                let dt = self.snap_tick(anchor.tick as f64 + tick - from.0, free) - anchor.tick;
                let dt = dt.max(-sel.iter().filter_map(|i| origin.get(*i)).map(|p| p.tick).min().unwrap_or(0));
                let dv = self.y_to_norm(y, vr) - from.1;
                let pts = origin
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        if !sel.contains(&i) {
                            return *p;
                        }
                        let f = |v: f64| r.denorm(r.norm(v) + dv);
                        Pt { tick: p.tick + dt, value: f(p.value), out: p.out.map(f), ..*p }
                    })
                    .collect();
                // the moved points keep their places in the origin: the selection is re-read from it each time
                let keep = self.sel.clone();
                self.gesture(g, "Move points", pts, cx);
                self.sel = keep;
            }
            Drag::Out { g, i, origin } => {
                let mut pts = (**origin).clone();
                pts[*i].out = Some(r.denorm(self.y_to_norm(y, vr)));
                self.gesture(g, "Out value", pts, cx);
            }
            Drag::Curve { g, i, origin, from_y, rising } => {
                // up bends the segment up, whichever way it goes
                let d = ((*from_y - y) / 80.0) as f64 * if *rising { 1.0 } else { -1.0 };
                let mut pts = (**origin).clone();
                pts[*i].curve = (origin[*i].curve + d).clamp(-1.0, 1.0);
                self.gesture(g, "Curve", pts, cx);
            }
            Drag::Rect { to, .. } => *to = point(px(x), px(y)),
            Drag::Erase { gone } => {
                if let Some((i, _)) = self.point_at(&lane, x, y) {
                    gone.insert(i);
                }
            }
            Drag::Pencil { stroke } => {
                let t = tick.round().max(0.0) as i64;
                if stroke.last().is_none_or(|l| t > l.0) {
                    stroke.push((t, self.y_to_norm(y, vr).clamp(0.0, 1.0)));
                }
            }
            Drag::Tension { g, origin, from_y, start } => {
                let t = (*start + ((*from_y - ev.position.y.as_f32()) / 200.0) as f64).clamp(0.0, 1.0);
                self.tension = (t * 100.0).round() / 100.0;
                if !self.sel.is_empty() {
                    let sel = self.sel.clone();
                    let pts = origin
                        .iter()
                        .enumerate()
                        .map(|(i, p)| {
                            if sel.contains(&i) { Pt { tension: self.tension, shape: Shape::Smooth, ..*p } } else { *p }
                        })
                        .collect();
                    self.gesture(g, "Tension", pts, cx);
                    self.sel = sel;
                }
            }
        }
        self.drag = Some(drag);
        cx.notify();
    }

    fn drag_up(&mut self, _ev: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else { return };
        match drag {
            Drag::Move { g, .. } | Drag::Out { g, .. } | Drag::Curve { g, .. } | Drag::Tension { g, .. } => {
                self.commit(g, cx)
            }
            Drag::Rect { from, to } => {
                if let Some(lane) = self.lane(cx) {
                    let vr = self.v_range(&lane);
                    let (x0, x1) = (from.x.as_f32().min(to.x.as_f32()), from.x.as_f32().max(to.x.as_f32()));
                    let (y0, y1) = (from.y.as_f32().min(to.y.as_f32()), from.y.as_f32().max(to.y.as_f32()));
                    for (i, p) in lane.points.iter().enumerate() {
                        let (x, y) = (self.tick_to_x(p.tick as f64), self.norm_to_y(lane.law().norm(p.value), vr));
                        if x >= x0 && x <= x1 && y >= y0 && y <= y1 {
                            self.sel.insert(i);
                        }
                    }
                }
            }
            Drag::Erase { gone } => {
                if let (Some(lane), false) = (self.lane(cx), gone.is_empty()) {
                    let pts =
                        lane.points.iter().enumerate().filter(|(i, _)| !gone.contains(i)).map(|(_, p)| *p).collect();
                    self.sel.clear();
                    self.transact("Erase points", pts, cx);
                }
            }
            Drag::Pencil { stroke } => {
                // the stroke's raw points replace the points under it, then the core thins them with the
                // parameter's own tolerance (I-21.5)
                if let (Some(lane), true) = (self.lane(cx), stroke.len() >= 2) {
                    let (t0, t1) = (stroke[0].0, stroke[stroke.len() - 1].0);
                    let mut pts: Vec<Pt> = lane.points.iter().filter(|p| p.tick < t0 || p.tick > t1).copied().collect();
                    pts.extend(stroke.iter().map(|(t, v)| Pt::new(*t, lane.law().denorm(*v), Shape::Curve)));
                    self.sel.clear();
                    self.transact("Draw freehand", pts, cx);
                    let params = Value::object([
                        ("lane", Value::text(lane.id.to_string())),
                        ("from", Value::int(t0)),
                        ("to", Value::int(t1)),
                    ]);
                    self.lane_action("automation.thin", params, cx);
                }
            }
        }
        cx.notify();
    }

    fn ruler_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let (x, _) = Self::local(&self.ruler_bounds, ev.position);
        self.cursor = music::snap(self.x_to_tick(x), self.grid_ticks()).max(0);
        let t = self.cursor;
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| pt.run_action("transport.locate".into(), Value::int(t)));
        });
        cx.notify();
    }

    fn apply_entry(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some((k, _)) = self.entry.take() else { return };
        let (Some(lane), Ok(v)) = (self.lane(cx), text.trim().replace(',', ".").parse::<f64>()) else { return };
        let (i, out) = (k & ((1 << 20) - 1), k >= 1 << 20);
        let mut pts = (*lane.points).clone();
        let Some(p) = pts.get_mut(i) else { return };
        if out {
            p.out = Some(v);
        } else {
            p.value = v;
        }
        self.transact("Point value", pts, cx);
    }

    /// The selected points (or all), changed by `f`
    fn change_selected(&mut self, label: &'static str, f: impl Fn(&Lane, &[Pt]) -> Vec<Pt>, cx: &mut Context<Self>) {
        let Some(lane) = self.lane(cx) else { return };
        let which: Vec<usize> =
            if self.sel.is_empty() { (0..lane.points.len()).collect() } else { self.sel.iter().copied().collect() };
        let chosen: Vec<Pt> = which.iter().filter_map(|i| lane.points.get(*i)).copied().collect();
        let changed = f(&lane, &chosen);
        let mut pts = (*lane.points).clone();
        for (i, p) in which.iter().zip(changed) {
            pts[*i] = p;
        }
        self.transact(label, pts, cx);
    }

    /// Flip in value (inside the range) or in time (inside the points' span), the selection or the lane (F-AUT-006)
    pub fn flip(&mut self, in_time: bool, cx: &mut Context<Self>) {
        self.change_selected(
            if in_time { "Flip horizontally" } else { "Flip vertically" },
            |lane, pts| {
                let (a, b) =
                    (pts.iter().map(|p| p.tick).min().unwrap_or(0), pts.iter().map(|p| p.tick).max().unwrap_or(0));
                let r = lane.law();
                pts.iter()
                    .map(|p| {
                        if in_time {
                            Pt { tick: a + b - p.tick, curve: -p.curve, ..*p }
                        } else {
                            let f = |v: f64| r.denorm(1.0 - r.norm(v));
                            Pt { value: f(p.value), out: p.out.map(f), curve: -p.curve, ..*p }
                        }
                    })
                    .collect()
            },
            cx,
        );
    }

    fn set_shape(&mut self, s: Shape, cx: &mut Context<Self>) {
        self.shape = s;
        if !self.sel.is_empty() {
            let t = self.tension;
            self.change_selected(
                "Progression",
                |_, pts| pts.iter().map(|p| Pt { shape: s, tension: t, ..*p }).collect(),
                cx,
            );
        }
        cx.notify();
    }

    /// Runs a lane edit of the core (I-21.5); a failure is shown, never redone here with a law of the editor's own
    fn lane_action(&mut self, action: &'static str, params: Value, cx: &mut Context<Self>) {
        let r = self.scx.core.update(cx, |core, _| core.port_mut().run_action(action.into(), params));
        self.error = match r {
            lm_port::Pending::Ready(Err(e)) => Some(e.label.key.to_string()),
            _ => None,
        };
        cx.notify();
    }

    /// The lane's span the edit applies to: the selection's ticks, or the whole lane
    fn span(&self, cx: &gpui::App, params: &mut Vec<(&'static str, Value)>) {
        let Some(lane) = self.lane(cx) else { return };
        let ticks: Vec<i64> = self.sel.iter().filter_map(|i| lane.points.get(*i)).map(|p| p.tick).collect();
        if let (Some(a), Some(z)) = (ticks.iter().min(), ticks.iter().max()) {
            params.push(("from", Value::int(*a)));
            params.push(("to", Value::int(*z)));
        }
    }

    /// Scale and offset the values (I-21.5), in the parameter's normalized law: the core's `automation.scale`
    fn scale_values(&mut self, factor: f64, offset: f64, cx: &mut Context<Self>) {
        let Some(id) = self.lane else { return };
        let mut params = vec![
            ("lane", Value::text(id.to_string())),
            ("scale", Value::real(factor).unwrap_or(Value::NULL)),
            ("offset", Value::real(offset).unwrap_or(Value::NULL)),
        ];
        self.span(cx, &mut params);
        self.lane_action("automation.scale", Value::object(params), cx);
    }

    /// Thin the lane with the parameter's own tolerance (I-21.5): the core's `automation.thin`
    fn thin_lane(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.lane else { return };
        let mut params = vec![("lane", Value::text(id.to_string()))];
        self.span(cx, &mut params);
        self.lane_action("automation.thin", Value::object(params), cx);
    }

    fn set_mode(&mut self, mode: &'static str, cx: &mut Context<Self>) {
        let Some(id) = self.lane else { return };
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| pt.apply(Label::new("Automation mode"), vec![EditOp::set(id, &["mode"], Value::text(mode))]));
        });
        cx.notify();
    }

    /// Re-arm: this lane back to its curve (I-21.5)
    fn rearm(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.lane else { return };
        let params = Value::object([("lane", Value::text(id.to_string()))]);
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| pt.run_action("automation.rearm".into(), params));
        });
        cx.notify();
    }

    fn toggle_play(&mut self, cx: &mut Context<Self>) {
        let now = HostTime::now();
        self.playing = match self.playing {
            Some((t0, from)) => {
                self.cursor = from + (now.since(t0) as f64 * 1e-9 * self.ticks_per_sec) as i64;
                None
            }
            None => Some((now, self.cursor)),
        };
        cx.notify();
    }

    fn menu_items(&self, m: Menu, cx: &gpui::App) -> Vec<MenuItem> {
        let root = self.scx.core.read(cx).port().root();
        let mode = self.lane(cx).map(|l| l.mode).unwrap_or_default();
        match m {
            Menu::Lane => Lane::all(self.scx.core.read(cx).port())
                .into_iter()
                .map(|l| MenuItem::new(format!("{} · {}", l.owner, l.param), self.lane == Some(l.id)).color(l.color))
                .collect(),
            Menu::Mode => MODES.iter().map(|(k, n)| MenuItem::new(tr(n), mode == *k)).collect(),
            Menu::ZoomX => ZOOM_X
                .iter()
                .enumerate()
                .map(|(i, z)| MenuItem::new(format!("{}%", z * 100.0), i == self.zoom_x))
                .collect(),
            Menu::ZoomY => ZOOM_Y
                .iter()
                .enumerate()
                .map(|(i, z)| {
                    MenuItem::new(
                        if *z == 0.0 { tr("Auto").to_string() } else { format!("{}%", z * 100.0) },
                        i == self.zoom_y,
                    )
                })
                .collect(),
            Menu::Grid => GRIDS.iter().enumerate().map(|(i, (l, _))| MenuItem::new(*l, i == self.grid)).collect(),
            Menu::Ghost => std::iter::once(MenuItem::new(tr("Clear ghost notes"), false))
                .chain(midi_items(root).into_iter().map(|(id, l)| MenuItem::new(l, self.ghost == Some(id))))
                .collect(),
            Menu::Tools => [
                "Scale values ×2",
                "Scale values ×½",
                "Raise values",
                "Lower values",
                "Thin the curve",
                "Select all",
                "Copy",
                "Paste",
            ]
            .iter()
            .map(|l| MenuItem::new(tr(l), false))
            .collect(),
        }
    }

    fn pick(&mut self, m: Menu, i: usize, cx: &mut Context<Self>) {
        self.menu = None;
        let root = self.scx.core.read(cx).port().root();
        match m {
            Menu::Lane => {
                if let Some(id) = Lane::ids(root).get(i).copied() {
                    self.open_lane(id, cx);
                }
            }
            Menu::Mode => self.set_mode(MODES[i].0, cx),
            Menu::ZoomX => {
                self.zoom_x = i;
                self.tpp = TPP_100 / ZOOM_X[i];
            }
            Menu::ZoomY => self.zoom_y = i,
            Menu::Grid => self.grid = i,
            Menu::Ghost => self.ghost = i.checked_sub(1).and_then(|k| midi_items(root).get(k).map(|x| x.0)),
            Menu::Tools => match i {
                0 => self.scale_values(2.0, 0.0, cx),
                1 => self.scale_values(0.5, 0.0, cx),
                2 => self.scale_values(1.0, 0.05, cx),
                3 => self.scale_values(1.0, -0.05, cx),
                4 => self.thin_lane(cx),
                5 => self.select_all(cx),
                6 => self.copy(cx),
                _ => self.paste(cx),
            },
        }
        cx.notify();
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        self.sel = (0..self.lane(cx).map_or(0, |l| l.points.len())).collect();
        cx.notify();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let Some(lane) = self.lane(cx) else { return };
        let sel: Vec<Pt> = self.sel.iter().filter_map(|i| lane.points.get(*i)).copied().collect();
        let a = sel.iter().map(|p| p.tick).min().unwrap_or(0);
        self.clipboard = sel.iter().map(|p| Pt { tick: p.tick - a, ..*p }).collect();
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(lane) = self.lane(cx) else { return };
        if self.clipboard.is_empty() {
            return;
        }
        let at = self.cursor;
        let span = self.clipboard.iter().map(|p| p.tick).max().unwrap_or(0);
        // the pasted span replaces what was there
        let mut pts: Vec<Pt> = lane.points.iter().filter(|p| p.tick < at || p.tick > at + span).copied().collect();
        let first = pts.len();
        pts.extend(self.clipboard.iter().map(|p| Pt { tick: p.tick + at, ..*p }));
        self.sel = (first..pts.len()).collect();
        self.transact("Paste points", pts, cx);
    }

    /// The keys are the registry's actions (`auto.*`); this is where they land
    fn listeners(&self, d: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        d.on_action(cx.listener(|ae, _: &AuSelectAll, _, cx| {
            ae.select_all(cx);
            cx.notify();
        }))
        .on_action(cx.listener(|ae, _: &AuSelectNone, _, cx| {
            ae.sel.clear();
            ae.menu = None;
            ae.entry = None;
            cx.notify();
        }))
        .on_action(cx.listener(|ae, _: &AuCopy, _, cx| {
            ae.copy(cx);
            cx.notify();
        }))
        .on_action(cx.listener(|ae, _: &AuCut, _, cx| {
            ae.copy(cx);
            ae.delete(cx);
            cx.notify();
        }))
        .on_action(cx.listener(|ae, _: &AuPaste, _, cx| {
            ae.paste(cx);
            cx.notify();
        }))
        .on_action(cx.listener(|ae, _: &AuDelete, _, cx| {
            ae.delete(cx);
            cx.notify();
        }))
        .on_action(cx.listener(|ae, _: &AuLeft, _, cx| {
            ae.cursor = (ae.cursor - ae.grid_ticks()).max(0);
            cx.notify();
        }))
        .on_action(cx.listener(|ae, _: &AuRight, _, cx| {
            ae.cursor += ae.grid_ticks();
            cx.notify();
        }))
        .on_action(cx.listener(|ae, _: &AuHome, _, cx| {
            ae.cursor = 0;
            ae.scroll = 0.0;
            cx.notify();
        }))
        .on_action(cx.listener(|ae, _: &AuPlay, _, cx| {
            ae.toggle_play(cx);
            cx.notify();
        }))
        .on_action(cx.listener(|ae, _: &AuToolDraw, _, cx| ae.set_tool(Tool::Draw, cx)))
        .on_action(cx.listener(|ae, _: &AuToolErase, _, cx| ae.set_tool(Tool::Erase, cx)))
        .on_action(cx.listener(|ae, _: &AuToolOut, _, cx| ae.set_tool(Tool::Out, cx)))
        .on_action(cx.listener(|ae, _: &AuToolPencil, _, cx| ae.set_tool(Tool::Pencil, cx)))
        .on_action(cx.listener(|ae, _: &AuToolSelect, _, cx| ae.set_tool(Tool::Select, cx)))
    }

    fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.tool = tool;
        cx.notify();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(lane) = self.lane(cx) else { return };
        let pts = lane.points.iter().enumerate().filter(|(i, _)| !self.sel.contains(i)).map(|(_, p)| *p).collect();
        self.sel.clear();
        self.transact("Delete points", pts, cx);
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let (dx, dy) = (d.x.as_f32(), d.y.as_f32());
        let (x, _) = Self::local(&self.curve_bounds, ev.position);
        match pick_wheel(&["au.view.grid_wheel", "au.view.zoom", "au.view.scroll"], &ev.modifiers) {
            Some("au.view.grid_wheel") => {
                self.grid = (self.grid as i32 - dy.signum() as i32).clamp(0, GRIDS.len() as i32 - 1) as usize;
            }
            Some("au.view.zoom") => {
                let anchor = self.x_to_tick(x);
                self.tpp = (self.tpp * (-dy * 0.01).exp() as f64).clamp(TPP_100 / 8.0, TPP_100 * 8.0);
                self.scroll = (anchor - x as f64 * self.tpp).max(0.0);
            }
            Some(_) => {
                let d = if dx.abs() > dy.abs() { dx } else { dy };
                self.scroll = (self.scroll - d as f64 * self.tpp).max(0.0);
            }
            None => {}
        }
        cx.notify();
    }

    fn on_pinch(&mut self, ev: &PinchEvent, cx: &mut Context<Self>) {
        let (x, _) = Self::local(&self.curve_bounds, ev.position);
        let anchor = self.x_to_tick(x);
        self.tpp = (self.tpp / (1.0 + ev.delta as f64).max(0.1)).clamp(TPP_100 / 8.0, TPP_100 * 8.0);
        self.scroll = (anchor - x as f64 * self.tpp).max(0.0);
        cx.notify();
    }
}

fn midi_items(root: &Value) -> Vec<(NodeId, String)> {
    model::tracks(root)
        .iter()
        .flat_map(|t| t.items.iter().filter(|i| !i.audio).map(move |i| (i.id, format!("{} · {}", t.name, i.name))))
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Rendering

struct CurvePaint {
    lane: Lane,
    sel: BTreeSet<usize>,
    gone: BTreeSet<usize>,
    ghost: Option<Arc<NoteMap>>,
    scroll: f64,
    tpp: f64,
    vr: (f64, f64),
    grid: i64,
    play: Option<f64>,
    cursor: i64,
    rect: Option<(Point<Pixels>, Point<Pixels>)>,
    stroke: Vec<(i64, f64)>,
}

impl CurvePaint {
    fn x(&self, t: f64) -> f32 {
        ((t - self.scroll) / self.tpp) as f32
    }
    fn t(&self, x: f32) -> f64 {
        self.scroll + x as f64 * self.tpp
    }
    fn y(&self, v: f64, h: f32) -> f32 {
        4.0 + (1.0 - ((v - self.vr.0) / (self.vr.1 - self.vr.0).max(1e-9)) as f32) * (h - 8.0)
    }

    fn paint(&self, b: Bounds<Pixels>, window: &mut Window) {
        let o = b.origin;
        let (w, h) = (b.size.width.as_f32(), b.size.height.as_f32());
        let r = |x: f32, y: f32, ww: f32, hh: f32| Bounds::new(point(o.x + px(x), o.y + px(y)), size(px(ww), px(hh)));
        window.paint_quad(fill(b, c(THEME.ae_bg)));
        // the legacy: eight horizontal lines, and the grid of the quantization, beats and bars
        for i in 0..=8 {
            window.paint_quad(fill(r(0.0, 4.0 + i as f32 * (h - 8.0) / 8.0, w, 1.0), c(THEME.bg2)));
        }
        let (t0, t1) = (self.t(0.0), self.t(w));
        let mut lines = |step: i64, col: u32| {
            if (step as f64 / self.tpp) < 6.0 {
                return;
            }
            let mut t = (t0 / step as f64).floor() as i64 * step;
            while (t as f64) <= t1 {
                window.paint_quad(fill(r(self.x(t as f64), 0.0, 1.0, h), c(col)));
                t += step;
            }
        };
        lines(self.grid, THEME.pr_line);
        lines(BEAT, THEME.ae_beat);
        lines(BAR, THEME.ae_bar);
        // ghost notes (F-AUT-009), by height in their own key range
        if let Some(g) = &self.ghost {
            let (lo, hi) = g.values().fold((127u8, 0u8), |(a, z), x| (a.min(x.key), z.max(x.key)));
            let span = (hi as f32 - lo as f32 + 1.0).max(12.0);
            for x in g.values().filter(|x| x.end() as f64 >= t0 && (x.start as f64) <= t1) {
                let y = h - 4.0 - (x.key as f32 - lo as f32 + 1.0) / span * (h - 8.0);
                window.paint_quad(fill(
                    r(self.x(x.start as f64), y, (x.length as f64 / self.tpp) as f32, 3.0),
                    with_alpha(THEME.text3, 0.35),
                ));
            }
        }
        // the curve: one stroke and one fill, a sample every 3 px, exact at each point (an out value is a jump)
        let pts: Vec<Pt> =
            self.lane.points.iter().enumerate().filter(|(i, _)| !self.gone.contains(i)).map(|(_, p)| *p).collect();
        let range = self.lane.law();
        let at = |t: f64| range.norm(value_at(&pts, t, range));
        let mut line: Vec<Point<Pixels>> = Vec::new();
        let mut k = pts.partition_point(|p| (p.tick as f64) < self.t(-3.0));
        let mut x = -3.0;
        while x <= w + 3.0 {
            let t = self.t(x);
            while k < pts.len() && (pts[k].tick as f64) <= t {
                let p = pts[k];
                let px_ = self.x(p.tick as f64);
                if k > 0 {
                    line.push(o + point(px(px_), px(self.y(at(p.tick as f64 - 0.5), h))));
                }
                line.push(o + point(px(px_), px(self.y(range.norm(p.value), h))));
                line.push(o + point(px(px_), px(self.y(range.norm(p.leaving()), h))));
                k += 1;
            }
            line.push(o + point(px(x), px(self.y(at(t), h))));
            x += 3.0;
        }
        let col = THEME.ae_graph;
        if line.len() >= 2 && !pts.is_empty() {
            let mut area = line.clone();
            area.push(o + point(px(w + 3.0), px(h)));
            area.push(o + point(px(-3.0), px(h)));
            let mut pb = PathBuilder::fill();
            pb.add_polygon(&area, true);
            if let Ok(p) = pb.build() {
                window.paint_path(p, gpui::rgba(col));
            }
            let mut pb = PathBuilder::stroke(px(1.6));
            pb.move_to(line[0]);
            for p in &line[1..] {
                pb.line_to(*p);
            }
            if let Ok(p) = pb.build() {
                window.paint_path(p, c(THEME.ae_bar));
            }
        }
        // the points, their out values and the curve handles (only the visible ones)
        for (i, p) in self.lane.points.iter().enumerate() {
            let px_ = self.x(p.tick as f64);
            if !(-10.0..=w + 10.0).contains(&px_) || self.gone.contains(&i) {
                continue;
            }
            let edge = if self.sel.contains(&i) { c(THEME.blue) } else { gpui::rgba(THEME.ae_in) };
            let dot = |window: &mut Window, y: f32, filled: bool| {
                let bb = r(px_ - POINT_R, y - POINT_R, POINT_R * 2.0, POINT_R * 2.0);
                window.paint_quad(gpui::quad(
                    bb,
                    px(POINT_R),
                    if filled { edge } else { c(THEME.bg1) },
                    px(1.5),
                    edge,
                    Default::default(),
                ));
            };
            dot(window, self.y(range.norm(p.value), h), true);
            if let Some(out) = p.out {
                dot(window, self.y(range.norm(out), h), false);
            }
            if i > 0 && p.shape == Shape::Curve {
                let a = self.lane.points[i - 1];
                let mid = (a.tick + p.tick) as f64 / 2.0;
                let (hx, hy) = (self.x(mid), self.y(at(mid), h));
                window.paint_quad(gpui::quad(
                    r(hx - 3.0, hy - 3.0, 6.0, 6.0),
                    px(1.0),
                    with_alpha(col, 0.6),
                    px(0.0),
                    c(col),
                    Default::default(),
                ));
            }
        }
        if self.stroke.len() >= 2 {
            let mut pb = PathBuilder::stroke(px(1.0));
            for (i, (t, v)) in self.stroke.iter().enumerate() {
                let p = o + point(px(self.x(*t as f64)), px(self.y(*v, h)));
                if i == 0 { pb.move_to(p) } else { pb.line_to(p) }
            }
            if let Ok(p) = pb.build() {
                window.paint_path(p, c(THEME.yellow));
            }
        }
        if let Some((a, z)) = self.rect {
            let bb = Bounds::new(o + point(a.x.min(z.x), a.y.min(z.y)), size((a.x - z.x).abs(), (a.y - z.y).abs()));
            window.paint_quad(gpui::quad(
                bb,
                px(0.0),
                gpui::rgba(THEME.selection),
                px(1.0),
                c(THEME.blue),
                Default::default(),
            ));
        }
        window.paint_quad(fill(r(self.x(self.cursor as f64), 0.0, 1.0, h), with_alpha(THEME.cursor, 0.35)));
        if let Some(t) = self.play {
            let x = self.x(t);
            if x >= 0.0 && x < w {
                window.paint_quad(fill(r(x, 0.0, 1.0, h), c(THEME.cursor)));
            }
        }
    }
}

impl Render for AutomationEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = HostTime::now();
        {
            // I-3.6: after an undo the selection may point past the list: dropped
            let port = self.scx.core.read(cx).port();
            if port.drain(&mut self.feed).is_some() {
                let n = self.lane.and_then(|id| Lane::read(port, id)).map(|l| l.points.len());
                match n {
                    None => self.lane = None,
                    Some(n) => self.sel.retain(|i| *i < n),
                }
            }
        }
        let core = self.scx.core.read(cx);
        let pos = core.port().telemetry().position;
        let map = core.port().tempo_map();
        let play = match self.playing {
            Some((t0, from)) => Some(from as f64 + now.since(t0) as f64 * 1e-9 * self.ticks_per_sec),
            None => pos.playing.then(|| map.tick_at(pos.at(now) as i64) as f64),
        };
        if play.is_some() {
            window.request_animation_frame();
        }
        let entity = cx.entity();
        let lane = self.lane(cx);
        let root = div()
            .id("automation")
            .size_full()
            .flex()
            .flex_col()
            .bg(c(THEME.bg1))
            .text_color(c(THEME.text))
            .track_focus(&self.focus)
            .key_context("AutomationEditor");
        self.listeners(root, cx)
            .child(self.render_toolbar(lane.as_ref(), cx))
            .child(match lane {
                None => div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(c(THEME.text2))
                    .child(tr("No automation lane"))
                    .into_any_element(),
                // a parameter the core gives no law for is not drawn nor edited (one law: the core's)
                Some(lane) if lane.range.is_none() => div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(c(THEME.red))
                    .child(tr("No range for this parameter"))
                    .into_any_element(),
                Some(lane) => div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .child(self.render_values(&lane))
                    .child(self.render_center(lane, play, cx))
                    .into_any_element(),
            })
            .children(self.drag.is_some().then(|| kit::capture(entity.downgrade(), Self::drag_move, Self::drag_up)))
            .children(self.menu.map(|(m, at)| {
                let items = self.menu_items(m, cx);
                let (e, e2) = (entity.downgrade(), entity.downgrade());
                popup(
                    "ae-menu",
                    at,
                    items,
                    move |i, _, cx| {
                        let _ = e.update(cx, |ae, cx| ae.pick(m, i, cx));
                    },
                    move |_, cx| {
                        let _ = e2.update(cx, |ae, cx| {
                            ae.menu = None;
                            cx.notify();
                        });
                    },
                )
            }))
            .children(self.entry.as_ref().map(|(_, e)| {
                let (a, b) = (entity.downgrade(), entity.downgrade());
                e.render(
                    move |text, _, cx| {
                        let _ = a.update(cx, |ae, cx| ae.apply_entry(&text, cx));
                    },
                    move |_, cx| {
                        let _ = b.update(cx, |ae, cx| {
                            ae.entry = None;
                            cx.notify();
                        });
                    },
                )
            }))
    }
}

impl AutomationEditor {
    fn open_menu(
        m: Menu,
    ) -> impl Fn(&mut AutomationEditor, &MouseDownEvent, &mut Window, &mut Context<AutomationEditor>) {
        move |ae, ev, _, cx| {
            ae.menu =
                if ae.menu.is_some_and(|x| x.0 == m) { None } else { Some((m, ev.position + point(px(0.), px(10.)))) };
            cx.notify();
        }
    }

    /// The legacy's toolbar, and the lane's name, mode and re-arm (I-21.5)
    fn render_toolbar(&self, lane: Option<&Lane>, cx: &Context<Self>) -> impl IntoElement {
        let small = |id: &'static str, label: SharedString, on: bool, col: u32, help: &'static str| {
            button(id, label, on, col)
                .debug_selector(move || id.to_string())
                .h(px(20.))
                .min_w(px(24.))
                .px(px(6.))
                .tooltip(tip(help))
        };
        let tool = |t: Tool, id: &'static str, icon: &'static str, help: &'static str| {
            icon_button(id, icon, self.tool == t, help).on_click(cx.listener(move |ae, _, _, cx| {
                ae.tool = t;
                cx.notify();
            }))
        };
        let shape = |s: Shape, id: &'static str, icon: &'static str, help: &'static str| {
            icon_button(id, icon, self.shape == s, help).on_click(cx.listener(move |ae, _, _, cx| ae.set_shape(s, cx)))
        };
        let combo = |id: &'static str, icon: Option<&'static str>, m: Menu, text: SharedString, help: &'static str| {
            match icon {
                Some(i) => dropdown_icon(id, i, text, false),
                None => dropdown(id, text, false),
            }
            .debug_selector(move || id.to_string())
            .tooltip(tip(help))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::open_menu(m)))
        };
        let playing = self.playing.is_some();
        // the hand overrode this lane's parameter: shown until the re-arm (I-21.5)
        let hand_won = lane.is_some_and(|l| {
            let t = self.scx.core.read(cx).port().telemetry();
            t.hand_won.iter().any(|(n, p)| Some(*n) == l.target && p.as_ref() == l.param)
        });
        let mode = lane.map(|l| l.mode.clone()).unwrap_or_default();
        let mode_name = MODES.iter().find(|m| m.0 == mode).map_or("Read", |m| m.1);
        div()
            .h(px(ROW_H))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(3.))
            .px(px(4.))
            .bg(c(THEME.bg2))
            .border_b_1()
            .border_color(c(THEME.line))
            .overflow_hidden()
            .child(
                icon_button(
                    "ae-play",
                    if playing { "pause" } else { "play" },
                    playing,
                    "Play/pause current clip (Space)",
                )
                .on_click(cx.listener(|ae, _, _, cx| ae.toggle_play(cx))),
            )
            .child(icon_button("ae-stop", "stop", false, "Stop playing of current clip (Space)").on_click(cx.listener(
                |ae, _, _, cx| {
                    ae.playing = None;
                    ae.cursor = 0;
                    cx.notify();
                },
            )))
            .child(sep())
            .child(tool(Tool::Draw, "ae-mode-draw", "edit_draw", "Draw mode (Shift+D)"))
            .child(tool(Tool::Erase, "ae-mode-erase", "edit_erase", "Erase mode (Shift+E)"))
            .child(tool(Tool::Out, "ae-mode-out", "edit_draw_outvalue", "Draw outValues mode (Shift+C)"))
            .child(tool(Tool::Pencil, "ae-mode-pencil", "edit_draw_small", "Draw freehand (Shift+P)"))
            .child(tool(Tool::Select, "ae-mode-select", "edit_select", "Select mode (Shift+S)"))
            .child(sep())
            .child(
                icon_button("ae-flip-y", "flip_y", false, "Flip vertically")
                    .on_click(cx.listener(|ae, _, _, cx| ae.flip(false, cx))),
            )
            .child(
                icon_button("ae-flip-x", "flip_x", false, "Flip horizontally")
                    .on_click(cx.listener(|ae, _, _, cx| ae.flip(true, cx))),
            )
            .child(sep())
            .child(shape(Shape::Step, "ae-discrete", "progression_discrete", "Discrete progression"))
            .child(shape(Shape::Curve, "ae-linear", "progression_linear", "Linear progression"))
            .child(shape(Shape::Smooth, "ae-cubic", "progression_cubic_hermite", "Cubic Hermite progression"))
            .child(
                div()
                    .id("ae-tension")
                    .debug_selector(|| "ae-tension".into())
                    .flex_none()
                    .px(px(6.))
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .rounded(px(3.))
                    .bg(c(THEME.bg1))
                    .text_size(px(11.))
                    .cursor(CursorStyle::ResizeUpDown)
                    .tooltip(tip("Tension value for spline"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|ae, ev: &MouseDownEvent, _, cx| {
                            let origin = ae.lane(cx).map(|l| l.points).unwrap_or_default();
                            ae.drag = Some(Drag::Tension {
                                g: None,
                                origin,
                                from_y: ev.position.y.as_f32(),
                                start: ae.tension,
                            });
                            cx.notify();
                        }),
                    )
                    .child(format!("{} {:.2}", tr("Tension:"), self.tension)),
            )
            .child(sep())
            .child(combo(
                "ae-zoom-x",
                Some("zoom_x"),
                Menu::ZoomX,
                format!("{}%", ZOOM_X[self.zoom_x] * 100.0).into(),
                "Horizontal zooming",
            ))
            .child(combo(
                "ae-zoom-y",
                Some("zoom_y"),
                Menu::ZoomY,
                if self.zoom_y == 0 { tr("Auto").into() } else { format!("{}%", ZOOM_Y[self.zoom_y] * 100.0).into() },
                "Vertical zooming",
            ))
            .child(combo("ae-grid", Some("quantize"), Menu::Grid, GRIDS[self.grid].0.into(), "Quantization"))
            .child(combo(
                "ae-ghost",
                Some("automation_ghost_note"),
                Menu::Ghost,
                if self.ghost.is_some() { tr("Ghost notes").into() } else { "".into() },
                "Ghost notes",
            ))
            .child(combo("ae-tools", Some("tool"), Menu::Tools, "".into(), "Scale, thin, copy and paste the points"))
            .child(div().flex_1())
            .children(
                self.error.clone().map(|e| {
                    div().text_size(px(11.)).text_color(c(THEME.red)).child(trf("The core refused: {0}", &[&e]))
                }),
            )
            .children(lane.map(|l| {
                div().text_size(px(11.)).text_color(c(THEME.text2)).child(format!("{} · {}", l.owner, l.param))
            }))
            .child(combo("ae-lane", Some("automation"), Menu::Lane, tr("Lane").into(), "The lane shown"))
            .child(label(tr("Mode")))
            .child(combo("ae-lane-mode", None, Menu::Mode, tr(mode_name).into(), "Off, Read, Touch, Latch or Write"))
            .child(
                small(
                    "ae-rearm",
                    tr(if hand_won { "Hand won ↺" } else { "Re-arm" }).into(),
                    hand_won,
                    THEME.red,
                    "This lane back to its curve",
                )
                .on_click(cx.listener(|ae, _, _, cx| ae.rearm(cx))),
            )
    }

    /// The legacy's value scale on the left
    fn render_values(&self, lane: &Lane) -> impl IntoElement {
        let vr = self.v_range(lane);
        let h = self.curve_bounds.get().size.height.as_f32().max(40.0);
        let mut area = div().flex_1().relative().overflow_hidden();
        for i in 0..=4 {
            let v = vr.0 + (vr.1 - vr.0) * i as f64 / 4.0;
            let y = 4.0 + (1.0 - i as f32 / 4.0) * (h - 8.0);
            area = area.child(
                div()
                    .absolute()
                    .top(px(y - 6.0))
                    .right(px(6.))
                    .text_size(px(9.5))
                    .text_color(c(THEME.text2))
                    .child(lane.law().format(lane.law().denorm(v))),
            );
        }
        div()
            .w(px(VALUES_W))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(c(THEME.bg2))
            .border_r_1()
            .border_color(c(THEME.line))
            .child(div().h(px(TOP_MARGIN)).bg(c(THEME.bg3)))
            .child(area)
    }

    fn render_center(&self, lane: Lane, play: Option<f64>, cx: &Context<Self>) -> impl IntoElement {
        let root = self.scx.core.read(cx).port().root();
        let ghost = self.ghost.and_then(|g| Clip::read(root, g)).map(|c| c.notes);
        let vr = self.v_range(&lane);
        let paint = CurvePaint {
            lane,
            sel: self.sel.clone(),
            gone: match &self.drag {
                Some(Drag::Erase { gone }) => gone.clone(),
                _ => BTreeSet::new(),
            },
            ghost,
            scroll: self.scroll,
            tpp: self.tpp,
            vr,
            grid: self.grid_ticks(),
            play,
            cursor: self.cursor,
            rect: match &self.drag {
                Some(Drag::Rect { from, to }) => Some((*from, *to)),
                _ => None,
            },
            stroke: match &self.drag {
                Some(Drag::Pencil { stroke }) => stroke.clone(),
                _ => Vec::new(),
            },
        };
        let (cb, rb) = (self.curve_bounds.clone(), self.ruler_bounds.clone());
        let (scroll, tpp, cursor) = (self.scroll, self.tpp, self.cursor);
        let cursor_style = match self.tool {
            Tool::Erase => CursorStyle::OperationNotAllowed,
            Tool::Pencil => CursorStyle::Crosshair,
            _ => CursorStyle::Arrow,
        };
        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .id("ae-ruler")
                    .h(px(TOP_MARGIN))
                    .flex_none()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::ruler_down))
                    .child(
                        canvas(
                            move |b, _, _| rb.set(b),
                            move |b, _, window, cx| {
                                let o = b.origin;
                                let w = b.size.width.as_f32();
                                let x = |t: f64| ((t - scroll) / tpp) as f32;
                                window.paint_quad(fill(b, c(THEME.bg3)));
                                let step = BAR * ((60.0 * tpp / BAR as f64).ceil() as i64).max(1);
                                let mut t = (scroll / step as f64).floor() as i64 * step;
                                while x(t as f64) < w {
                                    let xx = x(t as f64);
                                    window.paint_quad(fill(
                                        Bounds::new(point(o.x + px(xx), o.y + px(3.0)), size(px(1.0), px(13.0))),
                                        c(THEME.text3),
                                    ));
                                    paint_text(
                                        window,
                                        cx,
                                        format!("{}", t / BAR + 1).into(),
                                        o.x + px(xx + 3.0),
                                        o.y + px(1.0),
                                        10.0,
                                        c(THEME.text2),
                                    );
                                    t += step;
                                }
                                let cx_ = x(cursor as f64);
                                let mut pb = PathBuilder::fill();
                                pb.add_polygon(
                                    &[
                                        o + point(px(cx_ - 5.0), px(0.0)),
                                        o + point(px(cx_ + 5.0), px(0.0)),
                                        o + point(px(cx_), px(7.0)),
                                    ],
                                    true,
                                );
                                if let Ok(p) = pb.build() {
                                    window.paint_path(p, c(THEME.cursor));
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
            .child(
                div()
                    .id("ae-curve")
                    .debug_selector(|| "ae-curve".into())
                    .flex_1()
                    .overflow_hidden()
                    .cursor(cursor_style)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::curve_down))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::curve_down))
                    .on_scroll(cx.listener(|ae, ev: &ScrollEvent, _, cx| ae.on_scroll(ev, cx)))
                    .on_pinch(cx.listener(|ae, ev: &PinchEvent, _, cx| ae.on_pinch(ev, cx)))
                    .child(canvas(move |b, _, _| cb.set(b), move |b, _, window, _| paint.paint(b, window)).size_full()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_end_at_their_point() {
        let r = Range(ParamRange::linear(0.0, 1.0));
        let pts = vec![Pt::new(0, 0.0, Shape::Curve), Pt::new(960, 1.0, Shape::Step)];
        assert_eq!(value_at(&pts, 480.0, r), 0.0, "a step segment holds the previous value");
        let pts = vec![Pt::new(0, 0.0, Shape::Curve), Pt::new(960, 1.0, Shape::Curve)];
        assert!((value_at(&pts, 480.0, r) - 0.5).abs() < 1e-6, "a straight curve");
        let pts = vec![Pt { out: Some(0.8), ..Pt::new(0, 0.0, Shape::Curve) }, Pt::new(960, 0.8, Shape::Curve)];
        assert!((value_at(&pts, 10.0, r) - 0.8).abs() < 1e-6, "the out value leaves the point");
    }

    #[test]
    fn a_point_writes_only_what_it_says() {
        let p = Pt::new(10, 0.5, Shape::Curve);
        let v = p.to_value();
        assert!(v.get("shape").is_none() && v.get("out").is_none() && v.get("tension").is_none());
        assert_eq!(Pt::read(&v), p);
    }
}
