//! Owner: agent B. The step editor: MIDI items edited as steps (F-PAD-002, P-23). It takes the place of the
//! legacy's Pattern Editor (legado/src/gui/editors/PatternEditor.cpp), whose layout it keeps: a toolbar (play and
//! stop; the pattern chosen; remove, add and clone steps), then one row per instrument with its name on the left
//! and the steps on the right, sixteen to a bar, the beats in two shades, a lit step brighter with its velocity; a
//! click turns a step on or off, the wheel changes its velocity.
//!
//! What the new model changes (decisoes-autonomas.md, B-PE-*): there is no pattern, so the toolbar picks the MIDI
//! items shown (each a row group); a drum kit is ONE instrument, so its item shows one row per piece it plays; a
//! melodic item is one row of pitched steps (Alt+drag a step's note); the item's `step` and `swing` are set here.
//!
//! The core is reached only through the port: `AddNote`/`RemoveNote` in one transaction on release (the document
//! forbids them inside a gesture), the item's numbers by `EditOp::set`.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use gpui::{
    Bounds, Context, CursorStyle, FocusHandle, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, Render, ScrollEvent, SharedString, Window, canvas, div, fill, point, prelude::*, px, size,
};
use lm_gui_plugins::Gesture;
use lm_port::{EditOp, Feed, HostTime, Label, NodeId, Topic, Value};

use super::ScreenCx;
use super::piano_roll::kit::{self, MenuItem, dropdown, icon_button, label, paint_text, popup, sep, tip, with_alpha};
use super::piano_roll::music::{BAR, BEAT, Clip, Note, NoteChange, key_name};
use super::piano_roll::{NEW, note_ops};
use crate::keymap;
use crate::model;
use crate::ui::{THEME, button, c, tr, trf};

const CTX: &str = "Steps";
const ROW_H: f32 = 26.0;
const STEP_ROW_H: f32 = 20.0;
const HEADER_H: f32 = 18.0;
const NAMES_W: f32 = 150.0;

/// The General MIDI drum map: the pieces a drum kit's item shows (the instrument plays them by these keys)
pub(crate) const GM_DRUMS: &[(u8, &str)] = &[
    (36, "Kick"),
    (37, "Side stick"),
    (38, "Snare"),
    (39, "Clap"),
    (41, "Floor tom"),
    (42, "Hi-hat closed"),
    (44, "Hi-hat pedal"),
    (45, "Low tom"),
    (46, "Hi-hat open"),
    (48, "High tom"),
    (49, "Crash"),
    (51, "Ride"),
    (53, "Ride bell"),
    (56, "Cowbell"),
];

/// A row of a group: a piece of a kit, or the pitched steps of a melodic item
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Row {
    Key(u8),
    Pitched,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Items,
    Step,
}

const STEP_SIZES: [(&str, i64); 4] = [("1/8", BEAT / 2), ("1/16", BEAT / 4), ("1/32", BEAT / 8), ("1/24", BAR / 24)];

enum Drag {
    /// a drag along a row turns steps on (or off); the notes change on release, in one transaction
    Paint {
        item: NodeId,
        row: Row,
        on: bool,
        steps: BTreeSet<i64>,
    },
    /// Alt+drag on a pitched step: its note
    Pitch {
        g: Option<Gesture>,
        item: NodeId,
        n: u64,
        origin: Note,
        from_y: f32,
    },
    Swing {
        g: Option<Gesture>,
        items: Vec<(NodeId, f64)>,
        from_y: f32,
    },
}

/// An item as the step editor shows it
#[derive(Clone, Debug)]
struct Group {
    clip: Clip,
    kit: bool,
    rows: Vec<Row>,
}

