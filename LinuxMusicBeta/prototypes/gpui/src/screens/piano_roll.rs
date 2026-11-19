//! Owner: agent B. The Piano Roll: the notes of one MIDI item (F-NOT-001..023, F-PAS-001, P-24, P-26).
//!
//! Layout (rethought, not the legacy's): one toolbar row; the keyboard, the ruler with the item's own loop and
//! cursor (P-26), the note grid, and under it the velocity or pan lane; on the right an inspector with the
//! selection's numbers (drag a number to change every selected note, double click to type it). Menus are
//! dropdowns of the toolbar; the keyboard's menu is on the keys' right button.
//!
//! Drawing: the grid, the notes, the ghost notes, the repeats and the lane are each one canvas that paints only
//! what is visible: one quad per visible note (report 2.2: the cost is per primitive), the note names only when the
//! note is wide enough. A drag is a gesture (one undo step), begun at the first real movement so a click leaves no
//! empty step; the mouse is captured during a drag, so it keeps working outside the editor.
//!
//! `kit` at the end holds what the three editors of agent B share (dropdowns, popups, value entry, text in a
//! canvas, the mouse capture).

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::{Arc, Once};
use std::time::Instant;

use fake_core::notes::{
    self, BAR, BEAT, CHORDS, Clip, GRIDS, KEY_NAMES, Note, NoteChange, NoteMap, QuantizeWhat, SCALES, format_bbt,
    format_len, key_name,
};
use fake_core::port::{EditOp, NoteRef, Ref};
use fake_core::{Feed, Field, GestureId, NodeId, Topic, Value};
use gpui::{
    Bounds, Context, CursorStyle, FocusHandle, KeyBinding, KeyDownEvent, KeyUpEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, PinchEvent, Pixels, Point, Render, ScrollEvent,
    SharedString, Window, actions, canvas, div, fill, point, prelude::*, px, size,
};

use self::kit::{MenuItem, ValueEntry, dropdown, label, paint_text, popup, primary, sep, tip, with_alpha};
use super::ScreenCx;
use crate::ui::{THEME, button, c, caption, tr, trf};

actions!(piano_roll, [PlayItem]);

const TOOLBAR_H: f32 = 32.0;
const RULER_H: f32 = 20.0;
const KEYS_W: f32 = 58.0;
const INSPECTOR_W: f32 = 200.0;
const LANE_H: f32 = 64.0;
const EDGE_PX: f32 = 5.0;
/// ticks per second at the fake's 120 BPM
const TICKS_PER_SEC: f64 = 2.0 * BEAT as f64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Draw,
    Select,
    Erase,
    Bend,
    Knife,
    Strum,
}

