//! The arrangement's editing, as the legacy song editor does it (F-ARR-001..006, F-CLP-002..008, F-CLP-022,
//! F-TRK-024, F-TRK-025, F-TRK-031, F-TRK-033, F-REG-006, F-REG-007): the tools (draw, knife, select), the grid and
//! its snapping, the selection of several items, moving them as a group, resizing and trimming by the edges, the
//! knife, copy and paste at the cursor, glue, the item's, the track's and the I/O menus, the loop and punch handles.
//! Every change goes through the port: edits (`EditOp`) and the actions of `lm-session`.

use std::collections::HashSet;

use gpui::{AnyElement, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Stateful};
use lm_port::{GestureId, PathStep};

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tool {
    Draw,
    Knife,
    Select,
}

/// Where the knife's grid starts (F-CLP-007)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KnifeFrom {
    Left,
    Right,
    Global,
}

/// The grid, as a fraction of a bar (F-ARR-009): 8, 4, 2, 1 bars, 1/2, 1/4, 1/8, 1/16 of a bar
pub(super) const GRID: [(f64, &str); 8] =
    [(8.0, "8"), (4.0, "4"), (2.0, "2"), (1.0, "1"), (0.5, "1/2"), (0.25, "1/4"), (0.125, "1/8"), (0.0625, "1/16")];

#[derive(Clone, Copy, Debug, PartialEq)]
enum LoopMode {
    Start,
    End,
    /// a new range drawn from this tick
    New(i64),
}

pub(super) struct LoopDrag {
    punch: bool,
    mode: LoopMode,
    other: (i64, i64),
    gesture: GestureId,
}

/// What the item's menu does (F-CLP-008, F-CLP-022, F-CLP-023)
#[derive(Clone, Copy, Debug, PartialEq)]
enum ClipCmd {
    Delete,
    Cut,
    Copy,
    Paste,
    Mute,
    Split,
    Duplicate,
    Glue,
    Transpose(i64),
    ClearNotes,
    ResetGainFades,
    Open,
}

/// What the track's menu does (F-TRK-031)
#[derive(Clone, Copy, Debug, PartialEq)]
enum TrackCmd {
    Clone,
    Remove,
    Clear,
    ExportMidi,
    EnterFolder,
    LeaveFolder,
}

/// What the I/O button's menu does (F-TRK-024)
#[derive(Clone, Debug, PartialEq)]
enum IoCmd {
    Input(Option<i64>),
    MidiChannel(i64),
    Output(NodeId),
    PerOutput,
    Monitor,
}

pub(super) struct EditState {
    pub tool: Tool,
    pub grid: usize,
    pub knife_from: KnifeFrom,
    /// The items selected (the page's `selected_item` is the one the keys and the editors act on)
    pub selection: HashSet<NodeId>,
    /// The track a click made the selected one (F-ARR-012)
    pub track: Option<NodeId>,
    /// The selection rectangle, in timeline coordinates: (from, to)
    marquee: Option<((f32, f32), (f32, f32))>,
    /// The pointer over the timeline, in its coordinates
    mouse: Option<(f32, f32)>,
    /// The item's menu: the item under the pointer, the tick, the track under it, where it opened
    clip_menu: Option<(Option<NodeId>, i64, Option<NodeId>, gpui::Point<Pixels>)>,
    track_menu: Option<(NodeId, gpui::Point<Pixels>)>,
    io_menu: Option<(NodeId, gpui::Point<Pixels>)>,
    /// A track to remove and whether "don't ask again" is on
    confirm_remove: Option<(NodeId, bool)>,
    loop_drag: Option<LoopDrag>,
}

impl Default for EditState {
    fn default() -> Self {
        EditState {
            tool: Tool::Draw,
            grid: 5,
            knife_from: KnifeFrom::Left,
            selection: HashSet::new(),
            track: None,
            marquee: None,
            mouse: None,
            clip_menu: None,
            track_menu: None,
            io_menu: None,
            confirm_remove: None,
            loop_drag: None,
        }
    }
}

impl EditState {
    pub fn open_track_menu(&mut self, track: NodeId, pos: gpui::Point<Pixels>) {
        self.close_menus();
        self.track_menu = Some((track, pos));
    }

    fn close_menus(&mut self) {
        self.clip_menu = None;
        self.track_menu = None;
        self.io_menu = None;
    }
}

// --- the grid ----------------------------------------------------------------------------------------------------

/// The bar a tick is in: its start and its length, by the tempo map (the meter changes count)
fn bar_info(map: &TempoMap, tick: i64) -> (i64, i64) {
    let bar = map.bar_beat_tick(tick.max(0)).bar;
    let a = map.bar_start(bar).unwrap_or(0);
    let b = map.bar_start(bar + 1).unwrap_or(a + 3840);
    (a, (b - a).max(1))
}

/// One grid step at a tick
fn step_at(map: &TempoMap, tick: i64, frac: f64) -> i64 {
    let (_, len) = bar_info(map, tick);
    ((len as f64 * frac).round() as i64).max(1)
}

/// The nearest grid line (absolute)
fn snap_abs(map: &TempoMap, tick: i64, frac: f64) -> i64 {
    let tick = tick.max(0);
    let (a, len) = bar_info(map, tick);
    if frac >= 1.0 {
        let n = frac as i64;
        let bar = map.bar_beat_tick(tick).bar;
        let lo = (bar - 1).div_euclid(n) * n + 1;
        let t0 = map.bar_start(lo).unwrap_or(a);
        let t1 = map.bar_start(lo + n).unwrap_or(t0 + len * n);
        return if tick - t0 < t1 - tick { t0 } else { t1 };
    }
    let step = step_at(map, tick, frac);
    a + ((tick - a) as f64 / step as f64).round() as i64 * step
}

/// A distance in whole grid steps (the legacy's default: the clip keeps its place off the grid, F-CLP-002)
fn snap_delta(map: &TempoMap, at: i64, delta: f64, frac: f64) -> i64 {
    let step = step_at(map, at, frac) as f64;
    ((delta / step).round() * step) as i64
}

impl CompositionPage {
    fn tempo(&self, cx: &Context<Self>) -> TempoMap {
        model::tempo_map(self.core.read(cx).port().root())
    }

    fn frac(&self) -> f64 {
        GRID[self.ed.grid.min(GRID.len() - 1)].0
    }

    /// A timeline position (x) as a tick on the grid; Alt or Ctrl: free
    fn snapped(&self, x: f32, mods: &Modifiers, cx: &Context<Self>) -> i64 {
        let raw = self.view.tick_at(x).max(0.0) as i64;
        if mods.alt || mods.control { raw } else { snap_abs(&self.tempo(cx), raw, self.frac()) }
    }

