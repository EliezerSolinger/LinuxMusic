//! Owner: agent D. The ruler's markers and regions on the Composition page (F-REG-011, F-REG-012), two lanes under
//! the ruler, as the legacy's timeline:
//! - markers: M adds one at the cursor ("Marker N"); a double click on the lane adds one there; drag moves; a double
//!   click on a marker renames it; the keys 1–9 jump to markers 1–9; the menu (right click): rename, play from here,
//!   remove;
//! - regions: drag on the empty lane draws one; Shift+R adds one over the loop; drag moves it, its ends resize it;
//!   **a click makes it the loop**; overlapping is allowed; the menu: rename, loop over it, play from its start,
//!   export it as audio, remove.
//!
//! Every change is an edit through the port (`EditOp` on the root's `markers` and `regions`; a drag is one gesture,
//! one undo step). The page holds a `MarksState` and hooks the layer, the keys and the lanes' height.

use std::cell::Cell;

use gpui::{
    AnyElement, App, Context, Entity, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Subscription, Window, div, prelude::*, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use lm_gui_plugins::Gesture;
use lm_port::{EditOp, Label, NodeId, PathStep, ROOT, Ref, Value};

use super::composition::CompositionPage;
use crate::core::Core;
use crate::model::{self, MarkerView, RegionView};
use crate::ui::{THEME, c, tr, track_color, trf};
use crate::view::TimelineView;

/// One lane's height; the two lanes sit under the ruler
pub const LANE_H: f32 = 15.0;
pub const MARKS_H: f32 = 2.0 * LANE_H;
/// How near (px) a click must be to a marker or a region's end
const GRAB: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Marker,
    RegionBody,
    RegionStart,
    RegionEnd,
    /// drawing a new region on the empty lane
    NewRegion,
}

struct Drag {
    part: Part,
    id: Option<NodeId>,
    x0: f32,
    from0: i64,
    to0: i64,
    moved: bool,
    gesture: Option<Gesture>,
    /// a new region's other end, while drawing
    anchor: i64,
}

/// What the lanes keep between frames: the drag, the menu, the name being typed
pub struct MarksState {
    core: Entity<Core>,
    view: Cell<TimelineView>,
    drag: Option<Drag>,
    menu: Option<(NodeId, f32)>,
    /// The menu of an empty lane (right click): the lane (0 markers, 1 regions), the tick, where it opened
    empty_menu: Option<(usize, i64, f32)>,
    rename: Option<(NodeId, Entity<InputState>, Subscription)>,
    /// a new region being drawn: its ticks, shown until the release
    drawing: Option<(i64, i64)>,
}

impl MarksState {
    pub fn new(core: Entity<Core>) -> MarksState {
        MarksState {
            core,
            view: Cell::new(TimelineView::default()),
            drag: None,
            menu: None,
            empty_menu: None,
            rename: None,
            drawing: None,
        }
    }

    fn root(&self, cx: &App) -> Value {
        self.core.read(cx).port().root().clone()
    }

    fn apply(&self, label: &'static str, ops: Vec<EditOp>, cx: &mut App) {
        self.core.update(cx, |c, cx| {
            c.report(|pt| pt.apply(Label::new(label), ops));
            cx.notify();
        });
    }

    fn run(&self, action: &str, params: Value, cx: &mut App) {
        self.core.update(cx, |c, cx| {
            c.report(|pt| pt.run_action(action.into(), params));
            cx.notify();
        });
    }

    /// The grid a marker or a region snaps to: a beat
    fn snap(&self, tick: f64, cx: &App) -> i64 {
        let ppq = model::ppq(self.core.read(cx).port().root()) as f64;
        ((tick / ppq).round() * ppq).max(0.0) as i64
    }

    fn cursor_tick(&self, cx: &App) -> i64 {
        let port = self.core.read(cx).port();
        port.tempo_map().tick_at(port.telemetry().position.samples)
    }

