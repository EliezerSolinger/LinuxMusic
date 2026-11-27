//! The tempo lane under the ruler (F-REG-013, F-REG-014): the project's tempo map as points you can make, drag,
//! edit and remove, as the legacy's tempo track does it.
//! - a double click on the lane (or Shift+T at the cursor, or the lane's menu "Tempo here") makes a point on the
//!   beat and asks its BPM;
//! - a drag moves it: sideways its place, up and down its BPM (Alt: hundredths); the first point stays on tick 0;
//!   one drag is one undo step;
//! - the point's menu: BPM, "Ramp to the next" (the tempo goes linearly to the next point's), the time signature
//!   (beats per bar 1-16, beat value 1-32; the change goes to the start of its bar), and "Remove the change";
//! - a change that moves recorded audio tied to the tempo (items with a timing lane) shows "Render N items" with
//!   Undo; Render is the core's action `items.rerender`, with its own progress.
//!
//! Every change is the whole `tempoMap` list set through the port (the points have no ids), validated by lm-time
//! before it is sent: a map the time kernel would refuse is never written.

use gpui::{MouseDownEvent, MouseMoveEvent};
use lm_gui_plugins::Gesture;
use lm_port::{PathStep, ROOT};
use lm_time::{Bpm, Ppq, TempoPoint, TimeSignature};

use super::*;

/// How near (px) a press must be to a point
const GRAB: f32 = 6.0;
const LANE_TOP: f32 = view::RULER_H + marks::MARKS_H;
/// The beat values the menu offers
const BEAT_VALUES: [i64; 6] = [1, 2, 4, 8, 16, 32];

/// A point of the map as the document holds it
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Pt {
    pub tick: i64,
    pub bpm: String,
    pub num: i64,
    pub den: i64,
    pub ramp: bool,
}

impl Pt {
    fn value(&self) -> Value {
        Value::object([
            ("tick", Value::int(self.tick)),
            ("bpm", Value::text(self.bpm.clone())),
            ("timeSig", Value::list([Value::int(self.num), Value::int(self.den)])),
            ("ramp", Value::bool(self.ramp)),
        ])
    }

    fn bpm_f64(&self) -> f64 {
        Bpm::parse(&self.bpm).map_or(120.0, Bpm::to_f64)
    }
}

/// The points of the root, in order
pub(super) fn read(root: &Value) -> Vec<Pt> {
    let mut v: Vec<Pt> = model::list(root, "tempoMap")
        .map(|p| {
            let sig = |i: usize| p.get("timeSig").and_then(|s| s.at(i)).and_then(Value::as_int).unwrap_or(4);
            let bpm = model::text(p, "bpm");
            Pt {
                tick: model::int(p, "tick", 0),
                bpm: if bpm.is_empty() { "120".into() } else { bpm.to_string() },
                num: sig(0),
                den: sig(1),
                ramp: model::flag(p, "ramp"),
            }
        })
        .collect();
    if v.is_empty() {
        v.push(Pt { tick: 0, bpm: "120".into(), num: 4, den: 4, ramp: false });
    }
    v
}

/// In order, the first at tick 0, none of them a ramp past the last
fn normalized(mut pts: Vec<Pt>) -> Vec<Pt> {
    pts.sort_by_key(|p| p.tick);
    if let Some(first) = pts.first_mut() {
        first.tick = 0;
    }
    if let Some(last) = pts.last_mut() {
        last.ramp = false;
    }
    pts
}

/// Whether lm-time accepts the points (strictly rising, a valid signature each, no ramp at the end)
fn valid(root: &Value, pts: &[Pt]) -> bool {
    let Ok(ppq) = Ppq::new(model::ppq(root)) else { return false };
    let mut out = Vec::new();
    for p in pts {
        let Ok(bpm) = Bpm::parse(&p.bpm) else { return false };
        let (Ok(n), Ok(d)) = (u8::try_from(p.num), u8::try_from(p.den)) else { return false };
        let Ok(ts) = TimeSignature::new(n, d) else { return false };
        let point = TempoPoint::new(p.tick, bpm, ts);
        out.push(if p.ramp { point.linear() } else { point });
    }
    TempoMap::new(model::sample_rate(root), ppq, &out).is_ok()
}