    // --- selection ---------------------------------------------------------------------------------------------

    /// The selected items, in the arrangement's order (the ones that still exist)
    pub(super) fn sel_ids(&self, cx: &Context<Self>) -> Vec<NodeId> {
        let mut out = Vec::new();
        for t in self.tracks(cx) {
            for it in &t.items {
                if self.ed.selection.contains(&it.id)
                    || (self.ed.selection.is_empty() && self.selected_item == Some(it.id))
                {
                    out.push(it.id);
                }
            }
        }
        out
    }

    fn ids_value(ids: &[NodeId]) -> Value {
        Value::list(ids.iter().map(|i| Value::text(i.to_string())))
    }

    pub(super) fn select_all(&mut self, cx: &mut Context<Self>) {
        let all: Vec<NodeId> = self.tracks(cx).iter().flat_map(|t| t.items.iter().map(|i| i.id)).collect();
        self.selected_item = all.first().copied();
        self.ed.selection = all.into_iter().collect();
        cx.notify();
    }

    pub(super) fn select_none(&mut self, cx: &mut Context<Self>) {
        self.selected_item = None;
        self.ed.selection.clear();
        self.ed.close_menus();
        cx.notify();
    }

    fn track_at_y(&self, my: f32) -> Option<NodeId> {
        self.track_hits.borrow().iter().find(|(y, h, _)| my >= *y && my <= y + h).map(|(_, _, t)| *t)
    }

    fn item_at(&self, mx: f32, my: f32) -> Option<NodeId> {
        self.hits
            .borrow()
            .iter()
            .rev()
            .find(|(x, y, w, h, ..)| mx >= *x && mx <= x + w && my >= *y && my <= y + h)
            .map(|h| h.4)
    }

    // --- the keys ----------------------------------------------------------------------------------------------

    /// S: splits the selected items that cross the cursor; none selected: every item that does (F-ARR-005)
    pub(super) fn split(&mut self, cx: &mut Context<Self>) {
        let tick = self.cursor_tick(cx);
        let ids = self.sel_ids(cx);
        self.action("arrange.split", Value::object([("items", Self::ids_value(&ids)), ("tick", Value::int(tick))]), cx);
    }

    /// Ctrl+D (F-ARR-006)
    pub(super) fn duplicate(&mut self, cx: &mut Context<Self>) {
        let ids = self.sel_ids(cx);
        if !ids.is_empty() {
            self.action("arrange.duplicate", Value::object([("items", Self::ids_value(&ids))]), cx);
        }
    }

    pub(super) fn delete_item(&mut self, cx: &mut Context<Self>) {
        let ids = self.sel_ids(cx);
        if ids.is_empty() {
            return;
        }
        self.selected_item = None;
        self.ed.selection.clear();
        self.apply("Delete", ids.into_iter().map(|id| EditOp::Remove { id: Ref::Id(id) }).collect(), cx);
    }

    pub(super) fn copy_sel(&mut self, cx: &mut Context<Self>) {
        let ids = self.sel_ids(cx);
        if !ids.is_empty() {
            self.action("arrange.copy", Value::object([("items", Self::ids_value(&ids))]), cx);
        }
    }

    pub(super) fn cut_sel(&mut self, cx: &mut Context<Self>) {
        let ids = self.sel_ids(cx);
        if ids.is_empty() {
            return;
        }
        self.selected_item = None;
        self.ed.selection.clear();
        self.action("arrange.cut", Value::object([("items", Self::ids_value(&ids))]), cx);
    }

    /// Ctrl+V: at the cursor, on the track of the first selected item, else the one the pointer is over, else the
    /// selected track (F-ARR-006)
    pub(super) fn paste_at_cursor(&mut self, cx: &mut Context<Self>) {
        let tick = self.cursor_tick(cx);
        let root = self.core.read(cx).port().root().clone();
        let first = self.sel_ids(cx).first().and_then(|id| {
            model::tracks(&root).into_iter().find(|t| t.items.iter().any(|i| i.id == *id)).map(|t| t.id)
        });
        let hover = self.ed.mouse.and_then(|(_, my)| self.track_at_y(my));
        self.paste_on(tick, first.or(hover).or(self.ed.track), cx);
    }

    fn paste_on(&mut self, tick: i64, track: Option<NodeId>, cx: &mut Context<Self>) {
        let mut p = vec![("at", Value::int(tick))];
        if let Some(t) = track {
            p.push(("track", Value::text(t.to_string())));
        }
        self.action("arrange.paste", Value::object(p), cx);
    }

    /// Glue: two or more MIDI items of a track become one (F-CLP-022)
    pub(super) fn glue(&mut self, cx: &mut Context<Self>) {
        let ids = self.sel_ids(cx);
        if ids.len() >= 2 {
            self.selected_item = ids.first().copied();
            self.ed.selection = ids.iter().take(1).copied().collect();
            self.action("clip.glue", Value::object([("items", Self::ids_value(&ids))]), cx);
        }
    }

    /// ← and →: a bar back or forward by the tempo map (F-ARR-003)
    pub(super) fn cursor_bar(&mut self, dir: i64, cx: &mut Context<Self>) {
        let map = self.tempo(cx);
        let tick = self.cursor_tick(cx);
        let bar = map.bar_beat_tick(tick.max(0)).bar;
        let here = map.bar_start(bar).unwrap_or(0);
        let to = if dir > 0 {
            map.bar_start(bar + 1).unwrap_or(here)
        } else if tick > here {
            here
        } else {
            map.bar_start((bar - 1).max(1)).unwrap_or(0)
        };
        self.action("transport.locate", Value::int(to.max(0)), cx);
    }

    /// Shift+Insert / Shift+Delete: a bar of the meter in or out at the cursor (F-ARR-002)
    pub(super) fn shift_bar(&mut self, insert: bool, cx: &mut Context<Self>) {
        let map = self.tempo(cx);
        let tick = self.cursor_tick(cx);
        let at = map.bar_start(map.bar_beat_tick(tick.max(0)).bar).unwrap_or(0);
        self.action(
            if insert { "arrange.insert_bar" } else { "arrange.remove_bar" },
            Value::object([("tick", Value::int(at))]),
            cx,
        );
    }