pub struct StepEditor {
    scx: ScreenCx,
    focus: FocusHandle,
    feed: Feed,
    /// the items shown, one group each
    pub items: Vec<NodeId>,
    pub step_w: f32,
    pub scroll_x: f32,
    scroll_y: f32,
    show_all: bool,
    drag: Option<Drag>,
    menu: Option<(Menu, Point<Pixels>)>,
    playing: Option<HostTime>,
    ticks_per_sec: f64,
    cells: Rc<RefCell<HashMap<NodeId, Bounds<Pixels>>>>,
}

impl StepEditor {
    pub fn new(scx: ScreenCx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (feed, items, tps) = scx.core.update(cx, |core, _| {
            let root = core.port().root();
            // the target, else the drum kits' items and the repeating ones (the patterns)
            let items: Vec<NodeId> = match scx.target {
                Some(t) => vec![t],
                None => midi_items(root)
                    .into_iter()
                    .filter(|(_, _, kit, repeats)| *kit || *repeats)
                    .map(|x| x.0)
                    .take(4)
                    .collect(),
            };
            let map = model::tempo_map(root);
            let tps = map.tick_at(map.sample_rate() as i64) as f64;
            (core.port_mut().feed(Topic::All), items, tps.max(1.0))
        });
        StepEditor {
            focus: cx.focus_handle(),
            feed,
            items,
            step_w: 18.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            show_all: false,
            drag: None,
            menu: None,
            playing: None,
            ticks_per_sec: tps,
            cells: Rc::new(RefCell::new(HashMap::new())),
            scx,
        }
    }

    fn group(&self, id: NodeId, cx: &gpui::App) -> Option<Group> {
        let root = self.scx.core.read(cx).port().root();
        let clip = Clip::read(root, id)?;
        let kit = is_kit(root, id);
        let rows = if kit {
            let used: BTreeSet<u8> = clip.notes.values().map(|x| x.key).collect();
            let mut keys: Vec<u8> =
                GM_DRUMS.iter().map(|(k, _)| *k).filter(|k| self.show_all || used.contains(k)).collect();
            // a key the kit's map does not name still gets its row
            keys.extend(used.iter().filter(|k| !GM_DRUMS.iter().any(|(g, _)| g == *k)));
            if keys.is_empty() {
                keys = vec![36, 38, 42];
            }
            keys.into_iter().map(Row::Key).collect()
        } else {
            vec![Row::Pitched]
        };
        Some(Group { clip, kit, rows })
    }

    /// The steps of an item: its repeating part (`loop`) or its length, in steps
    pub fn steps(c: &Clip) -> i64 {
        (c.loop_len.unwrap_or(c.length) / c.step.max(1)).max(1)
    }

    fn in_row(row: Row, x: &Note) -> bool {
        match row {
            Row::Key(k) => x.key == k,
            Row::Pitched => true,
        }
    }

    fn at_step(c: &Clip, row: Row, s: i64) -> Vec<(u64, Note)> {
        let (a, z) = (s * c.step, (s + 1) * c.step);
        c.notes
            .iter()
            .filter(|(_, x)| Self::in_row(row, x) && x.start >= a && x.start < z)
            .map(|(n, x)| (*n, x.clone()))
            .collect()
    }

    /// A new pitched step's key: the nearest earlier note's, else the legacy's default (57)
    fn pitched_key(c: &Clip, s: i64) -> u8 {
        let t = s * c.step;
        c.notes.values().filter(|x| x.start < t).max_by_key(|x| x.start).map_or(57, |x| x.key)
    }

    fn set_steps(c: &Clip, row: Row, steps: &BTreeSet<i64>, on: bool) -> Vec<NoteChange> {
        let mut out = Vec::new();
        let mut temp = 0;
        for &s in steps {
            let here = Self::at_step(c, row, s);
            if on && here.is_empty() {
                let key = match row {
                    Row::Key(k) => k,
                    Row::Pitched => Self::pitched_key(c, s),
                };
                out.push((NEW + temp, Some(Note::new(key, s * c.step, c.step, 0.8))));
                temp += 1;
            } else if !on {
                out.extend(here.into_iter().map(|(n, _)| (n, None)));
            }
        }
        out
    }

