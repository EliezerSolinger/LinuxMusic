//! Owner: agent B. The automation editor: the curve of one parameter's lane (F-AUT-001..011, F-CLP-081/082,
//! P-27, P-33).
//!
//! A lane belongs to its parameter's owner (spec item 2) and covers the whole song, so this editor has no item: it
//! shows the lane picked in the toolbar, with the value axis in the parameter's own units (dB, Hz on a log scale,
//! %, a choice's names). Each point has a shape to the next one: Step, Curve (bent by `curve`, -1..1, with a handle
//! in the middle of the segment) or Smooth (a spline with the lane's tension); a point can leave at another value
//! than it arrives (a jump, P-27). A choice or stepped parameter never interpolates (spec item 1): its lane is drawn
//! and edited in steps. A lane may move several parameters (a shared control, P-33): the inspector lists them.
//!
//! The core is reached only through the `CorePort`: a point is a node (`Insert` into the lane's `points`, `Set`
//! with `Field::Point`, `Remove`), the lane's own numbers are `Field::Clip(key)`.
//!
//! Drawing: the curve is one stroked path and one filled path (a sample every 3 px of the visible width), the
//! points one quad each, only the visible ones (report 2.2).

use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use fake_core::notes::{self, AutoLane, BAR, GRIDS, NoteMap, ParamInfo, Point, PointChange, PointMap, SHAPES, Shape, format_bbt};
use fake_core::port::{EditOp, Ref};
use fake_core::{Feed, Field, GestureId, NodeId, Topic, Value};
use gpui::{
    Bounds, Context, CursorStyle, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PathBuilder, PinchEvent, Pixels, Render, ScrollEvent, SharedString, Window, canvas, div, fill, point, prelude::*,
    px, size,
};

use super::ScreenCx;
use super::piano_roll::kit::{self, MenuItem, ValueEntry, dropdown, label, paint_text, popup, primary, sep, tip, with_alpha};
use crate::ui::{THEME, button, c, caption, tr, trf};

const TOOLBAR_H: f32 = 32.0;
const RULER_H: f32 = 20.0;
const AXIS_W: f32 = 64.0;
const INSPECTOR_W: f32 = 210.0;
const POINT_R: f32 = 4.0;
/// Point numbers from here up are temp numbers of points to create
const NEW: u64 = 1 << 63;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Draw,
    Pencil,
    Select,
    Erase,
}