    pub(super) fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.ed.tool = tool;
        self.ed.marquee = None;
        cx.notify();
    }

    // --- creating a clip on an empty row (F-TRK-033) ---------------------------------------------------------------

    /// A click on an empty part of a track's row with the draw tool: a MIDI item a bar long on the bar under it
    /// (instrument tracks); the new item is the selected one
    pub(super) fn create_clip_at(&mut self, track: NodeId, tick: f64, cx: &mut Context<Self>) {
        let root = self.core.read(cx).port().root().clone();
        let Some(t) = model::tracks(&root).into_iter().find(|t| t.id == track) else { return };
        if t.instrument.is_none() || t.folder {
            return;
        }
        let map = model::tempo_map(&root);
        let bar = map.bar_beat_tick(tick.max(0.0) as i64).bar;
        let start = map.bar_start(bar).unwrap_or(0);
        let length = map.bar_start(bar + 1).unwrap_or(start + 4 * model::ppq(&root) as i64) - start;
        let node = Value::object([
            ("kind", Value::text("midi")),
            ("name", Value::text(t.name.to_string())),
            ("start", Value::int(start)),
            ("length", Value::int(length.max(1))),
        ]);
        let op = EditOp::Insert { temp: 0, parent: Ref::Id(track), list: "items".into(), index: t.items.len(), node };
        let made = self.core.update(cx, |core, _| core.port_mut().apply(Label::new("New item"), vec![op]).ok());
        if let Some(item) = made.and_then(|a| a.created.first().map(|(_, id)| *id)) {
            self.selected_item = Some(item);
            self.ed.selection = [item].into_iter().collect();
        }
        cx.notify();
    }

    // --- the pointer ---------------------------------------------------------------------------------------------

    pub(super) fn on_timeline_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx); // the arrangement's keys go to the page
        self.ed.close_menus();
        let b = *self.timeline_bounds.borrow();
        let (mx, my) = ((ev.position.x - b.origin.x).as_f32(), (ev.position.y - b.origin.y).as_f32());
        if my < view::RULER_H + marks::MARKS_H + TEMPO_H {
            return;
        }
        // an automation lane: a double click opens it in the automation editor (agent B's)
        let lane = self.lane_hits.borrow().iter().find(|(y, h, ..)| my >= *y && my <= y + h).copied();
        if let Some((_, _, lane, track)) = lane {
            self.ed.track = Some(track);
            if ev.click_count == 2 {
                let _ = self.workspace.update(cx, |ws, cx| ws.open(Screen::Automation, Some(lane), window, cx));
            }
            return;
        }
        let ctrl = ev.modifiers.control || ev.modifiers.platform;
        let hit = self
            .hits
            .borrow()
            .iter()
            .rev()
            .find(|(x, y, w, h, ..)| mx >= *x && mx <= x + w && my >= *y && my <= y + h)
            .copied();
        let row = self.track_at_y(my);
        if let Some(t) = row {
            self.ed.track = Some(t);
        }
        let Some((x, y, w, _, item, fi, fo)) = hit else {
            // the empty row: Ctrl or the select tool draws a rectangle; the draw tool makes a clip (F-TRK-033)
            if ctrl || self.ed.tool == Tool::Select {
                if !ctrl && !ev.modifiers.shift {
                    self.selected_item = None;
                    self.ed.selection.clear();
                }
                self.ed.marquee = Some(((mx, my), (mx, my)));
                cx.notify();
                return;
            }
            self.selected_item = None;
            self.ed.selection.clear();
            if self.ed.tool == Tool::Draw
                && let Some(track) = row
            {
                let tick = self.view.tick_at(mx);
                self.create_clip_at(track, tick, cx);
            }
            cx.notify();
            return;
        };
        if self.ed.tool == Tool::Knife && !ctrl {
            let tick = self.knife_tick(item, mx, ev.modifiers.alt, cx);
            self.action(
                "arrange.split",
                Value::object([("items", Self::ids_value(&[item])), ("tick", Value::int(tick))]),
                cx,
            );
            return;
        }
        if ctrl {
            // Ctrl+click toggles (F-CLP-005)
            if !self.ed.selection.remove(&item) {
                self.ed.selection.insert(item);
                self.selected_item = Some(item);
            } else if self.selected_item == Some(item) {
                self.selected_item = self.ed.selection.iter().next().copied();
            }
            cx.notify();
            return;
        }
        if !self.ed.selection.contains(&item) {
            self.ed.selection = [item].into_iter().collect();
        }
        self.selected_item = Some(item);
        if ev.click_count == 2 {
            return self.open_item(window, cx);
        }
        // the top 7 px: the fade handles at the corners, the gain handle between; the edges resize and trim
        let edge = 6.0;
        let zone = if my - y <= 7.0 {
            if mx <= x + fi.max(10.0) {
                ItemZone::FadeIn
            } else if mx >= x + w - fo.max(10.0) {
                ItemZone::FadeOut
            } else {
                ItemZone::Gain
            }
        } else if w > 3.0 * edge && mx <= x + edge {
            ItemZone::TrimStart
        } else if w > 3.0 * edge && mx >= x + w - edge {
            ItemZone::ResizeEnd
        } else {
            ItemZone::Move
        };
        let Some((_, it)) = self.selected(cx) else { return };
        let start_value = match zone {
            ItemZone::Move | ItemZone::TrimStart => it.start as f64,
            ItemZone::ResizeEnd => it.length as f64,
            ItemZone::FadeIn => it.fade_in as f64,
            ItemZone::FadeOut => it.fade_out as f64,
            ItemZone::Gain => it.gain,
        };
        let others: Vec<(NodeId, i64)> = if zone == ItemZone::Move {
            self.tracks(cx)
                .iter()
                .flat_map(|t| t.items.iter())
                .filter(|i| i.id != item && self.ed.selection.contains(&i.id))
                .map(|i| (i.id, i.start))
                .collect()
        } else {
            Vec::new()
        };
        let gesture = self.core.update(cx, |core, _| core.port_mut().begin_gesture(Label::new("Item")));
        self.item_drag = Some(ItemDrag {
            item,
            zone,
            start: (ev.position.x.as_f32(), ev.position.y.as_f32()),
            start_value,
            length: it.length,
            others,
            gesture,
        });
        cx.stop_propagation();
        cx.notify();
    }

    /// Where the knife would cut an item at x, on the grid from the left edge, the right edge or the song's start
    fn knife_tick(&self, item: NodeId, mx: f32, free: bool, cx: &Context<Self>) -> i64 {
        let raw = self.view.tick_at(mx).max(0.0) as i64;
        if free {
            return raw;
        }
        let Some(it) = self.tracks(cx).iter().flat_map(|t| t.items.iter()).find(|i| i.id == item).cloned() else {
            return raw;
        };
        let map = self.tempo(cx);
        let frac = self.frac();
        let step = step_at(&map, it.start, frac) as f64;
        match self.ed.knife_from {
            KnifeFrom::Global => snap_abs(&map, raw, frac),
            KnifeFrom::Left => it.start + (((raw - it.start) as f64 / step).round() * step) as i64,
            KnifeFrom::Right => {
                let end = it.start + it.length;
                end - (((end - raw) as f64 / step).round() * step) as i64
            }
        }
    }

    pub(super) fn drag_item(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = &self.item_drag else { return };
        let (item, zone, sv, len0, origin, others) =
            (d.item, d.zone, d.start_value, d.length, d.start, d.others.clone());
        let dx = (ev.position.x.as_f32() - origin.0) as f64 * self.view.ticks_per_px;
        let dy = (ev.position.y.as_f32() - origin.1) as f64;
        let map = self.tempo(cx);
        let frac = self.frac();
        let free = ev.modifiers.alt || ev.modifiers.control;
        let mut ops = Vec::new();
        // an edge or a start, by the legacy's rule: the distance in grid steps; Shift: the edge on a grid line;
        // Alt or Ctrl: free (F-CLP-002, F-CLP-003)
        let moved = |at: f64, raw: f64| -> i64 {
            if free {
                raw as i64
            } else if ev.modifiers.shift {
                snap_abs(&map, raw as i64, frac)
            } else {
                at as i64 + snap_delta(&map, at as i64, raw - at, frac)
            }
        };
        match zone {
            ItemZone::Move => {
                let new = moved(sv, sv + dx).max(0);
                ops.push(EditOp::set(item, &["start"], Value::int(new)));
                for (id, s0) in &others {
                    ops.push(EditOp::set(*id, &["start"], Value::int((s0 + new - sv as i64).max(0))));
                }
            }
            ItemZone::ResizeEnd => {
                let start = self.selected(cx).map_or(0, |(_, it)| it.start);
                let end = moved((start as f64) + sv, start as f64 + sv + dx);
                ops.push(EditOp::set(item, &["length"], Value::int((end - start).max(1))));
            }
            ItemZone::TrimStart => {
                let end = sv as i64 + len0;
                let new = moved(sv, sv + dx).clamp(0, end - 1);
                ops.push(EditOp::set(item, &["start"], Value::int(new)));
                ops.push(EditOp::set(item, &["length"], Value::int((end - new).max(1))));
            }
            ItemZone::FadeIn => ops.push(EditOp::set(item, &["fadeIn"], Value::int((sv + dx).max(0.0) as i64))),
            ItemZone::FadeOut => ops.push(EditOp::set(item, &["fadeOut"], Value::int((sv - dx).max(0.0) as i64))),
            ItemZone::Gain => ops.push(EditOp::set(
                item,
                &["gain"],
                Value::real(view::db_to_gain((view::gain_to_db(sv).max(-60.0) - dy * 0.3).clamp(-60.0, 12.0)).min(4.0))
                    .unwrap_or_default(),
            )),
        }
        self.core.update(cx, |core, _| core.port_mut().gesture_apply(&self.item_drag.as_ref().unwrap().gesture, ops));
        cx.notify();
    }

    /// The pointer moved over the page: the knife's line, the rectangle, the loop's handles
    pub(super) fn edit_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let b = *self.timeline_bounds.borrow();
        let (mx, my) = ((ev.position.x - b.origin.x).as_f32(), (ev.position.y - b.origin.y).as_f32());
        self.ed.mouse = Some((mx, my));
        if let Some((a, _)) = self.ed.marquee {
            self.ed.marquee = Some((a, (mx, my)));
            cx.notify();
        } else if self.ed.tool == Tool::Knife {
            cx.notify();
        }
        self.loop_move(ev, cx);
    }

    /// The left button went up: items, the volume knob, the rectangle, the loop
    pub(super) fn release(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.drag.take() {
            self.core.update(cx, |core, _| core.port_mut().gesture_commit(d.gesture));
        }
        if let Some(d) = self.item_drag.take() {
            let now = self.selected(cx).map(|(_, it)| it);
            let delta = now.as_ref().map_or(0, |n| n.start - d.start_value as i64);
            if d.zone == ItemZone::TrimStart {
                // the trim again as `clip.trim_start`: the content stays where it was (F-CLP-003)
                let tick = now.map_or(0, |n| n.start);
                self.core.update(cx, |core, _| {
                    core.port_mut().gesture_cancel(d.gesture);
                    if delta != 0 {
                        let params =
                            Value::object([("item", Value::text(d.item.to_string())), ("tick", Value::int(tick))]);
                        let _ = core.port_mut().run_action("clip.trim_start".into(), params);
                    }
                });
            } else if d.zone == ItemZone::Move && self.move_automation && delta != 0 {
                // the move again as `items.move`, which takes the automation under the items along: one undo step
                let mut ids = vec![Value::text(d.item.to_string())];
                ids.extend(d.others.iter().map(|(i, _)| Value::text(i.to_string())));
                let params = Value::object([
                    ("items", Value::list(ids)),
                    ("delta", Value::int(delta)),
                    ("automation", Value::bool(true)),
                ]);
                self.core.update(cx, |core, _| {
                    core.port_mut().gesture_cancel(d.gesture);
                    let _ = core.port_mut().run_action("items.move".into(), params);
                });
            } else {
                self.core.update(cx, |core, _| core.port_mut().gesture_commit(d.gesture));
            }
        }
        if let Some((a, z)) = self.ed.marquee.take() {
            let (x0, x1) = (a.0.min(z.0), a.0.max(z.0));
            let (y0, y1) = (a.1.min(z.1), a.1.max(z.1));
            let hit: Vec<NodeId> = self
                .hits
                .borrow()
                .iter()
                .filter(|(x, y, w, h, ..)| *x <= x1 && x + w >= x0 && *y <= y1 && y + h >= y0)
                .map(|h| h.4)
                .collect();
            if let Some(first) = hit.first() {
                self.selected_item = Some(*first);
            }
            self.ed.selection.extend(hit);
        }
        if let Some(d) = self.ed.loop_drag.take() {
            self.core.update(cx, |core, _| core.port_mut().gesture_commit(d.gesture));
        }
        cx.notify();
    }

    // --- the loop and punch handles, the Shift modes (F-REG-006, F-REG-007) --------------------------------------

    fn set_range(&mut self, punch: bool, from: i64, to: i64, gesture: Option<&GestureId>, cx: &mut Context<Self>) {
        let key = if punch { "punch" } else { "loop" };
        let ops = vec![
            EditOp::set(lm_port::ROOT, &["transport", key, "from"], Value::int(from)),
            EditOp::set(lm_port::ROOT, &["transport", key, "to"], Value::int(to)),
            EditOp::set(lm_port::ROOT, &["transport", key, "on"], Value::bool(true)),
        ];
        match gesture {
            Some(g) => self.core.update(cx, |core, _| core.port_mut().gesture_apply(g, ops)),
            None => self.apply(if punch { "Punch" } else { "Loop" }, ops, cx),
        }
    }

    /// A press on the ruler: true when it was for the loop. The top half draws the loop (or, with Alt, the punch)
    /// and drags its ends; Shift+left sets its start and Shift+right its end, on the grid, Ctrl for free
    pub(super) fn loop_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) -> bool {
        let b = *self.timeline_bounds.borrow();
        let (mx, my) = ((ev.position.x - b.origin.x).as_f32(), (ev.position.y - b.origin.y).as_f32());
        let punch = ev.modifiers.alt;
        let song = model::song(self.core.read(cx).port().root());
        let (from, to) = if punch { song.punch_range } else { song.loop_range };
        let tick = self.snapped(mx, &ev.modifiers, cx);
        let min = step_at(&self.tempo(cx), tick, self.frac());
        if ev.modifiers.shift && !ev.modifiers.alt {
            // the two-button mode: Shift+left = start, Shift+right = end; never a zero size; passing swaps
            let (a, z) = if ev.button == MouseButton::Right {
                (from.min(tick - min).max(0), tick.max(from + min))
            } else {
                (tick.min(to - min).max(0), to.max(tick + min))
            };
            self.set_range(punch, a, z, None, cx);
            return true;
        }
        if ev.button != MouseButton::Left || my >= 11.0 {
            return false;
        }
        let near = |t: i64| ((self.view.x(t as f64) - mx).abs()) <= 6.0;
        let mode = if to > from && near(from) {
            LoopMode::Start
        } else if to > from && near(to) {
            LoopMode::End
        } else {
            LoopMode::New(tick)
        };
        let gesture = self.core.update(cx, |core, _| core.port_mut().begin_gesture(Label::new("Loop")));
        self.ed.loop_drag = Some(LoopDrag { punch, mode, other: (from, to), gesture });
        true
    }

    fn loop_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = &self.ed.loop_drag else { return };
        let (punch, mode, (from, to)) = (d.punch, d.mode, d.other);
        let b = *self.timeline_bounds.borrow();
        let x = (ev.position.x - b.origin.x).as_f32();
        let tick = self.snapped(x, &ev.modifiers, cx);
        let min = step_at(&self.tempo(cx), tick, self.frac());
        let (a, z) = match mode {
            LoopMode::Start => (tick.min(to - min).max(0), to),
            LoopMode::End => (from, tick.max(from + min)),
            LoopMode::New(anchor) => (anchor.min(tick), anchor.max(tick + if tick == anchor { min } else { 0 })),
        };
        let g = self.ed.loop_drag.as_ref().map(|d| &d.gesture).unwrap();
        let ops = [("from", a), ("to", z)];
        let key = if punch { "punch" } else { "loop" };
        let mut v: Vec<EditOp> =
            ops.iter().map(|(k, t)| EditOp::set(lm_port::ROOT, &["transport", key, k], Value::int(*t))).collect();
        v.push(EditOp::set(lm_port::ROOT, &["transport", key, "on"], Value::bool(true)));
        self.core.update(cx, |core, _| core.port_mut().gesture_apply(g, v));
        cx.notify();
    }

    // --- the right button ----------------------------------------------------------------------------------------

    /// The right button on the timeline; true when it was an item or an empty row (a lane has its own menu).
    /// Ctrl+right mutes the item, Shift+right removes it (F-CLP-006); the plain one opens the menu
    pub(super) fn right_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) -> bool {
        let b = *self.timeline_bounds.borrow();
        let (mx, my) = ((ev.position.x - b.origin.x).as_f32(), (ev.position.y - b.origin.y).as_f32());
        if my < view::RULER_H + marks::MARKS_H + TEMPO_H {
            return false;
        }
        if self.lane_hits.borrow().iter().any(|(y, h, ..)| my >= *y && my <= y + h) {
            return false;
        }
        let item = self.item_at(mx, my);
        let tick = self.view.tick_at(mx).max(0.0) as i64;
        cx.stop_propagation();
        if self.ed.tool == Tool::Knife {
            // the right button cancels the knife (F-CLP-007): back to drawing
            self.ed.tool = Tool::Draw;
            cx.notify();
            return true;
        }
        if let Some(id) = item {
            if ev.modifiers.shift {
                self.apply("Delete", vec![EditOp::Remove { id: Ref::Id(id) }], cx);
                return true;
            }
            if ev.modifiers.control || ev.modifiers.platform {
                self.mute_items(&[id], cx);
                return true;
            }
            if !self.ed.selection.contains(&id) {
                self.ed.selection = [id].into_iter().collect();
            }
            self.selected_item = Some(id);
        }
        self.ed.clip_menu = Some((item, tick, self.track_at_y(my), ev.position));
        cx.notify();
        true
    }

    fn mute_items(&mut self, ids: &[NodeId], cx: &mut Context<Self>) {
        let tracks = self.tracks(cx);
        let ops = ids
            .iter()
            .filter_map(|id| tracks.iter().flat_map(|t| t.items.iter()).find(|i| i.id == *id))
            .map(|i| EditOp::set(i.id, &["mute"], Value::bool(!i.mute)))
            .collect::<Vec<_>>();
        self.apply("Mute", ops, cx);
    }

    fn run_clip_cmd(
        &mut self,
        cmd: ClipCmd,
        item: Option<NodeId>,
        tick: i64,
        track: Option<NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ed.close_menus();
        let targets = match item {
            Some(i) if self.ed.selection.contains(&i) => self.sel_ids(cx),
            Some(i) => vec![i],
            None => Vec::new(),
        };
        match cmd {
            ClipCmd::Delete => self.delete_item(cx),
            ClipCmd::Cut => self.cut_sel(cx),
            ClipCmd::Copy => self.copy_sel(cx),
            ClipCmd::Paste => self.paste_on(tick, track.or(self.ed.track), cx),
            ClipCmd::Mute => self.mute_items(&targets, cx),
            ClipCmd::Split => self.split(cx),
            ClipCmd::Duplicate => self.duplicate(cx),
            ClipCmd::Glue => self.glue(cx),
            ClipCmd::Transpose(n) => self.action(
                "clip.transpose",
                Value::object([("items", Self::ids_value(&targets)), ("semitones", Value::int(n))]),
                cx,
            ),
            ClipCmd::ClearNotes => {
                self.action("clip.clear_notes", Value::object([("items", Self::ids_value(&targets))]), cx)
            }
            ClipCmd::ResetGainFades => {
                let ops = targets
                    .iter()
                    .flat_map(|i| {
                        [
                            EditOp::set(*i, &["gain"], Value::real(1.0).unwrap_or_default()),
                            EditOp::set(*i, &["fadeIn"], Value::int(0)),
                            EditOp::set(*i, &["fadeOut"], Value::int(0)),
                        ]
                    })
                    .collect();
                self.apply("Reset gain and fades", ops, cx);
            }
            ClipCmd::Open => self.open_item(window, cx),
        }
        cx.notify();
    }

    // --- menus ---------------------------------------------------------------------------------------------------

    fn menu_box(&self, id: &'static str, pos: gpui::Point<Pixels>, w: f32, cx: &Context<Self>) -> Stateful<gpui::Div> {
        let origin = self.page_bounds.borrow().origin;
        div()
            .id(id)
            .debug_selector(move || id.into())
            .absolute()
            .left(pos.x - origin.x)
            .top(pos.y - origin.y)
            .w(px(w))
            .occlude()
            .py(px(4.))
            .rounded(px(6.))
            .bg(c(THEME.bg3))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .text_color(c(THEME.text))
            .on_mouse_down_out(cx.listener(|p, _, _, cx| {
                p.ed.close_menus();
                cx.notify();
            }))
    }

    fn menu_row(id: String, label: String, enabled: bool) -> Stateful<gpui::Div> {
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
    }

    fn rule() -> AnyElement {
        div().my(px(3.)).h(px(1.)).bg(c(THEME.line)).into_any_element()
    }

    /// The item's menu (F-CLP-008) or, on an empty row, Paste
    pub(super) fn render_clip_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (item, tick, track, pos) = self.ed.clip_menu?;
        let me = self.tracks(cx);
        let it = item.and_then(|i| me.iter().flat_map(|t| t.items.iter()).find(|x| x.id == i));
        let many = self.sel_ids(cx).len();
        let plural = many > 1;
        let has = self.core.read(cx).port().root().get("tracks").is_some();
        let mut rows: Vec<(String, ClipCmd, bool)> = Vec::new();
        if it.is_some() {
            let s = |a: &str, b: &str| if plural { tr_str(b).to_string() } else { tr_str(a).to_string() };
            rows.push((s("Delete", "Delete selection"), ClipCmd::Delete, true));
            rows.push((s("Cut", "Cut selection"), ClipCmd::Cut, true));
            rows.push((s("Copy", "Copy selection"), ClipCmd::Copy, true));
        }
        rows.push((tr_str("Paste").to_string(), ClipCmd::Paste, has));
        if let Some(i) = it {
            rows.push((if i.mute { tr_str("Unmute") } else { tr_str("Mute") }.to_string(), ClipCmd::Mute, true));
            rows.push((tr_str("Split at the cursor").to_string(), ClipCmd::Split, true));
            rows.push((tr_str("Duplicate").to_string(), ClipCmd::Duplicate, true));
            rows.push((tr_str("Join the selection").to_string(), ClipCmd::Glue, plural));
            if !i.audio {
                rows.push((tr_str("Open in the piano roll").to_string(), ClipCmd::Open, true));
                rows.push((tr_str("Transpose +1").to_string(), ClipCmd::Transpose(1), true));
                rows.push((tr_str("Transpose -1").to_string(), ClipCmd::Transpose(-1), true));
                rows.push((tr_str("Transpose +12").to_string(), ClipCmd::Transpose(12), true));
                rows.push((tr_str("Transpose -12").to_string(), ClipCmd::Transpose(-12), true));
                rows.push((tr_str("Clear all notes").to_string(), ClipCmd::ClearNotes, true));
            } else {
                rows.push((tr_str("Reset gain and fades").to_string(), ClipCmd::ResetGainFades, true));
            }
        }
        let list: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .flat_map(|(n, (label, cmd, enabled))| {
                let mut v = Vec::new();
                if n == 3 && it.is_some() || (n == 4 && it.is_some() && false) {
                    v.push(Self::rule());
                }
                let row = Self::menu_row(format!("clip-menu-{n}"), label, enabled);
                v.push(if enabled {
                    row.on_click(
                        cx.listener(move |p, _, window, cx| p.run_clip_cmd(cmd, item, tick, track, window, cx)),
                    )
                    .into_any_element()
                } else {
                    row.into_any_element()
                });
                v
            })
            .collect();
        Some(self.menu_box("clip-menu", pos, 220.0, cx).children(list).into_any_element())
    }

    fn run_track_cmd(&mut self, cmd: TrackCmd, track: NodeId, cx: &mut Context<Self>) {
        self.ed.close_menus();
        let id = Value::text(track.to_string());
        match cmd {
            TrackCmd::Clone => self.action("track.clone", Value::object([("track", id)]), cx),
            TrackCmd::Clear => self.action("track.clear", Value::object([("track", id)]), cx),
            TrackCmd::Remove => self.remove_track(track, cx),
            TrackCmd::ExportMidi => self.export_track_midi(track, cx),
            TrackCmd::EnterFolder | TrackCmd::LeaveFolder => self.folder_step(track, cmd == TrackCmd::EnterFolder, cx),
        }
        cx.notify();
    }

    /// Remove: asks first, unless "don't ask again" was ticked (the preference `interface.confirmTrackRemoval`)
    fn remove_track(&mut self, track: NodeId, cx: &mut Context<Self>) {
        let ask = model::at(self.core.read(cx).port().preferences(), &["interface", "confirmTrackRemoval"])
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if ask {
            self.ed.confirm_remove = Some((track, false));
        } else {
            self.remove_track_now(track, cx);
        }
    }

    fn remove_track_now(&mut self, track: NodeId, cx: &mut Context<Self>) {
        self.apply("Remove track", vec![EditOp::Remove { id: Ref::Id(track) }], cx);
        self.ed.track = None;
    }

    fn answer_remove(&mut self, yes: bool, cx: &mut Context<Self>) {
        let Some((track, dont_ask)) = self.ed.confirm_remove.take() else { return };
        if yes {
            if dont_ask {
                self.core.update(cx, |core, _| {
                    let path = vec![PathStep::key("interface"), PathStep::key("confirmTrackRemoval")];
                    let _ = core.port_mut().set_preference(path, Value::bool(false));
                });
            }
            self.remove_track_now(track, cx);
        }
        cx.notify();
    }

    /// "Enter the folder above" / "Leave the folder": the track's `parent` (F-TRK-031, F-TRK-007)
    fn folder_step(&mut self, track: NodeId, enter: bool, cx: &mut Context<Self>) {
        let tracks = self.tracks(cx);
        let Some(i) = tracks.iter().position(|t| t.id == track) else { return };
        let parent = if enter {
            // only a folder directly above (or a track in the same folder as the one above)
            match i.checked_sub(1).map(|j| &tracks[j]) {
                Some(above) if above.folder => Some(above.id),
                Some(above) => above.parent,
                None => None,
            }
        } else {
            None
        };
        let value = parent.map_or(Value::NULL, |p| Value::text(p.to_string()));
        self.apply("Folder", vec![EditOp::set(track, &["parent"], value)], cx);
    }

    /// "Export the track as MIDI": the system's dialog, then `midi.export` for this track
    fn export_track_midi(&mut self, track: NodeId, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let (folder, name) = {
            let st = core.read(cx).port().status();
            let p = st.get("project");
            let t = |k: &str| p.and_then(|p| p.get(k)).and_then(Value::as_text).unwrap_or("").to_string();
            (t("folder"), t("name"))
        };
        let tname = self.tracks(cx).iter().find(|t| t.id == track).map_or(name.clone(), |t| t.name.to_string());
        crate::screens::files::choose_new(
            cx,
            std::path::Path::new(&folder),
            &format!("{tname}.mid"),
            move |path, cx| {
                core.update(cx, |c, cx| {
                    let params = Value::object([
                        ("path", Value::text(path.display().to_string())),
                        ("track", Value::text(track.to_string())),
                    ]);
                    let _ = c.port_mut().run_action("midi.export".into(), params);
                    cx.notify();
                });
            },
        );
    }

    /// The track's menu (F-TRK-031), on the right button over its header
    pub(super) fn render_track_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (track, pos) = self.ed.track_menu?;
        let tracks = self.tracks(cx);
        let t = tracks.iter().find(|t| t.id == track)?;
        let i = tracks.iter().position(|x| x.id == track)?;
        let can_enter =
            i > 0 && (tracks[i - 1].folder || tracks[i - 1].parent.is_some()) && t.parent != Some(tracks[i - 1].id);
        let rows = [
            (tr_str("Clone this track"), TrackCmd::Clone, true),
            (tr_str("Remove this track"), TrackCmd::Remove, true),
            (tr_str("Clear this track"), TrackCmd::Clear, !t.folder),
            (tr_str("Export the track as MIDI"), TrackCmd::ExportMidi, t.instrument.is_some()),
            (tr_str("Enter the folder above"), TrackCmd::EnterFolder, can_enter),
            (tr_str("Leave the folder"), TrackCmd::LeaveFolder, t.parent.is_some()),
        ];
        let list: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(n, (label, cmd, enabled))| {
                let row = Self::menu_row(format!("track-menu-{n}"), label.to_string(), enabled);
                if enabled {
                    row.on_click(cx.listener(move |p, _, _, cx| p.run_track_cmd(cmd, track, cx))).into_any_element()
                } else {
                    row.into_any_element()
                }
            })
            .collect();
        Some(self.menu_box("track-menu", pos, 220.0, cx).children(list).into_any_element())
    }

    /// The question before removing a track, with "Don't ask again" (F-TRK-031)
    pub(super) fn render_confirm_remove(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (track, dont_ask) = self.ed.confirm_remove?;
        let name = self.tracks(cx).iter().find(|t| t.id == track).map_or_else(String::new, |t| t.name.to_string());
        Some(
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
                        .id("confirm-remove-track")
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
                                .child(trf("Are you sure you want to remove the track {0}?", &[&name])),
                        )
                        .child(
                            div()
                                .id("confirm-track-dont-ask")
                                .debug_selector(|| "confirm-track-dont-ask".into())
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .cursor_pointer()
                                .child(div().size(px(10.)).rounded(px(2.)).bg(c(if dont_ask {
                                    THEME.green
                                } else {
                                    THEME.bg4
                                })))
                                .child(div().text_size(px(11.)).child(tr("Don't ask again")))
                                .on_click(cx.listener(|p, _, _, cx| {
                                    if let Some((_, d)) = p.ed.confirm_remove.as_mut() {
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
                                    button("confirm-track-cancel", tr("Cancel"), false, 0)
                                        .on_click(cx.listener(|p, _, _, cx| p.answer_remove(false, cx))),
                                )
                                .child(
                                    button("confirm-track-ok", tr("OK"), true, THEME.red)
                                        .debug_selector(|| "confirm-track-ok".into())
                                        .on_click(cx.listener(|p, _, _, cx| p.answer_remove(true, cx))),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    // --- the I/O button (F-TRK-024) ------------------------------------------------------------------------------

    pub(super) fn open_io_menu(&mut self, track: NodeId, pos: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        self.ed.close_menus();
        self.ed.io_menu = Some((track, pos));
        cx.notify();
    }

    fn run_io_cmd(&mut self, cmd: IoCmd, track: NodeId, cx: &mut Context<Self>) {
        self.ed.close_menus();
        let tracks = self.tracks(cx);
        let Some(t) = tracks.iter().find(|t| t.id == track) else { return };
        match cmd {
            IoCmd::Input(None) => self.apply("Input", vec![EditOp::set(track, &["input"], Value::NULL)], cx),
            IoCmd::Input(Some(ch)) => {
                let v = Value::object([("device", Value::text("in")), ("channels", Value::list([Value::int(ch)]))]);
                self.apply("Input", vec![EditOp::set(track, &["input"], v)], cx);
            }
            IoCmd::MidiChannel(ch) => {
                let v = Value::object([
                    ("ports", Value::list(Vec::<Value>::new())),
                    ("channel", Value::int(ch)),
                    ("followSelection", Value::bool(false)),
                ]);
                self.apply("MIDI input", vec![EditOp::set(track, &["midiInput"], v)], cx);
            }
            IoCmd::Output(to) => {
                if let Some(o) = t.outputs.first() {
                    self.apply("Output", vec![EditOp::set(o.id, &["to"], Value::text(to.to_string()))], cx);
                }
            }
            IoCmd::PerOutput => self.action(
                "mixer.create_channels_for_outputs",
                Value::object([("track", Value::text(track.to_string()))]),
                cx,
            ),
            IoCmd::Monitor => {
                let now = t.monitor.to_string();
                self.cycle_monitor(track, &now, cx);
            }
        }
    }

    pub(super) fn render_io_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (track, pos) = self.ed.io_menu?;
        let core = self.core.read(cx);
        let root = core.port().root();
        let tracks = model::tracks(root);
        let t = tracks.iter().find(|t| t.id == track)?;
        let inputs = core.port().telemetry().inputs.len() as i64;
        let channels = model::channels(root);
        let mut rows: Vec<(String, Option<IoCmd>, bool)> = Vec::new();
        if t.instrument.is_some() {
            rows.push((tr_str("MIDI input").to_string(), None, false));
            rows.push((tr_str("All channels (Omni)").to_string(), Some(IoCmd::MidiChannel(-1)), true));
            for ch in 0..16 {
                rows.push((trf("Channel {0}", &[&(ch + 1)]), Some(IoCmd::MidiChannel(ch)), true));
            }
        } else {
            rows.push((tr_str("Audio input").to_string(), None, false));
            rows.push((tr_str("None").to_string(), Some(IoCmd::Input(None)), t.input.is_some()));
            for ch in 1..=inputs.min(32) {
                rows.push((trf("Input {0}", &[&ch]), Some(IoCmd::Input(Some(ch))), t.input != Some(ch)));
            }
        }
        rows.push((tr_str("Monitoring").to_string(), Some(IoCmd::Monitor), true));
        rows.push((tr_str("Output to").to_string(), None, false));
        let to = t.outputs.first().map(|o| o.to);
        for ch in &channels {
            rows.push((ch.name.to_string(), Some(IoCmd::Output(ch.id)), to != Some(ch.id)));
        }
        if t.outputs.len() > 1 {
            rows.push((tr_str("One mixer channel per output").to_string(), Some(IoCmd::PerOutput), true));
        }
        let list: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(n, (label, cmd, enabled))| {
                let head = cmd.is_none();
                let row = Self::menu_row(format!("io-menu-{n}"), label, enabled && !head)
                    .when(head, |d| d.font_weight(FontWeight::BOLD).text_color(c(THEME.text2)));
                match cmd {
                    Some(cmd) if enabled => row
                        .on_click(cx.listener(move |p, _, _, cx| p.run_io_cmd(cmd.clone(), track, cx)))
                        .into_any_element(),
                    _ => row.into_any_element(),
                }
            })
            .collect();
        Some(
            self.menu_box("io-menu", pos, 230.0, cx)
                .max_h(px(480.))
                .overflow_y_scroll()
                .children(list)
                .into_any_element(),
        )
    }

    // --- the toolbar and the overlays ----------------------------------------------------------------------------

    /// The tools and the grid, in the corner over the headers (the legacy song editor's toolbar)
    pub(super) fn render_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let tool = self.ed.tool;
        let from = self.ed.knife_from;
        div()
            .h(px(view::RULER_H))
            .px(px(4.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(3.))
            .bg(c(THEME.bg3))
            .child(
                button("tool-draw", "✎", tool == Tool::Draw, THEME.blue)
                    .debug_selector(|| "tool-draw".into())
                    .on_click(cx.listener(|p, _, _, cx| p.set_tool(Tool::Draw, cx))),
            )
            .child(
                button("tool-knife", "✂", tool == Tool::Knife, THEME.blue)
                    .debug_selector(|| "tool-knife".into())
                    .on_click(cx.listener(|p, _, _, cx| p.set_tool(Tool::Knife, cx))),
            )
            .child(
                button("tool-select", "▭", tool == Tool::Select, THEME.blue)
                    .debug_selector(|| "tool-select".into())
                    .on_click(cx.listener(|p, _, _, cx| p.set_tool(Tool::Select, cx))),
            )
            .when(tool == Tool::Knife, |d| {
                d.child(
                    button(
                        "knife-from",
                        match from {
                            KnifeFrom::Left => tr_str("From left"),
                            KnifeFrom::Right => tr_str("From right"),
                            KnifeFrom::Global => tr_str("Global"),
                        },
                        false,
                        0,
                    )
                    .on_click(cx.listener(|p, _, _, cx| {
                        p.ed.knife_from = match p.ed.knife_from {
                            KnifeFrom::Left => KnifeFrom::Right,
                            KnifeFrom::Right => KnifeFrom::Global,
                            KnifeFrom::Global => KnifeFrom::Left,
                        };
                        cx.notify();
                    })),
                )
            })
            .child(div().flex_1())
            .child(
                button("grid", format!("▦ {}", GRID[self.ed.grid.min(GRID.len() - 1)].1), false, 0)
                    .debug_selector(|| "grid".into())
                    .on_click(cx.listener(|p, _, _, cx| {
                        p.ed.grid = (p.ed.grid + 1) % GRID.len();
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// The knife's line and the selection rectangle, over the timeline
    pub(super) fn edit_overlays(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if let Some((a, z)) = self.ed.marquee {
            out.push(
                div()
                    .absolute()
                    .left(px(a.0.min(z.0)))
                    .top(px(a.1.min(z.1)))
                    .w(px((a.0 - z.0).abs()))
                    .h(px((a.1 - z.1).abs()))
                    .border_1()
                    .border_color(c(THEME.cursor))
                    .bg(gpui::rgba(0xffffff1f))
                    .into_any_element(),
            );
        }
        if self.ed.tool == Tool::Knife
            && let Some((mx, my)) = self.ed.mouse
            && let Some(item) = self.item_at(mx, my)
        {
            let tick = self.knife_tick(item, mx, false, cx);
            let x = self.view.x(tick as f64);
            if let Some((_, y, _, h, ..)) = self.hits.borrow().iter().find(|h| h.4 == item) {
                out.push(
                    div().absolute().left(px(x)).top(px(*y)).w(px(1.)).h(px(*h)).bg(c(THEME.cursor)).into_any_element(),
                );
            }
        }
        out
    }
}

/// The third row of a tall header (F-TRK-025): the MIDI input ("Omni", the channel, "-") or the audio input ("-",
/// "Input: N", "N and N+1") and the number of the mixer channel the main output goes to
pub(super) fn third_row(t: &TrackView, root: &Value) -> AnyElement {
    let input = if t.instrument.is_some() {
        let ch = model::at(root, &["tracks"])
            .and_then(|l| l.items().find(|x| x.get("id").and_then(Value::as_text) == Some(&t.id.to_string())).cloned())
            .and_then(|x| x.get("midiInput").and_then(|m| m.get("channel")).and_then(Value::as_int))
            .unwrap_or(-1);
        if ch < 0 { tr_str("Omni").to_string() } else { format!("{}", ch + 1) }
    } else {
        let chans: Vec<i64> = model::at(root, &["tracks"])
            .and_then(|l| l.items().find(|x| x.get("id").and_then(Value::as_text) == Some(&t.id.to_string())).cloned())
            .and_then(|x| x.get("input").filter(|i| !i.is_null()).and_then(|i| i.get("channels")).cloned())
            .map(|l| l.items().filter_map(Value::as_int).collect())
            .unwrap_or_default();
        match chans.as_slice() {
            [] => "-".to_string(),
            [a] => trf("Input: {0}", &[a]),
            [a, b, ..] => trf("Input: {0} and {1}", &[a, b]),
        }
    };
    let channels = model::channels(root);
    let ch = t
        .outputs
        .first()
        .and_then(|o| channels.iter().position(|c| c.id == o.to))
        .map_or_else(|| "M".to_string(), |i| format!("{}", i + 1));
    div()
        .flex()
        .flex_row()
        .gap(px(8.))
        .text_size(px(9.5))
        .text_color(c(THEME.text2))
        .child(div().flex_1().overflow_hidden().child(input))
        .child(div().child(format!("→ {ch}")))
        .into_any_element()
}