    // --- changing, through the port ----------------------------------------------------------------------------

    fn apply(&mut self, label: &'static str, ops: Vec<EditOp>, cx: &mut Context<Self>) {
        if ops.is_empty() {
            return;
        }
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| pt.apply(Label::new(label), ops));
        });
        cx.notify();
    }

    fn gesture(&self, g: &mut Option<Gesture>, label: &'static str, ops: Vec<EditOp>, cx: &mut Context<Self>) {
        if ops.is_empty() {
            return;
        }
        self.scx.core.update(cx, |core, _| {
            // a gesture closed under the drag (the release was lost) refuses the edits: the drag writes nothing more
            let open = *g.get_or_insert_with(|| core.begin_gesture(Label::new(label)));
            core.gesture_apply(open, ops);
        });
    }

    fn commit(&self, g: Option<Gesture>, cx: &mut Context<Self>) {
        if let Some(g) = g {
            self.scx.core.update(cx, |core, _| core.gesture_commit(g));
        }
    }

    /// The item's steps end at `end`: the repeating part (`loop`) if it has one, else its length
    fn set_end(c: &Clip, end: i64) -> Vec<EditOp> {
        let end = end.max(c.step);
        if c.loop_len.is_some() {
            let mut ops = vec![EditOp::set(c.id, &["loop"], Value::int(end))];
            if c.length < end {
                ops.push(EditOp::set(c.id, &["length"], Value::int(end)));
            }
            ops
        } else {
            vec![EditOp::set(c.id, &["length"], Value::int(end))]
        }
    }

    /// Add steps: a bar more on every item shown (the legacy's "add steps")
    pub fn add_steps(&mut self, cx: &mut Context<Self>) {
        let ops: Vec<EditOp> = self
            .items
            .iter()
            .filter_map(|id| self.group(*id, cx))
            .flat_map(|g| Self::set_end(&g.clip, Self::end(&g.clip) + BAR))
            .collect();
        self.apply("Add steps", ops, cx);
    }

    /// Remove steps: a bar less; the notes past the new end go with it
    pub fn remove_steps(&mut self, cx: &mut Context<Self>) {
        let mut ops = Vec::new();
        for g in self.items.iter().filter_map(|id| self.group(*id, cx)) {
            let end = Self::end(&g.clip);
            if end <= BAR {
                continue;
            }
            let gone: Vec<NoteChange> = g
                .clip
                .notes
                .iter()
                .filter(|(_, x)| x.start >= end - BAR && x.start < end)
                .map(|(n, _)| (*n, None))
                .collect();
            ops.extend(note_ops(g.clip.id, &g.clip.notes, gone));
            ops.extend(Self::set_end(&g.clip, end - BAR));
        }
        self.apply("Remove steps", ops, cx);
    }

    /// Clone steps: doubled, the second half a copy of the first (the legacy's "clone steps")
    pub fn clone_steps(&mut self, cx: &mut Context<Self>) {
        let mut ops = Vec::new();
        for g in self.items.iter().filter_map(|id| self.group(*id, cx)) {
            let end = Self::end(&g.clip);
            let copies: Vec<NoteChange> = g
                .clip
                .notes
                .values()
                .filter(|x| x.start < end)
                .enumerate()
                .map(|(i, x)| (NEW + i as u64, Some(Note { start: x.start + end, ..x.clone() })))
                .collect();
            ops.extend(note_ops(g.clip.id, &g.clip.notes, copies));
            ops.extend(Self::set_end(&g.clip, end * 2));
        }
        self.apply("Clone steps", ops, cx);
    }

    fn end(c: &Clip) -> i64 {
        c.loop_len.unwrap_or(c.length)
    }

    // --- the mouse ---------------------------------------------------------------------------------------------

    /// The item, row and step under a window position
    fn cell_at(&self, p: Point<Pixels>, cx: &gpui::App) -> Option<(NodeId, Row, i64)> {
        let cells = self.cells.borrow().clone();
        for (id, b) in cells {
            if !b.contains(&p) {
                continue;
            }
            let g = self.group(id, cx)?;
            let r = ((p.y - b.origin.y).as_f32() / STEP_ROW_H) as usize;
            let s = (((p.x - b.origin.x).as_f32() + self.scroll_x) / self.step_w).floor() as i64;
            if r < g.rows.len() && s >= 0 && s < Self::steps(&g.clip) {
                return Some((id, g.rows[r], s));
            }
        }
        None
    }

    fn cells_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.menu = None;
        let Some((item, row, s)) = self.cell_at(ev.position, cx) else { return };
        let Some(g) = self.group(item, cx) else { return };
        let here = Self::at_step(&g.clip, row, s);
        if keymap::fires(CTX, "st.pitch", ev.button, ev.click_count, &ev.modifiers)
            && row == Row::Pitched
            && !here.is_empty()
        {
            let (n, origin) = here[0].clone();
            self.drag = Some(Drag::Pitch { g: None, item, n, origin, from_y: ev.position.y.as_f32() });
        } else {
            // the first step decides: a drag lights steps (from a dark one) or darkens them (from a lit one, or with
            // the right button)
            let on = ev.button == MouseButton::Left && here.is_empty();
            self.drag = Some(Drag::Paint { item, row, on, steps: [s].into_iter().collect() });
        }
        cx.notify();
    }

    fn drag_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut drag) = self.drag.take() else { return };
        match &mut drag {
            Drag::Paint { item, steps, .. } => {
                if let Some(g) = self.group(*item, cx) {
                    let b = self.cells.borrow().get(item).copied().unwrap_or_default();
                    let s = (((ev.position.x - b.origin.x).as_f32() + self.scroll_x) / self.step_w).floor() as i64;
                    let s = s.clamp(0, Self::steps(&g.clip) - 1);
                    let last = steps.iter().next_back().copied().unwrap_or(s);
                    steps.extend(s.min(last)..=s.max(last));
                }
            }
            Drag::Pitch { g, item, n, origin, from_y } => {
                let d = ((*from_y - ev.position.y.as_f32()) / 6.0) as i32;
                let key = (origin.key as i32 + d).clamp(0, 127) as u8;
                let old = self.group(*item, cx).map(|g| g.clip.notes).unwrap_or_default();
                let ops = note_ops(*item, &old, vec![(*n, Some(Note { key, ..origin.clone() }))]);
                self.gesture(g, "Step pitch", ops, cx);
            }
            Drag::Swing { g, items, from_y } => {
                let d = ((*from_y - ev.position.y.as_f32()) / 200.0) as f64;
                let ops = items
                    .iter()
                    .map(|(id, v0)| {
                        EditOp::set(
                            *id,
                            &["swing"],
                            Value::real(((v0 + d).clamp(0.0, 1.0) * 100.0).round() / 100.0).unwrap_or(Value::NULL),
                        )
                    })
                    .collect();
                self.gesture(g, "Swing", ops, cx);
            }
        }
        self.drag = Some(drag);
        cx.notify();
    }

    fn drag_up(&mut self, _ev: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        match self.drag.take() {
            Some(Drag::Paint { item, row, on, steps }) => {
                if let Some(g) = self.group(item, cx) {
                    let ops = note_ops(item, &g.clip.notes, Self::set_steps(&g.clip, row, &steps, on));
                    self.apply(if on { "Add steps" } else { "Remove steps" }, ops, cx);
                }
            }
            Some(Drag::Pitch { g, .. }) | Some(Drag::Swing { g, .. }) => self.commit(g, cx),
            None => {}
        }
        cx.notify();
    }

    fn cells_wheel(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let (dx, dy) = (d.x.as_f32(), d.y.as_f32());
        let which = keymap::pick_wheel(CTX, &["st.view.zoom", "st.view.scroll_h", "st.step.velocity"], &ev.modifiers);
        if which.is_none() {
            return;
        }
        if which == Some("st.view.zoom") {
            self.step_w = (self.step_w * (-dy * 0.01).exp()).clamp(8.0, 64.0);
        } else if which == Some("st.view.scroll_h") || dx.abs() > dy.abs() {
            self.scroll_x = (self.scroll_x - if dx.abs() > 0.01 { dx } else { dy }).max(0.0);
        } else if let Some((item, row, s)) = self.cell_at(ev.position, cx)
            && let Some(g) = self.group(item, cx)
            && !Self::at_step(&g.clip, row, s).is_empty()
        {
            // the wheel on a lit step: its velocity, ±5 % a notch (F-PAD-002)
            if dy.abs() > 0.5 {
                let dv = if dy > 0.0 { 0.05 } else { -0.05 };
                let ch = Self::at_step(&g.clip, row, s)
                    .into_iter()
                    .map(|(n, x)| (n, Some(Note { velocity: (x.velocity + dv).clamp(0.0, 1.0), ..x })))
                    .collect();
                let ops = note_ops(item, &g.clip.notes, ch);
                self.apply("Step velocity", ops, cx);
            }
            return;
        } else {
            self.scroll_y = (self.scroll_y - dy).max(0.0);
        }
        cx.notify();
    }

    fn menu_items(&self, m: Menu, cx: &gpui::App) -> Vec<MenuItem> {
        let root = self.scx.core.read(cx).port().root();
        match m {
            Menu::Items => {
                midi_items(root).into_iter().map(|(id, l, _, _)| MenuItem::new(l, self.items.contains(&id))).collect()
            }
            Menu::Step => {
                let step = self.items.first().and_then(|id| self.group(*id, cx)).map_or(BEAT / 4, |g| g.clip.step);
                STEP_SIZES.iter().map(|(l, t)| MenuItem::new(*l, *t == step)).collect()
            }
        }
    }

    fn pick(&mut self, m: Menu, i: usize, cx: &mut Context<Self>) {
        self.menu = None;
        match m {
            Menu::Items => {
                let id = midi_items(self.scx.core.read(cx).port().root()).get(i).map(|x| x.0);
                if let Some(id) = id {
                    if let Some(k) = self.items.iter().position(|x| *x == id) {
                        self.items.remove(k);
                    } else {
                        self.items.push(id);
                    }
                }
            }
            Menu::Step => {
                let ops =
                    self.items.iter().map(|id| EditOp::set(*id, &["step"], Value::int(STEP_SIZES[i].1))).collect();
                self.apply("Step size", ops, cx);
            }
        }
        cx.notify();
    }
}