const TOOLS: [(Tool, &str, &str); 4] = [
    (Tool::Draw, "Draw", "Click adds a point, drag moves it; Shift+click draws a line of points from the last one (Shift+D)"),
    (Tool::Pencil, "Pencil", "Draw the curve freehand; it becomes the fewest points that follow the stroke (Shift+P)"),
    (Tool::Select, "Select", "Select (Shift+S); Ctrl/Cmd held selects in any tool"),
    (Tool::Erase, "Erase", "Erase (Shift+E)"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Lane,
    Grid,
    Shape,
    Ghost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prop {
    Time,
    Value,
    Jump,
    Curve,
    Tension,
    Scale,
}

enum Drag {
    /// moving the selection (a new point is a selection of one)
    Move { g: Option<GestureId>, origin: Vec<(u64, Point)>, anchor: Point, from: (f64, f64), free: bool },
    /// the jump of a point (P-27): the value it leaves at
    Jump { g: Option<GestureId>, n: u64, origin: Point },
    /// the bend of a Curve segment, by its middle handle
    Curve { g: Option<GestureId>, n: u64, origin: Point, from_y: f32, rising: bool },
    Rect { from: gpui::Point<Pixels>, to: gpui::Point<Pixels>, toggle: bool, before: Rc<BTreeSet<u64>> },
    Erase { g: Option<GestureId> },
    Pencil { stroke: Vec<(i64, f64)> },
    Scrub { g: Option<GestureId>, prop: Prop, origin: Vec<(u64, Point)>, from_y: f32 },
}

/// Point changes as the port's edits: a point is a node of kind `p`
fn point_ops(lane: NodeId, changes: Vec<PointChange>) -> Vec<EditOp> {
    changes
        .into_iter()
        .filter_map(|(n, p)| {
            let id = NodeId { kind: b'p', number: n };
            Some(match p {
                Some(p) if n >= NEW => EditOp::Insert { temp: (n - NEW) as u32, parent: Ref::Id(lane), list: "points", index: 0, node: Value::Point(p) },
                Some(p) => EditOp::set(id, Field::Point, Value::Point(p)),
                None if n >= NEW => return None,
                None => EditOp::Remove { id: Ref::Id(id) },
            })
        })
        .collect()
}

pub struct AutomationEditor {
    scx: ScreenCx,
    focus: FocusHandle,
    feed: Feed,
    pub lane: Option<NodeId>,
    ghost: Option<NodeId>,
    pub tool: Tool,
    shape: Shape,
    grid: usize,
    pub scroll: f64,
    pub tpp: f64,
    /// the visible part of the value range, normalized 0..1 (bottom, top)
    pub v_lo: f64,
    pub v_hi: f64,
    auto_fit: bool,
    pub sel: Rc<BTreeSet<u64>>,
    drag: Option<Drag>,
    menu: Option<(Menu, gpui::Point<Pixels>)>,
    entry: Option<(Prop, ValueEntry)>,
    hover: Option<(f32, f32)>,
    clipboard: Vec<Point>,
    cursor: i64,
    last_added: Option<u64>,
    curve_bounds: Rc<Cell<Bounds<Pixels>>>,
    ruler_bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl AutomationEditor {
    pub fn new(scx: ScreenCx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (feed, lane) = scx.core.update(cx, |core, _| {
            let lane = core.port().root().music.lanes.first().map(|l| l.id);
            (core.port_mut().feed(Topic::All), lane)
        });
        AutomationEditor {
            focus: cx.focus_handle(),
            feed,
            lane,
            ghost: None,
            tool: Tool::Draw,
            shape: Shape::Curve,
            grid: 2,
            scroll: 0.0,
            tpp: 16.0,
            v_lo: 0.0,
            v_hi: 1.0,
            auto_fit: false,
            sel: Rc::new(BTreeSet::new()),
            drag: None,
            menu: None,
            entry: None,
            hover: None,
            clipboard: Vec::new(),
            cursor: 0,
            last_added: None,
            curve_bounds: Rc::new(Cell::new(Bounds::default())),
            ruler_bounds: Rc::new(Cell::new(Bounds::default())),
            scx,
        }
    }

    fn lane<'a>(&self, cx: &'a gpui::App) -> Option<&'a AutoLane> {
        self.lane.and_then(|id| self.scx.core.read(cx).port().root().music.lane(id))
    }

    fn points(&self, cx: &gpui::App) -> Arc<PointMap> {
        self.lane(cx).map(|l| l.points.clone()).unwrap_or_default()
    }

    fn param(&self, cx: &gpui::App) -> ParamInfo {
        self.lane(cx).map_or(notes::PARAM_VOLUME, |l| l.param)
    }

    fn grid_ticks(&self) -> i64 {
        GRIDS[self.grid].1
    }

    // --- geometry: x <-> tick, y <-> normalized value ---------------------------------------------------------

    fn h(&self) -> f32 {
        self.curve_bounds.get().size.height.as_f32().max(1.0)
    }
    fn x_to_tick(&self, x: f32) -> f64 {
        self.scroll + x as f64 * self.tpp
    }
    pub fn tick_to_x(&self, t: f64) -> f32 {
        ((t - self.scroll) / self.tpp) as f32
    }
    /// normalized value at a height (6 px of margin top and bottom)
    fn y_to_norm(&self, y: f32) -> f64 {
        let h = self.h() - 12.0;
        let t = (1.0 - (y - 6.0) / h) as f64;
        self.v_lo + t * (self.v_hi - self.v_lo)
    }
    pub fn norm_to_y(&self, v: f64) -> f32 {
        let h = self.h() - 12.0;
        6.0 + (1.0 - ((v - self.v_lo) / (self.v_hi - self.v_lo)) as f32) * h
    }

    fn local(&self, b: &Rc<Cell<Bounds<Pixels>>>, p: gpui::Point<Pixels>) -> (f32, f32) {
        let o = b.get().origin;
        ((p.x - o.x).as_f32(), (p.y - o.y).as_f32())
    }

    /// The point under (x, y), and whether the hit was on its jump handle (the out value)
    fn point_at(&self, pts: &[(u64, Point)], param: &ParamInfo, x: f32, y: f32) -> Option<(u64, bool)> {
        let mut best: Option<(u64, bool, f32)> = None;
        for (n, p) in pts {
            let px_ = self.tick_to_x(p.tick as f64);
            if (px_ - x).abs() > 7.0 {
                continue;
            }
            for (jump, v) in [(false, p.value), (true, p.out())] {
                if jump && p.jump.is_none() {
                    continue;
                }
                let d = (self.norm_to_y(param.norm(v)) - y).abs() + (px_ - x).abs();
                if d < 10.0 && best.is_none_or(|b| d < b.2) {
                    best = Some((*n, jump, d));
                }
            }
        }
        best.map(|b| (b.0, b.1))
    }

    /// The Curve segment whose middle handle is under (x, y)
    fn handle_at(&self, pts: &[(u64, Point)], lane: &AutoLane, x: f32, y: f32) -> Option<(u64, bool)> {
        for w in pts.windows(2) {
            let (n, a) = w[0];
            let b = w[1].1;
            if a.shape != Shape::Curve || lane.param.discrete() {
                continue;
            }
            let mid = (a.tick + b.tick) as f64 / 2.0;
            let hx = self.tick_to_x(mid);
            let hy = self.norm_to_y(lane.param.norm(lane.value_at(mid)));
            if (hx - x).abs() < 6.0 && (hy - y).abs() < 6.0 {
                return Some((n, b.value >= a.out()));
            }
        }
        None
    }

    // --- changing, through the port ----------------------------------------------------------------------------

    /// One transaction; returns the numbers the core gave the new points (empty while `Waiting`)
    fn transact(&mut self, label: &'static str, changes: Vec<PointChange>, cx: &mut Context<Self>) -> Vec<u64> {
        let Some(id) = self.lane else { return Vec::new() };
        if changes.is_empty() {
            return Vec::new();
        }
        let before = changes.iter().any(|(n, _)| *n >= NEW).then(|| self.points(cx));
        self.scx.core.update(cx, |core, _| {
            let _ = core.port_mut().apply(label, point_ops(id, changes));
        });
        cx.notify();
        before.map(|b| self.added_since(&b, cx)).unwrap_or_default()
    }

    fn gesture(&self, g: &mut Option<GestureId>, label: &'static str, changes: Vec<PointChange>, cx: &mut Context<Self>) -> Vec<u64> {
        let Some(id) = self.lane else { return Vec::new() };
        let before = changes.iter().any(|(n, _)| *n >= NEW).then(|| self.points(cx));
        self.scx.core.update(cx, |core, _| {
            let port = core.port_mut();
            let g = g.get_or_insert_with(|| port.begin_gesture(label));
            port.gesture_apply(g, point_ops(id, changes));
        });
        before.map(|b| self.added_since(&b, cx)).unwrap_or_default()
    }

    fn set_lane_field(&mut self, label: &'static str, key: &'static str, v: Value, cx: &mut Context<Self>) {
        let Some(id) = self.lane else { return };
        self.scx.core.update(cx, |core, _| {
            let _ = core.port_mut().apply(label, vec![EditOp::set(id, Field::Clip(key), v)]);
        });
        cx.notify();
    }

    fn added_since(&self, before: &PointMap, cx: &gpui::App) -> Vec<u64> {
        self.points(cx).keys().filter(|n| !before.contains_key(n)).copied().collect()
    }

    fn temps(k: usize) -> Vec<u64> {
        (0..k as u64).map(|i| NEW + i).collect()
    }

    fn commit(&self, g: Option<GestureId>, cx: &mut Context<Self>) {
        if let Some(g) = g {
            self.scx.core.update(cx, |core, _| core.port_mut().gesture_commit(g));
        }
    }

    pub fn set_sel(&mut self, s: BTreeSet<u64>) {
        self.sel = Rc::new(s);
    }

    fn selected(&self, pts: &PointMap) -> Vec<(u64, Point)> {
        self.sel.iter().filter_map(|n| pts.get(n).map(|p| (*n, *p))).collect()
    }

    fn targets(&self, pts: &PointMap) -> Vec<u64> {
        if self.sel.is_empty() { pts.keys().copied().collect() } else { self.sel.iter().copied().filter(|n| pts.contains_key(n)).collect() }
    }

    fn snap_tick(&self, t: f64, free: bool) -> i64 {
        if free { t.round().max(0.0) as i64 } else { notes::snap(t, self.grid_ticks()).max(0) }
    }

    pub fn open_lane(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.lane = Some(id);
        self.set_sel(BTreeSet::new());
        self.v_lo = 0.0;
        self.v_hi = 1.0;
        self.last_added = None;
        cx.notify();
    }

    /// Fits the visible value range to the points (F-AUT-008's automatic vertical zoom)
    fn fit_values(&mut self, cx: &Context<Self>) {
        let Some(l) = self.lane(cx) else { return };
        let vals: Vec<f64> = l.points.values().flat_map(|p| [l.param.norm(p.value), l.param.norm(p.out())]).collect();
        if vals.is_empty() {
            return;
        }
        let (lo, hi) = vals.iter().fold((1.0f64, 0.0f64), |(a, b), v| (a.min(*v), b.max(*v)));
        let pad = ((hi - lo) * 0.1).max(0.05);
        self.v_lo = (lo - pad).max(0.0);
        self.v_hi = (hi + pad).min(1.0);
    }

    // --- the mouse ---------------------------------------------------------------------------------------------

    fn curve_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.menu = None;
        self.entry = None;
        let Some(lane) = self.lane(cx).cloned() else { return };
        let (x, y) = self.local(&self.curve_bounds, ev.position);
        let pts = lane.sorted();
        let param = lane.param;
        let tick = self.x_to_tick(x);
        let hit = self.point_at(&pts, &param, x, y);
        if ev.button == MouseButton::Right {
            // the right button: a jump goes back to the arrival value; else erase (dragging erases more)
            if let Some((n, true)) = hit {
                let p = lane.points[&n];
                self.transact("Remove the jump", vec![(n, Some(Point { jump: None, ..p }))], cx);
                return;
            }
            let mut g = None;
            if let Some((n, _)) = hit {
                self.gesture(&mut g, "Erase points", vec![(n, None)], cx);
            }
            self.drag = Some(Drag::Erase { g });
            cx.notify();
            return;
        }
        if ev.click_count >= 2 {
            if let Some((n, jump)) = hit {
                // double click: type the value (F-AUT-005)
                self.set_sel([n].into_iter().collect());
                let p = lane.points[&n];
                let v = if jump { p.out() } else { p.value };
                let text = if param.choices.is_empty() { format!("{v:.2}") } else { format!("{}", v.round()) };
                self.entry = Some((if jump { Prop::Jump } else { Prop::Value }, ValueEntry::new(text, ev.position, window, cx)));
                cx.notify();
                return;
            }
        }
        let tool = if primary(&ev.modifiers) { Tool::Select } else { self.tool };
        match tool {
            Tool::Erase => {
                let mut g = None;
                if let Some((n, _)) = hit {
                    self.gesture(&mut g, "Erase points", vec![(n, None)], cx);
                }
                self.drag = Some(Drag::Erase { g });
            }
            Tool::Pencil => {
                let v = param.norm(param.denorm(self.y_to_norm(y)));
                self.drag = Some(Drag::Pencil { stroke: vec![(self.snap_tick(tick, true), v)] });
            }
            Tool::Draw | Tool::Select => {
                if let Some((n, jump)) = hit {
                    let p = lane.points[&n];
                    if jump || ev.modifiers.alt {
                        // Alt on a point, or its out handle: the jump (P-27)
                        self.drag = Some(Drag::Jump { g: None, n, origin: p });
                    } else {
                        if !self.sel.contains(&n) {
                            let mut s = if ev.modifiers.shift { (*self.sel).clone() } else { BTreeSet::new() };
                            s.insert(n);
                            self.set_sel(s);
                        }
                        let origin = self.selected(&lane.points);
                        self.drag = Some(Drag::Move { g: None, origin, anchor: p, from: (tick, self.y_to_norm(y)), free: false });
                    }
                } else if let Some((n, rising)) = self.handle_at(&pts, &lane, x, y) {
                    self.drag = Some(Drag::Curve { g: None, n, origin: lane.points[&n], from_y: y, rising });
                } else if tool == Tool::Select {
                    let p = point(px(x), px(y));
                    let before = if ev.modifiers.shift { self.sel.clone() } else { Rc::new(BTreeSet::new()) };
                    self.drag = Some(Drag::Rect { from: p, to: p, toggle: ev.modifiers.shift, before });
                } else {
                    let t = self.snap_tick(tick, ev.modifiers.alt);
                    let v = param.denorm(self.y_to_norm(y));
                    let shape = if param.discrete() { Shape::Step } else { self.shape };
                    let last = self.last_added.and_then(|n| lane.points.get(&n).copied());
                    if let (true, Some(a)) = (ev.modifiers.shift, last) {
                        // Shift+click: a straight line of points from the last one, one per grid step
                        let step = self.grid_ticks();
                        let (t0, t1) = (a.tick.min(t), a.tick.max(t));
                        let (va, vb) = if a.tick <= t { (param.norm(a.value), param.norm(v)) } else { (param.norm(v), param.norm(a.value)) };
                        let mut line = Vec::new();
                        let mut k = t0 + step;
                        while k <= t1 {
                            let f = (k - t0) as f64 / (t1 - t0).max(1) as f64;
                            line.push(Point::new(k, param.denorm(va + (vb - va) * f), shape));
                            k += step;
                        }
                        // the old points inside the line's span go
                        let mut ch: Vec<PointChange> = pts.iter().filter(|(_, p)| p.tick > t0 && p.tick <= t1).map(|(n, _)| (*n, None)).collect();
                        ch.extend(Self::temps(line.len()).into_iter().zip(line.into_iter().map(Some)));
                        let new = self.transact("Draw a line", ch, cx);
                        self.last_added = new.last().copied();
                    } else {
                        let p = Point::new(t, v, shape);
                        let mut g = None;
                        // a point at the same tick is replaced
                        let mut ch: Vec<PointChange> = pts.iter().filter(|(_, q)| q.tick == t).map(|(m, _)| (*m, None)).collect();
                        ch.push((NEW, Some(p)));
                        let new = self.gesture(&mut g, "Add a point", ch, cx);
                        if let Some(n) = new.first().copied() {
                            self.last_added = Some(n);
                            self.set_sel([n].into_iter().collect());
                            self.drag = Some(Drag::Move { g, origin: vec![(n, p)], anchor: p, from: (t as f64, param.norm(p.value)), free: ev.modifiers.alt });
                        } else {
                            self.commit(g, cx);
                        }
                    }
                }
            }
        }
        cx.notify();
    }

    fn drag_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut drag) = self.drag.take() else { return };
        let Some(lane) = self.lane(cx).cloned() else { return };
        let param = lane.param;
        let (x, y) = self.local(&self.curve_bounds, ev.position);
        let tick = self.x_to_tick(x);
        match &mut drag {
            Drag::Move { g, origin, anchor, from, free } => {
                let free = *free || ev.modifiers.alt;
                let dt = self.snap_tick(anchor.tick as f64 + tick - from.0, free) - anchor.tick;
                let dt = dt.max(-origin.iter().map(|(_, p)| p.tick).min().unwrap_or(0));
                let dv = self.y_to_norm(y) - from.1;
                let ch = origin
                    .iter()
                    .map(|(n, p)| {
                        let f = |v: f64| param.denorm(param.norm(v) + dv);
                        (*n, Some(Point { tick: p.tick + dt, value: f(p.value), jump: p.jump.map(f), ..*p }))
                    })
                    .collect();
                self.gesture(g, "Move points", ch, cx);
            }
            Drag::Jump { g, n, origin } => {
                let v = param.denorm(self.y_to_norm(y));
                self.gesture(g, "Jump", vec![(*n, Some(Point { jump: Some(v), ..*origin }))], cx);
            }
            Drag::Curve { g, n, origin, from_y, rising } => {
                // up bends the segment up, whichever way it goes
                let d = ((*from_y - y) / 80.0) as f64 * if *rising { 1.0 } else { -1.0 };
                let curve = (origin.curve + d).clamp(-1.0, 1.0);
                self.gesture(g, "Curve", vec![(*n, Some(Point { curve, ..*origin }))], cx);
            }
            Drag::Rect { to, .. } => *to = point(px(x), px(y)),
            Drag::Erase { g } => {
                if let Some((n, _)) = self.point_at(&lane.sorted(), &param, x, y) {
                    self.gesture(g, "Erase points", vec![(n, None)], cx);
                }
            }
            Drag::Pencil { stroke } => {
                let t = self.snap_tick(tick, true);
                if stroke.last().is_none_or(|l| t > l.0) {
                    stroke.push((t, param.norm(param.denorm(self.y_to_norm(y)))));
                }
            }
            Drag::Scrub { g, prop, origin, from_y } => {
                let d = (*from_y - ev.position.y.as_f32()) as f64 * if ev.modifiers.shift { 0.25 } else { 1.0 };
                let grid = self.grid_ticks();
                let ch: Vec<PointChange> = match prop {
                    Prop::Time => origin.iter().map(|(n, p)| (*n, Some(Point { tick: (p.tick + (d / 6.0) as i64 * grid).max(0), ..*p }))).collect(),
                    Prop::Value => origin.iter().map(|(n, p)| (*n, Some(Point { value: param.denorm(param.norm(p.value) + d / 300.0), ..*p }))).collect(),
                    Prop::Jump => origin.iter().map(|(n, p)| (*n, Some(Point { jump: Some(param.denorm(param.norm(p.out()) + d / 300.0)), ..*p }))).collect(),
                    Prop::Curve => origin.iter().map(|(n, p)| (*n, Some(Point { curve: (p.curve + d / 100.0).clamp(-1.0, 1.0), shape: Shape::Curve, ..*p }))).collect(),
                    Prop::Scale => {
                        // value scaling: the selection's values around their middle, 1 % a pixel
                        let vals: Vec<f64> = origin.iter().map(|(_, p)| param.norm(p.value)).collect();
                        let mid = (vals.iter().cloned().fold(1.0, f64::min) + vals.iter().cloned().fold(0.0, f64::max)) / 2.0;
                        notes::scale_points(origin, &param, mid, (1.0 + d / 100.0).max(0.0), 0.0)
                    }
                    Prop::Tension => {
                        let Some(id) = self.lane else { return };
                        let v = (origin.first().map_or(lane.tension, |(_, p)| p.value) + d / 200.0).clamp(0.0, 1.0);
                        self.scx.core.update(cx, |core, _| {
                            let port = core.port_mut();
                            let g = g.get_or_insert_with(|| port.begin_gesture("Tension"));
                            port.gesture_apply(g, vec![EditOp::set(id, Field::Clip("tension"), Value::Real(v))]);
                        });
                        Vec::new()
                    }
                };
                if !ch.is_empty() {
                    self.gesture(g, "Point property", ch, cx);
                }
            }
        }
        self.drag = Some(drag);
        cx.notify();
    }

    fn drag_up(&mut self, _ev: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else { return };
        match drag {
            Drag::Move { g, .. } | Drag::Jump { g, .. } | Drag::Curve { g, .. } | Drag::Erase { g } | Drag::Scrub { g, .. } => self.commit(g, cx),
            Drag::Rect { from, to, toggle, before } => {
                let pts = self.points(cx);
                let param = self.param(cx);
                let (x0, x1) = (from.x.as_f32().min(to.x.as_f32()), from.x.as_f32().max(to.x.as_f32()));
                let (y0, y1) = (from.y.as_f32().min(to.y.as_f32()), from.y.as_f32().max(to.y.as_f32()));
                let mut s = (*before).clone();
                for (n, p) in pts.iter() {
                    let (x, y) = (self.tick_to_x(p.tick as f64), self.norm_to_y(param.norm(p.value)));
                    if x >= x0 && x <= x1 && y >= y0 && y <= y1 && !(toggle && s.remove(n)) {
                        s.insert(*n);
                    }
                }
                self.set_sel(s);
            }
            Drag::Pencil { stroke } => {
                // the stroke becomes the fewest points that follow it, replacing the points under it
                let Some(lane) = self.lane(cx).cloned() else { return };
                if stroke.len() >= 2 {
                    let (t0, t1) = (stroke[0].0, stroke[stroke.len() - 1].0);
                    let kept = notes::thin(&stroke, 0.01);
                    let mut ch: Vec<PointChange> = lane.points.iter().filter(|(_, p)| p.tick >= t0 && p.tick <= t1).map(|(n, _)| (*n, None)).collect();
                    let shape = if lane.param.discrete() { Shape::Step } else { Shape::Curve };
                    ch.extend(Self::temps(kept.len()).into_iter().zip(kept.iter().map(|(t, v)| Some(Point::new(*t, lane.param.denorm(*v), shape)))));
                    let new = self.transact("Draw freehand", ch, cx);
                    self.last_added = new.last().copied();
                }
            }
        }
        cx.notify();
    }

    fn hover_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.drag.is_none() {
            self.hover = Some(self.local(&self.curve_bounds, ev.position));
            cx.notify();
        }
    }

    fn ruler_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let (x, _) = self.local(&self.ruler_bounds, ev.position);
        let t = notes::snap(self.x_to_tick(x), self.grid_ticks()).max(0);
        self.cursor = t;
        let s = notes::tick_to_sample(t as f64);
        self.scx.core.update(cx, |core, _| {
            let _ = core.port_mut().run_action("transport.locate", Value::Real(s));
        });
        cx.notify();
    }

    fn scrub_down(&mut self, prop: Prop, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let pts = self.points(cx);
        let mut origin = self.selected(&pts);
        if prop == Prop::Tension {
            // the lane's tension rides in the value slot of a stand-in point
            let t = self.lane(cx).map_or(0.5, |l| l.tension);
            origin = vec![(0, Point::new(0, t, Shape::Smooth))];
        }
        if origin.is_empty() {
            return;
        }
        if ev.click_count >= 2 && prop != Prop::Scale {
            let p = origin[0].1;
            let text = match prop {
                Prop::Time => format_bbt(p.tick),
                Prop::Value | Prop::Tension => format!("{:.2}", p.value),
                Prop::Jump => format!("{:.2}", p.out()),
                Prop::Curve => format!("{:.2}", p.curve),
                Prop::Scale => String::new(),
            };
            self.entry = Some((prop, ValueEntry::new(text, ev.position, window, cx)));
        } else {
            self.drag = Some(Drag::Scrub { g: None, prop, origin, from_y: ev.position.y.as_f32() });
        }
        cx.notify();
    }

    fn apply_entry(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some((prop, _)) = self.entry.take() else { return };
        let pts = self.points(cx);
        let param = self.param(cx);
        let num = text.trim().replace(',', ".").parse::<f64>().ok();
        if prop == Prop::Tension {
            if let Some(v) = num {
                self.set_lane_field("Tension", "tension", Value::Real(v), cx);
            }
            return;
        }
        let parse_bbt = |s: &str| -> Option<i64> {
            let v: Vec<i64> = s.split('.').map(|x| x.trim().parse().ok()).collect::<Option<Vec<i64>>>()?;
            Some((v.first()? - 1) * BAR + (v.get(1).copied().unwrap_or(1) - 1) * notes::BEAT + v.get(2).copied().unwrap_or(0))
        };
        let ch = self
            .selected(&pts)
            .into_iter()
            .filter_map(|(n, p)| {
                let q = match prop {
                    Prop::Time => Point { tick: parse_bbt(text)?, ..p },
                    Prop::Value => Point { value: num?.clamp(param.min, param.max), ..p },
                    Prop::Jump => Point { jump: Some(num?.clamp(param.min, param.max)), ..p },
                    Prop::Curve => Point { curve: num?, shape: Shape::Curve, ..p },
                    _ => p,
                };
                Some((n, Some(q)))
            })
            .collect();
        self.transact("Point value", ch, cx);
    }

    fn set_shape(&mut self, s: Shape, cx: &mut Context<Self>) {
        self.shape = s;
        let pts = self.points(cx);
        let ch = self.selected(&pts).into_iter().map(|(n, p)| (n, Some(Point { shape: s, ..p }))).collect();
        self.transact("Shape", ch, cx);
    }

    pub fn flip(&mut self, in_time: bool, cx: &mut Context<Self>) {
        let pts = self.points(cx);
        let param = self.param(cx);
        let which = self.targets(&pts);
        self.transact(if in_time { "Flip horizontally" } else { "Flip vertically" }, notes::flip_points(&pts, &which, &param, in_time), cx);
    }

    fn toggle_armed(&mut self, cx: &mut Context<Self>) {
        let armed = self.lane(cx).is_some_and(|l| l.armed);
        self.set_lane_field("Record automation", "armed", Value::Bool(!armed), cx);
    }

    fn menu_items(&self, m: Menu, cx: &gpui::App) -> Vec<MenuItem> {
        let music = &self.scx.core.read(cx).port().root().music;
        match m {
            Menu::Lane => {
                let mut last_owner = None;
                music
                    .lanes
                    .iter()
                    .map(|l| {
                        let it = MenuItem::new(tr(l.param.name), self.lane == Some(l.id)).color(l.color);
                        let it = if last_owner != Some(l.owner) { it.header(l.owner_name.clone()) } else { it };
                        last_owner = Some(l.owner);
                        it
                    })
                    .collect()
            }
            Menu::Grid => GRIDS.iter().enumerate().map(|(i, (l, _))| MenuItem::new(*l, i == self.grid)).collect(),
            Menu::Shape => SHAPES.iter().map(|(s, name)| MenuItem::new(tr(name), *s == self.shape)).collect(),
            Menu::Ghost => std::iter::once(MenuItem::new(tr("No ghost notes"), self.ghost.is_none()))
                .chain(music.clips.iter().map(|c| MenuItem::new(format!("{} · {}", c.track_name, c.name), self.ghost == Some(c.id)).color(c.color)))
                .collect(),
        }
    }

    fn pick(&mut self, m: Menu, i: usize, cx: &mut Context<Self>) {
        self.menu = None;
        let (lane, clip) = {
            let music = &self.scx.core.read(cx).port().root().music;
            (music.lanes.get(i).map(|l| l.id), i.checked_sub(1).and_then(|k| music.clips.get(k)).map(|c| c.id))
        };
        match m {
            Menu::Lane => {
                if let Some(id) = lane {
                    self.open_lane(id, cx);
                }
            }
            Menu::Grid => self.grid = i,
            Menu::Shape => self.set_shape(SHAPES[i].0, cx),
            Menu::Ghost => self.ghost = clip,
        }
        cx.notify();
    }

    fn key_down(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = ev.keystroke.key.as_str();
        let m = ev.keystroke.modifiers;
        let p = primary(&m);
        let pts = self.points(cx);
        let handled = match (k, p, m.shift) {
            ("z", true, _) => {
                self.drag = None;
                self.scx.core.update(cx, |core, _| {
                    let _ = core.port_mut().undo();
                });
                true
            }
            ("a", true, false) => {
                self.set_sel(pts.keys().copied().collect());
                true
            }
            ("a", true, true) | ("escape", ..) => {
                self.set_sel(BTreeSet::new());
                self.menu = None;
                self.entry = None;
                true
            }
            ("c", true, _) | ("x", true, _) => {
                let sel = self.selected(&pts);
                let a = sel.iter().map(|(_, p)| p.tick).min().unwrap_or(0);
                self.clipboard = sel.iter().map(|(_, p)| Point { tick: p.tick - a, ..*p }).collect();
                if k == "x" {
                    self.transact("Cut points", sel.into_iter().map(|(n, _)| (n, None)).collect(), cx);
                }
                true
            }
            ("v", true, _) => {
                let at = self.cursor;
                let ch = Self::temps(self.clipboard.len()).into_iter().zip(self.clipboard.iter()).map(|(n, p)| (n, Some(Point { tick: p.tick + at, ..*p }))).collect();
                let new = self.transact("Paste points", ch, cx);
                self.set_sel(new.into_iter().collect());
                true
            }
            ("delete" | "backspace", ..) => {
                let sel = self.selected(&pts);
                self.transact("Delete points", sel.into_iter().map(|(n, _)| (n, None)).collect(), cx);
                self.set_sel(BTreeSet::new());
                true
            }
            ("left" | "right", ..) => {
                let d = if k == "right" { 1 } else { -1 };
                self.cursor = (self.cursor + d * self.grid_ticks()).max(0);
                true
            }
            ("up" | "down", ..) => {
                let d = if k == "up" { 0.1 } else { -0.1 } * (self.v_hi - self.v_lo);
                let d = d.clamp(-self.v_lo, 1.0 - self.v_hi);
                self.v_lo += d;
                self.v_hi += d;
                self.auto_fit = false;
                true
            }
            ("home", ..) => {
                self.cursor = 0;
                self.scroll = 0.0;
                true
            }
            (_, false, true) => {
                match k {
                    "d" => self.tool = Tool::Draw,
                    "p" => self.tool = Tool::Pencil,
                    "s" => self.tool = Tool::Select,
                    "e" => self.tool = Tool::Erase,
                    _ => return,
                }
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let (dx, dy) = (d.x.as_f32(), d.y.as_f32());
        let (x, y) = self.local(&self.curve_bounds, ev.position);
        let m = ev.modifiers;
        if primary(&m) && m.alt {
            self.grid = (self.grid as i32 - dy.signum() as i32).clamp(0, GRIDS.len() as i32 - 1) as usize;
        } else if primary(&m) {
            let anchor = self.x_to_tick(x);
            self.tpp = (self.tpp * (-dy * 0.01).exp() as f64).clamp(0.5, 800.0);
            self.scroll = (anchor - x as f64 * self.tpp).max(0.0);
        } else if m.alt {
            // the value range zooms around the mouse
            let anchor = self.y_to_norm(y);
            let span = ((self.v_hi - self.v_lo) * (dy * 0.01).exp() as f64).clamp(0.02, 1.0);
            let t = (anchor - self.v_lo) / (self.v_hi - self.v_lo);
            self.v_lo = (anchor - t * span).clamp(0.0, 1.0 - span);
            self.v_hi = self.v_lo + span;
            self.auto_fit = false;
        } else {
            // one wheel scrolls in time: the lane has no vertical scroll of its own
            let d = if dx.abs() > dy.abs() { dx } else { dy };
            self.scroll = (self.scroll - d as f64 * self.tpp).max(0.0);
        }
        cx.notify();
    }

    fn on_pinch(&mut self, ev: &PinchEvent, cx: &mut Context<Self>) {
        let (x, _) = self.local(&self.curve_bounds, ev.position);
        let anchor = self.x_to_tick(x);
        self.tpp = (self.tpp / (1.0 + ev.delta as f64).max(0.1)).clamp(0.5, 800.0);
        self.scroll = (anchor - x as f64 * self.tpp).max(0.0);
        cx.notify();
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Rendering

struct CurvePaint {
    lane: AutoLane,
    sel: Rc<BTreeSet<u64>>,
    ghost: Option<Arc<NoteMap>>,
    scroll: f64,
    tpp: f64,
    v_lo: f64,
    v_hi: f64,
    grid: i64,
    play: Option<f64>,
    cursor: i64,
    hover: Option<(f32, f32)>,
    rect: Option<(gpui::Point<Pixels>, gpui::Point<Pixels>)>,
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
        6.0 + (1.0 - ((v - self.v_lo) / (self.v_hi - self.v_lo)) as f32) * (h - 12.0)
    }

    fn paint(&self, b: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        let o = b.origin;
        let (w, h) = (b.size.width.as_f32(), b.size.height.as_f32());
        let r = |x: f32, y: f32, ww: f32, hh: f32| Bounds::new(point(o.x + px(x), o.y + px(y)), size(px(ww), px(hh)));
        window.paint_quad(fill(b, c(THEME.bg1)));
        let param = &self.lane.param;
        // horizontal lines: eighths of the range, or each choice
        let levels: Vec<f64> = if !param.choices.is_empty() {
            (0..param.choices.len()).map(|i| param.norm(i as f64)).collect()
        } else {
            (0..=8).map(|i| i as f64 / 8.0).collect()
        };
        for v in levels {
            let y = self.y(v, h);
            if (0.0..=h).contains(&y) {
                window.paint_quad(fill(r(0.0, y, w, 1.0), c(THEME.bg2)));
            }
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
        lines(self.grid, THEME.bg2);
        lines(notes::BEAT, THEME.bg3);
        lines(BAR, THEME.line2);
        // ghost notes (F-AUT-009): a MIDI item's notes, by height in their own key range
        if let Some(g) = &self.ghost {
            let (lo, hi) = g.values().fold((127u8, 0u8), |(a, z), x| (a.min(x.key), z.max(x.key)));
            let span = (hi as f32 - lo as f32 + 1.0).max(12.0);
            for x in g.values() {
                let (a, z) = (x.start as f64, x.end() as f64);
                if z < t0 || a > t1 {
                    continue;
                }
                let y = h - 6.0 - (x.key as f32 - lo as f32 + 1.0) / span * (h - 12.0);
                window.paint_quad(fill(r(self.x(a), y, ((z - a) / self.tpp) as f32, 3.0), with_alpha(THEME.text3, 0.35)));
            }
        }
        // the curve: one stroke and one fill, a sample every 3 px, exact at each point (a jump is a vertical step)
        let pts = self.lane.sorted();
        let col = self.lane.color;
        let at = |t: f64| param.norm(notes::value_at(&pts, t, self.lane.tension, param));
        let step_px = 3.0;
        let mut line: Vec<gpui::Point<Pixels>> = Vec::new();
        let mut k = pts.partition_point(|(_, p)| (p.tick as f64) < self.t(-step_px));
        let mut x = -step_px;
        while x <= w + step_px {
            let t = self.t(x);
            while k < pts.len() && (pts[k].1.tick as f64) <= t {
                let p = pts[k].1;
                let px_ = self.x(p.tick as f64);
                // a step holds until the point: the line reaches it at the old value first
                if k > 0 {
                    line.push(o + point(px(px_), px(self.y(at(p.tick as f64 - 0.5), h))));
                }
                line.push(o + point(px(px_), px(self.y(param.norm(p.value), h))));
                line.push(o + point(px(px_), px(self.y(param.norm(p.out()), h))));
                k += 1;
            }
            line.push(o + point(px(x), px(self.y(at(t), h))));
            x += step_px;
        }
        if line.len() >= 2 {
            let mut area = line.clone();
            area.push(o + point(px(w + step_px), px(h)));
            area.push(o + point(px(-step_px), px(h)));
            let mut pb = PathBuilder::fill();
            pb.add_polygon(&area, true);
            if let Ok(p) = pb.build() {
                window.paint_path(p, with_alpha(col, 0.14));
            }
            let mut pb = PathBuilder::stroke(px(1.6));
            pb.move_to(line[0]);
            for p in &line[1..] {
                pb.line_to(*p);
            }
            if let Ok(p) = pb.build() {
                window.paint_path(p, c(col));
            }
        }
        // the points, their jumps and the curve handles (only the visible ones)
        for (i, (n, p)) in pts.iter().enumerate() {
            let px_ = self.x(p.tick as f64);
            if !(-10.0..=w + 10.0).contains(&px_) {
                continue;
            }
            let selected = self.sel.contains(n);
            let edge = c(if selected { THEME.text } else { col });
            let dot = |window: &mut Window, y: f32, filled: bool| {
                let bb = r(px_ - POINT_R, y - POINT_R, POINT_R * 2.0, POINT_R * 2.0);
                window.paint_quad(gpui::quad(bb, px(POINT_R), if filled { edge } else { c(THEME.bg1) }, px(1.5), edge, Default::default()));
            };
            dot(window, self.y(param.norm(p.value), h), true);
            if let Some(j) = p.jump {
                dot(window, self.y(param.norm(j), h), false);
            }
            if p.shape == Shape::Curve && !param.discrete() {
                if let Some((_, q)) = pts.get(i + 1) {
                    let mid = (p.tick + q.tick) as f64 / 2.0;
                    let (hx, hy) = (self.x(mid), self.y(at(mid), h));
                    let near = self.hover.is_some_and(|(x, y)| (x - hx).abs() < 8.0 && (y - hy).abs() < 8.0);
                    let s = if near { 4.0 } else { 3.0 };
                    window.paint_quad(gpui::quad(r(hx - s, hy - s, s * 2.0, s * 2.0), px(1.0), with_alpha(col, if near { 1.0 } else { 0.6 }), px(0.0), c(col), Default::default()));
                }
            }
        }
        // the pencil's stroke as it is drawn
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
            window.paint_quad(gpui::quad(bb, px(0.0), gpui::rgba(THEME.selection), px(1.0), c(THEME.blue), Default::default()));
        }
        // the readout under the mouse: the time and the value
        if let Some((hx, hy)) = self.hover {
            let t = self.t(hx).max(0.0);
            let v = param.denorm(self.v_lo + (1.0 - ((hy - 6.0) / (h - 12.0)) as f64) * (self.v_hi - self.v_lo));
            let text = format!("{}  {}", format_bbt(t as i64), shown(param, v));
            paint_text(window, cx, text.into(), o.x + px(hx + 10.0), o.y + px((hy - 16.0).max(2.0)), 10.0, c(THEME.text2));
        }
        window.paint_quad(fill(r(self.x(self.cursor as f64), 0.0, 1.0, h), with_alpha(THEME.cursor, 0.35)));
        if let Some(t) = self.play {
            let x = self.x(t);
            if x >= 0.0 && x < w {
                window.paint_quad(fill(r(x, 0.0, 1.0, h), c(THEME.cursor)));
                // the value the parameter has now, on the curve
                let y = self.y(at(t), h);
                window.paint_quad(gpui::quad(r(x - 3.0, y - 3.0, 6.0, 6.0), px(3.0), c(THEME.cursor), px(0.0), c(THEME.cursor), Default::default()));
            }
        }
    }
}

/// A value as shown: a choice's translated name, or the number and its unit
fn shown(p: &ParamInfo, v: f64) -> String {
    if !p.choices.is_empty() { tr(p.choices[(v.round().max(0.0) as usize).min(p.choices.len() - 1)]).to_string() } else { p.format(v) }
}

impl Render for AutomationEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        // I-3.6: after an undo, selected points that are gone are dropped
        {
            let port = self.scx.core.read(cx).port();
            if let Some(diff) = port.drain(&mut self.feed) {
                if let Some(l) = self.lane.filter(|id| diff.touches(*id)).and_then(|id| port.root().music.lane(id)) {
                    if self.sel.iter().any(|n| !l.points.contains_key(n)) {
                        let keep = self.sel.iter().copied().filter(|n| l.points.contains_key(n)).collect();
                        self.sel = Rc::new(keep);
                    }
                }
            }
        }
        if self.auto_fit && self.drag.is_none() {
            self.fit_values(cx);
        }
        let pos = self.scx.core.read(cx).port().telemetry().position;
        if pos.playing {
            window.request_animation_frame();
        }
        let play = Some(notes::sample_to_tick(pos.at(now)));
        let entity = cx.entity();
        div()
            .id("automation")
            .size_full()
            .flex()
            .flex_col()
            .bg(c(THEME.bg1))
            .text_color(c(THEME.text))
            .track_focus(&self.focus)
            .key_context("AutomationEditor")
            .on_key_down(cx.listener(|ae, ev: &KeyDownEvent, _, cx| ae.key_down(ev, cx)))
            .child(self.render_toolbar(cx))
            .child(match self.lane(cx).cloned() {
                None => div().flex_1().flex().items_center().justify_center().text_color(c(THEME.text2)).child(tr("No automation lane")).into_any_element(),
                Some(lane) => div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .child(self.render_axis(&lane))
                    .child(self.render_center(lane, play, cx))
                    .child(self.render_inspector(cx))
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
    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let lane = self.lane(cx);
        let ghost = self.ghost.and_then(|g| self.scx.core.read(cx).port().root().music.clip(g)).map(|c| c.name.clone());
        let open = |m: Menu| {
            cx.listener(move |ae: &mut AutomationEditor, ev: &MouseDownEvent, _, cx| {
                ae.menu = if ae.menu.is_some_and(|x| x.0 == m) { None } else { Some((m, ev.position + point(px(0.), px(10.)))) };
                cx.notify();
            })
        };
        let armed = lane.is_some_and(|l| l.armed);
        let discrete = lane.is_some_and(|l| l.param.discrete());
        div()
            .h(px(TOOLBAR_H))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .px(px(6.))
            .bg(c(THEME.bg2))
            .border_b_1()
            .border_color(c(THEME.line))
            .overflow_hidden()
            .child(
                dropdown("ae-lane", lane.map_or_else(|| tr("No automation lane").to_string(), |l| format!("{} · {}", l.owner_name, tr(l.param.name))), true)
                    .debug_selector(|| "ae-lane".into())
                    .min_w(px(170.))
                    .child(div().size(px(8.)).rounded(px(2.)).bg(c(lane.map_or(THEME.text3, |l| l.color))))
                    .on_mouse_down(MouseButton::Left, open(Menu::Lane)),
            )
            .child(sep())
            .children(TOOLS.iter().map(|(t, name, help)| {
                let t = *t;
                let help: &'static str = help;
                button(SharedString::from(format!("ae-tool-{name}")), tr(name), self.tool == t, THEME.green)
                    .h(px(22.))
                    .px(px(6.))
                    .tooltip(tip(help))
                    .on_click(cx.listener(move |ae, _, _, cx| {
                        ae.tool = t;
                        cx.notify();
                    }))
            }))
            .child(sep())
            .child(label(tr("Shape")))
            .child(
                dropdown("ae-shape", if discrete { tr("Step") } else { tr(SHAPES.iter().find(|s| s.0 == self.shape).map_or("Curve", |s| s.1)) }, false)
                    .tooltip(tip("The shape of new points; with points selected, changes theirs"))
                    .on_mouse_down(MouseButton::Left, open(Menu::Shape)),
            )
            .child(label(tr("Grid")))
            .child(dropdown("ae-grid", GRIDS[self.grid].0, false).on_mouse_down(MouseButton::Left, open(Menu::Grid)))
            .child(sep())
            .child(button("ae-flip-h", tr("Flip ↔"), false, 0).h(px(22.)).tooltip(tip("Flip the selection (or the whole lane) in time")).on_click(cx.listener(|ae, _, _, cx| ae.flip(true, cx))))
            .child(button("ae-flip-v", tr("Flip ↕"), false, 0).h(px(22.)).tooltip(tip("Flip the selection (or the whole lane) in value")).on_click(cx.listener(|ae, _, _, cx| ae.flip(false, cx))))
            .child(
                button("ae-fit", tr("Fit"), self.auto_fit, THEME.blue)
                    .h(px(22.))
                    .tooltip(tip("Fit the height to the points (Alt+wheel zooms the values)"))
                    .on_click(cx.listener(|ae, _, _, cx| {
                        ae.auto_fit = !ae.auto_fit;
                        if !ae.auto_fit {
                            ae.v_lo = 0.0;
                            ae.v_hi = 1.0;
                        }
                        cx.notify();
                    })),
            )
            .child(
                dropdown("ae-ghost", ghost.map_or_else(|| tr("Ghost").to_string(), |g| trf("Ghost: {0}", &[&g])), self.ghost.is_some())
                    .tooltip(tip("Show a MIDI item's notes under the curve"))
                    .on_mouse_down(MouseButton::Left, open(Menu::Ghost)),
            )
            .child(div().flex_1())
            .child(
                button("ae-rec", tr("● Record"), armed, THEME.red)
                    .h(px(22.))
                    .tooltip(tip("Record: moving the control while the song records writes points (only the changes)"))
                    .on_click(cx.listener(|ae, _, _, cx| ae.toggle_armed(cx))),
            )
    }

    /// The value axis, in the parameter's units
    fn render_axis(&self, lane: &AutoLane) -> impl IntoElement {
        let param = lane.param;
        let (lo, hi) = (self.v_lo, self.v_hi);
        let h = self.curve_bounds.get().size.height.as_f32().max(40.0);
        let marks: Vec<f64> = if !param.choices.is_empty() {
            (0..param.choices.len()).map(|i| param.norm(i as f64)).filter(|v| *v >= lo && *v <= hi).collect()
        } else {
            (0..=4).map(|i| lo + (hi - lo) * i as f64 / 4.0).collect()
        };
        let mut area = div().flex_1().relative().overflow_hidden();
        for v in marks {
            let y = 6.0 + (1.0 - ((v - lo) / (hi - lo)) as f32) * (h - 12.0);
            area = area.child(div().absolute().top(px(y - 6.0)).right(px(4.)).text_size(px(9.5)).text_color(c(THEME.text2)).child(shown(&param, param.denorm(v))));
        }
        div()
            .w(px(AXIS_W))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(c(THEME.bg2))
            .border_r_1()
            .border_color(c(THEME.line))
            .child(div().h(px(RULER_H)).px(px(4.)).text_size(px(9.5)).text_color(c(THEME.text3)).child(param.unit))
            .child(area)
    }

    fn render_center(&self, lane: AutoLane, play: Option<f64>, cx: &Context<Self>) -> impl IntoElement {
        let ghost = self.ghost.and_then(|g| self.scx.core.read(cx).port().root().music.clip(g)).map(|c| c.notes.clone());
        let paint = CurvePaint {
            lane,
            sel: self.sel.clone(),
            ghost,
            scroll: self.scroll,
            tpp: self.tpp,
            v_lo: self.v_lo,
            v_hi: self.v_hi,
            grid: self.grid_ticks(),
            play,
            cursor: self.cursor,
            hover: if self.drag.is_none() { self.hover } else { None },
            rect: match &self.drag {
                Some(Drag::Rect { from, to, .. }) => Some((*from, *to)),
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
                    .h(px(RULER_H))
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
                                    window.paint_quad(fill(Bounds::new(point(o.x + px(xx), o.y + px(4.0)), size(px(1.0), px(16.0))), c(THEME.text3)));
                                    paint_text(window, cx, format!("{}", t / BAR + 1).into(), o.x + px(xx + 3.0), o.y + px(1.0), 10.0, c(THEME.text2));
                                    t += step;
                                }
                                let cx_ = x(cursor as f64);
                                let mut pb = PathBuilder::fill();
                                pb.add_polygon(&[o + point(px(cx_ - 5.0), px(0.0)), o + point(px(cx_ + 5.0), px(0.0)), o + point(px(cx_), px(7.0))], true);
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
                    .on_mouse_move(cx.listener(|ae, ev: &MouseMoveEvent, _, cx| ae.hover_move(ev, cx)))
                    .on_hover(cx.listener(|ae, on: &bool, _, cx| {
                        if !on {
                            ae.hover = None;
                            cx.notify();
                        }
                    }))
                    .on_scroll(cx.listener(|ae, ev: &ScrollEvent, _, cx| ae.on_scroll(ev, cx)))
                    .on_pinch(cx.listener(|ae, ev: &PinchEvent, _, cx| ae.on_pinch(ev, cx)))
                    .child(canvas(move |b, _, _| cb.set(b), move |b, _, window, cx| paint.paint(b, window, cx)).size_full()),
            )
    }

    fn render_inspector(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(lane) = self.lane(cx) else { return div() };
        let param = lane.param;
        let sel = self.selected(&lane.points);
        let row = |prop: Prop, name: &'static str, value: String| {
            div()
                .id(SharedString::from(format!("ae-prop-{name}")))
                .debug_selector(move || format!("ae-prop-{name}"))
                .flex()
                .flex_row()
                .items_center()
                .h(px(20.))
                .px(px(6.))
                .rounded(px(3.))
                .bg(c(THEME.bg1))
                .cursor(CursorStyle::ResizeUpDown)
                .hover(|s| s.bg(c(THEME.bg3)))
                .on_mouse_down(MouseButton::Left, cx.listener(move |ae, ev: &MouseDownEvent, window, cx| ae.scrub_down(prop, ev, window, cx)))
                .child(div().flex_1().text_size(px(11.)).text_color(c(THEME.text2)).child(tr(name)))
                .child(div().text_size(px(11.)).font_weight(gpui::FontWeight::BOLD).child(value))
        };
        let mut col = div()
            .w(px(INSPECTOR_W))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(8.))
            .bg(c(THEME.bg2))
            .border_l_1()
            .border_color(c(THEME.line))
            .overflow_hidden()
            .child(caption(tr("LANE")))
            .child(div().text_size(px(12.)).font_weight(gpui::FontWeight::BOLD).child(format!("{} · {}", lane.owner_name, tr(param.name))))
            .child(div().text_size(px(11.)).text_color(c(THEME.text2)).child(if param.choices.is_empty() {
                trf("{0} to {1} · {2} points", &[&param.format(param.min), &param.format(param.max), &lane.points.len()])
            } else {
                trf("A choice: steps only · {0} points", &[&lane.points.len()])
            }));
        if !lane.also.is_empty() {
            col = col.child(caption(tr("ALSO MOVES (SHARED CONTROL)"))).children(lane.also.iter().map(|a| div().text_size(px(11.)).text_color(c(THEME.cyan)).child(a.clone())));
        }
        if !param.discrete() {
            col = col.child(row(Prop::Tension, "Smooth tension", format!("{:.2}", lane.tension)));
        }
        col = col.child(div().h(px(4.)));
        if sel.is_empty() {
            return col.child(div().text_size(px(10.5)).text_color(c(THEME.text3)).child(tr(match self.tool {
                Tool::Draw => "Click adds a point and drags it; Shift+click draws a line of points from the last one; Alt+drag on a point gives it a jump; drag a segment's middle handle to bend it; double click a point to type its value; right button erases.",
                Tool::Pencil => "Draw the curve with the mouse; it becomes the fewest points that follow it.",
                Tool::Select => "Drag a rectangle; Shift toggles. Drag the selection to move it.",
                Tool::Erase => "Click or drag over points to erase them.",
            })));
        }
        let first = sel[0].1;
        let all = |f: &dyn Fn(&Point) -> String| {
            let v = f(&first);
            if sel.iter().all(|(_, p)| f(p) == v) { v } else { tr("mixed").to_string() }
        };
        col.child(caption(trf("{0} POINTS SELECTED", &[&sel.len()])))
            .child(row(Prop::Time, "Position", all(&|p| format_bbt(p.tick))))
            .child(row(Prop::Value, "Value", all(&|p| shown(&param, p.value))))
            .child(row(Prop::Jump, "Leaves at", all(&|p| if p.jump.is_some() { shown(&param, p.out()) } else { tr("same").to_string() })))
            .when(!param.discrete(), |d| {
                d.child(row(Prop::Curve, "Curve", all(&|p| match p.shape {
                    Shape::Curve => format!("{:+.2}", p.curve),
                    s => tr(SHAPES.iter().find(|x| x.0 == s).map_or("", |x| x.1)).to_string(),
                })))
            })
            .when(sel.len() > 1, |d| d.child(row(Prop::Scale, "Scale values", tr("drag").to_string())))
            .child(div().text_size(px(10.)).text_color(c(THEME.text3)).child(tr("Drag a value up or down (Shift: fine); double click to type it.")))
            .when(!param.discrete(), |d| {
                d.child(div().flex().flex_row().flex_wrap().gap(px(3.)).children(SHAPES.iter().map(|(s, name)| {
                    let s = *s;
                    button(SharedString::from(format!("ae-shape-{name}")), tr(name), sel.iter().all(|(_, p)| p.shape == s), THEME.blue)
                        .on_click(cx.listener(move |ae, _, _, cx| ae.set_shape(s, cx)))
                })))
            })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpui::{Modifiers, TestAppContext, VisualTestContext, point, px};

    use super::*;
    use crate::screens::Screen;
    use crate::screens::piano_roll::tests::{drag, open};

    #[gpui::test]
    fn add_drag_and_undo_a_point(cx: &mut TestAppContext) {
        let (ae, core, cx) = open(cx, AutomationEditor::new, Screen::Automation);
        let id = ae.read_with(cx, |ae, _| ae.lane.unwrap());
        let count = |cx: &mut VisualTestContext| core.read_with(cx, |core, _| core.port().root().music.lane(id).unwrap().points.len());
        let before = count(cx);
        let b = cx.debug_bounds("ae-curve").unwrap();
        // a click between two bars adds a point on the grid; dragging it up moves it: one gesture
        let x = ae.read_with(cx, |ae, _| ae.tick_to_x(BAR as f64 + notes::BEAT as f64 * 2.0 + 20.0));
        let from = point(b.origin.x + px(x), b.origin.y + b.size.height * 0.5);
        drag(cx, from, point(from.x, from.y - px(20.)), Modifiers::none());
        assert_eq!(count(cx), before + 1);
        let p = core
            .read_with(cx, |core, _| core.port().root().music.lane(id).unwrap().points.values().find(|p| p.tick == BAR + notes::BEAT * 2).copied())
            .expect("the point is on the grid");
        assert!(p.value > -27.0 + 0.5, "dragged up from the middle of -60..6 dB: {}", p.value);
        cx.executor().advance_clock(Duration::from_millis(20));
        core.update(cx, |core, _| core.port_mut().undo().ok().unwrap());
        assert_eq!(count(cx), before, "adding and dragging is one undo step");
    }

    #[gpui::test]
    fn choice_lane_keeps_steps_and_flip_is_reversible(cx: &mut TestAppContext) {
        let (ae, core, cx) = open(cx, AutomationEditor::new, Screen::Automation);
        // the choice lane (oscillator wave): a point drawn there is a step with a whole value
        let wave = core.read_with(cx, |core, _| core.port().root().music.lanes.iter().find(|l| !l.param.choices.is_empty()).unwrap().id);
        ae.update(cx, |ae, cx| ae.open_lane(wave, cx));
        cx.run_until_parked();
        let b = cx.debug_bounds("ae-curve").unwrap();
        let x = ae.read_with(cx, |ae, _| ae.tick_to_x(5.0 * BAR as f64));
        cx.simulate_click(point(b.origin.x + px(x), b.origin.y + b.size.height * 0.4), Modifiers::none());
        cx.run_until_parked();
        let pt = core.read_with(cx, |core, _| core.port().root().music.lane(wave).unwrap().points.values().find(|p| p.tick == 5 * BAR).copied()).unwrap();
        assert_eq!(pt.shape, Shape::Step);
        assert_eq!(pt.value, pt.value.round());
        let read = |cx: &mut VisualTestContext| -> Vec<(i64, f64)> {
            core.read_with(cx, |core, _| core.port().root().music.lane(wave).unwrap().sorted().iter().map(|(_, p)| (p.tick, p.value)).collect())
        };
        let before = read(cx);
        ae.update(cx, |ae, cx| {
            ae.set_sel(BTreeSet::new());
            ae.flip(true, cx);
        });
        assert_ne!(read(cx), before);
        ae.update(cx, |ae, cx| ae.flip(true, cx));
        assert_eq!(read(cx), before, "flipping twice in time is the identity");
    }
}