/// What moves recorded audio: the places and tempi (the signature only changes how bars are counted)
fn timing_key(pts: &[Pt]) -> Vec<(i64, String, bool)> {
    pts.iter().map(|p| (p.tick, p.bpm.clone(), p.ramp)).collect()
}

/// The audio items tied to the tempo (they have a timing lane and a musical time base): a tempo change asks to
/// render them again (F-REC-090)
pub(super) fn tied_items(root: &Value) -> usize {
    model::list(root, "tracks")
        .flat_map(|t| model::list(t, "items"))
        .filter(|i| &*model::text(i, "kind") == "audio" && &*model::text(i, "timebase") != "audio")
        .filter(|i| model::list(i, "lanes").any(|l| l.get("timing").is_some_and(|t| t.items().next().is_some())))
        .count()
}

/// A tempo's place in the lane, 0 (top) to 1 (bottom), in the range `lo..hi`
pub(super) fn bpm_y(bpm: f64, lo: f64, hi: f64) -> f32 {
    1.0 - ((bpm - lo) / (hi - lo).max(1.0)).clamp(0.0, 1.0) as f32
}

fn y_of(bpm: f64, lo: f64, hi: f64) -> f32 {
    LANE_TOP + 4.0 + bpm_y(bpm, lo, hi) * (TEMPO_H - 8.0)
}

struct Drag {
    index: usize,
    x0: f32,
    y0: f32,
    tick0: i64,
    bpm0: f64,
    range: (f64, f64),
    before: Vec<Pt>,
    gesture: Option<Gesture>,
}

struct Menu {
    /// The point, or none: the menu of the empty lane
    index: Option<usize>,
    tick: i64,
    x: f32,
}

/// What the lane keeps between frames
#[derive(Default)]
pub(super) struct TempoLane {
    drag: Option<Drag>,
    menu: Option<Menu>,
    /// The BPM being typed: the point's tick, its field
    input: Option<(i64, Entity<InputState>, Subscription)>,
    /// How many items a tempo change left to render again
    rerender: Option<usize>,
}

impl TempoLane {
    /// The BPM range the lane shows: 60..180 and wider for what is beyond it; frozen while a point is dragged
    pub(super) fn range(&self, bpms: impl Iterator<Item = f64>) -> (f64, f64) {
        if let Some(d) = &self.drag {
            return d.range;
        }
        let (mut lo, mut hi) = (60.0_f64, 180.0_f64);
        for b in bpms {
            lo = lo.min((b - 10.0).max(1.0));
            hi = hi.max(b + 10.0);
        }
        (lo, hi)
    }
}

impl CompositionPage {
    fn tempo_root(&self, cx: &Context<Self>) -> Value {
        self.core.read(cx).port().root().clone()
    }

    /// The beat under a tick, nearest
    fn beat_snap(&self, tick: f64, root: &Value) -> i64 {
        let ppq = model::ppq(root).max(1) as f64;
        ((tick / ppq).round() * ppq).max(0.0) as i64
    }

    /// Sets the points as one undo step; false (and nothing written) if lm-time would refuse them. A change of the
    /// tempo that touches audio tied to it asks to render again
    fn write_points(&mut self, label: &'static str, pts: Vec<Pt>, cx: &mut Context<Self>) -> bool {
        let root = self.tempo_root(cx);
        let pts = normalized(pts);
        if !valid(&root, &pts) {
            return false;
        }
        let before = read(&root);
        if before == pts {
            return true;
        }
        let value = Value::list(pts.iter().map(Pt::value));
        let op = EditOp::Set { id: Ref::Id(ROOT), path: vec![PathStep::key("tempoMap")], value };
        self.apply(label, vec![op], cx);
        self.ask_rerender(&before, &pts, &root);
        true
    }