    fn insert(&self, list: &str, node: Value, cx: &mut App) -> Option<NodeId> {
        let root = self.root(cx);
        let index = model::list(&root, list).count();
        let label = if list == "markers" { "Add a marker" } else { "Add a region" };
        let op = EditOp::Insert { temp: 1, parent: Ref::Id(ROOT), list: list.into(), index, node };
        self.core.update(cx, |c, cx| {
            let made = c.report(|pt| pt.apply(Label::new(label), vec![op]));
            cx.notify();
            made.and_then(|a| a.created.first().map(|(_, id)| *id))
        })
    }

    /// M: a marker at the cursor, "Marker N" (one per tick: on a marker's tick nothing is added)
    pub fn add_marker_at_cursor(&mut self, cx: &mut App) {
        let tick = self.cursor_tick(cx);
        self.add_marker(tick, cx);
    }

    fn add_marker(&mut self, tick: i64, cx: &mut App) -> Option<NodeId> {
        let markers = model::markers(&self.root(cx));
        if markers.iter().any(|m| m.tick == tick) {
            return None;
        }
        let name = trf("Marker {0}", &[&(markers.len() + 1)]);
        self.insert("markers", Value::object([("name", Value::text(name)), ("tick", Value::int(tick))]), cx)
    }

    /// "Add a marker here": the marker, its name asked
    fn add_marker_named(&mut self, tick: i64, window: &mut Window, cx: &mut Context<CompositionPage>) {
        self.empty_menu = None;
        if let Some(id) = self.add_marker(tick, cx) {
            self.start_rename(id, window, cx);
        }
    }

    /// A region over the loop if the tick is in it, else the bar of the tick (F-REG-012)
    fn add_region_at(&mut self, tick: i64, over_loop: bool, cx: &mut App) {
        self.empty_menu = None;
        let root = self.root(cx);
        let (from, to) = model::song(&root).loop_range;
        let (a, z) = if over_loop && to > from {
            (from, to)
        } else {
            let map = model::tempo_map(&root);
            let bar = map.bar_beat_tick(tick).bar;
            let a = map.bar_start(bar).unwrap_or(tick);
            (a, map.bar_start(bar + 1).unwrap_or(a + 4 * model::ppq(&root) as i64))
        };
        let n = model::regions(&root).len() + 1;
        let node = Value::object([
            ("name", Value::text(trf("Region {0}", &[&n]))),
            ("from", Value::int(a)),
            ("to", Value::int(z)),
        ]);
        self.insert("regions", node, cx);
    }

    /// The colour of a marker or a region (F-REG-011, F-REG-012); none: the theme's
    fn set_color(&mut self, id: NodeId, color: Option<u32>, cx: &mut App) {
        self.menu = None;
        self.apply(
            "Colour",
            vec![EditOp::Set {
                id: Ref::Id(id),
                path: vec![PathStep::key("color")],
                value: Value::int(i64::from(color.unwrap_or(0))),
            }],
            cx,
        );
    }

    /// Shift+R: a region over the loop
    pub fn add_region_over_loop(&mut self, cx: &mut App) {
        let root = self.root(cx);
        let (from, to) = model::song(&root).loop_range;
        if to > from {
            let n = model::regions(&root).len() + 1;
            let node = Value::object([
                ("name", Value::text(trf("Region {0}", &[&n]))),
                ("from", Value::int(from)),
                ("to", Value::int(to)),
            ]);
            self.insert("regions", node, cx);
        }
    }

    /// 1–9: the cursor to marker n (from 1, in time order)
    pub fn jump(&mut self, n: usize, cx: &mut App) {
        if let Some(m) = model::markers(&self.root(cx)).get(n.saturating_sub(1)) {
            let tick = m.tick;
            self.run("transport.locate", Value::int(tick), cx);
        }
    }

    /// A region becomes the loop (a click on it, F-REG-012)
    fn loop_over(&self, r: &RegionView, cx: &mut App) {
        let set = |k: &str, v: Value| EditOp::Set {
            id: Ref::Id(ROOT),
            path: vec![PathStep::key("transport"), PathStep::key("loop"), PathStep::key(k)],
            value: v,
        };
        self.apply(
            "Loop over the region",
            vec![set("from", Value::int(r.from)), set("to", Value::int(r.to)), set("on", Value::bool(true))],
            cx,
        );
    }