/// The MIDI items: (id, "track · item", the track plays a drum kit, the item repeats)
fn midi_items(root: &Value) -> Vec<(NodeId, String, bool, bool)> {
    model::tracks(root)
        .iter()
        .flat_map(|t| {
            let kit = t.kit;
            t.items.iter().filter(|i| !i.audio).map(move |i| {
                let repeats = Clip::read(root, i.id).is_some_and(|c| c.loop_len.is_some());
                (i.id, format!("{} · {}", t.name, i.name), kit, repeats)
            })
        })
        .collect()
}

fn is_kit(root: &Value, item: NodeId) -> bool {
    midi_items(root).iter().any(|(id, _, kit, _)| *id == item && *kit)
}

// ---------------------------------------------------------------------------------------------------------------
// Rendering

struct CellsPaint {
    clip: Clip,
    rows: Vec<Row>,
    step_w: f32,
    scroll_x: f32,
    play_step: Option<i64>,
    /// a paint drag's preview: the steps and whether they light
    preview: Option<(Row, BTreeSet<i64>, bool)>,
}

impl CellsPaint {
    fn paint(&self, b: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        let o = b.origin;
        let w = b.size.width.as_f32();
        let cl = &self.clip;
        let n = StepEditor::steps(cl);
        let per_beat = (BEAT / cl.step.max(1)).max(1);
        let first = (self.scroll_x / self.step_w).floor().max(0.0) as i64;
        let last = (((self.scroll_x + w) / self.step_w).ceil() as i64).min(n);
        for (r, row) in self.rows.iter().enumerate() {
            let y = r as f32 * STEP_ROW_H;
            for s in first..last {
                let x = s as f32 * self.step_w - self.scroll_x;
                let cell = Bounds::new(
                    point(o.x + px(x + 1.0), o.y + px(y + 2.0)),
                    size(px(self.step_w - 2.0), px(STEP_ROW_H - 4.0)),
                );
                let here = StepEditor::at_step(cl, *row, s);
                let lit = match &self.preview {
                    Some((pr, steps, on)) if pr == row && steps.contains(&s) => *on,
                    _ => !here.is_empty(),
                };
                if !lit {
                    // the legacy: the beats in two shades
                    let shade =
                        if (s / per_beat) % 2 == 0 { c(THEME.step_off) } else { with_alpha(THEME.step_off, 0.6) };
                    window.paint_quad(gpui::quad(cell, px(2.0), shade, px(0.0), shade, Default::default()));
                    continue;
                }
                // lit: brighter with the velocity (F-PAD-002)
                let v =
                    if here.is_empty() { 0.8 } else { here.iter().map(|(_, x)| x.velocity).fold(0.0, f64::max) } as f32;
                window.paint_quad(gpui::quad(
                    cell,
                    px(2.0),
                    with_alpha(THEME.step_on, 0.3 + 0.7 * v),
                    px(1.0),
                    with_alpha(THEME.step_on, 1.0),
                    Default::default(),
                ));
                if *row == Row::Pitched && self.step_w >= 20.0 && !here.is_empty() {
                    let t = key_name(here[0].1.key).into();
                    paint_text(
                        window,
                        cx,
                        t,
                        cell.origin.x + px(3.0),
                        cell.origin.y + px(2.0),
                        9.0,
                        c(THEME.on_accent),
                    );
                }
            }
        }
        if let Some(p) = self.play_step {
            let x = p as f32 * self.step_w - self.scroll_x;
            if x >= -self.step_w && x < w {
                window.paint_quad(fill(
                    Bounds::new(point(o.x + px(x), o.y), size(px(self.step_w), b.size.height)),
                    with_alpha(THEME.cursor, 0.12),
                ));
            }
        }
    }
}