const TOOLS: [(Tool, &str, &str); 6] = [
    (Tool::Draw, "Draw", "Draw notes (Shift+D)"),
    (Tool::Select, "Select", "Select (Shift+S); Ctrl/Cmd held selects in any tool"),
    (Tool::Erase, "Erase", "Erase (Shift+E)"),
    (Tool::Bend, "Bend", "Pitch bend curve of a note (Shift+T)"),
    (Tool::Knife, "Knife", "Cut the notes along a line (Shift+K); Shift on release drops the short ends"),
    (Tool::Strum, "Strum", "Drag a chord to spread its starts in a staircase (Shift+J)"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapMode {
    /// the start or the end, whichever is nearer, lands on the grid
    Snap,
    /// moves by whole grid steps, keeping the offset from the grid
    Nudge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaneMode {
    Velocity,
    Pan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rec {
    Off,
    /// notes from the keyboard land where the item's (or the song's) cursor is
    Live,
    /// one step at a time at the step cursor (F-PAS-001)
    Step,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecMode {
    Overdub,
    Replace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Clip,
    Grid,
    Snap,
    Length,
    Root,
    Scale,
    Chord,
    Quantize,
    Tools,
    Ghost,
    Record,
    Keys(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prop {
    Key,
    Start,
    Length,
    Velocity,
    Pan,
    Detune,
}

const PROPS: [(Prop, &str); 6] = [
    (Prop::Key, "Key"),
    (Prop::Start, "Start"),
    (Prop::Length, "Length"),
    (Prop::Velocity, "Velocity"),
    (Prop::Pan, "Pan"),
    (Prop::Detune, "Detune"),
];

/// What a value typed in the entry goes to
#[derive(Clone, Copy, Debug, PartialEq)]
enum EntryTarget {
    Prop(Prop),
    /// the lane value of the notes at this tick (or the selected ones)
    Lane(f64),
}

enum Drag {
    /// a new note (a chord) whose length follows the mouse
    Draw { g: Option<GestureId>, ns: Vec<u64>, start: i64, base: Vec<Note>, from_x: f32 },
    Move { g: Option<GestureId>, origin: Vec<(u64, Note)>, anchor: Note, from: (f64, i32), copies: Option<Vec<u64>>, clicked: u64, toggle: bool },
    Resize { g: Option<GestureId>, origin: Vec<(u64, Note)>, anchor: Note, from: f64, left: bool, stretch: bool },
    Rect { from: Point<Pixels>, to: Point<Pixels>, toggle: bool, before: Rc<BTreeSet<u64>> },
    Erase { g: Option<GestureId> },
    Bend { g: Option<GestureId>, n: u64, origin: Note, from: (f64, f32) },
    Knife { from: Point<Pixels>, to: Point<Pixels> },
    Strum { g: Option<GestureId>, origin: Vec<(u64, Note)>, from: f64 },
    Lane { g: Option<GestureId>, last_x: f32, only_under: bool },
    Scrub { g: Option<GestureId>, prop: Prop, origin: Vec<(u64, Note)>, from_y: f32 },
    /// the item's end, its repeating part's end, or the editor's own loop on the ruler
    Marker { g: Option<GestureId>, which: Marker },
    RulerSelect { from: f64 },
    /// the keyboard: a glissando over the keys
    Keys,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    Length,
    Loop,
    LoopA,
    LoopB,
}

/// The editor's own transport (P-26): a cursor and a loop inside the item, played without the song
#[derive(Clone, Copy, Debug)]
struct ItemPlay {
    cursor: i64,
    loop_on: bool,
    loop_a: i64,
    loop_b: i64,
    since: Option<(Instant, i64)>,
}

impl ItemPlay {
    fn at(&self, now: Instant) -> i64 {
        match self.since {
            None => self.cursor,
            Some((t0, from)) => {
                let t = from + (now.duration_since(t0).as_secs_f64() * TICKS_PER_SEC) as i64;
                if self.loop_on && self.loop_b > self.loop_a && t >= self.loop_b {
                    self.loop_a + (t - self.loop_a) % (self.loop_b - self.loop_a)
                } else {
                    t
                }
            }
        }
    }
}

/// The computer keyboard as a piano (F-NOT-023): two rows, an octave and a half each
fn piano_key(k: &str) -> Option<i32> {
    const LOW: [&str; 17] = ["z", "s", "x", "d", "c", "v", "g", "b", "h", "n", "j", "m", ",", "l", ".", ";", "/"];
    const HIGH: [&str; 18] = ["q", "2", "w", "3", "e", "r", "5", "t", "6", "y", "7", "u", "i", "9", "o", "0", "p", "["];
    LOW.iter().position(|x| *x == k).map(|i| i as i32).or_else(|| HIGH.iter().position(|x| *x == k).map(|i| i as i32 + 12))
}

static BIND: Once = Once::new();

/// Note numbers from here up are temp numbers of notes to create (`EditOp::AddNote`)
pub(crate) const NEW: u64 = 1 << 63;

/// Note changes as the port's edits
pub(crate) fn note_ops(item: NodeId, changes: Vec<NoteChange>) -> Vec<EditOp> {
    changes
        .into_iter()
        .filter_map(|(n, x)| {
            let item = Ref::Id(item);
            Some(match x {
                Some(x) if n >= NEW => EditOp::AddNote { temp: (n - NEW) as u32, item, note: Value::Note(x) },
                Some(x) => EditOp::SetNote { item, n: NoteRef::N(n), field: Field::Note, value: Value::Note(x) },
                None if n >= NEW => return None,
                None => EditOp::RemoveNote { item, n: NoteRef::N(n) },
            })
        })
        .collect()
}

pub struct PianoRoll {
    scx: ScreenCx,
    focus: FocusHandle,
    feed: Feed,
    pub clip: Option<NodeId>,
    ghost: Option<NodeId>,
    pub tool: Tool,
    grid: usize,
    snap: SnapMode,
    /// None = the last note's length (the legacy's "last note")
    length: Option<usize>,
    last: Note,
    root: u8,
    scale: Option<usize>,
    chord: Option<usize>,
    lane: LaneMode,
    show_names: bool,
    /// ticks at the left edge, ticks per pixel
    pub scroll: f64,
    pub tpp: f64,
    pub key_h: f32,
    /// pixels from key 127's top to the grid's top
    pub scroll_y: f32,
    pub sel: Rc<BTreeSet<u64>>,
    drag: Option<Drag>,
    hover: Option<(f64, i32, f32, f32)>,
    clipboard: Vec<Note>,
    menu: Option<(Menu, Point<Pixels>)>,
    entry: Option<(EntryTarget, ValueEntry)>,
    rec: Rec,
    rec_mode: RecMode,
    rec_quantize: Option<usize>,
    octave: i32,
    /// keys held on the computer keyboard: (key, number, start)
    held: HashMap<String, (u8, Option<u64>, i64)>,
    step_cursor: i64,
    step_chord: Vec<u64>,
    pressed: BTreeSet<u8>,
    marked: BTreeSet<u8>,
    play: ItemPlay,
    grid_bounds: Rc<Cell<Bounds<Pixels>>>,
    lane_bounds: Rc<Cell<Bounds<Pixels>>>,
    keys_bounds: Rc<Cell<Bounds<Pixels>>>,
    ruler_bounds: Rc<Cell<Bounds<Pixels>>>,
    fitted: bool,
}

impl PianoRoll {
    pub fn new(scx: ScreenCx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        BIND.call_once(|| cx.bind_keys([KeyBinding::new("space", PlayItem, Some("PianoRoll"))]));
        let (feed, clip) = scx.core.update(cx, |core, _| {
            let clip = core.port().root().music.clips.first().map(|c| c.id);
            (core.port_mut().feed(Topic::All), clip)
        });
        let mut pr = PianoRoll {
            focus: cx.focus_handle(),
            feed,
            clip,
            ghost: None,
            tool: Tool::Draw,
            grid: 4,
            snap: SnapMode::Snap,
            length: None,
            last: Note::new(60, 0, BEAT / 4, 0.8),
            root: 9,
            scale: Some(1),
            chord: None,
            lane: LaneMode::Velocity,
            show_names: true,
            scroll: 0.0,
            tpp: 8.0,
            key_h: 10.0,
            scroll_y: 0.0,
            sel: Rc::new(BTreeSet::new()),
            drag: None,
            hover: None,
            clipboard: Vec::new(),
            menu: None,
            entry: None,
            rec: Rec::Off,
            rec_mode: RecMode::Overdub,
            rec_quantize: None,
            octave: 4,
            held: HashMap::new(),
            step_cursor: 0,
            step_chord: Vec::new(),
            pressed: BTreeSet::new(),
            marked: BTreeSet::new(),
            play: ItemPlay { cursor: 0, loop_on: true, loop_a: 0, loop_b: BAR, since: None },
            grid_bounds: Rc::new(Cell::new(Bounds::default())),
            lane_bounds: Rc::new(Cell::new(Bounds::default())),
            keys_bounds: Rc::new(Cell::new(Bounds::default())),
            ruler_bounds: Rc::new(Cell::new(Bounds::default())),
            fitted: false,
            scx,
        };
        pr.reset_item_loop(cx);
        pr
    }

    // --- reading -----------------------------------------------------------------------------------------------

    fn clip<'a>(&self, cx: &'a gpui::App) -> Option<&'a Clip> {
        self.clip.and_then(|id| self.scx.core.read(cx).port().root().music.clip(id))
    }

    fn notes(&self, cx: &gpui::App) -> Arc<NoteMap> {
        self.clip(cx).map(|c| c.notes.clone()).unwrap_or_default()
    }

    fn grid_ticks(&self) -> i64 {
        GRIDS[self.grid].1
    }

    fn new_len(&self) -> i64 {
        self.length.map_or(self.last.length, |i| GRIDS[i].1)
    }

    fn x_to_tick(&self, x: f32) -> f64 {
        self.scroll + x as f64 * self.tpp
    }
    fn tick_to_x(&self, t: f64) -> f32 {
        ((t - self.scroll) / self.tpp) as f32
    }
    fn y_to_key(&self, y: f32) -> i32 {
        127 - ((y + self.scroll_y) / self.key_h).floor() as i32
    }
    fn key_to_y(&self, k: i32) -> f32 {
        (127 - k) as f32 * self.key_h - self.scroll_y
    }

    /// The topmost note under (tick, key) and which part: -1 left edge, 0 body, 1 right edge
    fn note_at(&self, notes: &NoteMap, tick: f64, key: i32) -> Option<(u64, i8)> {
        let mut hit = None;
        for (n, x) in notes.iter() {
            if x.key as i32 == key && tick >= x.start as f64 && tick < x.end() as f64 {
                let w = (x.length as f64 / self.tpp) as f32;
                let e = EDGE_PX.min(w / 3.0);
                let dx_l = ((tick - x.start as f64) / self.tpp) as f32;
                let dx_r = ((x.end() as f64 - tick) / self.tpp) as f32;
                let part = if dx_r <= e { 1 } else if dx_l <= e && w > 12.0 { -1 } else { 0 };
                hit = Some((*n, part));
            }
        }
        hit
    }

    fn snap_tick(&self, t: f64, free: bool) -> i64 {
        if free { t.round() as i64 } else { notes::snap_floor(t, self.grid_ticks()) }
    }

    fn scale_key(&self, k: i32) -> i32 {
        match self.scale {
            Some(s) if self.chord.is_none() => notes::to_scale(k, self.root, s) as i32,
            _ => k.clamp(0, 127),
        }
    }

    // --- changing ----------------------------------------------------------------------------------------------

    /// One transaction of note edits through the port (AddNote, SetNote, RemoveNote). Returns the numbers the
    /// core gave the new notes, in the order of their temp numbers (empty while the result is `Waiting`)
    fn transact(&mut self, label: &'static str, changes: Vec<NoteChange>, cx: &mut Context<Self>) -> Vec<u64> {
        let Some(id) = self.clip else { return Vec::new() };
        if changes.is_empty() {
            return Vec::new();
        }
        let before = changes.iter().any(|(n, _)| *n >= NEW).then(|| self.notes(cx));
        self.scx.core.update(cx, |core, _| {
            let _ = core.port_mut().apply(label, note_ops(id, changes));
        });
        cx.notify();
        before.map(|b| self.added_since(&b, cx)).unwrap_or_default()
    }

    /// Updates a drag's gesture, opening it at the first change; returns the new notes' numbers like `transact`
    fn gesture(&self, g: &mut Option<GestureId>, label: &'static str, changes: Vec<NoteChange>, cx: &mut Context<Self>) -> Vec<u64> {
        let Some(id) = self.clip else { return Vec::new() };
        let before = changes.iter().any(|(n, _)| *n >= NEW).then(|| self.notes(cx));
        self.scx.core.update(cx, |core, _| {
            let port = core.port_mut();
            let g = g.get_or_insert_with(|| port.begin_gesture(label));
            port.gesture_apply(g, note_ops(id, changes));
        });
        before.map(|b| self.added_since(&b, cx)).unwrap_or_default()
    }

    /// Temp numbers for notes to create (the core gives the real ones)
    fn numbers(&self, k: usize, _cx: &mut Context<Self>) -> Vec<u64> {
        (0..k as u64).map(|i| NEW + i).collect()
    }

    /// The notes that appeared since `before` (the core's counter grows, so in creation order)
    fn added_since(&self, before: &NoteMap, cx: &gpui::App) -> Vec<u64> {
        self.notes(cx).keys().filter(|n| !before.contains_key(n)).copied().collect()
    }

    fn set_sel(&mut self, s: BTreeSet<u64>) {
        self.sel = Rc::new(s);
    }

    /// The selected notes, or all when nothing is selected
    fn targets(&self, notes: &NoteMap) -> Vec<u64> {
        if self.sel.is_empty() { notes.keys().copied().collect() } else { self.sel.iter().copied().filter(|n| notes.contains_key(n)).collect() }
    }

    fn selected(&self, notes: &NoteMap) -> Vec<(u64, Note)> {
        self.sel.iter().filter_map(|n| notes.get(n).map(|x| (*n, x.clone()))).collect()
    }

    fn open_clip(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.clip = Some(id);
        self.set_sel(BTreeSet::new());
        self.fitted = false;
        if self.ghost == Some(id) {
            self.ghost = None;
        }
        self.reset_item_loop(cx);
        cx.notify();
    }

    fn reset_item_loop(&mut self, cx: &Context<Self>) {
        if let Some(c) = self.clip(cx) {
            let (a, b) = (0, c.loop_len.min(c.length).max(BAR));
            self.play = ItemPlay { cursor: 0, loop_on: true, loop_a: a, loop_b: b, since: None };
            self.step_cursor = 0;
        }
    }

    /// Scrolls to the notes (once per item, on its first frame)
    fn fit(&mut self, view_w: f32, view_h: f32, cx: &Context<Self>) {
        let Some(c) = self.clip(cx) else { return };
        let (lo, hi) = c.notes.values().fold((127u8, 0u8), |(a, b), x| (a.min(x.key), b.max(x.key)));
        let (lo, hi) = if lo > hi { (48, 72) } else { (lo, hi) };
        let span = (hi - lo + 4) as f32;
        self.key_h = (view_h / span).clamp(6.0, 14.0);
        let mid = (lo as f32 + hi as f32) / 2.0;
        self.scroll_y = ((127.0 - mid) * self.key_h - view_h / 2.0).max(0.0);
        self.scroll = 0.0;
        let len = c.loop_len.min(4 * BAR).max(BAR) as f64;
        self.tpp = (len / (view_w as f64 - 20.0).max(100.0)).clamp(0.5, 200.0);
        self.fitted = true;
    }

    fn play_tick(&self, cx: &gpui::App, now: Instant) -> (Option<f64>, bool) {
        // the item's own transport first (P-26); else the song's cursor where the item is
        if self.play.since.is_some() {
            return (Some(self.play.at(now) as f64), true);
        }
        let Some(c) = self.clip(cx) else { return (None, false) };
        let pos = self.scx.core.read(cx).port().telemetry().position;
        let t = notes::sample_to_tick(pos.at(now)) - c.start as f64;
        if t < 0.0 || t >= c.length as f64 {
            return (None, false);
        }
        let t = if c.repeats() { t % c.loop_len as f64 } else { t };
        (Some(t), pos.playing)
    }

    fn toggle_play_item(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        if self.play.since.is_some() {
            self.play.cursor = self.play.at(now);
            self.play.since = None;
            self.release_all(cx);
        } else {
            let from = if self.play.loop_on && (self.play.cursor < self.play.loop_a || self.play.cursor >= self.play.loop_b) {
                self.play.loop_a
            } else {
                self.play.cursor
            };
            self.play.since = Some((now, from));
        }
        cx.notify();
    }

    // --- the mouse on the grid -------------------------------------------------------------------------------

    fn local(&self, b: &Rc<Cell<Bounds<Pixels>>>, p: Point<Pixels>) -> (f32, f32) {
        let o = b.get().origin;
        ((p.x - o.x).as_f32(), (p.y - o.y).as_f32())
    }

    fn grid_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.menu = None;
        self.entry = None;
        let Some(clip) = self.clip(cx).cloned() else { return };
        let (x, y) = self.local(&self.grid_bounds, ev.position);
        let tick = self.x_to_tick(x);
        let key = self.y_to_key(y);
        let free = ev.modifiers.alt;
        let notes = clip.notes.clone();
        let hit = self.note_at(&notes, tick, key);
        if ev.button == MouseButton::Right {
            // the knife cancels; elsewhere the right button erases (dragging erases more)
            if matches!(self.drag, Some(Drag::Knife { .. })) {
                self.drag = None;
            } else if self.tool != Tool::Bend {
                let mut g = None;
                if let Some((n, _)) = hit {
                    self.gesture(&mut g, "Erase notes", vec![(n, None)], cx);
                }
                self.drag = Some(Drag::Erase { g });
            } else if let Some((n, _)) = hit {
                // the bend tool: the right button removes the bend points (the first stays)
                let mut x = notes[&n].clone();
                x.bend.truncate(1);
                self.transact("Remove bend points", vec![(n, Some(x))], cx);
            }
            cx.notify();
            return;
        }
        let tool = if primary(&ev.modifiers) { Tool::Select } else { self.tool };
        match (tool, hit) {
            (Tool::Erase, _) => {
                let mut g = None;
                if let Some((n, _)) = hit {
                    self.gesture(&mut g, "Erase notes", vec![(n, None)], cx);
                }
                self.drag = Some(Drag::Erase { g });
            }
            (Tool::Knife, _) => {
                let p = point(px(x), px(y));
                self.drag = Some(Drag::Knife { from: p, to: p });
            }
            (Tool::Bend, Some((n, _))) => {
                self.drag = Some(Drag::Bend { g: None, n, origin: notes[&n].clone(), from: (tick, y) });
            }
            (Tool::Strum, Some((n, _))) => {
                // the chord: the selected notes, or the notes that start with the clicked one
                let mut origin = self.selected(&notes);
                if !self.sel.contains(&n) || origin.len() < 2 {
                    let s = notes[&n].start;
                    origin = notes.iter().filter(|(_, x)| (x.start - s).abs() < BEAT / 8).map(|(k, x)| (*k, x.clone())).collect();
                }
                self.set_sel(origin.iter().map(|(k, _)| *k).collect());
                self.drag = Some(Drag::Strum { g: None, origin, from: tick });
            }
            (Tool::Draw | Tool::Select, Some((n, part))) => {
                let x = notes[&n].clone();
                self.last = x.clone();
                let toggle = primary(&ev.modifiers) && self.tool != Tool::Select || ev.modifiers.platform && ev.modifiers.shift;
                if !self.sel.contains(&n) {
                    let mut s = if toggle { (*self.sel).clone() } else { BTreeSet::new() };
                    s.insert(n);
                    self.set_sel(s);
                }
                let origin = self.selected(&notes);
                if part != 0 {
                    self.drag = Some(Drag::Resize { g: None, origin, anchor: x, from: tick, left: part < 0, stretch: ev.modifiers.shift });
                } else {
                    self.drag = Some(Drag::Move { g: None, origin, anchor: x, from: (tick, key), copies: ev.modifiers.shift.then(Vec::new), clicked: n, toggle });
                }
            }
            (Tool::Select | Tool::Bend | Tool::Strum, None) => {
                let p = point(px(x), px(y));
                let before = if ev.modifiers.shift { self.sel.clone() } else { Rc::new(BTreeSet::new()) };
                self.drag = Some(Drag::Rect { from: p, to: p, toggle: ev.modifiers.shift, before });
            }
            (Tool::Draw, None) => {
                if tick < 0.0 || key < 0 || key > 127 {
                    return;
                }
                let start = self.snap_tick(tick, free);
                let root = self.scale_key(key);
                let len = self.new_len();
                let keys: Vec<u8> = match self.chord {
                    Some(ch) => notes::chord_keys(root as u8, ch),
                    None => vec![root as u8],
                };
                // Shift with a chord: an arpeggio, one grid step apart
                let arp = ev.modifiers.shift && keys.len() > 1;
                let base: Vec<Note> = keys
                    .iter()
                    .enumerate()
                    .map(|(i, k)| {
                        let s = if arp { start + i as i64 * self.grid_ticks() } else { start };
                        Note { key: *k, start: s, length: len, velocity: self.last.velocity, pan: self.last.pan, bend: Vec::new() }
                    })
                    .collect();
                let temps = self.numbers(base.len(), cx);
                let mut g = None;
                let ns = self.gesture(&mut g, "Draw notes", temps.iter().copied().zip(base.iter().cloned().map(Some)).collect(), cx);
                self.set_sel(ns.iter().copied().collect());
                self.drag = Some(Drag::Draw { g, ns, start, base, from_x: x });
                self.pressed = keys.into_iter().collect();
            }
        }
        cx.notify();
    }

    fn drag_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut drag) = self.drag.take() else { return };
        let (x, y) = self.local(&self.grid_bounds, ev.position);
        let tick = self.x_to_tick(x);
        let key = self.y_to_key(y);
        let free = ev.modifiers.alt;
        let grid = self.grid_ticks();
        let notes = self.notes(cx);
        match &mut drag {
            Drag::Draw { g, ns, start, base, from_x } => {
                if (x - *from_x).abs() > 3.0 {
                    let end = if free { tick.round() as i64 } else { notes::snap(tick, grid) };
                    let len = (end - *start).max(if free { 1 } else { grid });
                    let changes = ns.iter().zip(base.iter()).map(|(n, b)| (*n, Some(Note { length: len, ..b.clone() }))).collect();
                    self.last.length = len;
                    self.gesture(g, "Draw notes", changes, cx);
                }
            }
            Drag::Move { g, origin, anchor, from, copies, .. } => {
                let raw = tick - from.0;
                let dt = if free {
                    raw.round() as i64
                } else {
                    match self.snap {
                        SnapMode::Nudge => notes::snap(raw, grid),
                        SnapMode::Snap => {
                            // the start or the end, whichever is nearer to the grid
                            let s = anchor.start as f64 + raw;
                            let e = anchor.end() as f64 + raw;
                            let ds = notes::snap(s, grid) as f64 - s;
                            let de = notes::snap(e, grid) as f64 - e;
                            (raw + if ds.abs() <= de.abs() { ds } else { de }).round() as i64
                        }
                    }
                };
                let dt = dt.max(-origin.iter().map(|(_, x)| x.start).min().unwrap_or(0));
                let target = self.scale_key(anchor.key as i32 + (key - from.1));
                let dk = (target - anchor.key as i32)
                    .clamp(-(origin.iter().map(|(_, x)| x.key as i32).min().unwrap_or(0)), 127 - origin.iter().map(|(_, x)| x.key as i32).max().unwrap_or(127));
                if dt != 0 || dk != 0 || g.is_some() {
                    if copies.as_ref().is_some_and(|c| c.is_empty()) {
                        // Shift at the start of the drag: the copies move, the originals stay
                        *copies = Some(self.numbers(origin.len(), cx));
                    }
                    let ids: Vec<u64> = copies.clone().unwrap_or_else(|| origin.iter().map(|(n, _)| *n).collect());
                    let changes = ids
                        .iter()
                        .zip(origin.iter())
                        .map(|(n, (_, x))| (*n, Some(Note { start: x.start + dt, key: (x.key as i32 + dk) as u8, ..x.clone() })))
                        .collect();
                    if dk != 0 {
                        self.pressed = [(anchor.key as i32 + dk) as u8].into_iter().collect();
                    }
                    let new = self.gesture(g, "Move notes", changes, cx);
                    if !new.is_empty() {
                        *copies = Some(new.clone());
                        self.set_sel(new.into_iter().collect());
                    }
                }
            }
            Drag::Resize { g, origin, anchor, from, left, stretch } => {
                let raw = tick - *from;
                let changes: Vec<NoteChange> = if *left {
                    let ns = if free { (anchor.start as f64 + raw).round() as i64 } else { notes::snap(anchor.start as f64 + raw, grid) };
                    let d = ns.min(anchor.end() - 1) - anchor.start;
                    origin
                        .iter()
                        .map(|(n, x)| (*n, Some(Note { start: (x.start + d).clamp(0, x.end() - 1), length: (x.length - d).max(1), ..x.clone() })))
                        .collect()
                } else if *stretch {
                    // Shift: the selection stretches in proportion, from its first start
                    let s0 = origin.iter().map(|(_, x)| x.start).min().unwrap_or(0);
                    let f = ((anchor.end() as f64 + raw - s0 as f64) / (anchor.end() - s0).max(1) as f64).max(0.05);
                    origin
                        .iter()
                        .map(|(n, x)| {
                            let s = s0 + ((x.start - s0) as f64 * f).round() as i64;
                            (*n, Some(Note { start: s, length: ((x.length as f64 * f).round() as i64).max(1), ..x.clone() }))
                        })
                        .collect()
                } else {
                    let ne = if free { (anchor.end() as f64 + raw).round() as i64 } else { notes::snap(anchor.end() as f64 + raw, grid) };
                    let d = ne - anchor.end();
                    let min = if free { 1 } else { grid };
                    origin.iter().map(|(n, x)| (*n, Some(Note { length: (x.length + d).max(min.min(x.length)), ..x.clone() }))).collect()
                };
                if let Some((_, Some(x))) = changes.iter().find(|(n, _)| origin.first().is_some_and(|o| o.0 == *n)) {
                    self.last.length = x.length;
                }
                self.gesture(g, "Resize notes", changes, cx);
            }
            Drag::Rect { to, .. } => {
                *to = point(px(x), px(y));
            }
            Drag::Erase { g } => {
                if let Some((n, _)) = self.note_at(&notes, tick, key) {
                    self.gesture(g, "Erase notes", vec![(n, None)], cx);
                }
            }
            Drag::Bend { g, n, origin, from } => {
                // a line of the bend from where the drag began to the mouse, in semitones by height
                let at0 = ((from.0 - origin.start as f64).round() as i64).clamp(0, origin.length);
                let at1 = ((tick - origin.start as f64).round() as i64).clamp(0, origin.length);
                let v0 = origin.bend_at(at0);
                let v1 = ((from.1 - y) / self.key_h * 4.0).round() as f64 / 4.0 + v0;
                let (a, b) = if at0 <= at1 { ((at0, v0), (at1, v1)) } else { ((at1, v1), (at0, v0)) };
                let mut pts: Vec<notes::Bend> = origin.bend.iter().copied().filter(|p| p.at < a.0 || p.at > b.0).collect();
                if pts.is_empty() || pts[0].at > 0 && a.0 > 0 {
                    pts.push(notes::Bend { at: 0, semitones: origin.bend_at(0) }); // the first point is fixed
                }
                pts.push(notes::Bend { at: a.0, semitones: a.1 });
                if b.0 != a.0 {
                    pts.push(notes::Bend { at: b.0, semitones: b.1.clamp(-24.0, 24.0) });
                }
                pts.sort_by_key(|p| p.at);
                pts.dedup_by_key(|p| p.at);
                let x = Note { bend: pts, ..origin.clone() };
                self.gesture(g, "Bend", vec![(*n, Some(x))], cx);
            }
            Drag::Knife { to, .. } => {
                *to = point(px(x), px(y));
            }
            Drag::Strum { g, origin, from } => {
                let mut spread = (tick - *from).round() as i64;
                if !free {
                    spread = notes::snap(spread as f64, (grid / 4).max(1));
                }
                self.gesture(g, "Strum", notes::strum(origin, spread), cx);
            }
            Drag::Lane { .. } | Drag::Scrub { .. } | Drag::Marker { .. } | Drag::RulerSelect { .. } | Drag::Keys => {
                self.drag = Some(drag);
                return self.other_drag_move(ev, cx);
            }
        }
        self.drag = Some(drag);
        cx.notify();
    }

    fn drag_up(&mut self, ev: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else { return };
        let notes = self.notes(cx);
        let commit = |s: &Self, g: Option<GestureId>, cx: &mut Context<Self>| {
            if let Some(g) = g {
                s.scx.core.update(cx, |core, _| {
                    core.port_mut().gesture_commit(g);
                });
            }
        };
        match drag {
            Drag::Draw { g, .. } | Drag::Erase { g } | Drag::Bend { g, .. } | Drag::Strum { g, .. } | Drag::Lane { g, .. } | Drag::Scrub { g, .. } | Drag::Marker { g, .. } => {
                commit(self, g, cx)
            }
            Drag::Resize { g, .. } => commit(self, g, cx),
            Drag::Move { g, clicked, toggle, .. } => {
                if g.is_none() {
                    // a click, not a drag: Ctrl/Cmd toggles, a plain click selects only this note
                    let mut s = (*self.sel).clone();
                    if toggle {
                        if !s.remove(&clicked) {
                            s.insert(clicked);
                        }
                    } else {
                        s = [clicked].into_iter().collect();
                    }
                    self.set_sel(s);
                }
                commit(self, g, cx);
            }
            Drag::Rect { from, to, toggle, before } => {
                let (t0, t1) = (self.x_to_tick(from.x.as_f32().min(to.x.as_f32())), self.x_to_tick(from.x.as_f32().max(to.x.as_f32())));
                let (k0, k1) = (self.y_to_key(from.y.as_f32().max(to.y.as_f32())), self.y_to_key(from.y.as_f32().min(to.y.as_f32())));
                let mut s = (*before).clone();
                for (n, x) in notes.iter() {
                    if (x.key as i32) >= k0 && (x.key as i32) <= k1 && (x.end() as f64) > t0 && (x.start as f64) < t1 {
                        if toggle && s.contains(n) {
                            s.remove(n);
                        } else {
                            s.insert(*n);
                        }
                    }
                }
                self.set_sel(s);
            }
            Drag::Knife { from, to } => {
                let drop_short = ev.modifiers.shift;
                let which: Vec<u64> = if self.sel.is_empty() { notes.keys().copied().collect() } else { self.sel.iter().copied().collect() };
                let free = primary(&ev.modifiers);
                let mut changes = Vec::new();
                let (fx, fy, tx, ty) = (from.x.as_f32(), from.y.as_f32(), to.x.as_f32(), to.y.as_f32());
                let mut fresh = self.numbers(which.len(), cx).into_iter();
                for n in which {
                    let Some(x) = notes.get(&n) else { continue };
                    // where the line crosses the middle of the note's row
                    let row = self.key_to_y(x.key as i32) + self.key_h / 2.0;
                    let (y0, y1) = (fy.min(ty) - self.key_h / 2.0, fy.max(ty) + self.key_h / 2.0);
                    if row < y0 || row > y1 {
                        continue;
                    }
                    let lx = if (ty - fy).abs() < 1.0 { fx } else { fx + (tx - fx) * ((row - fy) / (ty - fy)).clamp(0.0, 1.0) };
                    let at = self.snap_tick(self.x_to_tick(lx) + if free { 0.0 } else { self.grid_ticks() as f64 / 2.0 }, free);
                    let mut one = notes::split(&notes, &[n], at, &mut || fresh.next().unwrap_or(0), drop_short);
                    changes.append(&mut one);
                }
                self.transact("Knife", changes, cx);
            }
            Drag::RulerSelect { .. } | Drag::Keys => {}
        }
        self.pressed.clear();
        cx.notify();
    }

    fn grid_hover(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.drag.is_some() {
            return;
        }
        let (x, y) = self.local(&self.grid_bounds, ev.position);
        let h = (self.x_to_tick(x), self.y_to_key(y), x, y);
        let old = self.hover;
        self.hover = Some(h);
        // redraw only when what the hover shows changes: the key row, the edge cursor
        let row_changed = old.is_none_or(|o| o.1 != h.1);
        let part = |o: Option<(f64, i32, f32, f32)>, cx: &Context<Self>| {
            o.and_then(|o| self.note_at(&self.notes(cx), o.0, o.1).map(|(_, p)| p))
        };
        if row_changed || part(old, cx) != part(Some(h), cx) || self.tool == Tool::Knife {
            cx.notify();
        }
    }

    // --- the lane, the ruler, the keyboard, the inspector -----------------------------------------------------

    fn lane_value(&self, y: f32) -> f64 {
        let h = self.lane_bounds.get().size.height.as_f32() - 6.0;
        let t = (1.0 - (y - 3.0) / h).clamp(0.0, 1.0) as f64;
        match self.lane {
            LaneMode::Velocity => (t * 100.0).round() / 100.0,
            LaneMode::Pan => ((t * 2.0 - 1.0) * 100.0).round() / 100.0,
        }
    }

    fn with_lane(&self, x: &Note, v: f64) -> Note {
        match self.lane {
            LaneMode::Velocity => Note { velocity: v, ..x.clone() },
            LaneMode::Pan => Note { pan: v, ..x.clone() },
        }
    }

    /// The notes whose start is between two x positions of the lane: the selected ones when there is a selection
    fn lane_hits(&self, notes: &NoteMap, x0: f32, x1: f32, only_under: bool) -> Vec<u64> {
        let (a, b) = (x0.min(x1) - 3.0, x0.max(x1) + 3.0);
        let mut v: Vec<(u64, f32)> = notes
            .iter()
            .filter(|(n, _)| self.sel.is_empty() || self.sel.contains(n))
            .map(|(n, x)| (*n, self.tick_to_x(x.start as f64)))
            .filter(|(_, x)| *x >= a && *x <= b)
            .collect();
        if only_under {
            v.sort_by(|p, q| (p.1 - x0).abs().total_cmp(&(q.1 - x0).abs()));
            v.truncate(1);
        }
        v.into_iter().map(|(n, _)| n).collect()
    }

    fn lane_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let (x, y) = self.local(&self.lane_bounds, ev.position);
        let notes = self.notes(cx);
        if ev.button == MouseButton::Middle {
            // the middle button: back to the default
            let d = if self.lane == LaneMode::Velocity { 0.8 } else { 0.0 };
            let ch = self.lane_hits(&notes, x, x, false).into_iter().map(|n| (n, Some(self.with_lane(&notes[&n], d)))).collect();
            self.transact("Reset", ch, cx);
            return;
        }
        if ev.click_count >= 2 {
            let t = self.x_to_tick(x);
            let v = self.lane_hits(&notes, x, x, false).first().map_or(0.8, |n| if self.lane == LaneMode::Velocity { notes[n].velocity } else { notes[n].pan });
            let shown = if self.lane == LaneMode::Velocity { v * 100.0 } else { v * 100.0 };
            self.entry = Some((EntryTarget::Lane(t), ValueEntry::new(format!("{shown:.0}"), ev.position, window, cx)));
            cx.notify();
            return;
        }
        let only_under = ev.modifiers.alt;
        let mut g = None;
        let v = self.lane_value(y);
        let ch: Vec<NoteChange> = self.lane_hits(&notes, x, x, only_under).into_iter().map(|n| (n, Some(self.with_lane(&notes[&n], v)))).collect();
        if let Some((_, Some(x))) = ch.first() {
            self.last.velocity = x.velocity;
            self.last.pan = x.pan;
        }
        self.gesture(&mut g, "Note velocity", ch, cx);
        self.drag = Some(Drag::Lane { g, last_x: x, only_under });
        cx.notify();
    }

    fn lane_wheel(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let (x, _) = self.local(&self.lane_bounds, ev.position);
        let notes = self.notes(cx);
        let d = ev.delta.pixel_delta(px(16.0)).y.as_f32();
        if d.abs() < 0.5 {
            return;
        }
        let step = if d > 0.0 { 0.01 } else { -0.01 };
        let ch = self
            .lane_hits(&notes, x, x, ev.modifiers.alt)
            .into_iter()
            .map(|n| {
                let x = &notes[&n];
                let v = if self.lane == LaneMode::Velocity { x.velocity + step } else { x.pan + step };
                (n, Some(self.with_lane(x, v)))
            })
            .collect();
        self.transact("Note velocity", ch, cx);
    }

    fn ruler_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some(c) = self.clip(cx).cloned() else { return };
        let (x, _) = self.local(&self.ruler_bounds, ev.position);
        let tick = self.x_to_tick(x);
        let near = |t: i64| (self.tick_to_x(t as f64) - x).abs() < 6.0;
        if primary(&ev.modifiers) {
            self.drag = Some(Drag::RulerSelect { from: tick });
        } else if near(c.length) {
            self.drag = Some(Drag::Marker { g: None, which: Marker::Length });
        } else if c.repeats() && near(c.loop_len) {
            self.drag = Some(Drag::Marker { g: None, which: Marker::Loop });
        } else if near(self.play.loop_a) {
            self.drag = Some(Drag::Marker { g: None, which: Marker::LoopA });
        } else if near(self.play.loop_b) {
            self.drag = Some(Drag::Marker { g: None, which: Marker::LoopB });
        } else {
            // a click moves the item's cursor (and the song's, to the same place)
            let t = notes::snap(tick, self.grid_ticks()).max(0);
            self.play.cursor = t;
            if self.play.since.is_some() {
                self.play.since = Some((Instant::now(), t));
            }
            self.step_cursor = t;
            let s = notes::tick_to_sample((c.start + t) as f64) as i64;
            self.scx.core.update(cx, |core, _| {
                let _ = core.port_mut().run_action("transport.locate", Value::Real(s as f64));
            });
        }
        cx.notify();
    }

    fn other_drag_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(mut drag) = self.drag.take() else { return };
        let notes = self.notes(cx);
        match &mut drag {
            Drag::Lane { g, last_x, only_under } => {
                let (x, y) = self.local(&self.lane_bounds, ev.position);
                let v = self.lane_value(y);
                let ch = self.lane_hits(&notes, *last_x, x, *only_under).into_iter().map(|n| (n, Some(self.with_lane(&notes[&n], v)))).collect();
                *last_x = x;
                self.gesture(g, "Note velocity", ch, cx);
            }
            Drag::Scrub { g, prop, origin, from_y } => {
                let d = (*from_y - ev.position.y.as_f32()) as f64;
                let fine = if ev.modifiers.shift { 0.25 } else { 1.0 };
                let grid = self.grid_ticks();
                let ch = origin
                    .iter()
                    .map(|(n, x)| {
                        let y = match prop {
                            Prop::Key => Note { key: (x.key as i32 + (d * fine / 6.0) as i32).clamp(0, 127) as u8, ..x.clone() },
                            Prop::Start => Note { start: (x.start + (d * fine / 6.0) as i64 * grid).max(0), ..x.clone() },
                            Prop::Length => Note { length: (x.length + (d * fine / 6.0) as i64 * grid).max(1), ..x.clone() },
                            Prop::Velocity => Note { velocity: x.velocity + d * fine / 200.0, ..x.clone() },
                            Prop::Pan => Note { pan: x.pan + d * fine / 100.0, ..x.clone() },
                            Prop::Detune => {
                                let v = (x.bend_at(0) + (d * fine / 8.0).round() / 4.0).clamp(-24.0, 24.0);
                                Note { bend: vec![notes::Bend { at: 0, semitones: v }], ..x.clone() }
                            }
                        };
                        (*n, Some(y))
                    })
                    .collect();
                self.gesture(g, "Note property", ch, cx);
            }
            Drag::Marker { g, which } => {
                let (x, _) = self.local(&self.ruler_bounds, ev.position);
                let t = notes::snap(self.x_to_tick(x), self.grid_ticks()).max(self.grid_ticks());
                match which {
                    Marker::Length | Marker::Loop => {
                        let Some(id) = self.clip else { return };
                        let key = if *which == Marker::Length { "length" } else { "loop" };
                        self.scx.core.update(cx, |core, _| {
                            let port = core.port_mut();
                            let g = g.get_or_insert_with(|| port.begin_gesture("Item length"));
                            port.gesture_apply(g, vec![EditOp::set(id, Field::Clip(key), Value::Real(t as f64))]);
                        });
                    }
                    Marker::LoopA => self.play.loop_a = (t - self.grid_ticks()).min(self.play.loop_b - self.grid_ticks()).max(0),
                    Marker::LoopB => self.play.loop_b = t.max(self.play.loop_a + self.grid_ticks()),
                }
            }
            Drag::RulerSelect { from } => {
                let (x, _) = self.local(&self.ruler_bounds, ev.position);
                let t = self.x_to_tick(x);
                let (a, b) = (from.min(t), from.max(t));
                self.set_sel(notes.iter().filter(|(_, n)| (n.start as f64) >= a && (n.start as f64) < b).map(|(n, _)| *n).collect());
            }
            Drag::Keys => {
                let (_, y) = self.local(&self.keys_bounds, ev.position);
                let k = self.y_to_key(y).clamp(0, 127) as u8;
                if !self.pressed.contains(&k) {
                    self.pressed = [k].into_iter().collect(); // a glissando
                }
            }
            _ => {}
        }
        self.drag = Some(drag);
        cx.notify();
    }

    fn keys_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let (x, y) = self.local(&self.keys_bounds, ev.position);
        let k = self.y_to_key(y).clamp(0, 127) as u8;
        if ev.button == MouseButton::Right {
            self.menu = Some((Menu::Keys(k), ev.position));
        } else if ev.modifiers.shift {
            // Shift+click: every note on this key
            let notes = self.notes(cx);
            self.set_sel(notes.iter().filter(|(_, n)| n.key == k).map(|(n, _)| *n).collect());
        } else {
            // the velocity is the horizontal position on the key
            self.last.velocity = ((x / KEYS_W) as f64).clamp(0.1, 1.0);
            self.pressed = [k].into_iter().collect();
            self.drag = Some(Drag::Keys);
        }
        cx.notify();
    }

    fn scrub_down(&mut self, prop: Prop, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let notes = self.notes(cx);
        let origin = self.selected(&notes);
        if origin.is_empty() {
            return;
        }
        if ev.click_count >= 2 {
            let x = &origin[0].1;
            let v = match prop {
                Prop::Key => x.key.to_string(),
                Prop::Start => format_bbt(x.start),
                Prop::Length => x.length.to_string(),
                Prop::Velocity => format!("{:.0}", x.velocity * 100.0),
                Prop::Pan => format!("{:.0}", x.pan * 100.0),
                Prop::Detune => format!("{:.2}", x.bend_at(0)),
            };
            self.entry = Some((EntryTarget::Prop(prop), ValueEntry::new(v, ev.position, window, cx)));
        } else {
            self.drag = Some(Drag::Scrub { g: None, prop, origin, from_y: ev.position.y.as_f32() });
        }
        cx.notify();
    }

    /// A value typed in the entry (Enter)
    fn apply_entry(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some((target, _)) = self.entry.take() else { return };
        let notes = self.notes(cx);
        let num = text.trim().replace(',', ".").parse::<f64>().ok();
        let ch: Vec<NoteChange> = match target {
            EntryTarget::Lane(t) => {
                let Some(v) = num else { return };
                let x = self.tick_to_x(t);
                let v = v / 100.0;
                self.lane_hits(&notes, x, x, false).into_iter().map(|n| (n, Some(self.with_lane(&notes[&n], v)))).collect()
            }
            EntryTarget::Prop(p) => {
                let parse_bbt = |s: &str| -> Option<i64> {
                    let v: Vec<i64> = s.split('.').map(|x| x.trim().parse().ok()).collect::<Option<Vec<i64>>>()?;
                    Some((v.first()? - 1) * BAR + (v.get(1).copied().unwrap_or(1) - 1) * BEAT + v.get(2).copied().unwrap_or(0))
                };
                self.selected(&notes)
                    .into_iter()
                    .filter_map(|(n, x)| {
                        let y = match p {
                            Prop::Key => Note { key: num? as u8, ..x },
                            Prop::Start => Note { start: parse_bbt(text)?, ..x },
                            Prop::Length => Note { length: num? as i64, ..x },
                            Prop::Velocity => Note { velocity: num? / 100.0, ..x },
                            Prop::Pan => Note { pan: num? / 100.0, ..x },
                            Prop::Detune => Note { bend: vec![notes::Bend { at: 0, semitones: num? }], ..x },
                        };
                        Some((n, Some(y)))
                    })
                    .collect()
            }
        };
        self.transact("Note property", ch, cx);
    }

    // --- the toolbar's menus ----------------------------------------------------------------------------------

    fn menu_items(&self, m: Menu, cx: &gpui::App) -> Vec<MenuItem> {
        let core = self.scx.core.read(cx);
        let music = &core.port().root().music;
        match m {
            Menu::Clip | Menu::Ghost => {
                let mut v = Vec::new();
                if m == Menu::Ghost {
                    v.push(MenuItem::new(tr("No ghost notes"), self.ghost.is_none()));
                }
                for c in &music.clips {
                    let on = if m == Menu::Clip { self.clip == Some(c.id) } else { self.ghost == Some(c.id) };
                    v.push(MenuItem::new(format!("{} · {}", c.track_name, c.name), on).color(c.color));
                }
                v
            }
            Menu::Grid => GRIDS.iter().enumerate().map(|(i, (l, _))| MenuItem::new(*l, i == self.grid)).collect(),
            Menu::Snap => vec![
                MenuItem::new(tr("Snap: the start or the end lands on the grid"), self.snap == SnapMode::Snap),
                MenuItem::new(tr("Nudge: moves by grid steps, keeps the offset"), self.snap == SnapMode::Nudge),
            ],
            Menu::Length => std::iter::once(MenuItem::new(tr("Last note"), self.length.is_none()))
                .chain(GRIDS.iter().enumerate().map(|(i, (l, _))| MenuItem::new(*l, self.length == Some(i))))
                .collect(),
            Menu::Root => KEY_NAMES.iter().enumerate().map(|(i, k)| MenuItem::new(*k, i as u8 == self.root)).collect(),
            Menu::Scale => std::iter::once(MenuItem::new(tr("No scale"), self.scale.is_none()))
                .chain(SCALES.iter().enumerate().map(|(i, (n, _))| MenuItem::new(tr(n), self.scale == Some(i))))
                .collect(),
            Menu::Chord => std::iter::once(MenuItem::new(tr("No chord"), self.chord.is_none()))
                .chain(CHORDS.iter().enumerate().map(|(i, (n, _))| MenuItem::new(tr(n), self.chord == Some(i))))
                .collect(),
            Menu::Quantize => vec![
                MenuItem::new(tr("Quantize starts and lengths"), false),
                MenuItem::new(tr("Quantize starts"), false),
                MenuItem::new(tr("Quantize lengths"), false),
                MenuItem::new(tr("Quantize starts, 50 %"), false),
            ],
            Menu::Tools => [
                "Glue (Shift+G)",
                "Fill (Shift+F)",
                "Cut overlaps (Shift+C)",
                "Min length as last",
                "Max length as last",
                "Reverse in time (Shift+R)",
                "Duplicate (Ctrl+D)",
                "Transpose an octave up",
                "Transpose an octave down",
                "Select all (Ctrl+A)",
                "Show note names",
            ]
            .iter()
            .enumerate()
            .map(|(i, l)| MenuItem::new(tr(l), i == 10 && self.show_names))
            .collect(),
            Menu::Record => vec![
                MenuItem::new(tr("Record from the keyboard"), self.rec == Rec::Live),
                MenuItem::new(tr("Step recording"), self.rec == Rec::Step),
                MenuItem::new(tr("Overdub"), self.rec_mode == RecMode::Overdub).header(tr("When recording")),
                MenuItem::new(tr("Replace"), self.rec_mode == RecMode::Replace),
                MenuItem::new(tr("No input quantize"), self.rec_quantize.is_none()).header(tr("Input quantize")),
                MenuItem::new("1/8", self.rec_quantize == Some(3)),
                MenuItem::new("1/16", self.rec_quantize == Some(4)),
                MenuItem::new("1/32", self.rec_quantize == Some(5)),
            ],
            Menu::Keys(k) => vec![
                MenuItem::new(trf("Mark {0}", &[&key_name(k)]), self.marked.contains(&k)),
                MenuItem::new(tr("Mark this note in every octave"), false),
                MenuItem::new(tr("Mark the current scale"), false),
                MenuItem::new(tr("Mark the current chord"), false),
                MenuItem::new(tr("Unmark all"), false),
                MenuItem::new(trf("Select every note on {0}", &[&key_name(k)]), false),
            ],
        }
    }

    fn pick(&mut self, m: Menu, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let notes = self.notes(cx);
        match m {
            Menu::Clip => {
                if let Some(id) = self.scx.core.read(cx).port().root().music.clips.get(i).map(|c| c.id) {
                    self.open_clip(id, cx);
                }
            }
            Menu::Ghost => {
                self.ghost = if i == 0 { None } else { self.scx.core.read(cx).port().root().music.clips.get(i - 1).map(|c| c.id) };
            }
            Menu::Grid => self.grid = i,
            Menu::Snap => self.snap = if i == 0 { SnapMode::Snap } else { SnapMode::Nudge },
            Menu::Length => self.length = i.checked_sub(1),
            Menu::Root => self.root = i as u8,
            Menu::Scale => self.scale = i.checked_sub(1),
            Menu::Chord => self.chord = i.checked_sub(1),
            Menu::Quantize => {
                let which = self.targets(&notes);
                let (what, s) = [(QuantizeWhat::Both, 1.0), (QuantizeWhat::Starts, 1.0), (QuantizeWhat::Lengths, 1.0), (QuantizeWhat::Starts, 0.5)][i];
                self.transact("Quantize", notes::quantize(&notes, &which, self.grid_ticks(), what, s), cx);
            }
            Menu::Tools => self.note_tool(i, cx),
            Menu::Record => match i {
                0 => self.rec = if self.rec == Rec::Live { Rec::Off } else { Rec::Live },
                1 => self.rec = if self.rec == Rec::Step { Rec::Off } else { Rec::Step },
                2 => self.rec_mode = RecMode::Overdub,
                3 => self.rec_mode = RecMode::Replace,
                4 => self.rec_quantize = None,
                k => self.rec_quantize = Some(k - 2),
            },
            Menu::Keys(k) => match i {
                0 => {
                    if !self.marked.remove(&k) {
                        self.marked.insert(k);
                    }
                }
                1 => {
                    let all: Vec<u8> = (0..128u8).filter(|x| x % 12 == k % 12).collect();
                    let on = !self.marked.contains(&k);
                    for x in all {
                        if on { self.marked.insert(x) } else { self.marked.remove(&x) };
                    }
                }
                2 => {
                    if let Some(s) = self.scale {
                        self.marked = (0..128u8).filter(|x| notes::in_scale(*x, self.root, s)).collect();
                    }
                }
                3 => self.marked = notes::chord_keys(k, self.chord.unwrap_or(0)).into_iter().collect(),
                4 => self.marked.clear(),
                _ => self.set_sel(notes.iter().filter(|(_, x)| x.key == k).map(|(n, _)| *n).collect()),
            },
        }
        let _ = window;
        cx.notify();
    }

    /// The note tools (F-NOT-011) and the rest of the Tools menu
    fn note_tool(&mut self, i: usize, cx: &mut Context<Self>) {
        let notes = self.notes(cx);
        let which = self.targets(&notes);
        let end = self.clip(cx).map_or(BAR, |c| c.loop_len);
        match i {
            0 => { self.transact("Glue", notes::glue(&notes, &which), cx); }
            1 => { self.transact("Fill", notes::fill(&notes, &which, end), cx); }
            2 => { self.transact("Cut overlaps", notes::cut_overlaps(&notes, &which), cx); }
            3 => { self.transact("Min length", notes::constrain_lengths(&notes, &which, self.last.length, true), cx); }
            4 => { self.transact("Max length", notes::constrain_lengths(&notes, &which, self.last.length, false), cx); }
            5 => { self.transact("Reverse", notes::reverse(&notes, &which), cx); }
            6 => self.duplicate(cx),
            7 => self.transpose(12, cx),
            8 => self.transpose(-12, cx),
            9 => self.set_sel(notes.keys().copied().collect()),
            _ => self.show_names = !self.show_names,
        }
    }

    fn transpose(&mut self, d: i32, cx: &mut Context<Self>) {
        let notes = self.notes(cx);
        let sel = self.selected(&notes);
        if sel.iter().any(|(_, x)| !(0..=127).contains(&(x.key as i32 + d))) {
            return; // every note stays in 0..127, or none moves
        }
        let ch = sel.into_iter().map(|(n, x)| (n, Some(Note { key: (x.key as i32 + d) as u8, ..x }))).collect();
        self.transact("Transpose", ch, cx);
    }

    fn shift(&mut self, dt: i64, cx: &mut Context<Self>) {
        let notes = self.notes(cx);
        let sel = self.selected(&notes);
        if sel.iter().any(|(_, x)| x.start + dt < 0) {
            return;
        }
        let ch = sel.into_iter().map(|(n, x)| (n, Some(Note { start: x.start + dt, ..x }))).collect();
        self.transact("Move notes", ch, cx);
    }

    fn duplicate(&mut self, cx: &mut Context<Self>) {
        let notes = self.notes(cx);
        let sel = self.selected(&notes);
        let (Some(a), Some(b)) = (sel.iter().map(|(_, x)| x.start).min(), sel.iter().map(|(_, x)| x.end()).max()) else { return };
        let g = self.grid_ticks();
        let span = ((b - a) + g - 1) / g * g;
        let ns = self.numbers(sel.len(), cx);
        let ch: Vec<NoteChange> = ns.iter().zip(sel.iter()).map(|(n, (_, x))| (*n, Some(Note { start: x.start + span, ..x.clone() }))).collect();
        let new = self.transact("Duplicate", ch, cx);
        self.set_sel(new.into_iter().collect());
    }

    fn copy(&mut self, cut: bool, cx: &mut Context<Self>) {
        let notes = self.notes(cx);
        let sel = self.selected(&notes);
        let a = sel.iter().map(|(_, x)| x.start).min().unwrap_or(0);
        self.clipboard = sel.iter().map(|(_, x)| Note { start: x.start - a, ..x.clone() }).collect();
        if cut {
            self.transact("Cut", sel.into_iter().map(|(n, _)| (n, None)).collect(), cx);
            self.set_sel(BTreeSet::new());
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        if self.clipboard.is_empty() {
            return;
        }
        // at the item's cursor, on the grid; what was pasted is the new selection
        let at = notes::snap_floor(self.play.at(Instant::now()) as f64, self.grid_ticks());
        let ns = self.numbers(self.clipboard.len(), cx);
        let ch = ns.iter().zip(self.clipboard.iter()).map(|(n, x)| (*n, Some(Note { start: x.start + at, ..x.clone() }))).collect();
        let new = self.transact("Paste", ch, cx);
        self.set_sel(new.into_iter().collect());
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        self.drag = None;
        self.scx.core.update(cx, |core, _| {
            let _ = core.port_mut().undo();
        });
        cx.notify();
    }

    // --- the keyboard ------------------------------------------------------------------------------------------

    fn key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = ev.keystroke.key.as_str();
        let m = ev.keystroke.modifiers;
        let p = primary(&m);
        let notes = self.notes(cx);
        // keys without modifiers play the instrument (F-NOT-023) and record when recording
        if !p && !m.alt && !m.shift && !m.control {
            if let Some(off) = piano_key(k) {
                if !ev.is_held {
                    let key = (self.octave * 12 + 12 + off).clamp(0, 127) as u8;
                    self.note_on(k, key, cx);
                }
                cx.stop_propagation();
                return;
            }
        }
        let handled = match (k, p, m.shift, m.alt) {
            ("z", true, _, _) => {
                self.undo(cx);
                true
            }
            ("a", true, false, _) => {
                self.set_sel(notes.keys().copied().collect());
                true
            }
            ("a", true, true, _) | ("escape", ..) => {
                self.set_sel(BTreeSet::new());
                self.menu = None;
                self.entry = None;
                if matches!(self.drag, Some(Drag::Knife { .. })) {
                    self.drag = None;
                }
                true
            }
            ("c", true, ..) => {
                self.copy(false, cx);
                true
            }
            ("x", true, ..) => {
                self.copy(true, cx);
                true
            }
            ("v", true, ..) => {
                self.paste(cx);
                true
            }
            ("d", true, ..) => {
                self.duplicate(cx);
                true
            }
            ("delete" | "backspace", ..) => {
                if self.rec == Rec::Step && self.sel.is_empty() {
                    // step recording: back one step
                    self.step_cursor = (self.step_cursor - self.new_len()).max(0);
                } else {
                    let sel = self.selected(&notes);
                    self.transact("Delete notes", sel.into_iter().map(|(n, _)| (n, None)).collect(), cx);
                    self.set_sel(BTreeSet::new());
                }
                true
            }
            ("up" | "down", _, _, _) => {
                let d = if k == "up" { 1 } else { -1 };
                if p {
                    self.transpose(12 * d, cx);
                } else if m.shift {
                    self.transpose(d, cx);
                } else {
                    self.scroll_y = (self.scroll_y - d as f32 * self.key_h * 2.0).max(0.0);
                }
                true
            }
            ("left" | "right", _, _, _) => {
                let d: i64 = if k == "right" { 1 } else { -1 };
                if self.rec == Rec::Step && !m.shift && !p {
                    self.step_cursor = (self.step_cursor + d * self.new_len()).max(0);
                } else if p {
                    self.shift(d * BAR, cx);
                } else if m.shift {
                    self.shift(d * if m.alt { 1 } else { self.grid_ticks() }, cx);
                } else {
                    self.scroll = (self.scroll + d as f64 * self.grid_ticks() as f64 * 4.0).max(0.0);
                }
                true
            }
            ("home", ..) => {
                self.play.cursor = 0;
                self.step_cursor = 0;
                self.scroll = 0.0;
                true
            }
            ("-", false, false, false) => {
                self.octave = (self.octave - 1).max(0);
                true
            }
            ("=", false, false, false) => {
                self.octave = (self.octave + 1).min(8);
                true
            }
            (_, true, false, false) if k.len() == 1 && k.as_bytes()[0].is_ascii_digit() => {
                // Ctrl+0..9: the note length (0 = last note)
                let d = (k.as_bytes()[0] - b'0') as usize;
                self.length = if d == 0 { None } else { Some((d - 1).min(GRIDS.len() - 1)) };
                true
            }
            (_, false, false, true) if k.len() == 1 && k.as_bytes()[0].is_ascii_digit() => {
                let d = (k.as_bytes()[0] - b'0') as usize;
                self.grid = d.saturating_sub(1).min(GRIDS.len() - 1);
                true
            }
            (_, false, true, false) => {
                // Shift+letter: tools and note tools
                match k {
                    "d" => self.tool = Tool::Draw,
                    "s" => self.tool = Tool::Select,
                    "e" => self.tool = Tool::Erase,
                    "t" => self.tool = Tool::Bend,
                    "k" => self.tool = Tool::Knife,
                    "j" => self.tool = Tool::Strum,
                    "g" => self.note_tool(0, cx),
                    "f" => self.note_tool(1, cx),
                    "c" => self.note_tool(2, cx),
                    "r" => self.note_tool(5, cx),
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
        let _ = window;
    }

    fn key_up(&mut self, ev: &KeyUpEvent, cx: &mut Context<Self>) {
        let k = ev.keystroke.key.clone();
        if let Some(h) = self.held.remove(&k) {
            self.note_off(h, cx);
        }
    }

    fn rec_tick(&self, cx: &gpui::App) -> Option<i64> {
        let (t, playing) = self.play_tick(cx, Instant::now());
        let t = t.filter(|_| playing)?;
        Some(match self.rec_quantize {
            Some(g) => notes::snap(t, GRIDS[g].1),
            None => t.round() as i64,
        })
    }

    fn note_on(&mut self, k: &str, key: u8, cx: &mut Context<Self>) {
        self.pressed.insert(key);
        let vel = self.last.velocity;
        match self.rec {
            Rec::Step => {
                // the note lands at the step cursor; keys held together make a chord
                let n = self.numbers(1, cx)[0];
                let x = Note { key, start: self.step_cursor, length: self.new_len(), velocity: vel, pan: self.last.pan, bend: Vec::new() };
                let new = self.transact("Step record", vec![(n, Some(x))], cx);
                self.step_chord.extend(new);
                self.held.insert(k.to_string(), (key, None, self.step_cursor));
            }
            Rec::Live => {
                let at = self.rec_tick(cx);
                self.held.insert(k.to_string(), (key, None, at.unwrap_or(-1)));
            }
            Rec::Off => {
                self.held.insert(k.to_string(), (key, None, -1));
            }
        }
        cx.notify();
    }

    fn note_off(&mut self, (key, _, start): (u8, Option<u64>, i64), cx: &mut Context<Self>) {
        self.pressed.remove(&key);
        match self.rec {
            Rec::Step => {
                if self.held.is_empty() {
                    // every key released: the cursor goes one step on
                    self.step_cursor += self.new_len();
                    self.step_chord.clear();
                }
            }
            Rec::Live if start >= 0 => {
                let end = self.rec_tick(cx).unwrap_or(start + BEAT / 4);
                let len = if end > start { end - start } else { BEAT / 8 };
                let notes = self.notes(cx);
                let n = self.numbers(1, cx)[0];
                let mut ch: Vec<NoteChange> = Vec::new();
                if self.rec_mode == RecMode::Replace {
                    // replace: the notes of this key under the new one go
                    ch.extend(notes.iter().filter(|(_, x)| x.key == key && x.start < start + len && x.end() > start).map(|(m, _)| (*m, None)));
                }
                ch.push((n, Some(Note { key, start, length: len, velocity: self.last.velocity, pan: self.last.pan, bend: Vec::new() })));
                self.transact("Record notes", ch, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    fn release_all(&mut self, cx: &mut Context<Self>) {
        let held: Vec<(u8, Option<u64>, i64)> = self.held.drain().map(|(_, v)| v).collect();
        for h in held {
            self.note_off(h, cx);
        }
    }

    // --- scrolling and zoom -----------------------------------------------------------------------------------

    fn on_scroll(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let (dx, dy) = (d.x.as_f32(), d.y.as_f32());
        let (x, y) = self.local(&self.grid_bounds, ev.position);
        let m = ev.modifiers;
        if primary(&m) && m.alt {
            // Ctrl+Alt+wheel: the quantization; Ctrl+Shift+wheel: the note length
            self.grid = (self.grid as i32 - dy.signum() as i32).clamp(0, GRIDS.len() as i32 - 1) as usize;
        } else if primary(&m) && m.shift {
            let l = self.length.map_or(0, |i| i + 1) as i32 - dy.signum() as i32;
            self.length = (l.clamp(0, GRIDS.len() as i32) as usize).checked_sub(1);
        } else if primary(&m) {
            self.zoom_x((-dy * 0.01).exp() as f64, x);
        } else if m.alt {
            let anchor = (y + self.scroll_y) / self.key_h;
            self.key_h = (self.key_h * (dy * 0.01).exp()).clamp(4.0, 28.0);
            self.scroll_y = (anchor * self.key_h - y).max(0.0);
        } else {
            let (dx, dy) = if m.shift && dx.abs() < 0.01 { (dy, 0.0) } else { (dx, dy) };
            self.scroll = (self.scroll - dx as f64 * self.tpp).max(0.0);
            let max = (128.0 * self.key_h - self.grid_bounds.get().size.height.as_f32()).max(0.0);
            self.scroll_y = (self.scroll_y - dy).clamp(0.0, max);
        }
        cx.notify();
    }

    fn zoom_x(&mut self, factor: f64, x: f32) {
        let anchor = self.x_to_tick(x);
        self.tpp = (self.tpp * factor).clamp(0.25, 400.0);
        self.scroll = (anchor - x as f64 * self.tpp).max(0.0);
    }

    fn on_pinch(&mut self, ev: &PinchEvent, cx: &mut Context<Self>) {
        let (x, _) = self.local(&self.grid_bounds, ev.position);
        self.zoom_x(1.0 / (1.0 + ev.delta as f64).max(0.1), x);
        cx.notify();
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Rendering

/// What the grid's canvas paints, captured once per frame (no clone of the notes: the maps are shared)
struct GridPaint {
    notes: Arc<NoteMap>,
    ghost: Option<Arc<NoteMap>>,
    sel: Rc<BTreeSet<u64>>,
    color: u32,
    scroll: f64,
    tpp: f64,
    key_h: f32,
    scroll_y: f32,
    grid: i64,
    loop_len: i64,
    length: i64,
    scale: Option<(u8, usize)>,
    marked: BTreeSet<u8>,
    hover_key: Option<i32>,
    play: Option<f64>,
    item_loop: Option<(i64, i64)>,
    step_cursor: Option<(i64, i64)>,
    rect: Option<(Point<Pixels>, Point<Pixels>)>,
    knife: Option<(Point<Pixels>, Point<Pixels>)>,
    names: bool,
}

impl GridPaint {
    fn x(&self, t: f64) -> f32 {
        ((t - self.scroll) / self.tpp) as f32
    }
    fn y(&self, k: i32) -> f32 {
        (127 - k) as f32 * self.key_h - self.scroll_y
    }

    fn paint(&self, b: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        let o = b.origin;
        let (w, h) = (b.size.width.as_f32(), b.size.height.as_f32());
        let r = |x: f32, y: f32, ww: f32, hh: f32| Bounds::new(point(o.x + px(x), o.y + px(y)), size(px(ww), px(hh)));
        window.paint_quad(fill(b, c(THEME.bg1)));
        // the rows: black keys darker, the scale lit, the marked semitones
        let k_top = self.scroll_y / self.key_h;
        let first = (127 - k_top.floor() as i32).min(127);
        let last = (127 - ((self.scroll_y + h) / self.key_h).ceil() as i32).max(0);
        for k in (last..=first).rev() {
            let y = self.y(k);
            let ku = k as u8;
            if notes::is_black(ku) {
                window.paint_quad(fill(r(0.0, y, w, self.key_h), with_alpha(THEME.window, 0.45)));
            }
            if let Some((root, s)) = self.scale {
                if notes::in_scale(ku, root, s) {
                    window.paint_quad(fill(r(0.0, y, w, self.key_h), with_alpha(THEME.green, if ku % 12 == root { 0.10 } else { 0.05 })));
                }
            }
            if self.marked.contains(&ku) {
                window.paint_quad(fill(r(0.0, y, w, self.key_h), with_alpha(THEME.yellow, 0.12)));
            }
            if self.hover_key == Some(k) {
                window.paint_quad(fill(r(0.0, y, w, self.key_h), with_alpha(THEME.text, 0.04)));
            }
            if ku % 12 == 0 {
                window.paint_quad(fill(r(0.0, y + self.key_h - 1.0, w, 1.0), c(THEME.bg3)));
            }
        }
        // the grid: bars, beats, the snapping grid when it is wide enough
        let t0 = self.scroll;
        let t1 = self.scroll + w as f64 * self.tpp;
        let mut lines = |step: i64, col: u32| {
            if (step as f64 / self.tpp) < 6.0 {
                return;
            }
            let mut t = (t0 / step as f64).floor() as i64 * step;
            while (t as f64) <= t1 {
                let x = self.x(t as f64);
                window.paint_quad(fill(r(x, 0.0, 1.0, h), c(col)));
                t += step;
            }
        };
        lines(self.grid, THEME.bg2);
        lines(BEAT, THEME.bg3);
        lines(BAR, THEME.line2);
        // the repeats (dimmed) and what is past the item's end
        let lx = self.x(self.loop_len as f64);
        if self.loop_len < self.length && lx < w {
            window.paint_quad(fill(r(lx.max(0.0), 0.0, w - lx.max(0.0), h), with_alpha(THEME.window, 0.25)));
        }
        let ex = self.x(self.length as f64);
        if ex < w {
            window.paint_quad(fill(r(ex.max(0.0), 0.0, w - ex.max(0.0), h), with_alpha(THEME.window, 0.55)));
        }
        if let Some((a, z)) = self.item_loop {
            let (xa, xz) = (self.x(a as f64), self.x(z as f64));
            window.paint_quad(fill(r(xa, 0.0, (xz - xa).max(0.0), h), gpui::rgba(THEME.loop_range)));
        }
        if let Some((s, l)) = self.step_cursor {
            let (xa, xz) = (self.x(s as f64), self.x((s + l) as f64));
            window.paint_quad(fill(r(xa, 0.0, (xz - xa).max(1.0), h), gpui::rgba(THEME.punch_range)));
        }
        let visible = |x: &Note, off: i64| {
            let s = (x.start + off) as f64;
            s < t1 && s + x.length as f64 > t0 && (x.key as i32) >= last && (x.key as i32) <= first
        };
        // ghost notes, under everything
        if let Some(g) = &self.ghost {
            for x in g.values().filter(|x| visible(x, 0)) {
                let (nx, ny) = (self.x(x.start as f64), self.y(x.key as i32));
                let nw = (x.length as f64 / self.tpp) as f32;
                window.paint_quad(gpui::quad(r(nx, ny + 1.0, nw.max(2.0), self.key_h - 2.0), px(2.0), with_alpha(THEME.text3, 0.18), px(1.0), with_alpha(THEME.text3, 0.5), Default::default()));
            }
        }
        // the repeats of the pattern, dimmed and not editable
        if self.loop_len < self.length {
            let first_rep = ((t0 / self.loop_len as f64).floor() as i64).max(1);
            let last_rep = ((t1.min(self.length as f64) / self.loop_len as f64).ceil() as i64).max(1);
            for rep in first_rep..last_rep {
                let off = rep * self.loop_len;
                for x in self.notes.values().filter(|x| visible(x, off) && x.start < self.loop_len) {
                    let (nx, ny) = (self.x((x.start + off) as f64), self.y(x.key as i32));
                    let nw = (x.length as f64 / self.tpp) as f32;
                    window.paint_quad(fill(r(nx, ny + 1.0, nw.max(2.0), self.key_h - 2.0), with_alpha(self.color, 0.22)));
                }
            }
        }
        // the notes
        let font_size = (self.key_h - 3.0).clamp(7.0, 10.0);
        for (n, x) in self.notes.iter().filter(|(_, x)| visible(x, 0)) {
            let selected = self.sel.contains(n);
            let (nx, ny) = (self.x(x.start as f64), self.y(x.key as i32));
            let nw = ((x.length as f64 / self.tpp) as f32).max(2.0);
            let body = with_alpha(self.color, 0.35 + 0.65 * x.velocity as f32);
            let border = if selected { c(THEME.text) } else { with_alpha(THEME.line, 0.9) };
            let bb = r(nx, ny + 0.5, nw, self.key_h - 1.0);
            window.paint_quad(gpui::quad(bb, px(2.0), if selected { with_alpha(THEME.text, 0.0) } else { body }, px(if selected { 1.5 } else { 1.0 }), border, Default::default()));
            if selected {
                window.paint_quad(fill(r(nx + 1.5, ny + 2.0, (nw - 3.0).max(1.0), self.key_h - 4.0), body));
            }
            // the pitch bend over the note
            if !x.bend.is_empty() && x.bend.iter().any(|p| p.semitones != 0.0) {
                let mut pb = PathBuilder::stroke(px(1.5));
                let mid = ny + self.key_h / 2.0;
                let pts: Vec<(f32, f32)> = if x.bend.len() == 1 {
                    vec![(0.0, x.bend[0].semitones as f32), (x.length as f32, x.bend[0].semitones as f32)]
                } else {
                    let mut v: Vec<(f32, f32)> = x.bend.iter().map(|p| (p.at as f32, p.semitones as f32)).collect();
                    v.push((x.length as f32, x.bend.last().unwrap().semitones as f32));
                    v
                };
                for (i, (at, st)) in pts.iter().enumerate() {
                    let p = point(o.x + px(nx + at / self.tpp as f32), o.y + px(mid - st * self.key_h));
                    if i == 0 { pb.move_to(p) } else { pb.line_to(p) }
                }
                if let Ok(p) = pb.build() {
                    window.paint_path(p, c(THEME.yellow));
                }
            }
            if self.names && nw > 22.0 && self.key_h >= 9.0 {
                paint_text(window, cx, key_name(x.key).into(), o.x + px(nx + 3.0), o.y + px(ny + (self.key_h - font_size) / 2.0 - 1.0), font_size, c(THEME.on_accent));
            }
        }
        if let Some((a, z)) = self.rect {
            let bb = Bounds::new(o + point(a.x.min(z.x), a.y.min(z.y)), size((a.x - z.x).abs(), (a.y - z.y).abs()));
            window.paint_quad(gpui::quad(bb, px(0.0), gpui::rgba(THEME.selection), px(1.0), c(THEME.blue), Default::default()));
        }
        if let Some((a, z)) = self.knife {
            let mut pb = PathBuilder::stroke(px(1.5));
            pb.move_to(o + a);
            pb.line_to(o + z + point(px(0.01), px(0.01)));
            if let Ok(p) = pb.build() {
                window.paint_path(p, c(THEME.red));
            }
        }
        if let Some(t) = self.play {
            let x = self.x(t);
            if x >= 0.0 && x < w {
                window.paint_quad(fill(r(x, 0.0, 1.0, h), c(THEME.cursor)));
            }
        }
    }
}

impl Render for PianoRoll {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        // I-3.6: the feed tells when the item changed under us (an undo): selected numbers that are gone are dropped
        {
            let core = self.scx.core.read(cx);
            if let Some(diff) = core.port().drain(&mut self.feed) {
                if let Some(id) = self.clip {
                    if diff.touches(id) {
                        if let Some(c) = core.port().root().music.clip(id) {
                            if self.sel.iter().any(|n| !c.notes.contains_key(n)) {
                                let keep: BTreeSet<u64> = self.sel.iter().copied().filter(|n| c.notes.contains_key(n)).collect();
                                self.sel = Rc::new(keep);
                            }
                        }
                    }
                }
            }
        }
        if self.clip.is_some_and(|id| self.scx.core.read(cx).port().root().music.clip(id).is_none()) {
            self.clip = None;
        }
        if !self.fitted && self.clip.is_some() {
            let gb = self.grid_bounds.get();
            if gb.size.width.as_f32() > 10.0 {
                self.fit(gb.size.width.as_f32(), gb.size.height.as_f32(), cx);
            }
        }
        let (play, moving) = self.play_tick(cx, now);
        if moving {
            window.request_animation_frame();
        }
        let entity = cx.entity();
        div()
            .id("piano-roll")
            .size_full()
            .flex()
            .flex_col()
            .bg(c(THEME.bg1))
            .text_color(c(THEME.text))
            .track_focus(&self.focus)
            .key_context("PianoRoll")
            .on_action(cx.listener(|pr, _: &PlayItem, _, cx| pr.toggle_play_item(cx)))
            .on_key_down(cx.listener(|pr, ev: &KeyDownEvent, window, cx| pr.key_down(ev, window, cx)))
            .on_key_up(cx.listener(|pr, ev: &KeyUpEvent, _, cx| pr.key_up(ev, cx)))
            .child(self.render_toolbar(cx))
            .child(match self.clip(cx) {
                None => div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(c(THEME.text2))
                    .child(tr("No MIDI item: pick one in the list above"))
                    .into_any_element(),
                Some(_) => div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .child(self.render_keys(cx))
                    .child(self.render_center(play, cx))
                    .child(self.render_inspector(cx))
                    .into_any_element(),
            })
            .children(self.drag.is_some().then(|| kit::capture(entity.downgrade(), Self::drag_move, Self::drag_up)))
            .children(self.menu.map(|(m, at)| {
                let items = self.menu_items(m, cx);
                let e = entity.downgrade();
                let e2 = entity.downgrade();
                popup(
                    "pr-menu",
                    at,
                    items,
                    move |i, w, cx| {
                        let _ = e.update(cx, |pr, cx| pr.pick(m, i, w, cx));
                    },
                    move |_, cx| {
                        let _ = e2.update(cx, |pr, cx| {
                            pr.menu = None;
                            cx.notify();
                        });
                    },
                )
            }))
            .children(self.entry.as_ref().map(|(_, e)| {
                let ent = entity.downgrade();
                let ent2 = entity.downgrade();
                e.render(
                    move |text, _, cx| {
                        let _ = ent.update(cx, |pr, cx| pr.apply_entry(&text, cx));
                    },
                    move |_, cx| {
                        let _ = ent2.update(cx, |pr, cx| {
                            pr.entry = None;
                            cx.notify();
                        });
                    },
                )
            }))
    }
}

impl PianoRoll {
    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let clip = self.clip(cx);
        let ghost = self.ghost.and_then(|g| self.scx.core.read(cx).port().root().music.clip(g)).map(|c| c.name.clone());
        let open = |m: Menu| cx.listener(move |pr: &mut PianoRoll, ev: &MouseDownEvent, _, cx| {
            pr.menu = if pr.menu.is_some_and(|x| x.0 == m) { None } else { Some((m, ev.position + point(px(0.), px(10.)))) };
            cx.notify();
        });
        let rec_on = self.rec != Rec::Off;
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
                dropdown("pr-clip", clip.map_or_else(|| tr("No MIDI item").to_string(), |c| format!("{} · {}", c.track_name, c.name)), true)
                    .debug_selector(|| "pr-clip".into())
                    .min_w(px(150.))
                    .child(div().size(px(8.)).rounded(px(2.)).bg(c(clip.map_or(THEME.text3, |c| c.color))))
                    .on_mouse_down(MouseButton::Left, open(Menu::Clip)),
            )
            .child(sep())
            .children(TOOLS.iter().map(|(t, name, help)| {
                let t = *t;
                let help: &'static str = help;
                button(SharedString::from(format!("pr-tool-{name}")), tr(name), self.tool == t, THEME.green)
                    .h(px(22.))
                    .px(px(6.))
                    .tooltip(tip(help))
                    .on_click(cx.listener(move |pr, _, _, cx| {
                        pr.tool = t;
                        cx.notify();
                    }))
            }))
            .child(sep())
            .child(label(tr("Grid")))
            .child(dropdown("pr-grid-size", GRIDS[self.grid].0, false).tooltip(tip("Quantization (Alt+1..9, Ctrl+Alt+wheel)")).on_mouse_down(MouseButton::Left, open(Menu::Grid)))
            .child(dropdown("pr-snap", tr(if self.snap == SnapMode::Snap { "Snap" } else { "Nudge" }), false).on_mouse_down(MouseButton::Left, open(Menu::Snap)))
            .child(label(tr("Length")))
            .child(
                dropdown("pr-len", self.length.map_or_else(|| trf("Last ({0})", &[&format_len(self.last.length)]), |i| GRIDS[i].0.to_string()), false)
                    .tooltip(tip("Length of a new note (Ctrl+0..9, Ctrl+Shift+wheel)"))
                    .on_mouse_down(MouseButton::Left, open(Menu::Length)),
            )
            .child(sep())
            .child(label(tr("Key")))
            .child(dropdown("pr-root", KEY_NAMES[self.root as usize], false).on_mouse_down(MouseButton::Left, open(Menu::Root)))
            .child(dropdown("pr-scale", self.scale.map_or(tr("No scale"), |s| tr(SCALES[s].0)), self.scale.is_some()).on_mouse_down(MouseButton::Left, open(Menu::Scale)))
            .child(
                dropdown("pr-chord", self.chord.map_or(tr("No chord"), |s| tr(CHORDS[s].0)), self.chord.is_some())
                    .tooltip(tip("A click draws the whole chord; Shift+click draws it as an arpeggio"))
                    .on_mouse_down(MouseButton::Left, open(Menu::Chord)),
            )
            .child(sep())
            .child(dropdown("pr-quantize", tr("Quantize"), false).on_mouse_down(MouseButton::Left, open(Menu::Quantize)))
            .child(dropdown("pr-tools", tr("Tools"), false).on_mouse_down(MouseButton::Left, open(Menu::Tools)))
            .child(
                dropdown("pr-ghost", ghost.map_or_else(|| tr("Ghost").to_string(), |g| trf("Ghost: {0}", &[&g])), self.ghost.is_some())
                    .tooltip(tip("Show another item's notes under these"))
                    .on_mouse_down(MouseButton::Left, open(Menu::Ghost)),
            )
            .child(div().flex_1())
            .child(
                button("pr-play", if self.play.since.is_some() { "■" } else { "▶" }, self.play.since.is_some(), THEME.green)
                    .h(px(22.))
                    .min_w(px(26.))
                    .tooltip(tip("Play this item in its own loop (Space)"))
                    .on_click(cx.listener(|pr, _, _, cx| pr.toggle_play_item(cx))),
            )
            .child(
                button("pr-loop", tr("Loop"), self.play.loop_on, THEME.green)
                    .h(px(22.))
                    .on_click(cx.listener(|pr, _, _, cx| {
                        pr.play.loop_on = !pr.play.loop_on;
                        cx.notify();
                    })),
            )
            .child(
                dropdown(
                    "pr-rec",
                    match self.rec {
                        Rec::Off => tr("Record").to_string(),
                        Rec::Live => trf("● Recording · octave {0}", &[&self.octave]),
                        Rec::Step => trf("● Step · {0}", &[&format_bbt(self.step_cursor)]),
                    },
                    rec_on,
                )
                .when(rec_on, |d| d.text_color(c(THEME.red)))
                .tooltip(tip("The computer keyboard plays (Z..M, Q..U; - and = change the octave)"))
                .on_mouse_down(MouseButton::Left, open(Menu::Record)),
            )
    }

    fn render_keys(&self, cx: &Context<Self>) -> impl IntoElement {
        let (key_h, scroll_y) = (self.key_h, self.scroll_y);
        let pressed = self.pressed.clone();
        let scale = self.scale.map(|s| (self.root, s));
        let bounds = self.keys_bounds.clone();
        let lane = self.lane;
        div()
            .w(px(KEYS_W))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(c(THEME.line))
            .child(div().h(px(RULER_H)).bg(c(THEME.bg3)))
            .child(
                div()
                    .id("pr-keys")
                    .debug_selector(|| "pr-keys".into())
                    .flex_1()
                    .overflow_hidden()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::keys_down))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::keys_down))
                    .on_scroll(cx.listener(|pr, ev: &ScrollEvent, _, cx| pr.on_scroll(ev, cx)))
                    .child(
                        canvas(
                            move |b, _, _| bounds.set(b),
                            move |b, _, window, cx| {
                                let o = b.origin;
                                let (w, h) = (b.size.width.as_f32(), b.size.height.as_f32());
                                window.paint_quad(fill(b, c(THEME.bg4)));
                                let first = (127 - (scroll_y / key_h).floor() as i32).min(127);
                                let last = (127 - ((scroll_y + h) / key_h).ceil() as i32).max(0);
                                for k in (last..=first).rev() {
                                    let y = (127 - k) as f32 * key_h - scroll_y;
                                    let ku = k as u8;
                                    let black = notes::is_black(ku);
                                    let mut col = if black { c(THEME.window) } else { c(THEME.text) };
                                    if pressed.contains(&ku) {
                                        col = c(THEME.green);
                                    }
                                    let kw = if black { w * 0.62 } else { w };
                                    window.paint_quad(fill(Bounds::new(point(o.x, o.y + px(y + 0.5)), size(px(kw), px(key_h - 1.0))), col));
                                    if let Some((root, s)) = scale {
                                        if notes::in_scale(ku, root, s) {
                                            window.paint_quad(fill(Bounds::new(point(o.x + px(w - 4.0), o.y + px(y + 1.0)), size(px(3.0), px(key_h - 2.0))), c(THEME.green)));
                                        }
                                    }
                                    if ku % 12 == 0 && key_h >= 7.0 {
                                        let fs = key_h.clamp(7.0, 10.0);
                                        paint_text(window, cx, key_name(ku).into(), o.x + px(w - 26.0), o.y + px(y + (key_h - fs) / 2.0 - 1.0), fs, c(THEME.bg1));
                                    }
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
            // the lane's name and switch, beside the lane
            .child(
                div()
                    .h(px(LANE_H))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .p(px(4.))
                    .bg(c(THEME.bg2))
                    .border_t_1()
                    .border_color(c(THEME.line))
                    .child(button("pr-lane-vel", tr("Vel"), lane == LaneMode::Velocity, THEME.blue).on_click(cx.listener(|pr, _, _, cx| {
                        pr.lane = LaneMode::Velocity;
                        cx.notify();
                    })))
                    .child(button("pr-lane-pan", tr("Pan"), lane == LaneMode::Pan, THEME.blue).on_click(cx.listener(|pr, _, _, cx| {
                        pr.lane = LaneMode::Pan;
                        cx.notify();
                    }))),
            )
    }

    fn render_center(&self, play: Option<f64>, cx: &Context<Self>) -> impl IntoElement {
        let clip = self.clip(cx).cloned();
        let Some(clip) = clip else { return div() };
        let ghost = self.ghost.and_then(|g| self.scx.core.read(cx).port().root().music.clip(g)).map(|c| c.notes.clone());
        let hover_key = self.hover.map(|h| h.1);
        let grid_paint = GridPaint {
            notes: clip.notes.clone(),
            ghost,
            sel: self.sel.clone(),
            color: clip.color,
            scroll: self.scroll,
            tpp: self.tpp,
            key_h: self.key_h,
            scroll_y: self.scroll_y,
            grid: self.grid_ticks(),
            loop_len: clip.loop_len,
            length: clip.length,
            scale: self.scale.map(|s| (self.root, s)),
            marked: self.marked.clone(),
            hover_key,
            play,
            item_loop: self.play.loop_on.then_some((self.play.loop_a, self.play.loop_b)),
            step_cursor: (self.rec == Rec::Step).then_some((self.step_cursor, self.new_len())),
            rect: match &self.drag {
                Some(Drag::Rect { from, to, .. }) => Some((*from, *to)),
                _ => None,
            },
            knife: match &self.drag {
                Some(Drag::Knife { from, to }) => Some((*from, *to)),
                _ => None,
            },
            names: self.show_names,
        };
        // the cursor tells what a click does
        let cursor = match (self.tool, self.hover.and_then(|h| self.note_at(&clip.notes, h.0, h.1).map(|x| x.1))) {
            (Tool::Draw | Tool::Select, Some(p)) if p != 0 => CursorStyle::ResizeLeftRight,
            (Tool::Draw | Tool::Select, Some(_)) => CursorStyle::OpenHand,
            (Tool::Erase, _) => CursorStyle::OperationNotAllowed,
            (Tool::Knife, _) => CursorStyle::Crosshair,
            (Tool::Bend, Some(_)) => CursorStyle::ResizeUpDown,
            _ => CursorStyle::Arrow,
        };
        let gb = self.grid_bounds.clone();
        let rb = self.ruler_bounds.clone();
        let lb = self.lane_bounds.clone();
        let (scroll, tpp) = (self.scroll, self.tpp);
        let (length, loop_len, repeats) = (clip.length, clip.loop_len, clip.repeats());
        let item_loop = (self.play.loop_on, self.play.loop_a, self.play.loop_b);
        let item_cursor = self.play.at(Instant::now());
        let lane_notes = clip.notes.clone();
        let lane_sel = self.sel.clone();
        let lane_mode = self.lane;
        let color = clip.color;
        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            // the ruler: bars, the item's end, the repeating part's end, the item's own loop and cursor (P-26)
            .child(
                div()
                    .id("pr-ruler")
                    .debug_selector(|| "pr-ruler".into())
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
                                if item_loop.0 {
                                    let (a, z) = (x(item_loop.1 as f64), x(item_loop.2 as f64));
                                    window.paint_quad(fill(Bounds::new(point(o.x + px(a), o.y + px(12.0)), size(px((z - a).max(0.0)), px(6.0))), c(THEME.green)));
                                }
                                let step = BAR * ((60.0 * tpp / BAR as f64).ceil() as i64).max(1);
                                let mut t = (scroll / step as f64).floor() as i64 * step;
                                while x(t as f64) < w {
                                    let xx = x(t as f64);
                                    window.paint_quad(fill(Bounds::new(point(o.x + px(xx), o.y + px(4.0)), size(px(1.0), px(16.0))), c(THEME.text3)));
                                    paint_text(window, cx, format!("{}", t / BAR + 1).into(), o.x + px(xx + 3.0), o.y + px(1.0), 10.0, c(THEME.text2));
                                    t += step;
                                }
                                let marker = |window: &mut Window, t: i64, col: u32| {
                                    let xx = x(t as f64);
                                    window.paint_quad(fill(Bounds::new(point(o.x + px(xx - 2.0), o.y), size(px(4.0), px(RULER_H))), c(col)));
                                };
                                if repeats {
                                    marker(window, loop_len, THEME.orange);
                                }
                                marker(window, length, THEME.red);
                                let cx_ = x(item_cursor as f64);
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
                    .id("pr-grid")
                    .debug_selector(|| "pr-grid".into())
                    .flex_1()
                    .overflow_hidden()
                    .cursor(cursor)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::grid_down))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::grid_down))
                    .on_mouse_move(cx.listener(|pr, ev: &MouseMoveEvent, _, cx| pr.grid_hover(ev, cx)))
                    .on_scroll(cx.listener(|pr, ev: &ScrollEvent, _, cx| pr.on_scroll(ev, cx)))
                    .on_pinch(cx.listener(|pr, ev: &PinchEvent, _, cx| pr.on_pinch(ev, cx)))
                    .child(canvas(move |b, _, _| gb.set(b), move |b, _, window, cx| grid_paint.paint(b, window, cx)).size_full()),
            )
            // the velocity or pan lane (F-NOT-010)
            .child(
                div()
                    .id("pr-lane")
                    .debug_selector(|| "pr-lane".into())
                    .h(px(LANE_H))
                    .flex_none()
                    .border_t_1()
                    .border_color(c(THEME.line))
                    .cursor(CursorStyle::ResizeUpDown)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::lane_down))
                    .on_mouse_down(MouseButton::Middle, cx.listener(Self::lane_down))
                    .on_scroll(cx.listener(|pr, ev: &ScrollEvent, _, cx| pr.lane_wheel(ev, cx)))
                    .child(
                        canvas(
                            move |b, _, _| lb.set(b),
                            move |b, _, window, _| {
                                let o = b.origin;
                                let (w, h) = (b.size.width.as_f32(), b.size.height.as_f32() - 6.0);
                                window.paint_quad(fill(b, c(THEME.bg2)));
                                let x = |t: f64| ((t - scroll) / tpp) as f32;
                                let base = if lane_mode == LaneMode::Pan { 3.0 + h / 2.0 } else { 3.0 + h };
                                window.paint_quad(fill(Bounds::new(point(o.x, o.y + px(base)), size(px(w), px(1.0))), c(THEME.bg4)));
                                for (n, nt) in lane_notes.iter() {
                                    let xx = x(nt.start as f64);
                                    if xx < -4.0 || xx > w + 4.0 {
                                        continue;
                                    }
                                    let v = if lane_mode == LaneMode::Velocity { nt.velocity as f32 } else { nt.pan as f32 * 0.5 + 0.5 };
                                    let top = 3.0 + (1.0 - v) * h;
                                    let selected = lane_sel.contains(n);
                                    let col = if selected { c(THEME.text) } else { c(color) };
                                    let (y0, y1) = (top.min(base), top.max(base));
                                    window.paint_quad(fill(Bounds::new(point(o.x + px(xx), o.y + px(y0)), size(px(2.0), px((y1 - y0).max(1.0)))), col));
                                    window.paint_quad(gpui::quad(Bounds::new(point(o.x + px(xx - 2.5), o.y + px(top - 2.5)), size(px(7.0), px(5.0))), px(2.0), col, px(0.0), col, Default::default()));
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
    }

    fn render_inspector(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(clip) = self.clip(cx) else { return div() };
        let sel = self.selected(&clip.notes);
        let row = |prop: Prop, name: &'static str, value: String| {
            div()
                .id(SharedString::from(format!("pr-prop-{name}")))
                .debug_selector(move || format!("pr-prop-{name}"))
                .flex()
                .flex_row()
                .items_center()
                .h(px(20.))
                .px(px(6.))
                .rounded(px(3.))
                .bg(c(THEME.bg1))
                .cursor(CursorStyle::ResizeUpDown)
                .hover(|s| s.bg(c(THEME.bg3)))
                .on_mouse_down(MouseButton::Left, cx.listener(move |pr, ev: &MouseDownEvent, window, cx| pr.scrub_down(prop, ev, window, cx)))
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
            .overflow_hidden();
        if sel.is_empty() {
            col = col
                .child(caption(tr("ITEM")))
                .child(div().text_size(px(12.)).font_weight(gpui::FontWeight::BOLD).child(clip.name.clone()))
                .child(div().text_size(px(11.)).text_color(c(THEME.text2)).child(trf("{0} notes · track {1}", &[&clip.notes.len(), &clip.track_name])))
                .child(div().text_size(px(11.)).text_color(c(THEME.text2)).child(trf("Length {0} bars", &[&(clip.length as f64 / BAR as f64)])))
                .when(clip.repeats(), |d| {
                    d.child(div().text_size(px(11.)).text_color(c(THEME.orange)).child(trf("Repeats every {0} bars", &[&(clip.loop_len as f64 / BAR as f64)])))
                })
                .child(div().h(px(6.)))
                .child(div().text_size(px(10.5)).text_color(c(THEME.text3)).child(tr(match self.tool {
                    Tool::Draw => "Click draws, drag sets the length; drag a note to move it, its edge to resize; Shift at the start of a drag copies; Alt ignores the grid; right button erases.",
                    Tool::Select => "Drag a rectangle; Shift toggles; Ctrl/Cmd+A selects all, Esc none.",
                    Tool::Erase => "Click or drag over notes to erase them.",
                    Tool::Bend => "Drag on a note to draw its pitch bend; right button keeps only the first point.",
                    Tool::Knife => "Drag a line, diagonal or not, across the notes to cut them; Shift on release drops the short ends; Esc cancels.",
                    Tool::Strum => "Drag a chord sideways: its starts spread in a staircase.",
                })));
        } else {
            let first = &sel[0].1;
            let all = |f: &dyn Fn(&Note) -> String| {
                let v = f(first);
                if sel.iter().all(|(_, x)| f(x) == v) { v } else { tr("mixed").to_string() }
            };
            col = col
                .child(caption(trf("{0} NOTES SELECTED", &[&sel.len()])))
                .child(row(Prop::Key, PROPS[0].1, all(&|x| key_name(x.key))))
                .child(row(Prop::Start, PROPS[1].1, all(&|x| format_bbt(x.start))))
                .child(row(Prop::Length, PROPS[2].1, all(&|x| format_len(x.length))))
                .child(row(Prop::Velocity, PROPS[3].1, all(&|x| format!("{:.0} %", x.velocity * 100.0))))
                .child(row(Prop::Pan, PROPS[4].1, all(&|x| if x.pan.abs() < 0.005 { tr("center").to_string() } else { format!("{:+.0}", x.pan * 100.0) })))
                .child(row(Prop::Detune, PROPS[5].1, all(&|x| if x.bend.len() > 1 { tr("curve").to_string() } else { format!("{:+.2} st", x.bend_at(0)) })))
                .child(div().text_size(px(10.)).text_color(c(THEME.text3)).child(tr("Drag a value up or down (Shift: fine); double click to type it.")))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(3.))
                        .child(button("pr-q", tr("Quantize"), false, 0).on_click(cx.listener(|pr, _, w, cx| pr.pick(Menu::Quantize, 0, w, cx))))
                        .child(button("pr-oct-up", tr("Oct +"), false, 0).on_click(cx.listener(|pr, _, _, cx| pr.transpose(12, cx))))
                        .child(button("pr-oct-down", tr("Oct −"), false, 0).on_click(cx.listener(|pr, _, _, cx| pr.transpose(-12, cx))))
                        .child(button("pr-dup", tr("Duplicate"), false, 0).on_click(cx.listener(|pr, _, _, cx| pr.duplicate(cx))))
                        .child(button("pr-del", tr("Delete notes"), false, THEME.red).on_click(cx.listener(|pr, _, _, cx| {
                            let notes = pr.notes(cx);
                            let sel = pr.selected(&notes);
                            pr.transact("Delete notes", sel.into_iter().map(|(n, _)| (n, None)).collect(), cx);
                            pr.set_sel(BTreeSet::new());
                        }))),
                );
        }
        col
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The kit the three editors share

pub mod kit {
    use std::rc::Rc;

    use gpui::{
        AnyView, App, AppContext, Context, Div, ElementId, Entity, FontWeight, Hsla, IntoElement, Modifiers,
        MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Rgba, SharedString, Stateful,
        Subscription, TextAlign, TextRun, WeakEntity, Window, anchored, canvas, deferred, div, prelude::*, px,
    };
    use gpui_component::input::{Input, InputEvent, InputState};
    use gpui_component::tooltip::Tooltip;

    use crate::ui::{THEME, c, tr};

    /// Cmd on macOS, Ctrl elsewhere
    pub fn primary(m: &Modifiers) -> bool {
        if cfg!(target_os = "macos") { m.platform } else { m.control }
    }

    pub fn with_alpha(col: u32, a: f32) -> Rgba {
        let mut x = c(col);
        x.a = a;
        x
    }

    /// A toolbar dropdown: the current choice and a ▾
    pub fn dropdown(id: impl Into<ElementId>, text: impl Into<SharedString>, active: bool) -> Stateful<Div> {
        div()
            .id(id.into())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(5.))
            .h(px(22.))
            .px(px(7.))
            .rounded(px(4.))
            .border_1()
            .border_color(c(if active { THEME.green } else { THEME.line2 }))
            .bg(c(THEME.bg3))
            .text_size(px(11.5))
            .cursor_pointer()
            .hover(|s| s.bg(c(THEME.bg4)))
            .child(text.into())
            .child(div().text_size(px(9.)).text_color(c(THEME.text2)).child("▾"))
    }

    pub fn label(text: impl Into<SharedString>) -> Div {
        div().flex_none().text_size(px(10.5)).text_color(c(THEME.text3)).child(text.into())
    }

    pub fn sep() -> Div {
        div().flex_none().w(px(1.)).h(px(18.)).mx(px(2.)).bg(c(THEME.line2))
    }

    pub fn tip(text: &'static str) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        move |window, cx| Tooltip::new(tr(text)).build(window, cx)
    }

    #[derive(Clone)]
    pub struct MenuItem {
        pub label: SharedString,
        pub checked: bool,
        pub color: Option<u32>,
        pub header: Option<SharedString>,
    }
    impl MenuItem {
        pub fn new(label: impl Into<SharedString>, checked: bool) -> Self {
            MenuItem { label: label.into(), checked, color: None, header: None }
        }
        pub fn color(mut self, c: u32) -> Self {
            self.color = Some(c);
            self
        }
        /// a section title shown above this item
        pub fn header(mut self, h: impl Into<SharedString>) -> Self {
            self.header = Some(h.into());
            self
        }
    }

    /// A popup list over everything, at `at` (window coordinates); a click outside dismisses it
    pub fn popup(
        id: &'static str,
        at: Point<Pixels>,
        items: Vec<MenuItem>,
        on_pick: impl Fn(usize, &mut Window, &mut App) + 'static,
        on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let pick = Rc::new(on_pick);
        deferred(
            anchored().position(at).snap_to_window_with_margin(px(8.)).child(
                div()
                    .id(id)
                    .debug_selector(move || id.to_string())
                    .occlude()
                    .min_w(px(160.))
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .p(px(4.))
                    .rounded(px(6.))
                    .bg(c(THEME.bg2))
                    .border_1()
                    .border_color(c(THEME.line2))
                    .shadow_lg()
                    .text_size(px(12.))
                    .on_mouse_down_out(move |_, w, cx| on_dismiss(w, cx))
                    .children(items.into_iter().enumerate().map(|(i, it)| {
                        let pick = pick.clone();
                        div()
                            .flex()
                            .flex_col()
                            .children(it.header.map(|h| {
                                div().px(px(8.)).pt(px(6.)).pb(px(2.)).text_size(px(10.)).text_color(c(THEME.text3)).child(h)
                            }))
                            .child(
                                div()
                                    .id(("item", i))
                                    .debug_selector(move || format!("{id}-{i}"))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.))
                                    .px(px(8.))
                                    .py(px(3.))
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(c(THEME.bg4)))
                                    .on_click(move |_, w, cx| pick(i, w, cx))
                                    .child(div().w(px(10.)).text_color(c(THEME.green)).child(if it.checked { "✓" } else { "" }))
                                    .children(it.color.map(|col| div().size(px(8.)).rounded(px(2.)).bg(c(col))))
                                    .child(div().when(it.checked, |d| d.font_weight(FontWeight::BOLD)).child(it.label)),
                            )
                    })),
            ),
        )
        .with_priority(1)
    }

    /// A small field to type a number (double click on a value), at the mouse; Enter applies, Esc or a click
    /// outside closes
    pub struct ValueEntry {
        state: Entity<InputState>,
        at: Point<Pixels>,
        _sub: Subscription,
    }

    impl ValueEntry {
        pub fn new<V: 'static>(value: String, at: Point<Pixels>, window: &mut Window, cx: &mut Context<V>) -> ValueEntry {
            let state = cx.new(|cx| InputState::new(window, cx).default_value(value));
            state.update(cx, |s, cx| s.focus(window, cx));
            let sub = cx.subscribe(&state, |_, _, _: &InputEvent, _| {});
            ValueEntry { state, at, _sub: sub }
        }

        pub fn render(
            &self,
            on_enter: impl Fn(String, &mut Window, &mut App) + 'static,
            on_close: impl Fn(&mut Window, &mut App) + 'static,
        ) -> impl IntoElement {
            let state = self.state.clone();
            let on_close = Rc::new(on_close);
            let close2 = on_close.clone();
            deferred(
                anchored().position(self.at).snap_to_window_with_margin(px(8.)).child(
                    div()
                        .id("value-entry")
                        .occlude()
                        .w(px(120.))
                        .p(px(4.))
                        .rounded(px(6.))
                        .bg(c(THEME.bg2))
                        .border_1()
                        .border_color(c(THEME.green))
                        .shadow_lg()
                        .on_mouse_down_out(move |_, w, cx| close2(w, cx))
                        .on_key_down(move |ev: &gpui::KeyDownEvent, w, cx| {
                            match ev.keystroke.key.as_str() {
                                "enter" => {
                                    let text = state.read(cx).value().to_string();
                                    on_enter(text, w, cx);
                                }
                                "escape" => on_close(w, cx),
                                _ => return,
                            }
                            cx.stop_propagation();
                        })
                        .child(Input::new(&self.state)),
                ),
            )
            .with_priority(2)
        }
    }

    /// Text inside a canvas (one shaped line; GPUI caches the shaping from frame to frame)
    pub fn paint_text(window: &mut Window, cx: &mut App, text: SharedString, x: Pixels, y: Pixels, size: f32, color: Rgba) {
        let font = window.text_style().font();
        let run = TextRun { len: text.len(), font, color: Hsla::from(color), background_color: None, underline: None, strikethrough: None };
        let line = window.text_system().shape_line(text, px(size), &[run], None);
        let _ = line.paint(gpui::point(x, y), px(size * 1.25), TextAlign::Left, None, window, cx);
    }

    /// The mouse captured during a drag: moves and the release reach the view wherever the mouse is
    pub fn capture<V: 'static>(
        view: WeakEntity<V>,
        on_move: fn(&mut V, &MouseMoveEvent, &mut Window, &mut Context<V>),
        on_up: fn(&mut V, &MouseUpEvent, &mut Window, &mut Context<V>),
    ) -> impl IntoElement {
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let v = view.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                    if phase == gpui::DispatchPhase::Bubble {
                        let _ = v.update(cx, |view, cx| on_move(view, ev, window, cx));
                    }
                });
                let v = view.clone();
                window.on_mouse_event(move |ev: &MouseUpEvent, phase, window, cx| {
                    if phase == gpui::DispatchPhase::Bubble && ev.button != MouseButton::Middle {
                        let _ = v.update(cx, |view, cx| on_up(view, ev, window, cx));
                    }
                });
            },
        )
        .absolute()
        .size(px(0.))
    }

    pub fn _unused(_: MouseDownEvent) {}
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;

    use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, WeakEntity, point, px};

    use super::*;
    use crate::core::Core;
    use crate::screens::Screen;

    pub(crate) fn open<V: Render + 'static>(
        cx: &mut TestAppContext,
        make: impl FnOnce(ScreenCx, &mut Window, &mut Context<V>) -> V + 'static,
        screen: Screen,
    ) -> (Entity<V>, Entity<Core>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::workspace::bind_keys(cx);
        });
        let core_slot: Rc<Cell<Option<Entity<Core>>>> = Rc::new(Cell::new(None));
        let slot = core_slot.clone();
        let (view, vcx) = cx.add_window_view(move |window, cx| {
            let core = cx.new(Core::new);
            slot.set(Some(core.clone()));
            let view = cx.new(|cx| make(ScreenCx { core, workspace: WeakEntity::new_invalid(), screen }, window, cx));
            gpui_component::Root::new(view, window, cx)
        });
        let _ = view;
        vcx.run_until_parked();
        let core = core_slot.take().unwrap();
        // the view inside the Root
        let v = vcx.update(|window, cx| {
            let root = window.root::<gpui_component::Root>().flatten().unwrap();
            root.read(cx).view().clone().downcast::<V>().unwrap()
        });
        // two frames: the first lays out, the second fits the item to the bounds
        vcx.run_until_parked();
        v.update(vcx, |_, cx| cx.notify());
        vcx.run_until_parked();
        (v, core, vcx)
    }

    pub(crate) fn drag(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>, m: Modifiers) {
        cx.simulate_mouse_down(from, MouseButton::Left, m);
        for k in 1..=8 {
            let f = k as f32 / 8.0;
            cx.simulate_mouse_move(point(from.x + (to.x - from.x) * f, from.y + (to.y - from.y) * f), MouseButton::Left, m);
        }
        cx.simulate_mouse_up(to, MouseButton::Left, m);
        cx.run_until_parked();
    }

    fn count(core: &Entity<Core>, id: NodeId, cx: &mut VisualTestContext) -> usize {
        core.read_with(cx, |core, _| core.port().root().music.clip(id).unwrap().notes.len())
    }

    /// Where a tick and key are on the screen
    fn at(pr: &Entity<PianoRoll>, tick: f64, key: i32, cx: &mut VisualTestContext) -> Point<Pixels> {
        let o = cx.debug_bounds("pr-grid").unwrap().origin;
        pr.read_with(cx, |pr, _| o + point(px(pr.tick_to_x(tick) + 1.0), px(pr.key_to_y(key) + pr.key_h / 2.0)))
    }

    #[gpui::test]
    fn draw_move_and_undo(cx: &mut TestAppContext) {
        let (pr, core, cx) = open(cx, PianoRoll::new, Screen::PianoRoll);
        let id = pr.read_with(cx, |pr, _| pr.clip.unwrap());
        let before = count(&core, id, cx);
        // a click on an empty place draws a note (the drum pattern has no key 40, E: in the scale A minor)
        let p = at(&pr, (BEAT * 2) as f64 + 10.0, 40, cx);
        cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        assert_eq!(count(&core, id, cx), before + 1, "a click draws one note");
        let (n, note) = core.read_with(cx, |core, _| {
            let c = core.port().root().music.clip(id).unwrap();
            c.notes.iter().find(|(_, x)| x.key == 40).map(|(n, x)| (*n, x.clone())).unwrap()
        });
        assert_eq!(note.start, BEAT * 2, "on the grid");
        // dragging its body three semitones up and a beat right moves it, as one undo step
        let from = at(&pr, note.start as f64 + 30.0, 40, cx);
        let to = at(&pr, note.start as f64 + 30.0 + BEAT as f64, 43, cx);
        drag(cx, from, to, Modifiers::none());
        let moved = core.read_with(cx, |core, _| core.port().root().music.clip(id).unwrap().notes[&n].clone());
        assert_eq!((moved.key, moved.start), (43, BEAT * 3));
        cx.executor().advance_clock(Duration::from_millis(20));
        core.update(cx, |core, _| core.port_mut().undo().ok().unwrap());
        let back = core.read_with(cx, |core, _| core.port().root().music.clip(id).unwrap().notes[&n].clone());
        assert_eq!((back.key, back.start), (40, BEAT * 2), "the whole drag is one undo step");
        core.update(cx, |core, _| core.port_mut().undo().ok().unwrap());
        assert_eq!(count(&core, id, cx), before, "and the drawing another");
    }

    #[gpui::test]
    fn velocity_lane_paints_the_selected_notes(cx: &mut TestAppContext) {
        let (pr, core, cx) = open(cx, PianoRoll::new, Screen::PianoRoll);
        let id = pr.read_with(cx, |pr, _| pr.clip.unwrap());
        let lane = cx.debug_bounds("pr-lane").unwrap();
        // sweep the whole lane at a quarter of its height: every note's velocity becomes about 25 %
        let y = lane.origin.y + lane.size.height * 0.75;
        drag(cx, point(lane.origin.x + px(1.), y), point(lane.origin.x + lane.size.width - px(2.), y), Modifiers::none());
        let (n_low, total) = core.read_with(cx, |core, _| {
            let c = core.port().root().music.clip(id).unwrap();
            (c.notes.values().filter(|x| (x.velocity - 0.25).abs() < 0.06).count(), c.notes.len())
        });
        let visible = pr.read_with(cx, |pr, cx| pr.notes(cx).values().filter(|x| pr.tick_to_x(x.start as f64) < lane.size.width.as_f32() - 6.0).count());
        assert!(n_low >= visible.min(total) * 9 / 10, "{n_low} of {visible} visible notes painted");
        core.update(cx, |core, _| core.port_mut().undo().ok().unwrap());
        let n_low = core.read_with(cx, |core, _| core.port().root().music.clip(id).unwrap().notes.values().filter(|x| (x.velocity - 0.25).abs() < 0.06).count());
        assert_eq!(n_low, 0, "the sweep is one undo step");
    }

    #[gpui::test]
    fn keyboard_select_transpose_quantize(cx: &mut TestAppContext) {
        let (pr, core, cx) = open(cx, PianoRoll::new, Screen::PianoRoll);
        let id = pr.read_with(cx, |pr, _| pr.clip.unwrap());
        let keys_before: Vec<u8> = core.read_with(cx, |core, _| core.port().root().music.clip(id).unwrap().notes.values().map(|x| x.key).collect());
        let grid = cx.debug_bounds("pr-grid").unwrap();
        cx.simulate_mouse_down(grid.center(), MouseButton::Right, Modifiers::none()); // focus (erases nothing on empty)
        cx.simulate_mouse_up(grid.center(), MouseButton::Right, Modifiers::none());
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-a shift-up" } else { "ctrl-a shift-up" });
        cx.run_until_parked();
        let keys_after: Vec<u8> = core.read_with(cx, |core, _| core.port().root().music.clip(id).unwrap().notes.values().map(|x| x.key).collect());
        assert_eq!(keys_before.iter().map(|k| k + 1).collect::<Vec<_>>(), keys_after, "Ctrl+A, Shift+Up: everything a semitone up");
        // the knife: a vertical line through the middle of a beat cuts the notes crossing it
        pr.update(cx, |pr, cx| {
            pr.tool = Tool::Knife;
            pr.set_sel(BTreeSet::new());
            cx.notify();
        });
        let before = count(&core, id, cx);
        let top = cx.debug_bounds("pr-grid").unwrap();
        let x = pr.read_with(cx, |pr, _| pr.tick_to_x(BEAT as f64 / 2.0 - 1.0));
        drag(cx, point(top.origin.x + px(x), top.origin.y + px(2.)), point(top.origin.x + px(x), top.origin.y + top.size.height - px(2.)), Modifiers::none());
        assert!(count(&core, id, cx) >= before, "the knife never loses notes");
    }

    #[gpui::test]
    fn toolbar_menu_switches_the_item(cx: &mut TestAppContext) {
        let (pr, _core, cx) = open(cx, PianoRoll::new, Screen::PianoRoll);
        let p = cx.debug_bounds("pr-clip").unwrap().center();
        cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        let item = cx.debug_bounds("pr-menu-2").expect("the item list is open").center();
        cx.simulate_click(item, Modifiers::none());
        cx.run_until_parked();
        let id = pr.read_with(cx, |pr, _| pr.clip.unwrap());
        assert_eq!(id.number, 7003, "the third item, the bass line, is open");
    }
}
