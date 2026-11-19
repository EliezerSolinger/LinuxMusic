//! Owner: agent B. The step editor: repeating MIDI items edited as steps (F-PAD-002, F-PAS, P-23). It replaces
//! the legacy's Beat+Bassline editor: there is no pattern model, a "pattern" is a MIDI item whose notes repeat
//! (spec item 2), and this view edits several of them at once, one group per item, one row per piece of a drum kit
//! or of a pads instrument of samples (P-23), or a single row of pitched steps for a melodic item (a bass line).
//!
//! Per item: the steps' count (add a bar, remove a bar, clone the steps: doubled by copying), the step's size, the
//! swing (applied when the item is compiled, not written into the notes). Per step: on or off (click; drag paints;
//! the right button erases), the velocity (the wheel, ±5 %, or the lane), and in the lane under the grid the
//! velocity, pan, pitch (a pitched row's note, a drum's detune) and gate (the length) of the selected row's steps.
//!
//! The core is reached only through the `CorePort` (AddNote, SetNote, RemoveNote; the item's numbers by
//! `Field::Clip`). Each group's cells are one canvas: one quad per cell, the visible ones (report 2.2).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use fake_core::notes::{self, BAR, BEAT, Clip, Note, NoteChange, key_name};
use fake_core::port::EditOp;
use fake_core::{Feed, Field, GestureId, NodeId, Topic, Value};
use gpui::{
    Bounds, Context, CursorStyle, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Render, ScrollEvent, SharedString, Window, canvas, div, fill, point, prelude::*, px, size,
};

use super::piano_roll::kit::{self, MenuItem, dropdown, label, paint_text, popup, primary, sep, tip, with_alpha};
use super::piano_roll::{NEW, note_ops};
use super::{Screen, ScreenCx};
use crate::ui::{THEME, button, c, tr, trf};

const TOOLBAR_H: f32 = 32.0;
const HEADER_H: f32 = 28.0;
const ROW_H: f32 = 20.0;
const NAMES_W: f32 = 150.0;
const LANE_H: f32 = 64.0;