impl Render for StepEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = HostTime::now();
        {
            // I-3.6: an item that is gone leaves the view
            let port = self.scx.core.read(cx).port();
            if port.drain(&mut self.feed).is_some() {
                let root = port.root();
                self.items.retain(|id| Clip::read(root, *id).is_some());
            }
        }
        let core = self.scx.core.read(cx);
        let pos = core.port().telemetry().position;
        let map = core.port().tempo_map();
        let tick = match self.playing {
            Some(t0) => Some(now.since(t0) as f64 * 1e-9 * self.ticks_per_sec),
            None => pos.playing.then(|| map.tick_at(pos.at(now) as i64) as f64),
        };
        if tick.is_some() {
            window.request_animation_frame();
        }
        let entity = cx.entity();
        let groups: Vec<Group> = self.items.iter().filter_map(|id| self.group(*id, cx)).collect();
        let mut list = div().flex().flex_col().mt(px(-self.scroll_y));
        if groups.is_empty() {
            list = list.child(div().p(px(12.)).text_color(c(THEME.text2)).child(tr("No items")));
        }
        for g in groups {
            list = list.child(self.render_group(g, tick, cx));
        }
        div()
            .id("steps")
            .size_full()
            .flex()
            .flex_col()
            .bg(c(THEME.bg1))
            .text_color(c(THEME.text))
            .track_focus(&self.focus)
            .key_context("StepEditor")
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .id("st-body")
                    .flex_1()
                    .overflow_hidden()
                    .on_scroll(cx.listener(|st, ev: &ScrollEvent, _, cx| st.cells_wheel(ev, cx)))
                    .child(list),
            )
            .children(self.drag.is_some().then(|| kit::capture(entity.downgrade(), Self::drag_move, Self::drag_up)))
            .children(self.menu.map(|(m, at)| {
                let items = self.menu_items(m, cx);
                let (e, e2) = (entity.downgrade(), entity.downgrade());
                popup(
                    "st-menu",
                    at,
                    items,
                    move |i, _, cx| {
                        let _ = e.update(cx, |st, cx| st.pick(m, i, cx));
                    },
                    move |_, cx| {
                        let _ = e2.update(cx, |st, cx| {
                            st.menu = None;
                            cx.notify();
                        });
                    },
                )
            }))
    }
}