    fn ask_rerender(&mut self, before: &[Pt], after: &[Pt], root: &Value) {
        if timing_key(before) != timing_key(after) {
            let n = tied_items(root);
            self.tempo.rerender = (n > 0).then_some(n);
        }
    }

    // --- making, editing, removing ---------------------------------------------------------------------------

    /// A point on the beat nearest `tick`, with the tempo and signature there; the index of the point at that tick
    /// (a new one or the one already there)
    fn add_point(&mut self, tick: f64, cx: &mut Context<Self>) -> Option<usize> {
        let root = self.tempo_root(cx);
        let tick = self.beat_snap(tick, &root);
        let mut pts = read(&root);
        if let Some(i) = pts.iter().position(|p| p.tick == tick) {
            return Some(i);
        }
        let map = model::tempo_map(&root);
        let sig = map.time_signature_at(tick);
        let bpm = map.bpm_at_sample(map.sample_at(tick));
        let before = pts.iter().rposition(|p| p.tick < tick).unwrap_or(0);
        // inside a ramp the new point keeps the ramp going to the next one
        let ramp = pts[before].ramp;
        pts.push(Pt {
            tick,
            bpm: Bpm::parse(&format!("{bpm:.2}")).map_or_else(|_| "120".into(), |b| b.to_string()),
            num: i64::from(sig.numerator()),
            den: i64::from(sig.denominator()),
            ramp,
        });
        if !self.write_points("Tempo change", pts, cx) {
            return None;
        }
        read(&self.tempo_root(cx)).iter().position(|p| p.tick == tick)
    }