    fn remove(&mut self, id: NodeId, cx: &mut App) {
        self.menu = None;
        self.apply("Remove", vec![EditOp::Remove { id: Ref::Id(id) }], cx);
    }

    fn start_rename(&mut self, id: NodeId, window: &mut Window, cx: &mut Context<CompositionPage>) {
        self.menu = None;
        let root = self.root(cx);
        let name = model::markers(&root)
            .into_iter()
            .find(|m| m.id == id)
            .map(|m| m.name)
            .or_else(|| model::regions(&root).into_iter().find(|r| r.id == id).map(|r| r.name))
            .unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name.to_string()));
        input.update(cx, |i, cx| i.focus(window, cx));
        let sub = cx.subscribe(&input, |page: &mut CompositionPage, st, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let v = st.read(cx).value().trim().to_string();
                page.marks.finish_rename(v, cx);
            }
        });
        self.rename = Some((id, input, sub));
    }

    fn finish_rename(&mut self, name: String, cx: &mut App) {
        if let Some((id, _, _)) = self.rename.take() {
            self.apply(
                "Rename",
                vec![EditOp::Set { id: Ref::Id(id), path: vec![PathStep::key("name")], value: Value::text(name) }],
                cx,
            );
        }
    }

    /// What is under x on a lane: a marker, a region's end or body
    fn hit(&self, root: &Value, lane: usize, x: f32) -> Option<(Part, NodeId)> {
        let v = self.view.get();
        if lane == 0 {
            return model::markers(root)
                .into_iter()
                .find(|m| (v.x(m.tick as f64) - x).abs() <= GRAB)
                .map(|m| (Part::Marker, m.id));
        }
        // the topmost (latest) region first
        model::regions(root).into_iter().rev().find_map(|r| {
            let (a, z) = (v.x(r.from as f64), v.x(r.to as f64));
            if (x - a).abs() <= GRAB {
                Some((Part::RegionStart, r.id))
            } else if (x - z).abs() <= GRAB {
                Some((Part::RegionEnd, r.id))
            } else if x > a && x < z {
                Some((Part::RegionBody, r.id))
            } else {
                None
            }
        })
    }

    pub fn on_down(
        &mut self,
        ev: &MouseDownEvent,
        x: f32,
        y: f32,
        window: &mut Window,
        cx: &mut Context<CompositionPage>,
    ) {
        let lane = if y < LANE_H { 0 } else { 1 };
        let root = self.root(cx);
        let v = self.view.get();
        let hit = self.hit(&root, lane, x);
        self.menu = None;
        self.empty_menu = None;
        if ev.button == MouseButton::Right {
            match hit {
                Some((_, id)) => self.menu = Some((id, x)),
                None => {
                    let t = self.snap(v.tick_at(x), cx);
                    self.empty_menu = Some((lane, t, x));
                }
            }
            cx.notify();
            return;
        }
        if ev.click_count == 2 {
            match hit {
                Some((_, id)) => self.start_rename(id, window, cx),
                None if lane == 0 => {
                    let t = self.snap(v.tick_at(x), cx);
                    self.add_marker(t, cx);
                }
                // a double click on the empty regions' lane: a region over the loop if the click is in it, else the
                // bar under the pointer
                None => {
                    let t = v.tick_at(x).max(0.0) as i64;
                    let (from, to) = model::song(&root).loop_range;
                    self.add_region_at(t, (from..to).contains(&t), cx);
                }
            }
            cx.notify();
            return;
        }
        let tick = self.snap(v.tick_at(x), cx);
        self.drag = Some(match hit {
            Some((part, id)) => {
                let (from0, to0) = match part {
                    Part::Marker => {
                        let m = model::markers(&root).into_iter().find(|m| m.id == id);
                        let t = m.map_or(0, |m| m.tick);
                        (t, t)
                    }
                    _ => model::regions(&root).into_iter().find(|r| r.id == id).map_or((0, 0), |r| (r.from, r.to)),
                };
                Drag { part, id: Some(id), x0: x, from0, to0, moved: false, gesture: None, anchor: 0 }
            }
            None if lane == 1 => Drag {
                part: Part::NewRegion,
                id: None,
                x0: x,
                from0: tick,
                to0: tick,
                moved: false,
                gesture: None,
                anchor: tick,
            },
            None => return,
        });
        cx.notify();
    }

    pub fn on_move(&mut self, ev: &MouseMoveEvent, x: f32, cx: &mut Context<CompositionPage>) {
        let Some(mut d) = self.drag.take() else { return };
        if ev.pressed_button != Some(MouseButton::Left) {
            self.drag = Some(d);
            return self.on_up(cx);
        }
        let v = self.view.get();
        if !d.moved && (x - d.x0).abs() < 3.0 {
            self.drag = Some(d);
            return;
        }
        d.moved = true;
        let dt = self.snap((x - d.x0) as f64 * v.ticks_per_px, cx);
        let dt = if x < d.x0 { -self.snap(((d.x0 - x) as f64) * v.ticks_per_px, cx) } else { dt };
        let ppq = model::ppq(&self.root(cx)) as i64;
        let min = (ppq / 4).max(1);
        if d.part == Part::NewRegion {
            let t = (d.anchor + dt).max(0);
            self.drawing = Some((d.anchor.min(t), d.anchor.max(t)));
            self.drag = Some(d);
            cx.notify();
            return;
        }
        let Some(id) = d.id else { return };
        let label = if d.part == Part::Marker { "Move the marker" } else { "Move the region" };
        let g = d
            .gesture
            .take()
            .unwrap_or_else(|| self.core.update(cx, |c, _| c.begin_gesture(Label::new(label))));
        let set = |k: &str, v: i64| EditOp::Set { id: Ref::Id(id), path: vec![PathStep::key(k)], value: Value::int(v) };
        let ops = match d.part {
            Part::Marker => vec![set("tick", (d.from0 + dt).max(0))],
            Part::RegionBody => {
                let shift = dt.max(-d.from0);
                vec![set("from", d.from0 + shift), set("to", d.to0 + shift)]
            }
            Part::RegionStart => vec![set("from", (d.from0 + dt).min(d.to0 - min).max(0))],
            Part::RegionEnd => vec![set("to", (d.to0 + dt).max(d.from0 + min))],
            Part::NewRegion => Vec::new(),
        };
        self.core.update(cx, |c, cx| {
            c.gesture_apply(g, ops);
            cx.notify();
        });
        d.gesture = Some(g);
        self.drag = Some(d);
    }

    pub fn on_up(&mut self, cx: &mut Context<CompositionPage>) {
        let Some(mut d) = self.drag.take() else { return };
        if let Some(g) = d.gesture.take() {
            self.core.update(cx, |c, cx| {
                c.gesture_commit(g);
                cx.notify();
            });
            return;
        }
        match d.part {
            Part::NewRegion => {
                if let Some((a, z)) = self.drawing.take()
                    && z > a
                {
                    let n = model::regions(&self.root(cx)).len() + 1;
                    let node = Value::object([
                        ("name", Value::text(trf("Region {0}", &[&n]))),
                        ("from", Value::int(a)),
                        ("to", Value::int(z)),
                    ]);
                    self.insert("regions", node, cx);
                }
            }
            // a click (no drag) on a region makes it the loop
            Part::RegionBody | Part::RegionStart | Part::RegionEnd if !d.moved => {
                if let Some(r) = model::regions(&self.root(cx)).into_iter().find(|r| Some(r.id) == d.id) {
                    self.loop_over(&r, cx);
                }
            }
            // a click on a marker moves the cursor there
            Part::Marker if !d.moved => self.run("transport.locate", Value::int(d.from0), cx),
            _ => {}
        }
        cx.notify();
    }

    /// The two lanes, under the ruler: an element over the timeline's canvas
    pub fn render(&self, view: TimelineView, root: &Value, width: f32, cx: &Context<CompositionPage>) -> AnyElement {
        self.view.set(view);
        let markers = model::markers(root);
        let regions = model::regions(root);
        let song = model::song(root);
        let mut children: Vec<AnyElement> = Vec::new();
        for r in &regions {
            let (a, z) = (view.x(r.from as f64), view.x(r.to as f64));
            if z < 0.0 || a > width {
                continue;
            }
            let is_loop = song.loop_on && song.loop_range == (r.from, r.to);
            let col = r.color.unwrap_or(THEME.blue);
            children.push(
                div()
                    .absolute()
                    .top(px(LANE_H + 1.0))
                    .left(px(a))
                    .w(px((z - a).max(2.0)))
                    .h(px(LANE_H - 2.0))
                    .rounded(px(3.))
                    .bg(gpui::rgba((col << 8) | 0x66))
                    .border_1()
                    .border_color(c(if is_loop { THEME.green } else { col }))
                    .overflow_hidden()
                    .px(px(4.))
                    .text_size(px(9.5))
                    .text_color(c(THEME.text))
                    .child(r.name.to_string())
                    .into_any_element(),
            );
        }
        if let Some((a, z)) = self.drawing {
            let (xa, xz) = (view.x(a as f64), view.x(z as f64));
            children.push(
                div()
                    .absolute()
                    .top(px(LANE_H + 1.0))
                    .left(px(xa))
                    .w(px((xz - xa).max(1.0)))
                    .h(px(LANE_H - 2.0))
                    .border_1()
                    .border_color(c(THEME.text2))
                    .into_any_element(),
            );
        }
        for (i, m) in markers.iter().enumerate() {
            let x = view.x(m.tick as f64);
            if !(-200.0..width).contains(&x) {
                continue;
            }
            let col = m.color.unwrap_or(THEME.orange);
            let label = if i < 9 { format!("{} {}", i + 1, m.name) } else { m.name.to_string() };
            children
                .push(div().absolute().top(px(0.)).left(px(x)).w(px(1.)).h(px(LANE_H)).bg(c(col)).into_any_element());
            children.push(
                div()
                    .absolute()
                    .top(px(1.))
                    .left(px(x + 1.0))
                    .px(px(3.))
                    .rounded_r(px(3.))
                    .bg(c(col))
                    .text_size(px(9.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(c(THEME.on_accent))
                    .child(label)
                    .into_any_element(),
            );
        }
        // the name being typed
        if let Some((id, input, _)) = &self.rename {
            let x = markers
                .iter()
                .find(|m| m.id == *id)
                .map(|m| view.x(m.tick as f64))
                .or_else(|| regions.iter().find(|r| r.id == *id).map(|r| view.x(r.from as f64)))
                .unwrap_or(0.0);
            let lane = if id.kind == b'g' { 0.0 } else { LANE_H };
            children.push(
                div()
                    .absolute()
                    .top(px(lane))
                    .left(px(x.max(0.0)))
                    .w(px(160.))
                    .child(crate::ui::text_entry(Input::new(input)))
                    .into_any_element(),
            );
        }
        div()
            .id("marks")
            .debug_selector(|| "marks".into())
            .absolute()
            .top(px(crate::view::RULER_H))
            .left(px(0.))
            .w_full()
            .h(px(MARKS_H))
            .bg(c(THEME.bg1))
            .border_b_1()
            .border_color(c(THEME.line))
            .child(div().absolute().top(px(LANE_H)).w_full().h(px(1.)).bg(c(THEME.line)))
            .children(children)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|p, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let (x, y) = p.marks_local(ev.position);
                    p.marks.on_down(ev, x, y, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|p, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let (x, y) = p.marks_local(ev.position);
                    p.marks.on_down(ev, x, y, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|p, ev: &MouseMoveEvent, _, cx| {
                let (x, _) = p.marks_local(ev.position);
                p.marks.on_move(ev, x, cx);
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|p, _: &MouseUpEvent, _, cx| p.marks.on_up(cx)))
            .into_any_element()
    }

    /// The menu of a marker or a region (right click), under the lanes
    pub fn render_menu(&self, root: &Value, cx: &Context<CompositionPage>) -> Option<AnyElement> {
        let Some((id, x)) = self.menu else { return self.render_empty_menu(root, cx) };
        let is_marker = id.kind == b'g';
        let item = |key: &'static str, sel: &'static str| {
            div()
                .id(sel)
                .debug_selector(move || sel.to_string())
                .px(px(10.))
                .py(px(4.))
                .text_size(px(12.))
                .cursor_pointer()
                .hover(|d| d.bg(c(THEME.bg4)))
                .child(tr(key))
        };
        let region = model::regions(root).into_iter().find(|r| r.id == id);
        let marker = model::markers(root).into_iter().find(|m| m.id == id);
        let start = marker.as_ref().map(|m| m.tick).or(region.as_ref().map(|r| r.from)).unwrap_or(0);
        let current = marker.as_ref().and_then(|m| m.color).or(region.as_ref().and_then(|r| r.color));
        let mut menu = div()
            .absolute()
            .top(px(crate::view::RULER_H + MARKS_H))
            .left(px(x))
            .min_w(px(170.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(
                item("Rename", "mark-rename")
                    .on_click(cx.listener(move |p, _, window, cx| p.marks.start_rename(id, window, cx))),
            )
            .child(self.swatches(id, current, cx))
            .child(item("Play from here", "mark-play").on_click(cx.listener(move |p, _, _, cx| {
                p.marks.menu = None;
                p.marks.run("transport.locate", Value::int(start), cx);
                p.marks.run("transport.play", Value::NULL, cx);
            })));
        if is_marker {
            let has = crate::model::tempo_map(root).points().iter().any(|p| p.tick == start);
            menu = menu.child(
                div()
                    .id("mark-tempo")
                    .debug_selector(|| "mark-tempo".into())
                    .px(px(10.))
                    .py(px(4.))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .hover(|d| d.bg(c(THEME.bg4)))
                    .child(format!("{}{}", if has { "✓ " } else { "" }, tr("Mark a tempo change")))
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.marks.menu = None;
                        p.mark_tempo_change(start, cx);
                    })),
            );
        }
        if let Some(r) = region.filter(|_| !is_marker) {
            let r2 = r.clone();
            menu = menu
                .child(item("Loop over the region", "mark-loop").on_click(cx.listener(move |p, _, _, cx| {
                    p.marks.menu = None;
                    p.marks.loop_over(&r2, cx);
                })))
                .child(item("Export the region as audio", "mark-export").on_click(cx.listener(
                    move |p, _, window, cx| {
                        p.marks.menu = None;
                        p.export_region(r.from, r.to, window, cx);
                    },
                )));
        }
        Some(
            menu.child(item("Remove", "mark-remove").on_click(cx.listener(move |p, _, _, cx| p.marks.remove(id, cx))))
                .into_any_element(),
        )
    }

    /// The colours of a marker or a region: the eight of the theme and "none"
    fn swatches(&self, id: NodeId, current: Option<u32>, cx: &Context<CompositionPage>) -> AnyElement {
        let mut row = div()
            .px(px(10.))
            .py(px(4.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(div().text_size(px(12.)).mr(px(4.)).child(tr("Colour")));
        for (n, col) in (0..8).map(|i| (i, track_color(i))).collect::<Vec<_>>() {
            row = row.child(
                div()
                    .id(("mark-color", n))
                    .debug_selector(move || format!("mark-color-{n}"))
                    .w(px(14.))
                    .h(px(14.))
                    .rounded(px(3.))
                    .bg(c(col))
                    .cursor_pointer()
                    .when(current == Some(col), |d| d.border_2().border_color(c(THEME.text)))
                    .on_click(cx.listener(move |p, _, _, cx| p.marks.set_color(id, Some(col), cx))),
            );
        }
        row.child(
            div()
                .id("mark-color-none")
                .debug_selector(|| "mark-color-none".into())
                .px(px(4.))
                .text_size(px(11.))
                .text_color(c(THEME.text2))
                .cursor_pointer()
                .child("✕")
                .on_click(cx.listener(move |p, _, _, cx| p.marks.set_color(id, None, cx))),
        )
        .into_any_element()
    }

    /// The menu of an empty lane: a marker here, or a region over the loop or of a bar
    fn render_empty_menu(&self, root: &Value, cx: &Context<CompositionPage>) -> Option<AnyElement> {
        let (lane, tick, x) = self.empty_menu?;
        let item = |sel: &'static str, key: &'static str| {
            div()
                .id(sel)
                .debug_selector(move || sel.to_string())
                .px(px(10.))
                .py(px(4.))
                .text_size(px(12.))
                .cursor_pointer()
                .hover(|d| d.bg(c(THEME.bg4)))
                .child(tr(key))
        };
        let mut menu = div()
            .absolute()
            .top(px(crate::view::RULER_H + MARKS_H))
            .left(px(x))
            .min_w(px(170.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg();
        if lane == 0 {
            menu = menu.child(
                item("mark-add", "Add a marker here")
                    .on_click(cx.listener(move |p, _, window, cx| p.marks.add_marker_named(tick, window, cx))),
            );
        } else {
            let (from, to) = model::song(root).loop_range;
            if to > from {
                menu = menu.child(
                    item("mark-region-loop", "Region over the loop")
                        .on_click(cx.listener(move |p, _, _, cx| p.marks.add_region_at(tick, true, cx))),
                );
            }
            menu = menu.child(
                item("mark-region-bar", "Region of a bar")
                    .on_click(cx.listener(move |p, _, _, cx| p.marks.add_region_at(tick, false, cx))),
            );
        }
        Some(menu.into_any_element())
    }

    /// The header beside the lanes
    pub fn header() -> AnyElement {
        let label = |t: &'static str| {
            div()
                .h(px(LANE_H))
                .px(px(8.))
                .flex()
                .items_center()
                .text_size(px(10.))
                .text_color(c(THEME.text2))
                .child(tr(t))
        };
        div()
            .h(px(MARKS_H))
            .bg(c(THEME.bg1))
            .border_b_1()
            .border_color(c(THEME.line))
            .child(label("Markers"))
            .child(label("Regions"))
            .into_any_element()
    }

    /// The markers' regions for Export's "Between markers": each marker to the next (or to `end`)
    pub fn between_markers(root: &Value, end: i64) -> Vec<(String, i64, i64)> {
        let markers: Vec<MarkerView> = model::markers(root);
        markers
            .iter()
            .enumerate()
            .map(|(i, m)| (m.name.to_string(), m.tick, markers.get(i + 1).map_or(end, |n| n.tick)))
            .filter(|(_, a, z)| z > a)
            .collect()
    }
}

/// The page's keys for the marks: M, Shift+R, 1–9 (the registry's, context "Composition")
pub fn actions(d: gpui::Div, cx: &mut Context<CompositionPage>) -> gpui::Div {
    use crate::actions::{
        AddMarker, AddRegion, Marker1, Marker2, Marker3, Marker4, Marker5, Marker6, Marker7, Marker8, Marker9,
    };
    d.on_action(cx.listener(|p, _: &AddMarker, _, cx| p.marks.add_marker_at_cursor(cx)))
        .on_action(cx.listener(|p, _: &AddRegion, _, cx| p.marks.add_region_over_loop(cx)))
        .on_action(cx.listener(|p, _: &Marker1, _, cx| p.marks.jump(1, cx)))
        .on_action(cx.listener(|p, _: &Marker2, _, cx| p.marks.jump(2, cx)))
        .on_action(cx.listener(|p, _: &Marker3, _, cx| p.marks.jump(3, cx)))
        .on_action(cx.listener(|p, _: &Marker4, _, cx| p.marks.jump(4, cx)))
        .on_action(cx.listener(|p, _: &Marker5, _, cx| p.marks.jump(5, cx)))
        .on_action(cx.listener(|p, _: &Marker6, _, cx| p.marks.jump(6, cx)))
        .on_action(cx.listener(|p, _: &Marker7, _, cx| p.marks.jump(7, cx)))
        .on_action(cx.listener(|p, _: &Marker8, _, cx| p.marks.jump(8, cx)))
        .on_action(cx.listener(|p, _: &Marker9, _, cx| p.marks.jump(9, cx)))
}