impl StepEditor {
    fn open_menu(m: Menu) -> impl Fn(&mut StepEditor, &MouseDownEvent, &mut Window, &mut Context<StepEditor>) {
        move |st, ev, _, cx| {
            st.menu =
                if st.menu.is_some_and(|x| x.0 == m) { None } else { Some((m, ev.position + point(px(0.), px(10.)))) };
            cx.notify();
        }
    }

    /// The legacy's toolbar: play, stop, the pattern (here the items), remove, add and clone steps
    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let small = |id: &'static str, l: SharedString, on: bool, help: &'static str| {
            button(id, l, on, THEME.green)
                .debug_selector(move || id.to_string())
                .h(px(20.))
                .min_w(px(24.))
                .px(px(6.))
                .tooltip(tip(help))
        };
        let playing = self.playing.is_some();
        let swing = self.items.first().and_then(|id| self.group(*id, cx)).map_or(0.0, |g| g.clip.swing);
        let step = self.items.first().and_then(|id| self.group(*id, cx)).map_or(BEAT / 4, |g| g.clip.step);
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
                    "st-play",
                    if playing { "pause" } else { "play" },
                    playing,
                    "Play/pause current pattern (Space)",
                )
                .on_click(cx.listener(|st, _, _, cx| {
                    st.playing = if st.playing.is_some() { None } else { Some(HostTime::now()) };
                    cx.notify();
                })),
            )
            .child(icon_button("st-stop", "stop", false, "Stop playback of current pattern (Space)").on_click(
                cx.listener(|st, _, _, cx| {
                    st.playing = None;
                    cx.notify();
                }),
            ))
            .child(sep())
            .child(
                dropdown("st-items", trf("Items: {0}", &[&self.items.len()]), false)
                    .debug_selector(|| "st-items".into())
                    .tooltip(tip("The MIDI items shown here"))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::open_menu(Menu::Items))),
            )
            .child(sep())
            .child(
                icon_button("st-remove-steps", "step_btn_remove", false, "Remove steps")
                    .on_click(cx.listener(|st, _, _, cx| st.remove_steps(cx))),
            )
            .child(
                icon_button("st-add-steps", "step_btn_add", false, "Add steps")
                    .on_click(cx.listener(|st, _, _, cx| st.add_steps(cx))),
            )
            .child(
                icon_button("st-clone-steps", "step_btn_duplicate", false, "Clone Steps")
                    .on_click(cx.listener(|st, _, _, cx| st.clone_steps(cx))),
            )
            .child(sep())
            .child(label(tr("Step")))
            .child(
                dropdown("st-step", STEP_SIZES.iter().find(|s| s.1 == step).map_or("1/16", |s| s.0), false)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::open_menu(Menu::Step))),
            )
            .child(
                div()
                    .id("st-swing")
                    .debug_selector(|| "st-swing".into())
                    .flex_none()
                    .px(px(6.))
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .rounded(px(3.))
                    .bg(c(THEME.bg1))
                    .text_size(px(11.))
                    .cursor(CursorStyle::ResizeUpDown)
                    .tooltip(tip("Swing: how late the off-beat steps play; drag up or down"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|st, ev: &MouseDownEvent, _, cx| {
                            let items = st
                                .items
                                .iter()
                                .filter_map(|id| st.group(*id, cx).map(|g| (*id, g.clip.swing)))
                                .collect();
                            st.drag = Some(Drag::Swing { g: None, items, from_y: ev.position.y.as_f32() });
                            cx.notify();
                        }),
                    )
                    .child(trf("Swing {0} %", &[&format!("{:.0}", swing * 100.0)])),
            )
            .child(
                small(
                    "st-all",
                    tr("All pieces").into(),
                    self.show_all,
                    "Show every piece of the kit, not only the ones it plays",
                )
                .on_click(cx.listener(|st, _, _, cx| {
                    st.show_all = !st.show_all;
                    cx.notify();
                })),
            )
    }

    fn render_group(&self, g: Group, tick: Option<f64>, cx: &Context<Self>) -> impl IntoElement {
        let id = g.clip.id;
        let play_step = tick.and_then(|t| {
            let t = t - g.clip.start as f64;
            let end = StepEditor::end(&g.clip) as f64;
            (t >= 0.0 && t < g.clip.length as f64).then(|| ((t % end) / g.clip.step as f64) as i64)
        });
        let preview = match &self.drag {
            Some(Drag::Paint { item, row, on, steps }) if *item == id => Some((*row, steps.clone(), *on)),
            _ => None,
        };
        let paint = CellsPaint {
            clip: g.clip.clone(),
            rows: g.rows.clone(),
            step_w: self.step_w,
            scroll_x: self.scroll_x,
            play_step,
            preview,
        };
        let cells = self.cells.clone();
        let names =
            div().w(px(NAMES_W)).flex_none().flex().flex_col().border_r_1().border_color(c(THEME.line)).children(
                g.rows.iter().map(|row| {
                    let name: SharedString = match row {
                        Row::Key(k) => match GM_DRUMS.iter().find(|(gk, _)| gk == k) {
                            Some((_, n)) if g.kit => format!("{}  {}", tr(n), key_name(*k)).into(),
                            _ => key_name(*k).into(),
                        },
                        Row::Pitched => tr("Notes").into(),
                    };
                    div()
                        .h(px(STEP_ROW_H))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .text_size(px(11.))
                        .overflow_hidden()
                        .child(name)
                }),
            );
        div()
            .flex_none()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(HEADER_H))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .bg(c(THEME.bg2))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(div().w(px(4.)).h(px(12.)).rounded(px(2.)).bg(c(g.clip.color)))
                    .child(div().text_size(px(11.)).child(format!("{} · {}", g.clip.track_name, g.clip.name)))
                    .child(label(trf("{0} steps", &[&Self::steps(&g.clip)]))),
            )
            .child(
                div().flex().flex_row().h(px(g.rows.len() as f32 * STEP_ROW_H)).child(names).child(
                    div()
                        .id(SharedString::from(format!("st-cells-{}", id.number)))
                        .debug_selector(move || format!("st-cells-{}", id.number))
                        .flex_1()
                        .h_full()
                        .overflow_hidden()
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, cx.listener(Self::cells_down))
                        .on_mouse_down(MouseButton::Right, cx.listener(Self::cells_down))
                        .child(
                            canvas(
                                move |b, _, _| {
                                    cells.borrow_mut().insert(id, b);
                                },
                                move |b, _, window, cx| paint.paint(b, window, cx),
                            )
                            .size_full(),
                        ),
                ),
            )
    }
}