/// A row of a group: a piece of a kit (its key), or the pitched steps of a melodic item
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Key(u8, &'static str),
    Pitched,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaneMode {
    Velocity,
    Pan,
    Pitch,
    Gate,
}

const LANES: [(LaneMode, &str); 4] = [(LaneMode::Velocity, "Velocity"), (LaneMode::Pan, "Pan"), (LaneMode::Pitch, "Pitch"), (LaneMode::Gate, "Note length")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Items,
    StepSize(NodeId),
}

const STEP_SIZES: [(&str, i64); 4] = [("1/8", BEAT / 2), ("1/16", BEAT / 4), ("1/32", BEAT / 8), ("1/16T", BEAT / 6)];

enum Drag {
    /// a drag along a row turns steps on (or off)
    Paint { g: Option<GestureId>, item: NodeId, row: Row, on: bool, last: i64 },
    /// Alt+drag on a pitched step: its note
    Pitch { g: Option<GestureId>, item: NodeId, n: u64, origin: Note, from_y: f32 },
    Lane { g: Option<GestureId>, last_x: f32 },
    Swing { g: Option<GestureId>, item: NodeId, origin: f64, from_y: f32 },
}

pub struct StepEditor {
    scx: ScreenCx,
    focus: FocusHandle,
    feed: Feed,
    /// the items shown, one group each
    pub items: Vec<NodeId>,
    /// the row the lane edits
    pub sel_row: Option<(NodeId, Row)>,
    lane: LaneMode,
    pub step_w: f32,
    pub scroll_x: f32,
    scroll_y: f32,
    show_all: bool,
    drag: Option<Drag>,
    menu: Option<(Menu, gpui::Point<Pixels>)>,
    cells: Rc<RefCell<HashMap<NodeId, Bounds<Pixels>>>>,
    lane_bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl StepEditor {
    pub fn new(scx: ScreenCx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (feed, items) = scx.core.update(cx, |core, _| {
            // the repeating items open at first: the patterns
            let items: Vec<NodeId> = core.port().root().music.clips.iter().filter(|c| c.repeats() && c.loop_len <= 4 * BAR).map(|c| c.id).collect();
            (core.port_mut().feed(Topic::All), items)
        });
        StepEditor {
            focus: cx.focus_handle(),
            feed,
            sel_row: None,
            items,
            lane: LaneMode::Velocity,
            step_w: 22.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            show_all: false,
            drag: None,
            menu: None,
            cells: Rc::new(RefCell::new(HashMap::new())),
            lane_bounds: Rc::new(Cell::new(Bounds::default())),
            scx,
        }
    }

    fn clip<'a>(&self, id: NodeId, cx: &'a gpui::App) -> Option<&'a Clip> {
        self.scx.core.read(cx).port().root().music.clip(id)
    }

    /// The rows of an item: the kit's pieces (the used ones, or all), or one pitched row
    pub fn rows(&self, c: &Clip) -> Vec<Row> {
        match c.kit {
            None => vec![Row::Pitched],
            Some(kit) => {
                let used = |k: u8| c.notes.values().any(|x| x.key == k && x.start < c.loop_len);
                let v: Vec<Row> = kit.iter().filter(|p| self.show_all || used(p.key)).map(|p| Row::Key(p.key, p.name)).collect();
                if v.is_empty() { kit.iter().take(4).map(|p| Row::Key(p.key, p.name)).collect() } else { v }
            }
        }
    }

    fn in_row(row: Row, x: &Note) -> bool {
        match row {
            Row::Key(k, _) => x.key == k,
            Row::Pitched => true,
        }
    }

    /// The notes of a row at a step
    fn at_step(c: &Clip, row: Row, s: i64) -> Vec<(u64, Note)> {
        let (a, z) = (s * c.step, (s + 1) * c.step);
        c.notes.iter().filter(|(_, x)| Self::in_row(row, x) && x.start >= a && x.start < z).map(|(n, x)| (*n, x.clone())).collect()
    }

    pub fn steps(c: &Clip) -> i64 {
        (c.loop_len / c.step.max(1)).max(1)
    }

    /// The key a new pitched step gets: the one of the nearest earlier step, or the item's lowest note
    fn pitched_key(c: &Clip, s: i64) -> u8 {
        let t = s * c.step;
        c.notes.values().filter(|x| x.start < t).max_by_key(|x| x.start).or_else(|| c.notes.values().min_by_key(|x| x.key)).map_or(45, |x| x.key)
    }

    // --- changing, through the port ----------------------------------------------------------------------------

    fn apply(&mut self, label: &'static str, ops: Vec<EditOp>, cx: &mut Context<Self>) {
        if ops.is_empty() {
            return;
        }
        self.scx.core.update(cx, |core, _| {
            let _ = core.port_mut().apply(label, ops);
        });
        cx.notify();
    }

    fn gesture(&self, g: &mut Option<GestureId>, label: &'static str, ops: Vec<EditOp>, cx: &mut Context<Self>) {
        if ops.is_empty() {
            return;
        }
        self.scx.core.update(cx, |core, _| {
            let port = core.port_mut();
            let g = g.get_or_insert_with(|| port.begin_gesture(label));
            port.gesture_apply(g, ops);
        });
    }

    fn commit(&self, g: Option<GestureId>, cx: &mut Context<Self>) {
        if let Some(g) = g {
            self.scx.core.update(cx, |core, _| core.port_mut().gesture_commit(g));
        }
    }

    /// The changes that turn a step on or off
    fn set_step(c: &Clip, row: Row, s: i64, on: bool) -> Vec<NoteChange> {
        let here = Self::at_step(c, row, s);
        match (on, here.is_empty()) {
            (true, true) => {
                let key = match row {
                    Row::Key(k, _) => k,
                    Row::Pitched => Self::pitched_key(c, s),
                };
                vec![(NEW + s as u64, Some(Note::new(key, s * c.step, c.step, 0.8)))]
            }
            (false, false) => here.into_iter().map(|(n, _)| (n, None)).collect(),
            _ => Vec::new(),
        }
    }

    fn set_clip_numbers(&mut self, item: NodeId, label: &'static str, loop_len: i64, extra: Vec<EditOp>, cx: &mut Context<Self>) {
        let Some(c) = self.clip(item, cx).cloned() else { return };
        let mut ops = vec![EditOp::set(item, Field::Clip("loop"), Value::Real(loop_len as f64))];
        if c.length < loop_len {
            ops.push(EditOp::set(item, Field::Clip("length"), Value::Real(loop_len as f64)));
        }
        ops.extend(extra);
        self.apply(label, ops, cx);
    }

    /// One bar more of steps (F-PAD-002 "add steps")
    fn add_bar(&mut self, item: NodeId, cx: &mut Context<Self>) {
        let Some(c) = self.clip(item, cx).cloned() else { return };
        self.set_clip_numbers(item, "Add steps", c.loop_len + BAR, Vec::new(), cx);
    }

    /// One bar less; the notes past the new end go with it
    fn remove_bar(&mut self, item: NodeId, cx: &mut Context<Self>) {
        let Some(c) = self.clip(item, cx).cloned() else { return };
        if c.loop_len <= BAR {
            return;
        }
        let end = c.loop_len - BAR;
        let gone: Vec<NoteChange> = c.notes.iter().filter(|(_, x)| x.start >= end).map(|(n, _)| (*n, None)).collect();
        self.set_clip_numbers(item, "Remove steps", end, note_ops(item, gone), cx);
    }

    /// The steps doubled, the second half a copy of the first (F-PAD-002 "clone steps")
    fn clone_steps(&mut self, item: NodeId, cx: &mut Context<Self>) {
        let Some(c) = self.clip(item, cx).cloned() else { return };
        let copies: Vec<NoteChange> = c
            .notes
            .values()
            .filter(|x| x.start < c.loop_len)
            .enumerate()
            .map(|(i, x)| (NEW + i as u64, Some(Note { start: x.start + c.loop_len, ..x.clone() })))
            .collect();
        self.set_clip_numbers(item, "Clone steps", c.loop_len * 2, note_ops(item, copies), cx);
    }

    // --- the mouse ---------------------------------------------------------------------------------------------

    /// The item, row and step under a window position
    fn cell_at(&self, p: gpui::Point<Pixels>, cx: &gpui::App) -> Option<(NodeId, Row, i64)> {
        let cells = self.cells.borrow();
        for (id, b) in cells.iter() {
            if !b.contains(&p) {
                continue;
            }
            let c = self.clip(*id, cx)?;
            let rows = self.rows(c);
            let r = ((p.y - b.origin.y).as_f32() / ROW_H) as usize;
            let s = (((p.x - b.origin.x).as_f32() + self.scroll_x) / self.step_w).floor() as i64;
            if r < rows.len() && s >= 0 && s < Self::steps(c) {
                return Some((*id, rows[r], s));
            }
        }
        None
    }

    fn cells_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.menu = None;
        let Some((item, row, s)) = self.cell_at(ev.position, cx) else { return };
        let Some(c) = self.clip(item, cx).cloned() else { return };
        self.sel_row = Some((item, row));
        let here = Self::at_step(&c, row, s);
        if ev.modifiers.alt && row == Row::Pitched && !here.is_empty() {
            let (n, origin) = here[0].clone();
            self.drag = Some(Drag::Pitch { g: None, item, n, origin, from_y: ev.position.y.as_f32() });
        } else {
            // the first step decides: a drag turns on (from an empty step) or off (from a lit one); right erases
            let on = ev.button == MouseButton::Left && here.is_empty();
            let mut g = None;
            self.gesture(&mut g, if on { "Add steps" } else { "Remove steps" }, note_ops(item, Self::set_step(&c, row, s, on)), cx);
            self.drag = Some(Drag::Paint { g, item, row, on, last: s });
        }
        cx.notify();
    }

    fn drag_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut drag) = self.drag.take() else { return };
        match &mut drag {
            Drag::Paint { g, item, row, on, last } => {
                if let Some(c) = self.clip(*item, cx).cloned() {
                    let b = self.cells.borrow().get(item).copied().unwrap_or_default();
                    let s = (((ev.position.x - b.origin.x).as_f32() + self.scroll_x) / self.step_w).floor() as i64;
                    let s = s.clamp(0, Self::steps(&c) - 1);
                    if s != *last {
                        let (a, z) = (s.min(*last), s.max(*last));
                        let ch: Vec<NoteChange> = (a..=z).flat_map(|k| Self::set_step(&c, *row, k, *on)).collect();
                        *last = s;
                        self.gesture(g, "Paint steps", note_ops(*item, ch), cx);
                    }
                }
            }
            Drag::Pitch { g, item, n, origin, from_y } => {
                let d = ((*from_y - ev.position.y.as_f32()) / 6.0) as i32;
                let key = (origin.key as i32 + d).clamp(0, 127) as u8;
                self.gesture(g, "Step pitch", note_ops(*item, vec![(*n, Some(Note { key, ..origin.clone() }))]), cx);
            }
            Drag::Lane { g, last_x } => {
                let x = ev.position.x.as_f32();
                let ops = self.lane_ops(*last_x, x, ev.position.y.as_f32(), cx);
                *last_x = x;
                self.gesture(g, "Step values", ops, cx);
            }
            Drag::Swing { g, item, origin, from_y } => {
                let v = (*origin + ((*from_y - ev.position.y.as_f32()) / 200.0) as f64).clamp(0.0, 1.0);
                self.gesture(g, "Swing", vec![EditOp::set(*item, Field::Clip("swing"), Value::Real((v * 100.0).round() / 100.0))], cx);
            }
        }
        self.drag = Some(drag);
        cx.notify();
    }

    fn drag_up(&mut self, _ev: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(d) = self.drag.take() {
            let g = match d {
                Drag::Paint { g, .. } | Drag::Pitch { g, .. } | Drag::Lane { g, .. } | Drag::Swing { g, .. } => g,
            };
            self.commit(g, cx);
        }
        cx.notify();
    }

    fn cells_wheel(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        if primary(&ev.modifiers) {
            self.step_w = (self.step_w * (-d.y.as_f32() * 0.01).exp()).clamp(8.0, 64.0);
            cx.notify();
            return;
        }
        if ev.modifiers.shift || d.x.as_f32().abs() > d.y.as_f32().abs() {
            let dx = if d.x.as_f32().abs() > 0.01 { d.x.as_f32() } else { d.y.as_f32() };
            self.scroll_x = (self.scroll_x - dx).max(0.0);
            cx.notify();
            return;
        }
        // the wheel on a lit step: its velocity, 5 % a notch (F-PAD-002)
        if let Some((item, row, s)) = self.cell_at(ev.position, cx) {
            if let Some(c) = self.clip(item, cx).cloned() {
                let here = Self::at_step(&c, row, s);
                if !here.is_empty() && d.y.as_f32().abs() > 0.5 {
                    let dv = if d.y.as_f32() > 0.0 { 0.05 } else { -0.05 };
                    let ch = here.into_iter().map(|(n, x)| (n, Some(Note { velocity: (x.velocity + dv).clamp(0.0, 1.0), ..x }))).collect();
                    self.apply("Step velocity", note_ops(item, ch), cx);
                    return;
                }
            }
        }
        self.scroll_y = (self.scroll_y - d.y.as_f32()).max(0.0);
        cx.notify();
    }

    // --- the lane ----------------------------------------------------------------------------------------------

    /// The selected row's value of a lane, 0..1 of its height, from a note
    fn lane_get(&self, row: Row, c: &Clip, x: &Note) -> f64 {
        match self.lane {
            LaneMode::Velocity => x.velocity,
            LaneMode::Pan => x.pan * 0.5 + 0.5,
            LaneMode::Pitch => match row {
                Row::Key(..) => (x.bend_at(0) + 12.0) / 24.0,
                Row::Pitched => (x.key as f64 - Self::pitch_base(c) as f64) / 24.0,
            },
            LaneMode::Gate => x.length as f64 / c.step as f64 / 2.0,
        }
    }

    fn lane_set(&self, row: Row, c: &Clip, x: &Note, t: f64) -> Note {
        let t = t.clamp(0.0, 1.0);
        match self.lane {
            LaneMode::Velocity => Note { velocity: (t * 100.0).round() / 100.0, ..x.clone() },
            LaneMode::Pan => Note { pan: ((t * 2.0 - 1.0) * 100.0).round() / 100.0, ..x.clone() },
            LaneMode::Pitch => match row {
                Row::Key(..) => Note { bend: vec![notes::Bend { at: 0, semitones: (t * 24.0 - 12.0).round() }], ..x.clone() },
                Row::Pitched => Note { key: (Self::pitch_base(c) as f64 + (t * 24.0).round()) as u8, ..x.clone() },
            },
            LaneMode::Gate => Note { length: ((t * 2.0 * c.step as f64).round() as i64).max(c.step / 8).max(1), ..x.clone() },
        }
    }

    /// A pitched row's lane spans two octaves from here
    fn pitch_base(c: &Clip) -> u8 {
        let lo = c.notes.values().map(|x| x.key).min().unwrap_or(45);
        let hi = c.notes.values().map(|x| x.key).max().unwrap_or(57);
        ((lo as i32 + hi as i32) / 2 - 12).clamp(0, 103) as u8
    }

    fn lane_ops(&self, x0: f32, x1: f32, y: f32, cx: &gpui::App) -> Vec<EditOp> {
        let Some((item, row)) = self.sel_row else { return Vec::new() };
        let Some(c) = self.clip(item, cx) else { return Vec::new() };
        let b = self.lane_bounds.get();
        let t = (1.0 - (y - b.origin.y.as_f32() - 3.0) / (b.size.height.as_f32() - 6.0)) as f64;
        let to_step = |x: f32| (((x - b.origin.x.as_f32() - NAMES_W) + self.scroll_x) / self.step_w).floor() as i64;
        let (a, z) = (to_step(x0.min(x1)), to_step(x0.max(x1)));
        let ch: Vec<NoteChange> = (a.max(0)..=z.min(Self::steps(c) - 1))
            .flat_map(|s| Self::at_step(c, row, s))
            .map(|(n, x)| (n, Some(self.lane_set(row, c, &x, t))))
            .collect();
        note_ops(item, ch)
    }

    fn lane_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let x = ev.position.x.as_f32();
        let mut g = None;
        let ops = self.lane_ops(x, x, ev.position.y.as_f32(), cx);
        self.gesture(&mut g, "Step values", ops, cx);
        self.drag = Some(Drag::Lane { g, last_x: x });
        cx.notify();
    }

    fn key_down(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = ev.keystroke.key.as_str();
        if primary(&ev.keystroke.modifiers) && k == "z" {
            self.drag = None;
            self.scx.core.update(cx, |core, _| {
                let _ = core.port_mut().undo();
            });
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn menu_items(&self, m: Menu, cx: &gpui::App) -> Vec<MenuItem> {
        let music = &self.scx.core.read(cx).port().root().music;
        match m {
            Menu::Items => music.clips.iter().map(|c| MenuItem::new(format!("{} · {}", c.track_name, c.name), self.items.contains(&c.id)).color(c.color)).collect(),
            Menu::StepSize(id) => {
                let step = music.clip(id).map_or(0, |c| c.step);
                STEP_SIZES.iter().map(|(l, t)| MenuItem::new(*l, *t == step)).collect()
            }
        }
    }

    fn pick(&mut self, m: Menu, i: usize, cx: &mut Context<Self>) {
        self.menu = None;
        match m {
            Menu::Items => {
                let id = self.scx.core.read(cx).port().root().music.clips.get(i).map(|c| c.id);
                if let Some(id) = id {
                    if let Some(k) = self.items.iter().position(|x| *x == id) {
                        self.items.remove(k);
                    } else {
                        self.items.push(id);
                    }
                }
            }
            Menu::StepSize(id) => self.apply("Step size", vec![EditOp::set(id, Field::Clip("step"), Value::Real(STEP_SIZES[i].1 as f64))], cx),
        }
        cx.notify();
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Rendering

struct CellsPaint {
    clip: Clip,
    rows: Vec<Row>,
    step_w: f32,
    scroll_x: f32,
    play_step: Option<i64>,
    sel_row: Option<usize>,
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
        // the swing shows as the off-beat steps drawn a little late
        let swing_px = (cl.swing * 0.5 * self.step_w as f64) as f32;
        for (r, row) in self.rows.iter().enumerate() {
            let y = r as f32 * ROW_H;
            if Some(r) == self.sel_row {
                window.paint_quad(fill(Bounds::new(point(o.x, o.y + px(y)), size(b.size.width, px(ROW_H))), with_alpha(THEME.blue, 0.10)));
            }
            for s in first..last {
                let x = s as f32 * self.step_w - self.scroll_x;
                // beats in two shades, so the bar reads at a glance
                let shade = c(if (s / per_beat) % 2 == 0 { THEME.bg3 } else { THEME.bg2 });
                let here = StepEditor::at_step(cl, *row, s);
                let off = if s % 2 == 1 { swing_px * 0.3 } else { 0.0 };
                let cell = Bounds::new(point(o.x + px(x + 1.0 + off), o.y + px(y + 2.0)), size(px(self.step_w - 2.0), px(ROW_H - 4.0)));
                if here.is_empty() {
                    window.paint_quad(gpui::quad(cell, px(3.0), shade, px(0.0), shade, Default::default()));
                    continue;
                }
                // lit: brighter with the velocity (F-PAD-002), and the velocity as a bar inside
                let v = here.iter().map(|(_, x)| x.velocity).fold(0.0, f64::max) as f32;
                window.paint_quad(gpui::quad(cell, px(3.0), with_alpha(cl.color, 0.35 + 0.65 * v), px(1.0), with_alpha(cl.color, 1.0), Default::default()));
                let vh = (ROW_H - 8.0) * v;
                window.paint_quad(fill(Bounds::new(point(cell.origin.x + px(2.0), cell.origin.y + px(ROW_H - 6.0 - vh)), size(px(2.0), px(vh))), c(THEME.on_accent)));
                if self.step_w >= 20.0 {
                    let text = match row {
                        Row::Pitched => Some(key_name(here[0].1.key)),
                        Row::Key(..) if here[0].1.bend_at(0) != 0.0 => Some(format!("{:+}", here[0].1.bend_at(0))),
                        _ => None,
                    };
                    if let Some(t) = text {
                        paint_text(window, cx, t.into(), cell.origin.x + px(5.0), cell.origin.y + px(2.0), 9.0, c(THEME.on_accent));
                    }
                }
            }
        }
        if let Some(p) = self.play_step {
            let x = p as f32 * self.step_w - self.scroll_x;
            if x >= -self.step_w && x < w {
                window.paint_quad(fill(Bounds::new(point(o.x + px(x), o.y), size(px(self.step_w), b.size.height)), with_alpha(THEME.cursor, 0.12)));
            }
        }
    }
}

impl Render for StepEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        {
            // I-3.6: an item that is gone leaves the view
            let port = self.scx.core.read(cx).port();
            if port.drain(&mut self.feed).is_some() {
                let music = &port.root().music;
                self.items.retain(|id| music.clip(*id).is_some());
            }
        }
        let pos = self.scx.core.read(cx).port().telemetry().position;
        if pos.playing {
            window.request_animation_frame();
        }
        let tick = notes::sample_to_tick(pos.at(now));
        let entity = cx.entity();
        let clips: Vec<Clip> = self.items.iter().filter_map(|id| self.clip(*id, cx).cloned()).collect();
        let mut body = div()
            .id("st-body")
            .flex_1()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_scroll(cx.listener(|st, ev: &ScrollEvent, _, cx| st.cells_wheel(ev, cx)));
        let mut list = div().flex().flex_col().mt(px(-self.scroll_y));
        if clips.is_empty() {
            list = list.child(div().p(px(16.)).text_color(c(THEME.text2)).child(tr("No item here: add one with the list above")));
        }
        for clip in clips {
            list = list.child(self.render_group(clip, tick, cx));
        }
        body = body.child(list);
        div()
            .id("steps")
            .size_full()
            .flex()
            .flex_col()
            .bg(c(THEME.bg1))
            .text_color(c(THEME.text))
            .track_focus(&self.focus)
            .key_context("StepEditor")
            .on_key_down(cx.listener(|st, ev: &KeyDownEvent, _, cx| st.key_down(ev, cx)))
            .child(self.render_toolbar(cx))
            .child(body)
            .child(self.render_lane(cx))
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
    fn open_menu(&self, m: Menu, cx: &Context<Self>) -> impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static {
        cx.listener(move |st: &mut StepEditor, ev: &MouseDownEvent, _, cx| {
            st.menu = if st.menu.is_some_and(|x| x.0 == m) { None } else { Some((m, ev.position + point(px(0.), px(10.)))) };
            cx.notify();
        })
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
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
                dropdown("st-items", trf("Items: {0}", &[&self.items.len()]), true)
                    .tooltip(tip("The MIDI items shown here: each repeating item is a pattern"))
                    .on_mouse_down(MouseButton::Left, self.open_menu(Menu::Items, cx)),
            )
            .child(
                button("st-all", tr("All rows"), self.show_all, THEME.blue)
                    .h(px(22.))
                    .tooltip(tip("Show every piece of the kit, not only the used ones"))
                    .on_click(cx.listener(|st, _, _, cx| {
                        st.show_all = !st.show_all;
                        cx.notify();
                    })),
            )
            .child(sep())
            .child(label(tr("Lane")))
            .children(LANES.iter().map(|(m, name)| {
                let m = *m;
                button(SharedString::from(format!("st-lane-{name}")), tr(name), self.lane == m, THEME.blue).h(px(22.)).on_click(cx.listener(move |st, _, _, cx| {
                    st.lane = m;
                    cx.notify();
                }))
            }))
            .child(sep())
            .child(button("st-zoom-out", "−", false, 0).h(px(22.)).on_click(cx.listener(|st, _, _, cx| {
                st.step_w = (st.step_w / 1.25).max(8.0);
                cx.notify();
            })))
            .child(button("st-zoom-in", "+", false, 0).h(px(22.)).on_click(cx.listener(|st, _, _, cx| {
                st.step_w = (st.step_w * 1.25).min(64.0);
                cx.notify();
            })))
            .child(div().flex_1())
            .child(label(tr("Click turns a step on or off, drag paints, right button erases, the wheel sets the velocity, Alt+drag a note's pitch")))
    }

    fn render_group(&self, clip: Clip, tick: f64, cx: &Context<Self>) -> impl IntoElement {
        let id = clip.id;
        let rows = self.rows(&clip);
        let steps = Self::steps(&clip);
        let play_step = {
            let t = tick - clip.start as f64;
            (t >= 0.0 && t < clip.length as f64).then(|| ((t % clip.loop_len as f64) / clip.step as f64) as i64)
        };
        let sel_row = self.sel_row.filter(|(i, _)| *i == id).and_then(|(_, r)| rows.iter().position(|x| *x == r));
        let paint = CellsPaint { clip: clip.clone(), rows: rows.clone(), step_w: self.step_w, scroll_x: self.scroll_x, play_step, sel_row };
        let cells = self.cells.clone();
        let step_label = STEP_SIZES.iter().find(|s| s.1 == clip.step).map_or_else(|| notes::format_len(clip.step), |s| s.0.to_string());
        let header = div()
            .h(px(HEADER_H))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .px(px(6.))
            .bg(c(THEME.bg2))
            .border_b_1()
            .border_color(c(THEME.line))
            .child(div().w(px(4.)).h(px(16.)).rounded(px(2.)).bg(c(clip.color)))
            .child(div().text_size(px(12.)).font_weight(gpui::FontWeight::BOLD).child(format!("{} · {}", clip.track_name, clip.name)))
            .child(label(trf("{0} steps", &[&steps])))
            .child(button(SharedString::from(format!("st-less-{}", id.number)), tr("− bar"), false, 0).tooltip(tip("Remove a bar of steps")).on_click(cx.listener(move |st, _, _, cx| st.remove_bar(id, cx))))
            .child(button(SharedString::from(format!("st-more-{}", id.number)), tr("+ bar"), false, 0).tooltip(tip("Add a bar of steps")).on_click(cx.listener(move |st, _, _, cx| st.add_bar(id, cx))))
            .child(
                button(SharedString::from(format!("st-clone-{}", id.number)), tr("Clone steps"), false, 0)
                    .tooltip(tip("Double the steps, the new half a copy of the old"))
                    .on_click(cx.listener(move |st, _, _, cx| st.clone_steps(id, cx))),
            )
            .child(sep())
            .child(label(tr("Step")))
            .child(dropdown(SharedString::from(format!("st-size-{}", id.number)), step_label, false).on_mouse_down(MouseButton::Left, self.open_menu(Menu::StepSize(id), cx)))
            .child(
                div()
                    .id(SharedString::from(format!("st-swing-{}", id.number)))
                    .px(px(6.))
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .rounded(px(3.))
                    .bg(c(THEME.bg1))
                    .text_size(px(11.))
                    .cursor(CursorStyle::ResizeUpDown)
                    .hover(|s| s.bg(c(THEME.bg3)))
                    .tooltip(tip("Swing: how late the off-beat steps play; drag up or down"))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |st, ev: &MouseDownEvent, _, cx| {
                        let origin = st.clip(id, cx).map_or(0.0, |c| c.swing);
                        st.drag = Some(Drag::Swing { g: None, item: id, origin, from_y: ev.position.y.as_f32() });
                        cx.notify();
                    }))
                    .child(trf("Swing {0} %", &[&format!("{:.0}", clip.swing * 100.0)])),
            )
            .child(div().flex_1())
            .child(
                button(SharedString::from(format!("st-pr-{}", id.number)), tr("Piano Roll"), false, 0)
                    .on_click(cx.listener(|st, _, window, cx| st.scx.open(Screen::PianoRoll, window, cx))),
            )
            .child(button(SharedString::from(format!("st-hide-{}", id.number)), "×", false, 0).tooltip(tip("Hide this item here")).on_click(cx.listener(move |st, _, _, cx| {
                st.items.retain(|x| *x != id);
                cx.notify();
            })));
        let names = div().w(px(NAMES_W)).flex_none().flex().flex_col().border_r_1().border_color(c(THEME.line)).children(rows.iter().enumerate().map(|(i, row)| {
            let row = *row;
            let selected = self.sel_row == Some((id, row));
            let name: SharedString = match row {
                Row::Key(k, name) => format!("{}  {}", tr(name), key_name(k)).into(),
                Row::Pitched => tr("Notes").into(),
            };
            div()
                .id(SharedString::from(format!("st-row-{}-{i}", id.number)))
                .h(px(ROW_H))
                .px(px(8.))
                .flex()
                .items_center()
                .text_size(px(11.))
                .overflow_hidden()
                .when(selected, |d| d.bg(with_alpha(THEME.blue, 0.18)))
                .cursor_pointer()
                .on_click(cx.listener(move |st, _, _, cx| {
                    st.sel_row = Some((id, row));
                    cx.notify();
                }))
                .child(name)
        }));
        div()
            .flex_none()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .h(px(rows.len() as f32 * ROW_H))
                    .child(names)
                    .child(
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

    fn render_lane(&self, cx: &Context<Self>) -> impl IntoElement {
        let sel = self.sel_row.and_then(|(id, row)| self.clip(id, cx).cloned().map(|c| (c, row)));
        let lb = self.lane_bounds.clone();
        let (step_w, scroll_x) = (self.step_w, self.scroll_x);
        let values: Vec<(i64, f64)> = sel
            .as_ref()
            .map(|(c, row)| (0..Self::steps(c)).filter_map(|s| Self::at_step(c, *row, s).first().map(|(_, x)| (s, self.lane_get(*row, c, x)))).collect())
            .unwrap_or_default();
        let color = sel.as_ref().map_or(THEME.text3, |(c, _)| c.color);
        let title: SharedString = match &sel {
            None => tr("Pick a row").into(),
            Some((c, Row::Key(_, name))) => format!("{} · {}", c.name, tr(name)).into(),
            Some((c, Row::Pitched)) => format!("{} · {}", c.name, tr("Notes")).into(),
        };
        div()
            .h(px(LANE_H))
            .flex_none()
            .flex()
            .flex_row()
            .border_t_1()
            .border_color(c(THEME.line))
            .bg(c(THEME.bg2))
            .child(
                div()
                    .w(px(NAMES_W))
                    .flex_none()
                    .p(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .border_r_1()
                    .border_color(c(THEME.line))
                    .child(div().text_size(px(10.)).text_color(c(THEME.text3)).child(tr(LANES.iter().find(|l| l.0 == self.lane).map_or("", |l| l.1))))
                    .child(div().text_size(px(11.)).child(title)),
            )
            .child(
                div()
                    .id("st-lane")
                    .debug_selector(|| "st-lane".into())
                    .flex_1()
                    .h_full()
                    .cursor(CursorStyle::ResizeUpDown)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::lane_down))
                    .child(
                        canvas(
                            move |b, _, _| {
                                // counted from the names' column, like the cells
                                lb.set(Bounds::new(point(b.origin.x - px(NAMES_W), b.origin.y), size(b.size.width + px(NAMES_W), b.size.height)));
                            },
                            move |b, _, window, _| {
                                let o = b.origin;
                                let h = b.size.height.as_f32() - 6.0;
                                for (s, v) in &values {
                                    let x = *s as f32 * step_w - scroll_x;
                                    let top = 3.0 + (1.0 - *v as f32).clamp(0.0, 1.0) * h;
                                    window.paint_quad(fill(Bounds::new(point(o.x + px(x + 3.0), o.y + px(top)), size(px((step_w - 6.0).max(2.0)), px(3.0 + h - top))), with_alpha(color, 0.8)));
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpui::{Modifiers, TestAppContext, VisualTestContext, point, px};

    use super::*;
    use crate::screens::piano_roll::tests::{drag, open};

    fn count(core: &gpui::Entity<crate::core::Core>, id: NodeId, cx: &mut VisualTestContext) -> usize {
        core.read_with(cx, |core, _| core.port().root().music.clip(id).unwrap().notes.len())
    }

    /// Where a row's step is on the screen
    fn at(st: &gpui::Entity<StepEditor>, id: NodeId, row: usize, s: i64, cx: &mut VisualTestContext) -> gpui::Point<Pixels> {
        let b = cx.debug_bounds(Box::leak(format!("st-cells-{}", id.number).into_boxed_str())).expect("the item is shown");
        st.read_with(cx, |st, _| b.origin + point(px(s as f32 * st.step_w - st.scroll_x + st.step_w / 2.0), px(row as f32 * ROW_H + ROW_H / 2.0)))
    }

    #[gpui::test]
    fn click_toggles_and_drag_paints_in_one_step(cx: &mut TestAppContext) {
        let (st, core, cx) = open(cx, StepEditor::new, Screen::Steps);
        let id = st.read_with(cx, |st, _| st.items[0]);
        let before = count(&core, id, cx);
        // the kick's row (the first): steps 1 to 3 are empty in the groove
        let p = at(&st, id, 0, 1, cx);
        cx.simulate_click(p, Modifiers::none());
        cx.run_until_parked();
        assert_eq!(count(&core, id, cx), before + 1, "a click turns a step on");
        cx.simulate_click(p, Modifiers::none());
        cx.run_until_parked();
        assert_eq!(count(&core, id, cx), before, "and off");
        let (a, z) = (at(&st, id, 0, 1, cx), at(&st, id, 0, 3, cx));
        drag(cx, a, z, Modifiers::none());
        assert_eq!(count(&core, id, cx), before + 3, "steps 1, 2 and 3 painted on");
        cx.executor().advance_clock(Duration::from_millis(20));
        core.update(cx, |core, _| core.port_mut().undo().ok().unwrap());
        assert_eq!(count(&core, id, cx), before, "the painting is one undo step");
    }

    #[gpui::test]
    fn add_remove_and_clone_steps(cx: &mut TestAppContext) {
        let (st, core, cx) = open(cx, StepEditor::new, Screen::Steps);
        let id = st.read_with(cx, |st, _| st.items[0]);
        let (loop0, n0) = core.read_with(cx, |core, _| {
            let c = core.port().root().music.clip(id).unwrap();
            (c.loop_len, c.notes.values().filter(|x| x.start < c.loop_len).count())
        });
        st.update(cx, |st, cx| st.clone_steps(id, cx));
        let (loop1, n1) = core.read_with(cx, |core, _| {
            let c = core.port().root().music.clip(id).unwrap();
            (c.loop_len, c.notes.len())
        });
        assert_eq!(loop1, loop0 * 2);
        assert!(n1 >= n0 * 2, "the copy doubled the notes");
        st.update(cx, |st, cx| st.remove_bar(id, cx));
        let (loop2, past) = core.read_with(cx, |core, _| {
            let c = core.port().root().music.clip(id).unwrap();
            (c.loop_len, c.notes.values().filter(|x| x.start >= c.loop_len).count())
        });
        assert_eq!(loop2, loop1 - BAR);
        assert_eq!(past, 0, "the notes past the end went with the bar");
    }
}