    fn start_input(&mut self, tick: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_input(cx);
        let root = self.tempo_root(cx);
        let Some(p) = read(&root).into_iter().find(|p| p.tick == tick) else { return };
        let state = cx.new(|cx| InputState::new(window, cx).default_value(p.bpm.clone()));
        state.update(cx, |s, cx| s.focus(window, cx));
        let sub = cx.subscribe_in(&state, window, |page: &mut CompositionPage, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                page.finish_input(cx);
            }
        });
        self.tempo.input = Some((tick, state, sub));
        self.tempo.menu = None;
        cx.notify();
    }

    fn finish_input(&mut self, cx: &mut Context<Self>) {
        let Some((tick, state, _)) = self.tempo.input.take() else { return };
        let text = state.read(cx).value().trim().replace(',', ".");
        if Bpm::parse(&text).is_ok() {
            let mut pts = read(&self.tempo_root(cx));
            if let Some(p) = pts.iter_mut().find(|p| p.tick == tick) {
                p.bpm = Bpm::parse(&text).map(|b| b.to_string()).unwrap_or(text);
                self.write_points("Tempo", pts, cx);
            }
        }
        cx.notify();
    }

    /// Shift+T: a tempo change at the cursor, its BPM asked (F-REG-014)
    pub(super) fn tempo_here(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tick = self.cursor_tick(cx) as f64;
        if let Some(i) = self.add_point(tick, cx) {
            let root = self.tempo_root(cx);
            if let Some(p) = read(&root).get(i) {
                let t = p.tick;
                self.start_input(t, window, cx);
            }
        }
    }

    /// "Mark a tempo change" on a marker (F-REG-011): a point of the map at the marker's tick, or none if it is
    /// already there (the first point stays)
    pub(crate) fn mark_tempo_change(&mut self, tick: i64, cx: &mut Context<Self>) {
        let pts = read(&self.tempo_root(cx));
        match pts.iter().position(|p| p.tick == tick) {
            Some(i) => self.remove_point(i, cx),
            None => {
                self.add_point(tick as f64, cx);
            }
        }
        cx.notify();
    }

    fn remove_point(&mut self, index: usize, cx: &mut Context<Self>) {
        self.tempo.menu = None;
        let mut pts = read(&self.tempo_root(cx));
        if index > 0 && index < pts.len() {
            pts.remove(index);
            self.write_points("Remove the tempo change", pts, cx);
        }
        cx.notify();
    }

    fn toggle_ramp(&mut self, index: usize, cx: &mut Context<Self>) {
        let mut pts = read(&self.tempo_root(cx));
        if index + 1 < pts.len() {
            pts[index].ramp = !pts[index].ramp;
            self.write_points("Ramp", pts, cx);
        }
        cx.notify();
    }

    /// The time signature of a point; a change past the first point goes to the start of its bar
    fn set_signature(&mut self, index: usize, num: i64, den: i64, cx: &mut Context<Self>) {
        let root = self.tempo_root(cx);
        let map = model::tempo_map(&root);
        let mut pts = read(&root);
        let Some(p) = pts.get(index) else { return };
        let tick = p.tick;
        let (num, den) = (num.clamp(1, 16), den);
        pts[index].num = num;
        pts[index].den = den;
        if index > 0 {
            let bar_start = map.bar_start(map.bar_beat_tick(tick).bar).unwrap_or(tick);
            if bar_start != tick {
                if pts.iter().enumerate().any(|(i, q)| i != index && q.tick == bar_start) {
                    return;
                }
                pts[index].tick = bar_start;
            }
        }
        if self.write_points("Time signature", pts, cx)
            && let Some(m) = self.tempo.menu.as_mut()
        {
            m.tick = if index > 0 { map.bar_start(map.bar_beat_tick(tick).bar).unwrap_or(tick) } else { tick };
        }
        cx.notify();
    }

    // --- the pointer -------------------------------------------------------------------------------------------

    fn tempo_local(&self, at: gpui::Point<Pixels>) -> (f32, f32) {
        let b = *self.timeline_bounds.borrow();
        ((at.x - b.origin.x).as_f32(), (at.y - b.origin.y).as_f32() - LANE_TOP)
    }

    fn point_at(&self, pts: &[Pt], x: f32) -> Option<usize> {
        pts.iter().position(|p| (self.view.x(p.tick as f64) - x).abs() <= GRAB)
    }

    pub(super) fn tempo_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (x, _) = self.tempo_local(ev.position);
        let root = self.tempo_root(cx);
        let pts = read(&root);
        let hit = self.point_at(&pts, x);
        self.finish_input(cx);
        self.tempo.menu = None;
        if ev.button == MouseButton::Right {
            let tick = self.beat_snap(self.view.tick_at(x), &root);
            self.tempo.menu = Some(Menu { index: hit, tick, x });
            cx.notify();
            return;
        }
        if ev.click_count == 2 {
            match hit {
                Some(i) => {
                    let tick = pts[i].tick;
                    self.start_input(tick, window, cx);
                }
                None => {
                    let tick = self.view.tick_at(x);
                    if let Some(i) = self.add_point(tick, cx) {
                        let t = read(&self.tempo_root(cx))[i].tick;
                        self.start_input(t, window, cx);
                    }
                }
            }
            return;
        }
        if let Some(i) = hit {
            let (lo, hi) = self.tempo.range(pts.iter().map(Pt::bpm_f64));
            let (_, y) = self.tempo_local(ev.position);
            self.tempo.drag = Some(Drag {
                index: i,
                x0: x,
                y0: y,
                tick0: pts[i].tick,
                bpm0: pts[i].bpm_f64(),
                range: (lo, hi),
                before: pts,
                gesture: None,
            });
        }
        cx.notify();
    }

    pub(super) fn tempo_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(mut d) = self.tempo.drag.take() else { return };
        if ev.pressed_button != Some(MouseButton::Left) {
            self.tempo.drag = Some(d);
            self.tempo_up(cx);
            return;
        }
        let (x, y) = self.tempo_local(ev.position);
        if d.gesture.is_none() && (x - d.x0).abs() < 3.0 && (y - d.y0).abs() < 3.0 {
            self.tempo.drag = Some(d);
            return;
        }
        let root = self.tempo_root(cx);
        let mut pts = read(&root);
        if d.index >= pts.len() {
            return;
        }
        let ppq = model::ppq(&root).max(1) as i64;
        // sideways: the place, on the beat, between its neighbours (the first stays on 0)
        if d.index > 0 {
            let want = self.beat_snap(d.tick0 as f64 + (x - d.x0) as f64 * self.view.ticks_per_px, &root);
            let min = pts[d.index - 1].tick + ppq;
            let max = pts.get(d.index + 1).map_or(i64::MAX, |n| n.tick - ppq);
            pts[d.index].tick = want.clamp(min, max.max(min));
        }
        // up and down: the BPM, whole unless Alt
        let (lo, hi) = d.range;
        let mut bpm = d.bpm0 + f64::from(d.y0 - y) * (hi - lo) / f64::from(TEMPO_H - 8.0);
        bpm = if holds("comp.tempo.fine", &ev.modifiers) { (bpm * 100.0).round() / 100.0 } else { bpm.round() };
        let bpm = bpm.clamp(1.0, 1000.0);
        if let Ok(b) = Bpm::parse(&format!("{bpm:.2}")) {
            pts[d.index].bpm = b.to_string();
        }
        let pts = normalized(pts);
        if valid(&root, &pts) {
            let g = d.gesture.take().unwrap_or_else(|| {
                self.core.update(cx, |core, _| core.begin_gesture(Label::new("Move the tempo point")))
            });
            let op = EditOp::Set {
                id: Ref::Id(ROOT),
                path: vec![PathStep::key("tempoMap")],
                value: Value::list(pts.iter().map(Pt::value)),
            };
            self.core.update(cx, |core, cx| {
                core.gesture_apply(g, vec![op]);
                cx.notify();
            });
            d.gesture = Some(g);
        }
        self.tempo.drag = Some(d);
    }

    pub(super) fn tempo_up(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.tempo.drag.take() else { return };
        if let Some(g) = d.gesture {
            self.core.update(cx, |core, _| core.gesture_commit(g));
            let root = self.tempo_root(cx);
            self.ask_rerender(&d.before, &read(&root), &root);
        }
        cx.notify();
    }

    // --- drawing ---------------------------------------------------------------------------------------------

    /// The lane's points with their labels, the hit area, the menu, the BPM field and the render prompt
    pub(super) fn tempo_overlays(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let root = self.core.read(cx).port().root();
        let pts = read(root);
        let (lo, hi) = self.tempo.range(pts.iter().map(Pt::bpm_f64));
        let width = self.timeline_bounds.borrow().size.width.as_f32();
        let mut out: Vec<AnyElement> = Vec::new();
        // the press and the right button on the lane
        out.push(
            div()
                .id("tempo-lane")
                .debug_selector(|| "tempo-lane".into())
                .absolute()
                .top(px(LANE_TOP))
                .left(px(0.))
                .w_full()
                .h(px(TEMPO_H))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|p, ev: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        p.tempo_down(ev, window, cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|p, ev: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        p.tempo_down(ev, window, cx);
                    }),
                )
                .into_any_element(),
        );
        for (k, p) in pts.iter().enumerate() {
            let x = self.view.x(p.tick as f64);
            if !(-60.0..width + 20.0).contains(&x) {
                continue;
            }
            let y = y_of(p.bpm_f64(), lo, hi);
            let dragging = self.tempo.drag.as_ref().is_some_and(|d| d.index == k && d.gesture.is_some());
            let mut label = format!(
                "{}{}",
                Bpm::parse(&p.bpm).map_or(p.bpm.clone(), |b| format!("{:.0}", b.to_f64())),
                if p.ramp { " ↗" } else { "" }
            );
            let sig_changes =
                if k == 0 { (p.num, p.den) != (4, 4) } else { (p.num, p.den) != (pts[k - 1].num, pts[k - 1].den) };
            if sig_changes {
                label = format!("{label}  {}/{}", p.num, p.den);
            }
            out.push(
                div()
                    .absolute()
                    .top(px(y - 3.5))
                    .left(px(x - 3.5))
                    .w(px(7.))
                    .h(px(7.))
                    .rounded(px(1.))
                    .bg(c(if dragging { THEME.text } else { THEME.orange }))
                    .into_any_element(),
            );
            out.push(
                div()
                    .absolute()
                    .top(px(LANE_TOP + TEMPO_H - 13.0))
                    .left(px(x + 6.0))
                    .text_size(px(9.))
                    .text_color(c(THEME.text3))
                    .child(label)
                    .into_any_element(),
            );
        }
        out.extend(self.tempo_input_overlay(&pts, width));
        out.extend(self.tempo_menu_overlay(&pts, cx));
        out.extend(self.tempo_render_prompt(cx));
        out
    }

    fn tempo_input_overlay(&self, pts: &[Pt], width: f32) -> Option<AnyElement> {
        let (tick, state, _) = self.tempo.input.as_ref()?;
        pts.iter().find(|p| p.tick == *tick)?;
        let x = self.view.x(*tick as f64).clamp(0.0, (width - 100.0).max(0.0));
        Some(
            div().absolute().top(px(LANE_TOP + 2.0)).left(px(x)).w(px(96.)).child(Input::new(state)).into_any_element(),
        )
    }

    fn tempo_menu_overlay(&self, pts: &[Pt], cx: &Context<Self>) -> Option<AnyElement> {
        let menu = self.tempo.menu.as_ref()?;
        let (tick, x) = (menu.tick, menu.x);
        let row = |id: String, label: String, enabled: bool| {
            let sel = id.clone();
            div()
                .id(SharedString::from(id))
                .debug_selector(move || sel.clone())
                .px(px(10.))
                .py(px(3.))
                .text_size(px(12.))
                .when(enabled, |d| d.cursor_pointer().hover(|d| d.bg(c(THEME.bg4))))
                .when(!enabled, |d| d.text_color(c(THEME.text3)))
                .child(label)
        };
        let rule = || div().my(px(3.)).h(px(1.)).bg(c(THEME.line));
        let mut body = div()
            .id("tempo-menu")
            .debug_selector(|| "tempo-menu".into())
            .absolute()
            .top(px(LANE_TOP + TEMPO_H))
            .left(px(x))
            .min_w(px(190.))
            .occlude()
            .py(px(4.))
            .rounded(px(6.))
            .bg(c(THEME.bg3))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .text_color(c(THEME.text))
            .on_mouse_down_out(cx.listener(|p, _, _, cx| {
                p.tempo.menu = None;
                cx.notify();
            }));
        match menu.index.and_then(|i| pts.get(i).map(|p| (i, p))) {
            None => {
                body = body.child(row("tempo-here".into(), tr("Tempo here").into(), true).on_click(cx.listener(
                    move |p, _, window, cx| {
                        p.tempo.menu = None;
                        if let Some(i) = p.add_point(tick as f64, cx) {
                            let t = read(&p.tempo_root(cx))[i].tick;
                            p.start_input(t, window, cx);
                        }
                    },
                )));
            }
            Some((i, pt)) => {
                let last = i + 1 >= pts.len();
                body = body
                    .child(
                        row("tempo-bpm".into(), format!("{}…", tr("BPM")), true)
                            .on_click(cx.listener(move |p, _, window, cx| p.start_input(tick, window, cx))),
                    )
                    .child(
                        row(
                            "tempo-ramp".into(),
                            format!("{}{}", if pt.ramp { "✓ " } else { "" }, tr("Ramp to the next")),
                            !last,
                        )
                        .on_click(cx.listener(move |p, _, _, cx| p.toggle_ramp(i, cx))),
                    )
                    .child(rule())
                    .child(
                        div()
                            .px(px(10.))
                            .py(px(3.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .child(div().flex_1().child(tr("Beats per bar")))
                            .child(row("tempo-num-down".into(), "−".into(), pt.num > 1).on_click(cx.listener(
                                move |p, _, _, cx| {
                                    let (n, d) = p.signature_of(i, cx);
                                    p.set_signature(i, n - 1, d, cx)
                                },
                            )))
                            .child(div().w(px(20.)).child(format!("{}", pt.num)))
                            .child(row("tempo-num-up".into(), "+".into(), pt.num < 16).on_click(cx.listener(
                                move |p, _, _, cx| {
                                    let (n, d) = p.signature_of(i, cx);
                                    p.set_signature(i, n + 1, d, cx)
                                },
                            ))),
                    )
                    .child(
                        div()
                            .px(px(10.))
                            .py(px(3.))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .text_size(px(12.))
                            .child(div().flex_1().child(tr("Beat value")))
                            .children(BEAT_VALUES.into_iter().map(|v| {
                                row(
                                    format!("tempo-den-{v}"),
                                    format!("{}{v}", if pt.den == v { "✓" } else { "" }),
                                    true,
                                )
                                .px(px(3.))
                                .on_click(cx.listener(move |p, _, _, cx| {
                                    let (n, _) = p.signature_of(i, cx);
                                    p.set_signature(i, n, v, cx)
                                }))
                            })),
                    )
                    .child(rule())
                    .child(
                        row("tempo-remove".into(), tr("Remove the change").into(), i > 0)
                            .on_click(cx.listener(move |p, _, _, cx| p.remove_point(i, cx))),
                    );
            }
        }
        Some(body.into_any_element())
    }

    fn signature_of(&self, index: usize, cx: &Context<Self>) -> (i64, i64) {
        read(self.core.read(cx).port().root()).get(index).map_or((4, 4), |p| (p.num, p.den))
    }

    /// "Render N items" with Undo, after a tempo change that moves recorded audio tied to the tempo
    fn tempo_render_prompt(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let n = self.tempo.rerender?;
        let btn = |id: &'static str, label: String| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .px(px(8.))
                .h(px(18.))
                .flex()
                .items_center()
                .rounded(px(3.))
                .bg(c(THEME.bg4))
                .text_size(px(11.))
                .cursor_pointer()
                .child(label)
        };
        Some(
            div()
                .id("tempo-render")
                .debug_selector(|| "tempo-render".into())
                .absolute()
                .top(px(LANE_TOP + 7.0))
                .right(px(10.))
                .h(px(20.))
                .px(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(4.))
                .bg(c(THEME.bg3))
                .border_1()
                .border_color(c(THEME.orange))
                .text_size(px(11.))
                .text_color(c(THEME.text))
                .child(trf("Render {0} items", &[&n]))
                .child(btn("tempo-render-go", tr("Render").into()).on_click(cx.listener(|p, _, _, cx| {
                    p.tempo.rerender = None;
                    p.action("items.rerender", Value::NULL, cx);
                })))
                .child(btn("tempo-render-undo", tr("Undo").into()).on_click(cx.listener(|p, _, _, cx| {
                    p.tempo.rerender = None;
                    p.core.update(cx, |core, _| {
                        core.report(|pt| pt.undo());
                    });
                    cx.notify();
                })))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(tick: i64, bpm: &str) -> Pt {
        Pt { tick, bpm: bpm.into(), num: 4, den: 4, ramp: false }
    }

    #[test]
    fn the_last_point_is_never_a_ramp_and_the_first_is_at_zero() {
        let mut a = pt(960, "100");
        a.ramp = true;
        let out = normalized(vec![pt(3840, "90"), a]);
        assert_eq!(out[0].tick, 0);
        assert!(!out.last().unwrap().ramp);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn the_lane_maps_bpm_to_height() {
        assert_eq!(bpm_y(180.0, 60.0, 180.0), 0.0);
        assert_eq!(bpm_y(60.0, 60.0, 180.0), 1.0);
        assert!(bpm_y(500.0, 60.0, 180.0) == 0.0);
    }

    #[test]
    fn a_point_is_written_as_the_document_holds_it() {
        let v = Pt { tick: 960, bpm: "119.88".into(), num: 3, den: 4, ramp: true }.value();
        assert_eq!(v.get("tick").and_then(Value::as_int), Some(960));
        assert_eq!(v.get("bpm").and_then(Value::as_text), Some("119.88"));
        assert_eq!(v.get("timeSig").and_then(|s| s.at(0)).and_then(Value::as_int), Some(3));
        assert_eq!(v.get("ramp").and_then(Value::as_bool), Some(true));
    }
}
