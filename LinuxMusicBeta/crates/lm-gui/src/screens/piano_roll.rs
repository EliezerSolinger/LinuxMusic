//! Owner: agent B. The Piano Roll: the notes of one MIDI item (F-NOT-001..023, F-PAS-001, P-24, P-26).
//!
//! The legacy's screen (legado/src/gui/editors/PianoRoll.cpp), kept: two toolbar rows (transport and recording;
//! the edit modes draw, erase, select and pitch bend; quantize and the note tools, with the knife and the strum; cut,
//! copy, paste — then the zoom, quantization, note length, key, scale, chord and snap combos and the ghost notes),
//! the keyboard on the left, the grid, and the note edit area under it with Velocity or Panning. What changed and
//! why is in decisoes-autonomas.md (B-PR-*): the item's own loop on the ruler (P-26), no icons (text), the ghost
//! picked in the editor.
//!
//! The item comes as the screen's target (double click on a MIDI item); with none, the first MIDI item opens
//! (F-NOT-022). Reads go through `port().root()`, edits through the port: `AddNote`, `SetNote` (one field) and
//! `RemoveNote`; adding and removing notes are transactions (the document forbids them inside a gesture), moving,
//! resizing and painting the note edit area are gestures.
//!
//! Drawing: one quad per visible note (the cost is per primitive), names only when they fit; the mouse is captured
//! while dragging (`kit::capture`).

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Bounds, Context, CursorStyle, FocusHandle, KeyDownEvent, KeyUpEvent, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathBuilder, PinchEvent, Pixels, Point, Render, ScrollEvent, SharedString,
    Subscription, Window, canvas, div, fill, point, prelude::*, px, size,
};
use lm_gui_plugins::Gesture;
use lm_port::{EditOp, Feed, HostTime, Label, NodeId, NoteRef, PathStep, Ref, Topic, Value};

use self::kit::{MenuItem, ValueEntry, dropdown, dropdown_icon, icon_button, paint_text, popup, sep, tip, with_alpha};
use self::music::{
    BAR, BEAT, CHORDS, Clip, GRIDS, KEY_NAMES, Note, NoteChange, NoteMap, QuantizeWhat, SCALES, format_bbt, format_len,
    key_name,
};
mod clip_text;
mod first_clip;

use super::ScreenCx;
use super::live::Live;
use super::piano_roll_bars::{self, Bar};
use super::piano_roll_curves::{self, Curve};
use crate::actions::{
    PrCopy, PrCut, PrCutOverlaps, PrDelete, PrFill, PrGlue, PrModeDraw, PrModeErase, PrModeKnife, PrModePitch,
    PrModeSelect, PrModeStrum, PrMoveBarLeft, PrMoveBarRight, PrMoveLeft, PrMoveRight, PrNextClip, PrOctaveDown,
    PrOctaveUp, PrPaste, PrPlay, PrPrevClip, PrReverse, PrSelectAll, PrSelectNone, PrTransposeDown, PrTransposeUp,
};
use crate::keymap;
use crate::model;
use crate::ui::{THEME, c, tr, track_color, trf};

mod keys;

const CTX: &str = "PianoRoll";

/// The mouse actions of this screen (`keymap/mouse_defs.rs`): the gestures are the user's, never read from the event
fn holds(id: &str, m: &Modifiers) -> bool {
    keymap::holds(CTX, id, m)
}

fn fires(id: &str, button: MouseButton, clicks: usize, m: &Modifiers) -> bool {
    keymap::fires(CTX, id, button, clicks, m)
}

const ROW_H: f32 = 26.0;
const RULER_H: f32 = 18.0;
const KEYS_W: f32 = 64.0;
const LANE_H: f32 = 64.0;
const EDGE_PX: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Draw,
    Erase,
    Select,
    Pitch,
    Knife,
    Strum,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapMode {
    Nudge,
    Snap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaneMode {
    Velocity,
    Panning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rec {
    Off,
    /// notes from the keyboard where the item's cursor is (the legacy's "record")
    Live,
    /// the same with the song playing (the legacy's "record while playing the song")
    Accompany,
    /// one step at a time (F-PAS-001)
    Step,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Quantize,
    Tools,
    ZoomX,
    ZoomY,
    Grid,
    Length,
    Key,
    Scale,
    Chord,
    Snap,
    Ghost,
    Lane,
    Keys(u8),
}

/// The legacy's zoom steps
const ZOOM_X: [f64; 7] = [0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
const ZOOM_Y: [f32; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];
/// Ticks per pixel and key height at 100 %
const TPP_100: f64 = 30.0;
const KEY_H_100: f32 = 12.0;

enum Drag {
    /// a new note (a chord) whose length follows the mouse
    Draw {
        g: Option<Gesture>,
        ns: Vec<u64>,
        start: i64,
        base: Vec<Note>,
    },
    Move {
        g: Option<Gesture>,
        origin: Vec<(u64, Note)>,
        anchor: Note,
        from: (f64, i32),
        copy: bool,
        delta: (i64, i32),
        clicked: u64,
        toggle: bool,
    },
    Resize {
        g: Option<Gesture>,
        origin: Vec<(u64, Note)>,
        anchor: Note,
        from: f64,
        stretch: bool,
        /// Shift+Ctrl: the notes after the dragged ones are pushed along (F-NOT-008)
        sticky: bool,
    },
    Rect {
        from: Point<Pixels>,
        to: Point<Pixels>,
        toggle: bool,
        before: Rc<BTreeSet<u64>>,
    },
    /// erasing: the notes go in one transaction on release (no removal inside a gesture)
    Erase {
        gone: BTreeSet<u64>,
    },
    Pitch {
        g: Option<Gesture>,
        n: u64,
        origin: Note,
        from: (f64, f32),
    },
    Knife {
        from: Point<Pixels>,
        to: Point<Pixels>,
    },
    Strum {
        g: Option<Gesture>,
        origin: Vec<(u64, Note)>,
        from: f64,
        from_y: f32,
    },
    Lane {
        g: Option<Gesture>,
        last_x: f32,
        only_under: bool,
    },
    Loop {
        which: u8,
    },
    RulerSelect {
        from: f64,
    },
    Keys,
}

/// The editor's own transport (P-26): a cursor and a loop inside the item, played without the song
#[derive(Clone, Copy, Debug)]
struct ItemPlay {
    cursor: i64,
    loop_on: bool,
    loop_a: i64,
    loop_b: i64,
    since: Option<(HostTime, i64)>,
    ticks_per_sec: f64,
}

impl ItemPlay {
    fn at(&self, now: HostTime) -> i64 {
        match self.since {
            None => self.cursor,
            Some((t0, from)) => {
                let t = from + (now.since(t0) as f64 * 1e-9 * self.ticks_per_sec) as i64;
                if self.loop_on && self.loop_b > self.loop_a && t >= self.loop_b {
                    self.loop_a + (t - self.loop_a) % (self.loop_b - self.loop_a)
                } else {
                    t
                }
            }
        }
    }
}

/// The computer keyboard as a piano (F-NOT-023), the legacy's two rows
fn piano_key(k: &str) -> Option<i32> {
    const LOW: [&str; 17] = ["z", "s", "x", "d", "c", "v", "g", "b", "h", "n", "j", "m", ",", "l", ".", ";", "/"];
    const HIGH: [&str; 18] = ["q", "2", "w", "3", "e", "r", "5", "t", "6", "y", "7", "u", "i", "9", "o", "0", "p", "["];
    LOW.iter()
        .position(|x| *x == k)
        .map(|i| i as i32)
        .or_else(|| HIGH.iter().position(|x| *x == k).map(|i| i as i32 + 12))
}

/// Note numbers from here up are temp numbers of notes to add (`EditOp::AddNote`)
pub(crate) const NEW: u64 = 1 << 63;

/// Note changes as the port's edits: an added note whole, a changed note field by field
pub(crate) fn note_ops(item: NodeId, old: &NoteMap, changes: Vec<NoteChange>) -> Vec<EditOp> {
    let mut ops = Vec::new();
    for (n, x) in changes {
        let it = Ref::Id(item);
        match x {
            Some(x) if n >= NEW => ops.push(EditOp::AddNote { temp: (n - NEW) as u32, item: it, note: x.to_value() }),
            Some(x) => {
                let Some(o) = old.get(&n) else { continue };
                for (key, v) in x.fields_changed(o) {
                    ops.push(EditOp::SetNote { item: it, n: NoteRef::N(n), path: vec![PathStep::key(key)], value: v });
                }
            }
            None if n >= NEW => {}
            None => ops.push(EditOp::RemoveNote { item: it, n: NoteRef::N(n) }),
        }
    }
    ops
}

pub struct PianoRoll {
    scx: ScreenCx,
    focus: FocusHandle,
    feed: Feed,
    /// The clips read from the tree, by revision (see `ClipCache`)
    clips: std::cell::RefCell<music::ClipCache>,
    pub clip: Option<NodeId>,
    ghost: Option<NodeId>,
    pub tool: Tool,
    /// which curve of a note the Pitch tool draws (bend, MPE pressure or timbre)
    curve: Curve,
    grid: usize,
    /// The quantization is the note's length ("Lock to the note", F-NOT-012)
    lock: bool,
    snap: SnapMode,
    /// None = the last note's length (the legacy's "Last note")
    length: Option<usize>,
    last: Note,
    root: u8,
    scale: Option<usize>,
    chord: Option<usize>,
    lane: LaneMode,
    zoom_x: usize,
    zoom_y: usize,
    pub scroll: f64,
    pub tpp: f64,
    pub key_h: f32,
    pub scroll_y: f32,
    pub sel: Rc<BTreeSet<u64>>,
    drag: Option<Drag>,
    hover: Option<(f64, i32)>,
    clipboard: Vec<Note>,
    menu: Option<(Menu, Point<Pixels>)>,
    entry: Option<(f64, ValueEntry)>,
    rec: Rec,
    octave: i32,
    held: HashMap<String, (u8, i64)>,
    /// the keys of the chord being gathered in step mode (sent when the last is released)
    step_chord: Vec<u8>,
    /// the notes this editor has sounding (I-15), released on mouse-up, focus loss and close
    live: Live,
    /// the keys the mouse is auditioning (the piano keys, a note drawn or moved)
    audition: BTreeSet<u8>,
    _subs: Vec<Subscription>,
    pressed: BTreeSet<u8>,
    marked: BTreeSet<u8>,
    play: ItemPlay,
    /// The core is playing the open item alone (`item.audition`), in a loop when the item's loop is on
    item_audio: bool,
    /// The transport was seen running since the audition started (it starts when the core's program is installed)
    audio_ran: bool,
    grid_bounds: Rc<Cell<Bounds<Pixels>>>,
    lane_bounds: Rc<Cell<Bounds<Pixels>>>,
    keys_bounds: Rc<Cell<Bounds<Pixels>>>,
    ruler_bounds: Rc<Cell<Bounds<Pixels>>>,
    fitted: bool,
}

impl PianoRoll {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (feed, clip, tps) = scx.core.update(cx, |core, _| {
            let root = core.port().root();
            // the target, or the first MIDI item; none in the project: the first instrument track gets one (F-NOT-022)
            let mut clip = scx.target.or_else(|| music::first_midi(root));
            if clip.is_none()
                && let Some(op) = first_clip::first_clip_op(root)
            {
                clip = core
                    .port_mut()
                    .apply(Label::new("New item"), vec![op])
                    .ok()
                    .and_then(|a| a.created.first().map(|(_, id)| *id));
            }
            let root = core.port().root();
            let map = model::tempo_map(root);
            let tps = map.tick_at(map.sample_rate() as i64) as f64;
            (core.port_mut().feed(Topic::All), clip, tps.max(1.0))
        });
        // the quantization and the note length the project kept (F-NOT-012)
        let kept = |k: &str| scx.core.read(cx).port().ui().get("pianoRoll").and_then(|p| p.get(k)).cloned();
        let kept_grid = kept("grid").and_then(|v| v.as_int()).map_or(4, |i| (i.max(0) as usize).min(GRIDS.len() - 1));
        let kept_length = kept("length")
            .and_then(|v| v.as_int())
            .and_then(|i| usize::try_from(i).ok())
            .map(|i| i.min(GRIDS.len() - 1));
        let kept_lock = kept("lock").and_then(|v| v.as_bool()).unwrap_or(false);
        let mut pr = PianoRoll {
            focus: cx.focus_handle(),
            feed,
            clips: Default::default(),
            clip,
            ghost: None,
            tool: Tool::Draw,
            curve: Curve::default(),
            grid: kept_grid,
            lock: kept_lock,
            snap: SnapMode::Nudge,
            length: kept_length,
            last: Note::new(60, 0, BEAT / 4, 0.8),
            root: 0,
            scale: None,
            chord: None,
            lane: LaneMode::Velocity,
            zoom_x: 3,
            zoom_y: 2,
            scroll: 0.0,
            tpp: TPP_100,
            key_h: KEY_H_100,
            scroll_y: 0.0,
            sel: Rc::new(BTreeSet::new()),
            drag: None,
            hover: None,
            clipboard: Vec::new(),
            menu: None,
            entry: None,
            rec: Rec::Off,
            octave: 4,
            held: HashMap::new(),
            step_chord: Vec::new(),
            live: Live::default(),
            audition: BTreeSet::new(),
            _subs: Vec::new(),
            pressed: BTreeSet::new(),
            marked: BTreeSet::new(),
            play: ItemPlay { cursor: 0, loop_on: false, loop_a: 0, loop_b: BAR, since: None, ticks_per_sec: tps },
            item_audio: false,
            audio_ran: false,
            grid_bounds: Rc::new(Cell::new(Bounds::default())),
            lane_bounds: Rc::new(Cell::new(Bounds::default())),
            keys_bounds: Rc::new(Cell::new(Bounds::default())),
            ruler_bounds: Rc::new(Cell::new(Bounds::default())),
            fitted: false,
            scx,
        };
        // the notes end, and step mode with them, when the editor loses the focus or the window, or closes
        let core = pr.scx.core.clone();
        let release = cx.on_release(move |pr, cx| {
            pr.live.release_all(&core, cx);
            if pr.item_audio {
                core.update(cx, |c, _| {
                    c.report(|pt| pt.run_action("item.audition_stop".into(), Value::NULL));
                });
            }
            if pr.rec == Rec::Step {
                core.update(cx, |c, _| {
                    c.report(|pt| pt.run_action("notes.step_stop".into(), Value::NULL));
                });
            }
        });
        let blur = cx.on_blur(&pr.focus, window, |pr, _, cx| pr.release_all(cx));
        let active = cx.observe_window_activation(window, |pr, window, cx| {
            if !window.is_window_active() {
                pr.release_all(cx);
            }
        });
        pr._subs = vec![release, blur, active];
        pr
    }

    // --- live notes (I-15) and the core's step recording -----------------------------------------------------------

    /// The track that plays the open clip
    fn clip_track(&self, cx: &gpui::App) -> Option<NodeId> {
        let clip = self.clip?;
        model::tracks(self.scx.core.read(cx).port().root())
            .into_iter()
            .find(|t| t.items.iter().any(|i| i.id == clip))
            .map(|t| t.id)
    }

    /// Starts a live note on the clip's instrument
    fn sound(&mut self, key: u8, velocity: f64, cx: &mut Context<Self>) {
        if let Some(t) = self.clip_track(cx) {
            let core = self.scx.core.clone();
            self.live.on(&core, t, key, (velocity * 127.0).round().clamp(1.0, 127.0) as u8, cx);
        }
    }

    fn silence(&mut self, key: u8, cx: &mut Context<Self>) {
        if let Some(t) = self.clip_track(cx) {
            let core = self.scx.core.clone();
            self.live.off(&core, t, key, cx);
        }
    }

    /// The mouse auditions exactly `keys` (what a note being drawn or moved plays, a piano key under the mouse): the
    /// ones no longer in it end, the new ones start
    fn audition(&mut self, keys: &[u8], cx: &mut Context<Self>) {
        let want: BTreeSet<u8> = keys.iter().copied().collect();
        let gone: Vec<u8> = self.audition.difference(&want).copied().collect();
        for k in gone {
            self.silence(k, cx);
        }
        let new: Vec<u8> = want.difference(&self.audition).copied().collect();
        let v = self.last.velocity;
        for k in new {
            self.sound(k, v, cx);
        }
        self.audition = want;
    }

    fn audition_stop(&mut self, cx: &mut Context<Self>) {
        self.audition(&[], cx);
    }

    /// An action of the core, with the clip as `item`
    fn step_action(&mut self, id: &'static str, extra: Vec<(&'static str, Value)>, cx: &mut Context<Self>) {
        let Some(item) = self.clip else { return };
        let mut fields = vec![("item", Value::text(item.to_string()))];
        fields.extend(extra);
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| pt.run_action(id.into(), Value::object(fields)));
        });
    }

    /// The core's step cursor in the clip, in ticks, and the step (`notes.step_record`, the interface document's
    /// `stepRecord`), while step mode is on for this clip
    fn step_state(&self, cx: &gpui::App) -> Option<(i64, i64)> {
        let v = self.scx.core.read(cx).port().ui().get("stepRecord")?.clone();
        if v.get("item").and_then(Value::as_text) != self.clip.map(|c| c.to_string()).as_deref() {
            return None;
        }
        Some((v.get("at").and_then(Value::as_int)?, v.get("step").and_then(Value::as_int)?))
    }

    // --- reading -----------------------------------------------------------------------------------------------

    pub fn clip(&self, cx: &gpui::App) -> Option<Clip> {
        self.clip.and_then(|id| self.clips.borrow_mut().get(self.scx.core.read(cx).port().root(), id))
    }

    pub fn notes(&self, cx: &gpui::App) -> Arc<NoteMap> {
        self.clip(cx).map(|c| c.notes).unwrap_or_default()
    }

    fn grid_ticks(&self) -> i64 {
        if self.lock { self.new_len() } else { GRIDS[self.grid].1 }
    }

    /// The quantization and the note length, kept in the interface document (F-NOT-012)
    fn remember_grid(&self, cx: &mut Context<Self>) {
        let v = Value::object([
            ("grid", Value::int(self.grid as i64)),
            ("length", Value::int(self.length.map_or(-1, |i| i as i64))),
            ("lock", Value::bool(self.lock)),
        ]);
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| pt.set_ui(vec![PathStep::key("pianoRoll")], v));
        });
    }

    fn new_len(&self) -> i64 {
        self.length.map_or(self.last.length, |i| GRIDS[i].1)
    }

    pub fn tick_to_x(&self, t: f64) -> f32 {
        ((t - self.scroll) / self.tpp) as f32
    }
    fn x_to_tick(&self, x: f32) -> f64 {
        self.scroll + x as f64 * self.tpp
    }
    fn y_to_key(&self, y: f32) -> i32 {
        127 - ((y + self.scroll_y) / self.key_h).floor() as i32
    }
    pub fn key_to_y(&self, k: i32) -> f32 {
        (127 - k) as f32 * self.key_h - self.scroll_y
    }

    /// The topmost note under (tick, key) and the part hit: 0 body, 1 the tail (resize)
    fn note_at(&self, notes: &NoteMap, tick: f64, key: i32) -> Option<(u64, i8)> {
        let mut hit = None;
        for (n, x) in notes.iter() {
            if x.key as i32 == key && tick >= x.start as f64 && tick < x.end() as f64 {
                let w = (x.length as f64 / self.tpp) as f32;
                let dx_r = ((x.end() as f64 - tick) / self.tpp) as f32;
                hit = Some((*n, if dx_r <= EDGE_PX.min(w / 3.0) { 1 } else { 0 }));
            }
        }
        hit
    }

    fn snap_tick(&self, t: f64, free: bool) -> i64 {
        if free { t.round() as i64 } else { music::snap_floor(t, self.grid_ticks()) }
    }

    /// With a scale chosen and no chord, drawn and moved notes stay in the scale
    fn scale_key(&self, k: i32) -> i32 {
        match self.scale {
            Some(s) if self.chord.is_none() => music::to_scale(k, self.root, s) as i32,
            _ => k.clamp(0, 127),
        }
    }

    // --- changing, through the port ----------------------------------------------------------------------------

    /// One transaction; returns the numbers the core gave the added notes, in temp order (none while `Waiting`)
    fn transact(&mut self, label: &'static str, changes: Vec<NoteChange>, cx: &mut Context<Self>) -> Vec<u64> {
        let Some(id) = self.clip else { return Vec::new() };
        if changes.is_empty() {
            return Vec::new();
        }
        let old = self.notes(cx);
        let ops = note_ops(id, &old, changes);
        let applied = self.scx.core.update(cx, |core, _| core.report(|pt| pt.apply(Label::new(label), ops)));
        cx.notify();
        let mut notes = applied.map(|a| a.notes).unwrap_or_default();
        notes.sort_by_key(|(t, _)| *t);
        notes.into_iter().map(|(_, n)| n).collect()
    }

    /// Updates a drag's gesture, opening it at the first change (field changes only)
    fn gesture(
        &self,
        g: &mut Option<Gesture>,
        label: &'static str,
        changes: Vec<NoteChange>,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.clip else { return };
        let old = self.notes(cx);
        let ops = note_ops(id, &old, changes);
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

    fn temps(k: usize) -> Vec<u64> {
        (0..k as u64).map(|i| NEW + i).collect()
    }

    pub fn set_sel(&mut self, s: BTreeSet<u64>) {
        self.sel = Rc::new(s);
    }

    /// The selected notes, or all when nothing is selected
    fn targets(&self, notes: &NoteMap) -> Vec<u64> {
        if self.sel.is_empty() {
            notes.keys().copied().collect()
        } else {
            self.sel.iter().copied().filter(|n| notes.contains_key(n)).collect()
        }
    }

    fn selected(&self, notes: &NoteMap) -> Vec<(u64, Note)> {
        self.sel.iter().filter_map(|n| notes.get(n).map(|x| (*n, x.clone()))).collect()
    }

    /// Scrolls to the notes, once, when the grid has its size
    fn fit(&mut self, view_h: f32, cx: &gpui::App) {
        let Some(c) = self.clip(cx) else { return };
        let (lo, hi) = c.notes.values().fold((127u8, 0u8), |(a, b), x| (a.min(x.key), b.max(x.key)));
        let mid = if lo > hi { 60.0 } else { (lo as f32 + hi as f32) / 2.0 };
        self.scroll_y = ((127.0 - mid) * self.key_h - view_h / 2.0).max(0.0);
        self.play.loop_b = c.loop_len.unwrap_or(c.length).max(BAR);
        self.fitted = true;
    }

    fn play_tick(&self, cx: &gpui::App, now: HostTime) -> (Option<f64>, bool) {
        if self.play.since.is_some() {
            return (Some(self.play.at(now) as f64), true);
        }
        let Some(c) = self.clip(cx) else { return (None, false) };
        let core = self.scx.core.read(cx);
        let pos = core.port().telemetry().position;
        let map = core.port().tempo_map();
        let t = map.tick_at(pos.at(now) as i64) as f64 - c.start as f64;
        if t < 0.0 || t >= c.length as f64 {
            return (None, pos.playing);
        }
        let t = c.loop_len.map_or(t, |l| t % l.max(1) as f64);
        (Some(t), pos.playing)
    }

    /// Space: the core plays the open item alone from the item's cursor, in a loop when the item's loop is on
    /// (`item.audition`); a port that cannot play it (none of the core's) gets the editor's own cursor
    fn toggle_play_item(&mut self, cx: &mut Context<Self>) {
        let now = HostTime::now();
        if self.item_audio {
            self.item_audio = false;
            self.scx.core.update(cx, |core, _| {
                core.report(|pt| pt.run_action("item.audition_stop".into(), Value::NULL));
            });
            self.release_all(cx);
            cx.notify();
            return;
        }
        if self.play.since.is_some() {
            self.play.cursor = self.play.at(now);
            self.play.since = None;
            self.release_all(cx);
            cx.notify();
            return;
        }
        let outside = self.play.cursor < self.play.loop_a || self.play.cursor >= self.play.loop_b;
        let from = if self.play.loop_on && outside { self.play.loop_a } else { self.play.cursor };
        if let Some(item) = self.clip {
            let mut fields = vec![("item", Value::text(item.to_string())), ("from", Value::int(from))];
            if self.play.loop_on {
                fields.push(("to", Value::int(self.play.loop_b)));
                fields.push(("loop", Value::bool(true)));
            }
            let params = Value::object(fields);
            let played = self
                .scx
                .core
                .update(cx, |core, _| core.report(|pt| pt.run_action("item.audition".into(), params)).is_some());
            if played {
                self.item_audio = true;
                cx.notify();
                return;
            }
        }
        self.play.since = Some((now, from));
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.play.since = None;
        self.play.cursor = 0;
        self.audio_ran = false;
        if std::mem::take(&mut self.item_audio) {
            self.scx.core.update(cx, |core, _| {
                core.report(|pt| pt.run_action("item.audition_stop".into(), Value::NULL));
            });
        }
        if self.rec == Rec::Accompany {
            self.record_into(false, cx);
            self.scx.core.update(cx, |core, _| {
                core.report(|pt| pt.run_action("transport.stop".into(), Value::NULL));
            });
        }
        if self.rec != Rec::Step {
            self.rec = Rec::Off;
        }
        self.release_all(cx);
        cx.notify();
    }

    // --- the mouse on the grid -------------------------------------------------------------------------------

    fn local(b: &Rc<Cell<Bounds<Pixels>>>, p: Point<Pixels>) -> (f32, f32) {
        let o = b.get().origin;
        ((p.x - o.x).as_f32(), (p.y - o.y).as_f32())
    }

    fn grid_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.menu = None;
        self.entry = None;
        let Some(clip) = self.clip(cx) else { return };
        let (x, y) = Self::local(&self.grid_bounds, ev.position);
        let tick = self.x_to_tick(x);
        let key = self.y_to_key(y);
        let free = holds("pr.grid.free", &ev.modifiers);
        let notes = clip.notes.clone();
        let hit = self.note_at(&notes, tick, key);
        if fires("pr.grid.erase", ev.button, ev.click_count, &ev.modifiers) {
            // the right button erases (dragging erases more); with the knife it cancels; in pitch bend it keeps the
            // first point of the curve
            match self.tool {
                Tool::Knife => self.drag = None,
                Tool::Pitch => {
                    if let Some((n, _)) = hit {
                        let mut x = notes[&n].clone();
                        match self.curve {
                            Curve::Bend => x.bend.truncate(1),
                            c => {
                                let first: Vec<_> = x.mpe.get(c).iter().copied().take(1).collect();
                                x.mpe = x.mpe.with(c, first);
                            }
                        }
                        self.transact("Remove curve points", vec![(n, Some(x))], cx);
                    }
                }
                _ => self.drag = Some(Drag::Erase { gone: hit.map(|h| h.0).into_iter().collect() }),
            }
            cx.notify();
            return;
        }
        let tool = if holds("pr.grid.select_override", &ev.modifiers) { Tool::Select } else { self.tool };
        match (tool, hit) {
            (Tool::Erase, _) => self.drag = Some(Drag::Erase { gone: hit.map(|h| h.0).into_iter().collect() }),
            (Tool::Knife, _) => {
                let p = point(px(x), px(y));
                self.drag = Some(Drag::Knife { from: p, to: p });
            }
            (Tool::Pitch, Some((n, _))) => {
                self.drag = Some(Drag::Pitch { g: None, n, origin: notes[&n].clone(), from: (tick, y) });
            }
            (Tool::Strum, Some((n, _))) => {
                // the chord: the selected notes, or the notes that start with the clicked one
                let mut origin = self.selected(&notes);
                if !self.sel.contains(&n) || origin.len() < 2 {
                    let s = notes[&n].start;
                    origin = notes
                        .iter()
                        .filter(|(_, x)| (x.start - s).abs() < BEAT / 8)
                        .map(|(k, x)| (*k, x.clone()))
                        .collect();
                }
                self.set_sel(origin.iter().map(|(k, _)| *k).collect());
                self.drag = Some(Drag::Strum { g: None, origin, from: tick, from_y: y });
            }
            (Tool::Draw | Tool::Select, Some((n, part))) => {
                // a click on a note makes its length, velocity and panning the "last" ones (F-NOT-001)
                let x = notes[&n].clone();
                self.last = x.clone();
                let toggle = holds("pr.grid.select_override", &ev.modifiers);
                if !self.sel.contains(&n) {
                    let mut s = if toggle { (*self.sel).clone() } else { BTreeSet::new() };
                    s.insert(n);
                    self.set_sel(s);
                }
                let origin = self.selected(&notes);
                if part == 1 {
                    self.drag = Some(Drag::Resize {
                        g: None,
                        origin,
                        anchor: x,
                        from: tick,
                        stretch: holds("pr.note.stretch", &ev.modifiers),
                        sticky: holds("pr.note.sticky", &ev.modifiers),
                    });
                } else {
                    self.pressed = [x.key].into_iter().collect();
                    self.audition(&[x.key], cx);
                    self.drag = Some(Drag::Move {
                        g: None,
                        origin,
                        anchor: x,
                        from: (tick, key),
                        copy: holds("pr.note.copy", &ev.modifiers),
                        delta: (0, 0),
                        clicked: n,
                        toggle,
                    });
                }
            }
            (Tool::Select | Tool::Pitch | Tool::Strum, None) => {
                let p = point(px(x), px(y));
                let before =
                    if holds("pr.grid.extend", &ev.modifiers) { self.sel.clone() } else { Rc::new(BTreeSet::new()) };
                self.drag = Some(Drag::Rect { from: p, to: p, toggle: holds("pr.grid.extend", &ev.modifiers), before });
            }
            (Tool::Draw, None) => {
                if tick < 0.0 || !(0..=127).contains(&key) {
                    return;
                }
                let start = self.snap_tick(tick, free);
                let base_key = self.scale_key(key);
                let len = self.new_len();
                let keys: Vec<u8> = match self.chord {
                    Some(ch) => music::chord_keys(base_key as u8, ch),
                    None => vec![base_key as u8],
                };
                // Shift with a chord: an arpeggio, one grid step apart
                let arp = holds("pr.draw.arpeggio", &ev.modifiers) && keys.len() > 1;
                let base: Vec<Note> = keys
                    .iter()
                    .enumerate()
                    .map(|(i, k)| {
                        let s = if arp { start + i as i64 * self.grid_ticks() } else { start };
                        Note {
                            key: *k,
                            start: s,
                            length: len,
                            velocity: self.last.velocity,
                            pan: self.last.pan,
                            bend: Vec::new(),
                            mpe: Default::default(),
                        }
                    })
                    .collect();
                let ch = Self::temps(base.len()).into_iter().zip(base.iter().cloned().map(Some)).collect();
                let ns = self.transact("Draw notes", ch, cx);
                self.set_sel(ns.iter().copied().collect());
                self.audition(&keys, cx);
                self.pressed = keys.into_iter().collect();
                self.drag = Some(Drag::Draw { g: None, ns, start, base });
            }
        }
        cx.notify();
    }

    fn drag_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut drag) = self.drag.take() else { return };
        let (x, y) = Self::local(&self.grid_bounds, ev.position);
        let tick = self.x_to_tick(x);
        let key = self.y_to_key(y);
        let free = holds("pr.grid.free", &ev.modifiers);
        let grid = self.grid_ticks();
        let notes = self.notes(cx);
        match &mut drag {
            Drag::Draw { g, ns, start, base } => {
                let end = if free { tick.round() as i64 } else { music::snap(tick, grid) };
                let len = (end - *start).max(if free { 1 } else { grid });
                if len != base[0].length || g.is_some() {
                    let changes = ns
                        .iter()
                        .zip(base.iter())
                        .map(|(n, b)| (*n, Some(Note { length: len, ..b.clone() })))
                        .collect();
                    self.last.length = len;
                    self.gesture(g, "Draw notes", changes, cx);
                }
            }
            Drag::Move { g, origin, anchor, from, copy, delta, .. } => {
                let raw = tick - from.0;
                let dt = if free {
                    raw.round() as i64
                } else {
                    match self.snap {
                        SnapMode::Nudge => music::snap(raw, grid),
                        SnapMode::Snap => {
                            // the start or the end, whichever is nearer, lands on the grid
                            let s = anchor.start as f64 + raw;
                            let e = anchor.end() as f64 + raw;
                            let ds = music::snap(s, grid) as f64 - s;
                            let de = music::snap(e, grid) as f64 - e;
                            (raw + if ds.abs() <= de.abs() { ds } else { de }).round() as i64
                        }
                    }
                };
                let dt = dt.max(-origin.iter().map(|(_, x)| x.start).min().unwrap_or(0));
                let target = self.scale_key(anchor.key as i32 + (key - from.1));
                let lo = origin.iter().map(|(_, x)| x.key as i32).min().unwrap_or(0);
                let hi = origin.iter().map(|(_, x)| x.key as i32).max().unwrap_or(127);
                let dk = (target - anchor.key as i32).clamp(-lo, 127 - hi);
                if dk != delta.1 {
                    let k = (anchor.key as i32 + dk) as u8;
                    self.pressed = [k].into_iter().collect();
                    self.audition(&[k], cx);
                }
                *delta = (dt, dk);
                // a copy (Shift at the start) is added on release; the originals move live
                if !*copy && (dt != 0 || dk != 0 || g.is_some()) {
                    let changes = origin
                        .iter()
                        .map(|(n, x)| {
                            (*n, Some(Note { start: x.start + dt, key: (x.key as i32 + dk) as u8, ..x.clone() }))
                        })
                        .collect();
                    self.gesture(g, "Move notes", changes, cx);
                }
            }
            Drag::Resize { g, origin, anchor, from, stretch, sticky } => {
                let raw = tick - *from;
                let changes: Vec<NoteChange> = if *stretch {
                    // Shift: the selection stretches in proportion, from its first start (F-NOT-008)
                    let s0 = origin.iter().map(|(_, x)| x.start).min().unwrap_or(0);
                    let f = ((anchor.end() as f64 + raw - s0 as f64) / (anchor.end() - s0).max(1) as f64).max(0.05);
                    origin
                        .iter()
                        .map(|(n, x)| {
                            let s = s0 + ((x.start - s0) as f64 * f).round() as i64;
                            let l = ((x.length as f64 * f).round() as i64).max(1);
                            (*n, Some(Note { start: s, length: l, ..x.clone() }))
                        })
                        .collect()
                } else {
                    let ne = if free {
                        (anchor.end() as f64 + raw).round() as i64
                    } else {
                        music::snap(anchor.end() as f64 + raw, grid)
                    };
                    let d = ne - anchor.end();
                    let min = if free { 1 } else { grid };
                    let mut out: Vec<NoteChange> = origin
                        .iter()
                        .map(|(n, x)| (*n, Some(Note { length: (x.length + d).max(min.min(x.length)), ..x.clone() })))
                        .collect();
                    if *sticky && d != 0 {
                        // the notes that begin at or after the dragged end are pushed by the same distance
                        let end = origin.iter().map(|(_, x)| x.end()).max().unwrap_or(0);
                        out.extend(
                            notes
                                .iter()
                                .filter(|(n, x)| x.start >= end && !origin.iter().any(|(o, _)| o == *n))
                                .map(|(n, x)| (*n, Some(Note { start: (x.start + d).max(0), ..x.clone() }))),
                        );
                    }
                    out
                };
                if let Some((_, Some(x))) = changes.first() {
                    self.last.length = x.length;
                }
                self.gesture(g, "Resize notes", changes, cx);
            }
            Drag::Rect { to, .. } | Drag::Knife { to, .. } => *to = point(px(x), px(y)),
            Drag::Erase { gone } => {
                if let Some((n, _)) = self.note_at(&notes, tick, key) {
                    gone.insert(n);
                }
            }
            Drag::Pitch { g, n, origin, from } if self.curve != Curve::Bend => {
                // a line of the MPE curve (0..1): a key's height of travel is a twelfth of the range
                let at0 = ((from.0 - origin.start as f64).round() as i64).max(0).min(origin.length.max(0));
                let at1 = ((tick - origin.start as f64).round() as i64).max(0).min(origin.length.max(0));
                let dv = f64::from(from.1 - y) / f64::from(self.key_h * 12.0);
                let pts = piano_roll_curves::line(origin.mpe.get(self.curve), at0, at1, dv);
                let x = Note { mpe: origin.mpe.with(self.curve, pts), ..origin.clone() };
                self.gesture(g, "MPE curve", vec![(*n, Some(x))], cx);
            }
            Drag::Pitch { g, n, origin, from } => {
                // a line of the bend from where the drag began to the mouse, in semitones by height
                let at0 = ((from.0 - origin.start as f64).round() as i64).max(0).min(origin.length.max(0));
                let at1 = ((tick - origin.start as f64).round() as i64).max(0).min(origin.length.max(0));
                let v0 = origin.bend_at(at0);
                let v1 = ((from.1 - y) / self.key_h * 4.0).round() as f64 / 4.0 + v0;
                let (a, b) = if at0 <= at1 { ((at0, v0), (at1, v1)) } else { ((at1, v1), (at0, v0)) };
                let mut pts: Vec<music::Bend> =
                    origin.bend.iter().copied().filter(|p| p.at < a.0 || p.at > b.0).collect();
                if !pts.iter().any(|p| p.at == 0) && a.0 > 0 {
                    pts.push(music::Bend { at: 0, semitones: origin.bend_at(0) }); // the first point is fixed
                }
                pts.push(music::Bend { at: a.0, semitones: a.1.clamp(-24.0, 24.0) });
                if b.0 != a.0 {
                    pts.push(music::Bend { at: b.0, semitones: b.1.clamp(-24.0, 24.0) });
                }
                pts.sort_by_key(|p| p.at);
                pts.dedup_by_key(|p| p.at);
                self.gesture(g, "Pitch bend", vec![(*n, Some(Note { bend: pts, ..origin.clone() }))], cx);
            }
            Drag::Strum { g, origin, from, from_y } => {
                let mut spread = (tick - *from).round() as i64;
                if !free {
                    spread = music::snap(spread as f64, (grid / 4).max(1));
                }
                // Shift bends the staircase: the pointer's height is the curve (up: the late notes crowd at the
                // end, down: at the start)
                let curve = if holds("pr.strum.curve", &ev.modifiers) {
                    2f64.powf(f64::from(*from_y - y) / f64::from(self.key_h) / 4.0)
                } else {
                    1.0
                };
                self.gesture(g, "Strum", music::strum_curved(origin, spread, curve), cx);
            }
            Drag::Lane { .. } | Drag::Loop { .. } | Drag::RulerSelect { .. } | Drag::Keys => {
                self.drag = Some(drag);
                return self.other_drag_move(ev, cx);
            }
        }
        self.drag = Some(drag);
        cx.notify();
    }

    fn drag_up(&mut self, ev: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        // the audition ends wherever the button comes up
        self.audition_stop(cx);
        let Some(drag) = self.drag.take() else { return };
        let notes = self.notes(cx);
        match drag {
            Drag::Draw { g, .. }
            | Drag::Pitch { g, .. }
            | Drag::Strum { g, .. }
            | Drag::Lane { g, .. }
            | Drag::Resize { g, .. } => self.commit(g, cx),
            Drag::Move { g, origin, copy, delta, clicked, toggle, .. } => {
                if copy && delta != (0, 0) {
                    let ch = Self::temps(origin.len())
                        .into_iter()
                        .zip(origin.iter())
                        .map(|(t, (_, x))| {
                            (
                                t,
                                Some(Note {
                                    start: x.start + delta.0,
                                    key: (x.key as i32 + delta.1).clamp(0, 127) as u8,
                                    ..x.clone()
                                }),
                            )
                        })
                        .collect();
                    let new = self.transact("Copy notes", ch, cx);
                    self.set_sel(new.into_iter().collect());
                } else if g.is_none() && delta == (0, 0) {
                    // a click: Ctrl/Cmd toggles, a plain click selects only this note
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
                self.commit(g, cx);
            }
            Drag::Erase { gone } => {
                self.transact("Erase notes", gone.into_iter().map(|n| (n, None)).collect(), cx);
            }
            Drag::Rect { from, to, toggle, before } => {
                let t0 = self.x_to_tick(from.x.as_f32().min(to.x.as_f32()));
                let t1 = self.x_to_tick(from.x.as_f32().max(to.x.as_f32()));
                let k0 = self.y_to_key(from.y.as_f32().max(to.y.as_f32()));
                let k1 = self.y_to_key(from.y.as_f32().min(to.y.as_f32()));
                let mut s = (*before).clone();
                for (n, x) in notes.iter() {
                    let inside =
                        (x.key as i32) >= k0 && (x.key as i32) <= k1 && (x.end() as f64) > t0 && (x.start as f64) < t1;
                    if inside && !(toggle && s.remove(n)) {
                        s.insert(*n);
                    }
                }
                self.set_sel(s);
            }
            Drag::Knife { from, to } => self.knife(
                from,
                to,
                holds("pr.knife.drop_short", &ev.modifiers),
                holds("pr.knife.free", &ev.modifiers),
                cx,
            ),
            Drag::Loop { .. } | Drag::RulerSelect { .. } | Drag::Keys => {}
        }
        self.pressed.clear();
        cx.notify();
    }

    /// The knife (F-NOT-005): cuts the selected notes (or all) along the line, diagonal or not; `drop_short` (Shift)
    /// removes the shorter piece; `free` (Ctrl) cuts off the grid
    fn knife(&mut self, from: Point<Pixels>, to: Point<Pixels>, drop_short: bool, free: bool, cx: &mut Context<Self>) {
        let notes = self.notes(cx);
        let which: Vec<u64> =
            if self.sel.is_empty() { notes.keys().copied().collect() } else { self.sel.iter().copied().collect() };
        let (fx, fy, tx, ty) = (from.x.as_f32(), from.y.as_f32(), to.x.as_f32(), to.y.as_f32());
        let mut temps = Self::temps(which.len()).into_iter();
        let mut changes = Vec::new();
        for n in which {
            let Some(x) = notes.get(&n) else { continue };
            let row = self.key_to_y(x.key as i32) + self.key_h / 2.0;
            if row < fy.min(ty) - self.key_h / 2.0 || row > fy.max(ty) + self.key_h / 2.0 {
                continue;
            }
            let lx = if (ty - fy).abs() < 1.0 { fx } else { fx + (tx - fx) * ((row - fy) / (ty - fy)).clamp(0.0, 1.0) };
            let at = if free {
                self.x_to_tick(lx).round() as i64
            } else {
                music::snap(self.x_to_tick(lx), self.grid_ticks())
            };
            changes.extend(music::split(&notes, &[n], at, &mut || temps.next().unwrap_or(NEW), drop_short));
        }
        self.transact("Knife", changes, cx);
    }

    fn grid_hover(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.drag.is_some() {
            return;
        }
        let (x, y) = Self::local(&self.grid_bounds, ev.position);
        let h = (self.x_to_tick(x), self.y_to_key(y));
        let old = self.hover;
        self.hover = Some(h);
        let notes = self.notes(cx);
        let part = |o: Option<(f64, i32)>| o.and_then(|o| self.note_at(&notes, o.0, o.1).map(|(_, p)| p));
        if old.is_none_or(|o| o.1 != h.1) || part(old) != part(Some(h)) {
            cx.notify();
        }
    }

    // --- the note edit area (F-NOT-010) ------------------------------------------------------------------------

    fn lane_value(&self, y: f32) -> f64 {
        let h = self.lane_bounds.get().size.height.as_f32() - 6.0;
        let t = (1.0 - (y - 3.0) / h).clamp(0.0, 1.0) as f64;
        match self.lane {
            LaneMode::Velocity => (t * 100.0).round() / 100.0,
            LaneMode::Panning => ((t * 2.0 - 1.0) * 100.0).round() / 100.0,
        }
    }

    fn with_lane(&self, x: &Note, v: f64) -> Note {
        match self.lane {
            LaneMode::Velocity => Note { velocity: v.clamp(0.0, 1.0), ..x.clone() },
            LaneMode::Panning => Note { pan: v.clamp(-1.0, 1.0), ..x.clone() },
        }
    }

    /// The notes whose start is between two x of the area: only the selected ones when there is a selection
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
        let (x, y) = Self::local(&self.lane_bounds, ev.position);
        let notes = self.notes(cx);
        if ev.button == MouseButton::Middle {
            // the middle button: back to the default
            let d = if self.lane == LaneMode::Velocity { 0.8 } else { 0.0 };
            let ch = self
                .lane_hits(&notes, x, x, false)
                .into_iter()
                .map(|n| (n, Some(self.with_lane(&notes[&n], d))))
                .collect();
            self.transact("Reset", ch, cx);
            return;
        }
        if fires("pr.lane.entry", ev.button, ev.click_count, &ev.modifiers) {
            // a double click asks the value (F-NOT-010)
            let v = self
                .lane_hits(&notes, x, x, false)
                .first()
                .map_or(0.8, |n| if self.lane == LaneMode::Velocity { notes[n].velocity } else { notes[n].pan });
            self.entry =
                Some((self.x_to_tick(x), ValueEntry::new(format!("{:.0}", v * 100.0), ev.position, window, cx)));
            cx.notify();
            return;
        }
        let only_under = holds("pr.lane.under", &ev.modifiers);
        let v = self.lane_value(y);
        let ch: Vec<NoteChange> = self
            .lane_hits(&notes, x, x, only_under)
            .into_iter()
            .map(|n| (n, Some(self.with_lane(&notes[&n], v))))
            .collect();
        if let Some((_, Some(n))) = ch.first() {
            self.last.velocity = n.velocity;
            self.last.pan = n.pan;
        }
        let mut g = None;
        self.gesture(&mut g, "Note velocity", ch, cx);
        self.drag = Some(Drag::Lane { g, last_x: x, only_under });
        cx.notify();
    }

    fn lane_wheel(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let (x, _) = Self::local(&self.lane_bounds, ev.position);
        let notes = self.notes(cx);
        let d = ev.delta.pixel_delta(px(16.0)).y.as_f32();
        if d.abs() < 0.5 {
            return;
        }
        let step = if d > 0.0 { 0.01 } else { -0.01 };
        let ch = self
            .lane_hits(&notes, x, x, holds("pr.lane.under", &ev.modifiers))
            .into_iter()
            .map(|n| {
                let x = &notes[&n];
                let v = if self.lane == LaneMode::Velocity { x.velocity + step } else { x.pan + step };
                (n, Some(self.with_lane(x, v)))
            })
            .collect();
        self.transact("Note velocity", ch, cx);
    }

    fn apply_entry(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some((t, _)) = self.entry.take() else { return };
        let Ok(v) = text.trim().replace(',', ".").parse::<f64>() else { return };
        let notes = self.notes(cx);
        let x = self.tick_to_x(t);
        let ch = self
            .lane_hits(&notes, x, x, false)
            .into_iter()
            .map(|n| (n, Some(self.with_lane(&notes[&n], v / 100.0))))
            .collect();
        self.transact("Note velocity", ch, cx);
    }

    // --- the ruler and the keyboard ------------------------------------------------------------------------------

    fn ruler_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let (x, _) = Self::local(&self.ruler_bounds, ev.position);
        let tick = self.x_to_tick(x);
        let t = music::snap(tick, self.grid_ticks()).max(0);
        if holds("pr.ruler.select", &ev.modifiers) {
            self.drag = Some(Drag::RulerSelect { from: tick });
        } else if fires("pr.ruler.loop", ev.button, ev.click_count, &ev.modifiers) {
            // the loop points: the right button (or Shift) moves the nearer one
            let which = if (t - self.play.loop_a).abs() <= (t - self.play.loop_b).abs() { 0 } else { 1 };
            self.play.loop_on = true;
            self.drag = Some(Drag::Loop { which });
            self.set_loop(which, t);
        } else {
            // a click moves the item's cursor (and the song's, to the same place)
            self.play.cursor = t;
            if self.play.since.is_some() {
                self.play.since = Some((HostTime::now(), t));
            }
            if self.rec == Rec::Step {
                self.step_action("notes.step_record", vec![("at", Value::int(t))], cx);
            }
            if let Some(c) = self.clip(cx) {
                self.scx.core.update(cx, |core, _| {
                    core.report(|pt| pt.run_action("transport.locate".into(), Value::int(c.start + t)));
                });
            }
        }
        cx.notify();
    }

    fn set_loop(&mut self, which: u8, t: i64) {
        let g = self.grid_ticks();
        if which == 0 {
            self.play.loop_a = t.min(self.play.loop_b - g).max(0);
        } else {
            self.play.loop_b = t.max(self.play.loop_a + g);
        }
    }

    fn other_drag_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(mut drag) = self.drag.take() else { return };
        let notes = self.notes(cx);
        match &mut drag {
            Drag::Lane { g, last_x, only_under } => {
                let (x, y) = Self::local(&self.lane_bounds, ev.position);
                let v = self.lane_value(y);
                let ch = self
                    .lane_hits(&notes, *last_x, x, *only_under)
                    .into_iter()
                    .map(|n| (n, Some(self.with_lane(&notes[&n], v))))
                    .collect();
                *last_x = x;
                self.gesture(g, "Note velocity", ch, cx);
            }
            Drag::Loop { which } => {
                let (x, _) = Self::local(&self.ruler_bounds, ev.position);
                let t = music::snap(self.x_to_tick(x), self.grid_ticks()).max(0);
                let w = *which;
                self.set_loop(w, t);
            }
            Drag::RulerSelect { from } => {
                let (x, _) = Self::local(&self.ruler_bounds, ev.position);
                let t = self.x_to_tick(x);
                let (a, b) = (from.min(t), from.max(t));
                self.set_sel(
                    notes
                        .iter()
                        .filter(|(_, n)| (n.start as f64) >= a && (n.start as f64) < b)
                        .map(|(n, _)| *n)
                        .collect(),
                );
            }
            Drag::Keys => {
                let (_, y) = Self::local(&self.keys_bounds, ev.position);
                let k = self.y_to_key(y).clamp(0, 127) as u8;
                if !self.pressed.contains(&k) {
                    self.pressed = [k].into_iter().collect(); // a glissando
                    self.audition(&[k], cx);
                }
            }
            _ => {}
        }
        self.drag = Some(drag);
        cx.notify();
    }

    fn keys_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let (x, y) = Self::local(&self.keys_bounds, ev.position);
        let k = self.y_to_key(y).clamp(0, 127) as u8;
        if ev.button == MouseButton::Right {
            self.menu = Some((Menu::Keys(k), ev.position));
        } else {
            // the velocity is the horizontal position on the key (F-NOT-015)
            self.last.velocity = ((x / KEYS_W) as f64).clamp(0.1, 1.0);
            self.pressed = [k].into_iter().collect();
            self.audition(&[k], cx);
            self.drag = Some(Drag::Keys);
        }
        cx.notify();
    }

    // --- menus -------------------------------------------------------------------------------------------------

    fn open_menu(m: Menu) -> impl Fn(&mut PianoRoll, &MouseDownEvent, &mut Window, &mut Context<PianoRoll>) {
        move |pr, ev, _, cx| {
            pr.menu =
                if pr.menu.is_some_and(|x| x.0 == m) { None } else { Some((m, ev.position + point(px(0.), px(10.)))) };
            cx.notify();
        }
    }

    fn midi_items(&self, cx: &gpui::App) -> Vec<(NodeId, String, u32)> {
        let root = self.scx.core.read(cx).port().root();
        model::tracks(root)
            .iter()
            .flat_map(|t| {
                t.items
                    .iter()
                    .filter(|i| !i.audio)
                    .map(move |i| (i.id, format!("{} · {}", t.name, i.name), track_color(t.index)))
            })
            .collect()
    }

    fn menu_items(&self, m: Menu, cx: &gpui::App) -> Vec<MenuItem> {
        match m {
            Menu::Quantize => vec![
                MenuItem::new(tr("Quantize"), false),
                MenuItem::new(tr("Quantize positions"), false),
                MenuItem::new(tr("Quantize lengths"), false),
            ],
            Menu::Tools => [
                ("Glue", false),
                ("Knife", self.tool == Tool::Knife),
                ("Strum", self.tool == Tool::Strum),
                ("Fill", false),
                ("Cut overlaps", false),
                ("Min length as last", false),
                ("Max length as last", false),
                ("Reverse Notes", false),
            ]
            .iter()
            .map(|(l, on)| MenuItem::new(tr(l), *on))
            .collect(),
            Menu::ZoomX => ZOOM_X
                .iter()
                .enumerate()
                .map(|(i, z)| MenuItem::new(format!("{}%", z * 100.0), i == self.zoom_x))
                .collect(),
            Menu::ZoomY => ZOOM_Y
                .iter()
                .enumerate()
                .map(|(i, z)| MenuItem::new(format!("{}%", z * 100.0), i == self.zoom_y))
                .collect(),
            Menu::Grid => std::iter::once(MenuItem::new(tr("Lock to the note"), self.lock))
                .chain(GRIDS.iter().enumerate().map(|(i, (l, _))| MenuItem::new(*l, !self.lock && i == self.grid)))
                .collect(),
            Menu::Length => std::iter::once(MenuItem::new(tr("Last note"), self.length.is_none()))
                .chain(GRIDS.iter().enumerate().map(|(i, (l, _))| MenuItem::new(*l, self.length == Some(i))))
                .collect(),
            Menu::Key => std::iter::once(MenuItem::new(tr("No key"), self.scale.is_none()))
                .chain(
                    KEY_NAMES
                        .iter()
                        .enumerate()
                        .map(|(i, k)| MenuItem::new(*k, self.scale.is_some() && i as u8 == self.root)),
                )
                .collect(),
            Menu::Scale => std::iter::once(MenuItem::new(tr("No scale"), self.scale.is_none()))
                .chain(SCALES.iter().enumerate().map(|(i, (n, _))| MenuItem::new(tr(n), self.scale == Some(i))))
                .collect(),
            Menu::Chord => std::iter::once(MenuItem::new(tr("No chord"), self.chord.is_none()))
                .chain(CHORDS.iter().enumerate().map(|(i, (n, _))| MenuItem::new(tr(n), self.chord == Some(i))))
                .collect(),
            Menu::Snap => vec![
                MenuItem::new(tr("Nudge"), self.snap == SnapMode::Nudge),
                MenuItem::new(tr("Snap"), self.snap == SnapMode::Snap),
            ],
            Menu::Ghost => std::iter::once(MenuItem::new(tr("Clear ghost notes"), false))
                .chain(
                    self.midi_items(cx)
                        .into_iter()
                        .map(|(id, l, col)| MenuItem::new(l, self.ghost == Some(id)).color(col)),
                )
                .collect(),
            Menu::Lane => vec![
                MenuItem::new(tr("Velocity"), self.lane == LaneMode::Velocity),
                MenuItem::new(tr("Panning"), self.lane == LaneMode::Panning),
            ],
            Menu::Keys(k) => vec![
                MenuItem::new(tr("Mark/unmark current semitone"), self.marked.contains(&k)),
                MenuItem::new(tr("Mark/unmark all corresponding octave semitones"), false),
                MenuItem::new(tr("Mark current scale"), false),
                MenuItem::new(tr("Mark current chord"), false),
                MenuItem::new(tr("Unmark all"), false),
                MenuItem::new(tr("Select all notes on this key"), false),
            ],
        }
    }

    fn pick(&mut self, m: Menu, i: usize, cx: &mut Context<Self>) {
        self.menu = None;
        let notes = self.notes(cx);
        match m {
            Menu::Quantize => self.quantize(i, cx),
            Menu::Tools => match i {
                1 => self.tool = Tool::Knife,
                2 => self.tool = Tool::Strum,
                _ => self.note_tool(i, cx),
            },
            Menu::ZoomX => {
                self.zoom_x = i;
                self.tpp = TPP_100 / ZOOM_X[i];
            }
            Menu::ZoomY => {
                self.zoom_y = i;
                self.key_h = KEY_H_100 * ZOOM_Y[i];
            }
            Menu::Grid => {
                self.lock = i == 0;
                if let Some(g) = i.checked_sub(1) {
                    self.grid = g;
                }
                self.remember_grid(cx);
            }
            Menu::Length => {
                self.length = i.checked_sub(1);
                self.remember_grid(cx);
            }
            Menu::Key => match i.checked_sub(1) {
                None => self.scale = None,
                Some(r) => {
                    self.root = r as u8;
                    self.scale = self.scale.or(Some(0));
                }
            },
            Menu::Scale => self.scale = i.checked_sub(1),
            Menu::Chord => self.chord = i.checked_sub(1),
            Menu::Snap => self.snap = if i == 0 { SnapMode::Nudge } else { SnapMode::Snap },
            Menu::Ghost => self.ghost = i.checked_sub(1).and_then(|k| self.midi_items(cx).get(k).map(|x| x.0)),
            Menu::Lane => self.lane = if i == 0 { LaneMode::Velocity } else { LaneMode::Panning },
            Menu::Keys(k) => match i {
                0 => {
                    if !self.marked.remove(&k) {
                        self.marked.insert(k);
                    }
                }
                1 => {
                    let on = !self.marked.contains(&k);
                    for x in (0..128u8).filter(|x| x % 12 == k % 12) {
                        if on {
                            self.marked.insert(x);
                        } else {
                            self.marked.remove(&x);
                        }
                    }
                }
                2 => {
                    if let Some(s) = self.scale {
                        self.marked = (0..128u8).filter(|x| music::in_scale(*x, self.root, s)).collect();
                    }
                }
                3 => self.marked = music::chord_keys(k, self.chord.unwrap_or(0)).into_iter().collect(),
                4 => self.marked.clear(),
                _ => self.set_sel(notes.iter().filter(|(_, x)| x.key == k).map(|(n, _)| *n).collect()),
            },
        }
        cx.notify();
    }

    fn quantize(&mut self, i: usize, cx: &mut Context<Self>) {
        let notes = self.notes(cx);
        let which = self.targets(&notes);
        let what = [QuantizeWhat::Both, QuantizeWhat::Starts, QuantizeWhat::Lengths][i.min(2)];
        self.transact("Quantize", music::quantize(&notes, &which, self.grid_ticks(), what, 1.0), cx);
    }

    /// The note tools (F-NOT-011), by their place in the menu
    fn note_tool(&mut self, i: usize, cx: &mut Context<Self>) {
        let notes = self.notes(cx);
        let which = self.targets(&notes);
        let end = self.clip(cx).map_or(BAR, |c| c.length);
        let (label, ch) = match i {
            0 => ("Glue", music::glue(&notes, &which)),
            3 => ("Fill", music::fill(&notes, &which, end)),
            4 => ("Cut overlaps", music::cut_overlaps(&notes, &which)),
            5 => ("Min length as last", music::constrain_lengths(&notes, &which, self.last.length, true)),
            6 => ("Max length as last", music::constrain_lengths(&notes, &which, self.last.length, false)),
            7 => ("Reverse Notes", music::reverse(&notes, &which)),
            _ => return,
        };
        self.transact(label, ch, cx);
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

    /// A bar left or right: the length of the bar the first selected note is in, by the tempo map (F-NOT-013)
    fn shift_bar(&mut self, dir: i64, cx: &mut Context<Self>) {
        let notes = self.notes(cx);
        let at = self.selected(&notes).iter().map(|(_, x)| x.start).min().unwrap_or(0);
        let start = self.clip(cx).map_or(0, |c| c.start);
        let bars = piano_roll_bars::bars(self.scx.core.read(cx).port().root(), start, at as f64, at as f64);
        let len = bars.first().map_or(BAR, |b| b.len);
        self.shift(dir * len, cx);
    }

    /// Alt+left and Alt+right: the clip before or after this one on its track (F-NOT-009)
    fn neighbour_clip(&mut self, dir: i64, cx: &mut Context<Self>) {
        let Some(me) = self.clip else { return };
        let root = self.scx.core.read(cx).port().root().clone();
        let next = model::tracks(&root).into_iter().find(|t| t.items.iter().any(|i| i.id == me)).and_then(|t| {
            let mut midi: Vec<_> = t.items.iter().filter(|i| !i.audio).collect();
            midi.sort_by_key(|i| i.start);
            let at = midi.iter().position(|i| i.id == me)? as i64 + dir;
            usize::try_from(at).ok().and_then(|k| midi.get(k)).map(|i| i.id)
        });
        if let Some(id) = next {
            self.clip = Some(id);
            self.set_sel(BTreeSet::new());
            self.scroll = 0.0;
            self.fitted = false;
            cx.notify();
        }
    }

    fn copy(&mut self, cut: bool, cx: &mut Context<Self>) {
        let notes = self.notes(cx);
        let sel = self.selected(&notes);
        let a = sel.iter().map(|(_, x)| x.start).min().unwrap_or(0);
        self.clipboard = sel.iter().map(|(_, x)| Note { start: x.start - a, ..x.clone() }).collect();
        if !self.clipboard.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(clip_text::to_text(&self.clipboard)));
        }
        if cut {
            self.transact("Cut", sel.into_iter().map(|(n, _)| (n, None)).collect(), cx);
            self.set_sel(BTreeSet::new());
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        // the system's clipboard first: notes copied in another project or instance (F-NOT-019)
        if let Some(notes) = cx.read_from_clipboard().and_then(|c| c.text()).and_then(|t| clip_text::from_text(&t)) {
            self.clipboard = notes;
        }
        if self.clipboard.is_empty() {
            return;
        }
        // at the item's cursor, on the grid; what was pasted is the new selection (F-NOT-019)
        let at = music::snap_floor(self.play.at(HostTime::now()) as f64, self.grid_ticks());
        let ch = Self::temps(self.clipboard.len())
            .into_iter()
            .zip(self.clipboard.iter())
            .map(|(n, x)| (n, Some(Note { start: x.start + at, ..x.clone() })))
            .collect();
        let new = self.transact("Paste", ch, cx);
        self.set_sel(new.into_iter().collect());
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        if self.rec == Rec::Step && self.sel.is_empty() {
            // the last chord goes and the cursor comes back (the core's `notes.step_back`)
            self.step_action("notes.step_back", Vec::new(), cx);
        } else {
            let notes = self.notes(cx);
            let sel = self.selected(&notes);
            self.transact("Delete notes", sel.into_iter().map(|(n, _)| (n, None)).collect(), cx);
            self.set_sel(BTreeSet::new());
        }
        cx.notify();
    }

    // --- the keyboard: the piano keys (the commands are actions of the registry) ---------------------------------

    fn key_down(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        // keymap-ok: the piano keys are a key down and up pair that sounds a note, not a command
        let (k, m) = (ev.keystroke.key.as_str(), ev.keystroke.modifiers);
        if m.control || m.alt || m.shift || m.platform {
            return;
        }
        let Some(off) = piano_key(k) else { return };
        if !ev.is_held {
            let key = (self.octave * 12 + off).clamp(0, 127) as u8;
            self.note_on(k, key, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn key_up(&mut self, ev: &KeyUpEvent, cx: &mut Context<Self>) {
        // keymap-ok: the piano keys
        if let Some(h) = self.held.remove(&ev.keystroke.key) {
            self.note_off(h, cx);
        }
    }

    fn rec_tick(&self, cx: &gpui::App) -> Option<i64> {
        let (t, playing) = self.play_tick(cx, HostTime::now());
        t.filter(|_| playing).map(|t| t.round() as i64)
    }

    fn note_on(&mut self, k: &str, key: u8, cx: &mut Context<Self>) {
        self.pressed.insert(key);
        // the computer keyboard sounds the instrument in every mode (F-NOT-023)
        let v = self.last.velocity;
        self.sound(key, v, cx);
        let start = match self.rec {
            // the chord is sent to the core when the last key comes up
            Rec::Step => {
                if !self.step_chord.contains(&key) {
                    self.step_chord.push(key);
                }
                -1
            }
            Rec::Live | Rec::Accompany => self.rec_tick(cx).unwrap_or(-1),
            Rec::Off => -1,
        };
        self.held.insert(k.to_string(), (key, start));
        cx.notify();
    }

    fn note_off(&mut self, (key, start): (u8, i64), cx: &mut Context<Self>) {
        self.pressed.remove(&key);
        self.silence(key, cx);
        match self.rec {
            Rec::Step if self.held.is_empty() && !self.step_chord.is_empty() => {
                let keys =
                    Value::list(std::mem::take(&mut self.step_chord).into_iter().map(|k| Value::int(i64::from(k))));
                let v = self.last.velocity;
                self.step_action(
                    "notes.step_keys",
                    vec![("keys", keys), ("velocity", Value::real(v).unwrap_or(Value::NULL))],
                    cx,
                );
            }
            Rec::Live | Rec::Accompany if start >= 0 => {
                let end = self.rec_tick(cx).unwrap_or(start + BEAT / 4);
                let len = if end > start { end - start } else { BEAT / 8 };
                let x = Note {
                    key,
                    start,
                    length: len,
                    velocity: self.last.velocity,
                    pan: 0.0,
                    bend: Vec::new(),
                    mpe: Default::default(),
                };
                self.transact("Record notes", vec![(NEW, Some(x))], cx);
            }
            _ => {}
        }
        cx.notify();
    }

    fn release_all(&mut self, cx: &mut Context<Self>) {
        let held: Vec<(u8, i64)> = self.held.drain().map(|(_, v)| v).collect();
        for h in held {
            self.note_off(h, cx);
        }
        self.step_chord.clear();
        // whatever the mouse auditions, and anything a lost note-off left, ends too
        self.audition.clear();
        let core = self.scx.core.clone();
        self.live.release_all(&core, cx);
        self.pressed.clear();
    }

    /// Points the core's next recording at this clip (MIDI devices write into it while the song records) or frees it
    fn record_into(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(item) = self.clip else { return };
        let params = if on { Value::object([("item", Value::text(item.to_string()))]) } else { Value::empty_object() };
        self.scx.core.update(cx, |core, _| {
            core.report(|pt| pt.run_action("notes.record_into".into(), params));
        });
    }

    fn set_rec(&mut self, r: Rec, cx: &mut Context<Self>) {
        let was_step = self.rec == Rec::Step;
        let was_accompany = self.rec == Rec::Accompany;
        self.rec = if self.rec == r { Rec::Off } else { r };
        if was_accompany != (self.rec == Rec::Accompany) {
            self.record_into(self.rec == Rec::Accompany, cx);
        }
        if was_step && self.rec != Rec::Step {
            self.step_chord.clear();
            self.scx.core.update(cx, |core, _| {
                core.report(|pt| pt.run_action("notes.step_stop".into(), Value::NULL));
            });
        }
        match self.rec {
            Rec::Live if self.play.since.is_none() => self.toggle_play_item(cx),
            Rec::Accompany => self.scx.core.update(cx, |core, _| {
                core.report(|pt| pt.run_action("transport.play".into(), Value::NULL));
            }),
            Rec::Step => {
                let (at, step) = (self.play.cursor, self.new_len());
                self.step_action("notes.step_record", vec![("at", Value::int(at)), ("step", Value::int(step))], cx);
            }
            _ => {}
        }
        cx.notify();
    }

    // --- scrolling and zoom (F-NOT-013) -------------------------------------------------------------------------

    fn on_scroll(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.0));
        let (dx, dy) = (d.x.as_f32(), d.y.as_f32());
        let (x, _) = Self::local(&self.grid_bounds, ev.position);
        let ids = ["pr.view.grid_wheel", "pr.view.length_wheel", "pr.view.zoom", "pr.view.scroll_h", "pr.view.scroll"];
        match keymap::pick_wheel(CTX, &ids, &ev.modifiers) {
            Some("pr.view.grid_wheel") => {
                self.grid = (self.grid as i32 - dy.signum() as i32).clamp(0, GRIDS.len() as i32 - 1) as usize;
            }
            Some("pr.view.length_wheel") => {
                let l = self.length.map_or(0, |i| i + 1) as i32 - dy.signum() as i32;
                self.length = (l.clamp(0, GRIDS.len() as i32) as usize).checked_sub(1);
            }
            Some("pr.view.zoom") => {
                let anchor = self.x_to_tick(x);
                self.tpp = (self.tpp * (-dy * 0.01).exp() as f64).clamp(TPP_100 / 8.0, TPP_100 * 8.0);
                self.scroll = (anchor - x as f64 * self.tpp).max(0.0);
            }
            Some(which) => {
                // sideways: the wheel's turn goes to the horizontal scroll when the mouse has no sideways axis
                let (dx, dy) = if which == "pr.view.scroll_h" && dx.abs() < 0.01 { (dy, 0.0) } else { (dx, dy) };
                self.scroll = (self.scroll - dx as f64 * self.tpp).max(0.0);
                let max = (128.0 * self.key_h - self.grid_bounds.get().size.height.as_f32()).max(0.0);
                self.scroll_y = (self.scroll_y - dy).clamp(0.0, max);
            }
            None => {}
        }
        cx.notify();
    }

    fn on_pinch(&mut self, ev: &PinchEvent, cx: &mut Context<Self>) {
        let (x, _) = Self::local(&self.grid_bounds, ev.position);
        let anchor = self.x_to_tick(x);
        self.tpp = (self.tpp / (1.0 + ev.delta as f64).max(0.1)).clamp(TPP_100 / 8.0, TPP_100 * 8.0);
        self.scroll = (anchor - x as f64 * self.tpp).max(0.0);
        cx.notify();
    }

    fn set_curve(&mut self, c: Curve, cx: &mut Context<Self>) {
        self.curve = c;
        cx.notify();
    }

    fn set_tool(&mut self, t: Tool, cx: &mut Context<Self>) {
        self.tool = t;
        cx.notify();
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Rendering

/// What the grid's canvas paints, captured once per frame (no copy of the notes: the map is shared)
struct GridPaint {
    notes: Arc<NoteMap>,
    ghost: Option<Arc<NoteMap>>,
    sel: Rc<BTreeSet<u64>>,
    /// notes being erased (hidden until the release removes them) and a copy being dragged
    gone: BTreeSet<u64>,
    copies: Vec<Note>,
    scroll: f64,
    tpp: f64,
    key_h: f32,
    scroll_y: f32,
    grid: i64,
    /// The bars by the tempo map around the visible part (F-NOT-013)
    bars: Arc<Vec<Bar>>,
    length: i64,
    loop_len: Option<i64>,
    scale: Option<(u8, usize)>,
    marked: BTreeSet<u8>,
    play: Option<f64>,
    item_loop: Option<(i64, i64)>,
    step_cursor: Option<(i64, i64)>,
    rect: Option<(Point<Pixels>, Point<Pixels>)>,
    knife: Option<(Point<Pixels>, Point<Pixels>)>,
}

impl GridPaint {
    fn x(&self, t: f64) -> f32 {
        ((t - self.scroll) / self.tpp) as f32
    }
    fn y(&self, k: i32) -> f32 {
        (127 - k) as f32 * self.key_h - self.scroll_y
    }

    fn note(&self, window: &mut Window, cx: &mut gpui::App, o: Point<Pixels>, x: &Note, selected: bool, alpha: f32) {
        let (nx, ny) = (self.x(x.start as f64), self.y(x.key as i32));
        let nw = ((x.length as f64 / self.tpp) as f32).max(2.0);
        let bb = Bounds::new(point(o.x + px(nx), o.y + px(ny + 0.5)), size(px(nw), px(self.key_h - 1.0)));
        // the legacy: the note's opacity shows its velocity; the selected notes in the selection's color
        let body = with_alpha(
            if selected { THEME.pr_note_sel } else { THEME.pr_note },
            alpha * (0.35 + 0.65 * x.velocity as f32),
        );
        window.paint_quad(gpui::quad(bb, px(2.0), body, px(1.0), with_alpha(THEME.line, 0.9), Default::default()));
        if x.bend.iter().any(|p| p.semitones != 0.0) {
            let mut pb = PathBuilder::stroke(px(1.5));
            let mid = ny + self.key_h / 2.0;
            let mut pts: Vec<(f32, f32)> = x.bend.iter().map(|p| (p.at as f32, p.semitones as f32)).collect();
            pts.push((x.length as f32, x.bend.last().map_or(0.0, |p| p.semitones as f32)));
            for (i, (at, st)) in pts.iter().enumerate() {
                let p = point(o.x + px(nx + at / self.tpp as f32), o.y + px(mid - st * self.key_h));
                if i == 0 { pb.move_to(p) } else { pb.line_to(p) }
            }
            if let Ok(p) = pb.build() {
                window.paint_path(p, c(THEME.yellow));
            }
        }
        // the MPE curves over the note's height: pressure in green, timbre in red
        for (points, color) in [(&x.mpe.pressure, THEME.green), (&x.mpe.timbre, THEME.red)] {
            if points.is_empty() {
                continue;
            }
            let mut pb = PathBuilder::stroke(px(1.0));
            let mut pts: Vec<(f32, f32)> = points.iter().map(|p| (p.at as f32, p.value as f32)).collect();
            pts.push((x.length as f32, points.last().map_or(0.0, |p| p.value as f32)));
            for (i, (at, v)) in pts.iter().enumerate() {
                let p = point(
                    o.x + px(nx + at / self.tpp as f32),
                    o.y + px(ny + self.key_h - 1.0 - v * (self.key_h - 2.0)),
                );
                if i == 0 { pb.move_to(p) } else { pb.line_to(p) }
            }
            if let Ok(p) = pb.build() {
                window.paint_path(p, c(color));
            }
        }
        if nw > 24.0 && self.key_h >= 10.0 && crate::ui::note_names() {
            let fs = (self.key_h - 3.0).clamp(7.0, 10.0);
            let y = o.y + px(ny + (self.key_h - fs) / 2.0 - 1.0);
            paint_text(window, cx, key_name(x.key).into(), o.x + px(nx + 3.0), y, fs, c(THEME.on_accent));
        }
    }

    fn paint(&self, b: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        let o = b.origin;
        let (w, h) = (b.size.width.as_f32(), b.size.height.as_f32());
        let r = |x: f32, y: f32, ww: f32, hh: f32| Bounds::new(point(o.x + px(x), o.y + px(y)), size(px(ww), px(hh)));
        window.paint_quad(fill(b, c(THEME.pr_bg)));
        let first = (127 - (self.scroll_y / self.key_h).floor() as i32).min(127);
        let last = (127 - ((self.scroll_y + h) / self.key_h).ceil() as i32).max(0);
        for k in (last..=first).rev() {
            let y = self.y(k);
            let ku = k as u8;
            if music::is_black(ku) {
                window.paint_quad(fill(r(0.0, y, w, self.key_h), with_alpha(THEME.window, 0.45)));
            }
            if let Some((root, s)) = self.scale
                && music::in_scale(ku, root, s)
            {
                window.paint_quad(fill(r(0.0, y, w, self.key_h), with_alpha(THEME.green, 0.06)));
            }
            if self.marked.contains(&ku) {
                window.paint_quad(fill(r(0.0, y, w, self.key_h), gpui::rgba(THEME.pr_marked)));
            }
            if ku.is_multiple_of(12) {
                window.paint_quad(fill(r(0.0, y + self.key_h - 1.0, w, 1.0), c(THEME.bg3)));
            }
        }
        let (t0, t1) = (self.scroll, self.scroll + w as f64 * self.tpp);
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
        for bar in self.bars.iter() {
            if (bar.beat as f64 / self.tpp) >= 6.0 {
                let mut t = bar.start + bar.beat;
                while t < bar.start + bar.len {
                    window.paint_quad(fill(r(self.x(t as f64), 0.0, 1.0, h), c(THEME.pr_beat)));
                    t += bar.beat;
                }
            }
            window.paint_quad(fill(r(self.x(bar.start as f64), 0.0, 1.0, h), c(THEME.pr_bar)));
        }
        // past the repeating part (a pattern) and past the item's end
        if let Some(l) = self.loop_len {
            let lx = self.x(l as f64).max(0.0);
            if lx < w {
                window.paint_quad(fill(r(lx, 0.0, w - lx, h), with_alpha(THEME.window, 0.25)));
            }
        }
        let ex = self.x(self.length as f64).max(0.0);
        if ex < w {
            window.paint_quad(fill(r(ex, 0.0, w - ex, h), with_alpha(THEME.window, 0.5)));
        }
        if let Some((a, z)) = self.item_loop {
            let (xa, xz) = (self.x(a as f64), self.x(z as f64));
            window.paint_quad(fill(r(xa, 0.0, (xz - xa).max(0.0), h), gpui::rgba(THEME.loop_range)));
        }
        if let Some((s, l)) = self.step_cursor {
            let (xa, xz) = (self.x(s as f64), self.x((s + l) as f64));
            window.paint_quad(fill(r(xa, 0.0, (xz - xa).max(1.0), h), gpui::rgba(THEME.punch_range)));
        }
        let visible = |x: &Note| {
            (x.start as f64) < t1 && x.end() as f64 > t0 && (x.key as i32) >= last && (x.key as i32) <= first
        };
        if let Some(g) = &self.ghost {
            for x in g.values().filter(|x| visible(x)) {
                let (nx, ny) = (self.x(x.start as f64), self.y(x.key as i32));
                let nw = ((x.length as f64 / self.tpp) as f32).max(2.0);
                let bb = r(nx, ny + 1.0, nw, self.key_h - 2.0);
                window.paint_quad(gpui::quad(
                    bb,
                    px(2.0),
                    with_alpha(THEME.pr_ghost, 0.6),
                    px(1.0),
                    c(THEME.pr_ghost),
                    Default::default(),
                ));
            }
        }
        for (n, x) in self.notes.iter().filter(|(n, x)| visible(x) && !self.gone.contains(n)) {
            self.note(window, cx, o, x, self.sel.contains(n), 1.0);
        }
        for x in self.copies.iter().filter(|x| visible(x)) {
            self.note(window, cx, o, x, true, 0.6);
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
        let now = HostTime::now();
        // I-3.6: after an undo, selected notes that are gone are dropped; a removed item empties the editor
        {
            let port = self.scx.core.read(cx).port();
            if port.drain(&mut self.feed).is_some() {
                match self.clip.and_then(|id| self.clips.borrow_mut().get(port.root(), id)).map(|c| c.notes) {
                    None => self.clip = None,
                    Some(n) if self.sel.iter().any(|k| !n.contains_key(k)) => {
                        self.sel = Rc::new(self.sel.iter().copied().filter(|k| n.contains_key(k)).collect());
                    }
                    _ => {}
                }
            }
        }
        if !self.fitted && self.grid_bounds.get().size.height.as_f32() > 10.0 {
            let h = self.grid_bounds.get().size.height.as_f32();
            self.fit(h, cx);
        }
        let (play, moving) = self.play_tick(cx, now);
        if self.item_audio {
            let playing = self.scx.core.read(cx).port().telemetry().position.playing;
            if playing {
                self.audio_ran = true;
                if play.is_none() {
                    // an item that does not loop played to its end
                    self.stop(cx);
                }
            } else if self.audio_ran {
                self.item_audio = false;
                self.audio_ran = false;
            }
        }
        if moving {
            window.request_animation_frame();
        }
        let entity = cx.entity();
        let clip = self.clip(cx);
        let root = div()
            .id("piano-roll")
            .size_full()
            .flex()
            .flex_col()
            .bg(c(THEME.bg1))
            .text_color(c(THEME.text))
            .track_focus(&self.focus)
            .key_context("PianoRoll")
            .on_action(cx.listener(|pr, _: &PrPlay, _, cx| pr.toggle_play_item(cx)))
            .on_action(cx.listener(|pr, _: &PrModeDraw, _, cx| pr.set_tool(Tool::Draw, cx)))
            .on_action(cx.listener(|pr, _: &PrModeErase, _, cx| pr.set_tool(Tool::Erase, cx)))
            .on_action(cx.listener(|pr, _: &PrModeSelect, _, cx| pr.set_tool(Tool::Select, cx)))
            .on_action(cx.listener(|pr, _: &PrModePitch, _, cx| pr.set_tool(Tool::Pitch, cx)))
            .on_action(cx.listener(|pr, _: &PrModeKnife, _, cx| pr.set_tool(Tool::Knife, cx)))
            .on_action(cx.listener(|pr, _: &PrModeStrum, _, cx| pr.set_tool(Tool::Strum, cx)))
            .on_action(cx.listener(|pr, _: &PrGlue, _, cx| pr.note_tool(0, cx)))
            .on_action(cx.listener(|pr, _: &PrFill, _, cx| pr.note_tool(3, cx)))
            .on_action(cx.listener(|pr, _: &PrCutOverlaps, _, cx| pr.note_tool(4, cx)))
            .on_action(cx.listener(|pr, _: &PrReverse, _, cx| pr.note_tool(7, cx)))
            .on_action(cx.listener(|pr, _: &PrSelectAll, _, cx| {
                let all = pr.notes(cx).keys().copied().collect();
                pr.set_sel(all);
                cx.notify();
            }))
            .on_action(cx.listener(|pr, _: &PrSelectNone, _, cx| {
                pr.set_sel(BTreeSet::new());
                pr.menu = None;
                pr.entry = None;
                pr.drag = None;
                cx.notify();
            }))
            .on_action(cx.listener(|pr, _: &PrCopy, _, cx| pr.copy(false, cx)))
            .on_action(cx.listener(|pr, _: &PrCut, _, cx| pr.copy(true, cx)))
            .on_action(cx.listener(|pr, _: &PrPaste, _, cx| pr.paste(cx)))
            .on_action(cx.listener(|pr, _: &PrDelete, _, cx| pr.delete(cx)))
            .on_action(cx.listener(|pr, _: &PrTransposeUp, _, cx| pr.transpose(1, cx)))
            .on_action(cx.listener(|pr, _: &PrTransposeDown, _, cx| pr.transpose(-1, cx)))
            .on_action(cx.listener(|pr, _: &PrOctaveUp, _, cx| pr.transpose(12, cx)))
            .on_action(cx.listener(|pr, _: &PrOctaveDown, _, cx| pr.transpose(-12, cx)))
            .on_action(cx.listener(|pr, _: &PrMoveLeft, _, cx| pr.shift(-pr.grid_ticks(), cx)))
            .on_action(cx.listener(|pr, _: &PrMoveRight, _, cx| pr.shift(pr.grid_ticks(), cx)))
            .on_action(cx.listener(|pr, _: &PrMoveBarLeft, _, cx| pr.shift_bar(-1, cx)))
            .on_action(cx.listener(|pr, _: &PrMoveBarRight, _, cx| pr.shift_bar(1, cx)))
            .on_action(cx.listener(|pr, _: &PrPrevClip, _, cx| pr.neighbour_clip(-1, cx)))
            .on_action(cx.listener(|pr, _: &PrNextClip, _, cx| pr.neighbour_clip(1, cx)))
            // keymap-ok: the piano keys
            .on_key_down(cx.listener(|pr, ev: &KeyDownEvent, _, cx| pr.key_down(ev, cx)))
            // keymap-ok: the piano keys
            .on_key_up(cx.listener(|pr, ev: &KeyUpEvent, _, cx| pr.key_up(ev, cx)));
        self.key_actions(root, cx)
            .child(self.render_toolbars(clip.as_ref(), cx))
            .child(match clip {
                None => div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(c(THEME.text2))
                    .child(tr("No MIDI item"))
                    .into_any_element(),
                Some(clip) => div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .child(self.render_keys(cx))
                    .child(self.render_center(clip, play, cx))
                    .into_any_element(),
            })
            .children(self.drag.is_some().then(|| kit::capture(entity.downgrade(), Self::drag_move, Self::drag_up)))
            .children(self.menu.map(|(m, at)| {
                let items = self.menu_items(m, cx);
                let (e, e2) = (entity.downgrade(), entity.downgrade());
                popup(
                    "pr-menu",
                    at,
                    items,
                    move |i, _, cx| {
                        let _ = e.update(cx, |pr, cx| pr.pick(m, i, cx));
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
                let (a, b) = (entity.downgrade(), entity.downgrade());
                e.render(
                    move |text, _, cx| {
                        let _ = a.update(cx, |pr, cx| pr.apply_entry(&text, cx));
                    },
                    move |_, cx| {
                        let _ = b.update(cx, |pr, cx| {
                            pr.entry = None;
                            cx.notify();
                        });
                    },
                )
            }))
    }
}

impl PianoRoll {
    /// The legacy's two toolbar rows, with its icons
    fn render_toolbars(&self, clip: Option<&Clip>, cx: &Context<Self>) -> impl IntoElement {
        let row =
            || div().h(px(ROW_H)).flex_none().flex().flex_row().items_center().gap(px(2.)).px(px(4.)).overflow_hidden();
        let tool_button = |t: Tool, id: &'static str, icon: &'static str, help: &'static str| {
            icon_button(id, icon, self.tool == t, help).on_click(cx.listener(move |pr, _, _, cx| pr.set_tool(t, cx)))
        };
        let combo = |id: &'static str, icon: &'static str, m: Menu, text: SharedString, help: &'static str| {
            dropdown_icon(id, icon, text, false)
                .tooltip(tip(help))
                .on_mouse_down(MouseButton::Left, cx.listener(Self::open_menu(m)))
        };
        let playing = self.play.since.is_some() || self.item_audio;
        let tools_icon = match self.tool {
            Tool::Knife => "edit_knife",
            Tool::Strum => "arp_free",
            _ => "tool",
        };
        div()
            .flex_none()
            .flex()
            .flex_col()
            .bg(c(THEME.bg2))
            .border_b_1()
            .border_color(c(THEME.line))
            .child(
                row()
                    .child(
                        icon_button(
                            "pr-play",
                            if playing { "pause" } else { "play" },
                            playing,
                            "Play/pause current clip (Space)",
                        )
                        .on_click(cx.listener(|pr, _, _, cx| pr.toggle_play_item(cx))),
                    )
                    .child(
                        icon_button(
                            "pr-record",
                            "record",
                            self.rec == Rec::Live,
                            "Record notes from MIDI-device/channel-piano",
                        )
                        .on_click(cx.listener(|pr, _, _, cx| pr.set_rec(Rec::Live, cx))),
                    )
                    .child(
                        icon_button(
                            "pr-record-song",
                            "record_accompany",
                            self.rec == Rec::Accompany,
                            "Record notes from MIDI-device/channel-piano while playing song or pattern track",
                        )
                        .on_click(cx.listener(|pr, _, _, cx| pr.set_rec(Rec::Accompany, cx))),
                    )
                    .child(
                        icon_button(
                            "pr-record-step",
                            if self.rec == Rec::Step { "record_step_on" } else { "record_step_off" },
                            self.rec == Rec::Step,
                            "Record notes from MIDI-device/channel-piano, one step at the time",
                        )
                        .on_click(cx.listener(|pr, _, _, cx| pr.set_rec(Rec::Step, cx))),
                    )
                    .child(
                        icon_button("pr-stop", "stop", false, "Stop playing of current clip (Space)")
                            .on_click(cx.listener(|pr, _, _, cx| pr.stop(cx))),
                    )
                    .child(sep())
                    .child(tool_button(Tool::Draw, "pr-mode-draw", "edit_draw", "Draw mode (Shift+D)"))
                    .child(tool_button(Tool::Erase, "pr-mode-erase", "edit_erase", "Erase mode (Shift+E)"))
                    .child(tool_button(Tool::Select, "pr-mode-select", "edit_select", "Select mode (Shift+S)"))
                    .child(tool_button(Tool::Pitch, "pr-mode-pitch", "automation", "Pitch Bend mode (Shift+T)"))
                    .children((self.tool == Tool::Pitch).then(|| {
                        div().flex().flex_row().gap(px(2.)).children(Curve::ALL.map(|k| {
                            dropdown(k.id(), tr(k.label()), self.curve == k)
                                .on_click(cx.listener(move |pr, _, _, cx| pr.set_curve(k, cx)))
                        }))
                    }))
                    .child(sep())
                    .child(
                        icon_button("pr-quantize", "quantize", false, "Quantize")
                            .on_click(cx.listener(|pr, _, _, cx| pr.quantize(0, cx))),
                    )
                    .child(
                        dropdown("pr-quantize-menu", "", false)
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::open_menu(Menu::Quantize))),
                    )
                    .child(
                        dropdown_icon("pr-tools", tools_icon, "", matches!(self.tool, Tool::Knife | Tool::Strum))
                            .debug_selector(|| "pr-tools".into())
                            .tooltip(tip("Note tools"))
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::open_menu(Menu::Tools))),
                    )
                    .child(sep())
                    .child(
                        icon_button("pr-cut", "edit_cut", false, "Cut (Ctrl+X)")
                            .on_click(cx.listener(|pr, _, _, cx| pr.copy(true, cx))),
                    )
                    .child(
                        icon_button("pr-copy", "edit_copy", false, "Copy (Ctrl+C)")
                            .on_click(cx.listener(|pr, _, _, cx| pr.copy(false, cx))),
                    )
                    .child(
                        icon_button("pr-paste", "edit_paste", false, "Paste (Ctrl+V)")
                            .on_click(cx.listener(|pr, _, _, cx| pr.paste(cx))),
                    )
                    .child(div().flex_1())
                    .children(clip.map(|c| {
                        div()
                            .text_size(px(11.))
                            .text_color(c_(THEME.text2))
                            .child(format!("{} · {}", c.track_name, c.name))
                    })),
            )
            .child(
                row()
                    .child(combo(
                        "pr-zoom-x",
                        "zoom_x",
                        Menu::ZoomX,
                        format!("{}%", ZOOM_X[self.zoom_x] * 100.0).into(),
                        "Horizontal zooming",
                    ))
                    .child(combo(
                        "pr-zoom-y",
                        "zoom_y",
                        Menu::ZoomY,
                        format!("{}%", ZOOM_Y[self.zoom_y] * 100.0).into(),
                        "Vertical zooming",
                    ))
                    .child(combo(
                        "pr-grid",
                        "quantize",
                        Menu::Grid,
                        if self.lock { tr("Lock to the note").into() } else { GRIDS[self.grid].0.into() },
                        "Quantization",
                    ))
                    .child(combo(
                        "pr-length",
                        "note",
                        Menu::Length,
                        self.length
                            .map_or_else(
                                || trf("Last note ({0})", &[&format_len(self.last.length)]),
                                |i| GRIDS[i].0.to_string(),
                            )
                            .into(),
                        "Note length",
                    ))
                    .child(combo(
                        "pr-key",
                        "piano",
                        Menu::Key,
                        if self.scale.is_some() { KEY_NAMES[self.root as usize].into() } else { tr("No key").into() },
                        "Key",
                    ))
                    .child(combo(
                        "pr-scale",
                        "scale",
                        Menu::Scale,
                        self.scale.map_or(tr("No scale"), |s| tr(SCALES[s].0)).into(),
                        "Scale",
                    ))
                    .child(combo(
                        "pr-chord",
                        "chord",
                        Menu::Chord,
                        self.chord.map_or(tr("No chord"), |s| tr(CHORDS[s].0)).into(),
                        "Chord",
                    ))
                    .child(combo(
                        "pr-snap",
                        "gridmode",
                        Menu::Snap,
                        tr(if self.snap == SnapMode::Snap { "Snap" } else { "Nudge" }).into(),
                        "Snap mode",
                    ))
                    .child(combo(
                        "pr-ghost",
                        "ghost_note",
                        Menu::Ghost,
                        if self.ghost.is_some() { tr("Ghost notes").into() } else { "".into() },
                        "Ghost notes",
                    ))
                    .when(self.ghost.is_some(), |d| {
                        d.child(icon_button("pr-clear-ghost", "clear_ghost_note", false, "Clear ghost notes").on_click(
                            cx.listener(|pr, _, _, cx| {
                                pr.ghost = None;
                                cx.notify();
                            }),
                        ))
                    })
                    .when(self.rec != Rec::Off, |d| {
                        d.child(div().flex_1()).child(div().text_size(px(11.)).text_color(c(THEME.red)).child(
                            match self.rec {
                                Rec::Step => trf("Step {0}", &[&format_bbt(self.step_state(cx).map_or(0, |s| s.0))]),
                                _ => trf("Octave {0}", &[&self.octave]),
                            },
                        ))
                    }),
            )
    }

    fn render_keys(&self, cx: &Context<Self>) -> impl IntoElement {
        let (key_h, scroll_y) = (self.key_h, self.scroll_y);
        let pressed = self.pressed.clone();
        let marked = self.marked.clone();
        let bounds = self.keys_bounds.clone();
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
                                    let black = music::is_black(ku);
                                    let col = if pressed.contains(&ku) {
                                        c(THEME.key_active)
                                    } else if marked.contains(&ku) {
                                        c(THEME.yellow)
                                    } else if black {
                                        c(THEME.key_black)
                                    } else {
                                        c(THEME.key_white)
                                    };
                                    let kw = if black { w * 0.62 } else { w };
                                    let kb = Bounds::new(point(o.x, o.y + px(y + 0.5)), size(px(kw), px(key_h - 1.0)));
                                    window.paint_quad(fill(kb, col));
                                    if ku.is_multiple_of(12) && key_h >= 7.0 {
                                        let fs = key_h.clamp(7.0, 10.0);
                                        let ty = o.y + px(y + (key_h - fs) / 2.0 - 1.0);
                                        paint_text(
                                            window,
                                            cx,
                                            key_name(ku).into(),
                                            o.x + px(w - 26.0),
                                            ty,
                                            fs,
                                            c(THEME.bg1),
                                        );
                                    }
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
            // the note edit area's name: a click switches Velocity and Panning (the legacy's menu)
            .child(
                div()
                    .id("pr-lane-mode")
                    .debug_selector(|| "pr-lane-mode".into())
                    .h(px(LANE_H))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(c(THEME.bg2))
                    .border_t_1()
                    .border_color(c(THEME.line))
                    .text_size(px(10.5))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::open_menu(Menu::Lane)))
                    .child(tr(if self.lane == LaneMode::Velocity { "Velocity" } else { "Panning" })),
            )
    }

    fn render_center(&self, clip: Clip, play: Option<f64>, cx: &Context<Self>) -> impl IntoElement {
        let ghost = self
            .ghost
            .and_then(|g| self.clips.borrow_mut().get(self.scx.core.read(cx).port().root(), g))
            .map(|c| c.notes);
        let (gone, copies) = match &self.drag {
            Some(Drag::Erase { gone }) => (gone.clone(), Vec::new()),
            Some(Drag::Move { origin, copy: true, delta, .. }) if *delta != (0, 0) => (
                BTreeSet::new(),
                origin
                    .iter()
                    .map(|(_, x)| Note {
                        start: x.start + delta.0,
                        key: (x.key as i32 + delta.1).clamp(0, 127) as u8,
                        ..x.clone()
                    })
                    .collect(),
            ),
            _ => (BTreeSet::new(), Vec::new()),
        };
        let span = self.grid_bounds.get().size.width.as_f32().max(800.0) as f64 * self.tpp;
        let bars = Arc::new(piano_roll_bars::bars(
            self.scx.core.read(cx).port().root(),
            clip.start,
            self.scroll,
            self.scroll + span,
        ));
        let grid_paint = GridPaint {
            notes: clip.notes.clone(),
            ghost,
            sel: self.sel.clone(),
            gone,
            copies,
            scroll: self.scroll,
            tpp: self.tpp,
            key_h: self.key_h,
            scroll_y: self.scroll_y,
            grid: self.grid_ticks(),
            bars: bars.clone(),
            length: clip.length,
            loop_len: clip.loop_len,
            scale: self.scale.map(|s| (self.root, s)),
            marked: self.marked.clone(),
            play,
            item_loop: self.play.loop_on.then_some((self.play.loop_a, self.play.loop_b)),
            step_cursor: if self.rec == Rec::Step {
                self.step_state(cx).map(|(at, _)| (at, self.new_len()))
            } else {
                None
            },
            rect: match &self.drag {
                Some(Drag::Rect { from, to, .. }) => Some((*from, *to)),
                _ => None,
            },
            knife: match &self.drag {
                Some(Drag::Knife { from, to }) => Some((*from, *to)),
                _ => None,
            },
        };
        let cursor = match (self.tool, self.hover.and_then(|h| self.note_at(&clip.notes, h.0, h.1).map(|x| x.1))) {
            (Tool::Draw | Tool::Select, Some(1)) => CursorStyle::ResizeLeftRight,
            (Tool::Draw | Tool::Select, Some(_)) => CursorStyle::OpenHand,
            (Tool::Erase, _) => CursorStyle::OperationNotAllowed,
            (Tool::Knife, _) => CursorStyle::Crosshair,
            (Tool::Pitch, Some(_)) => CursorStyle::ResizeUpDown,
            _ => CursorStyle::Arrow,
        };
        let (gb, rb, lb) = (self.grid_bounds.clone(), self.ruler_bounds.clone(), self.lane_bounds.clone());
        let (scroll, tpp) = (self.scroll, self.tpp);
        let ruler_bars = bars.clone();
        let item_loop = (self.play.loop_on, self.play.loop_a, self.play.loop_b);
        let item_cursor = self.play.at(HostTime::now());
        let (lane_notes, lane_sel, lane_mode) = (clip.notes.clone(), self.sel.clone(), self.lane);
        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .id("pr-ruler")
                    .debug_selector(|| "pr-ruler".into())
                    .h(px(RULER_H))
                    .flex_none()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::ruler_down))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::ruler_down))
                    .child(
                        canvas(
                            move |b, _, _| rb.set(b),
                            move |b, _, window, cx| {
                                let o = b.origin;
                                let _w = b.size.width.as_f32();
                                let x = |t: f64| ((t - scroll) / tpp) as f32;
                                window.paint_quad(fill(b, c(THEME.bg3)));
                                if item_loop.0 {
                                    let (a, z) = (x(item_loop.1 as f64), x(item_loop.2 as f64));
                                    let lb = Bounds::new(
                                        point(o.x + px(a), o.y + px(12.0)),
                                        size(px((z - a).max(0.0)), px(6.0)),
                                    );
                                    window.paint_quad(fill(lb, c(THEME.loop_on)));
                                }
                                // a number on every n-th bar, as far apart as the zoom allows
                                let every = ruler_bars
                                    .first()
                                    .map_or(1, |b| ((60.0 * tpp / b.len as f64).ceil() as i64).max(1));
                                for bar in ruler_bars.iter().filter(|b| (b.number - 1) % every == 0) {
                                    let xx = x(bar.start as f64);
                                    let tb = Bounds::new(point(o.x + px(xx), o.y + px(4.0)), size(px(1.0), px(14.0)));
                                    window.paint_quad(fill(tb, c(THEME.text3)));
                                    paint_text(
                                        window,
                                        cx,
                                        format!("{}", bar.number).into(),
                                        o.x + px(xx + 3.0),
                                        o.y + px(1.0),
                                        10.0,
                                        c(THEME.text2),
                                    );
                                }
                                let cx_ = x(item_cursor as f64);
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
                    .child(
                        canvas(move |b, _, _| gb.set(b), move |b, _, window, cx| grid_paint.paint(b, window, cx))
                            .size_full(),
                    ),
            )
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
                                let base = if lane_mode == LaneMode::Panning { 3.0 + h / 2.0 } else { 3.0 + h };
                                window.paint_quad(fill(
                                    Bounds::new(point(o.x, o.y + px(base)), size(px(w), px(1.0))),
                                    c(THEME.bg4),
                                ));
                                for (n, nt) in lane_notes.iter() {
                                    let xx = x(nt.start as f64);
                                    if !(-4.0..=w + 4.0).contains(&xx) {
                                        continue;
                                    }
                                    let v = if lane_mode == LaneMode::Velocity {
                                        nt.velocity as f32
                                    } else {
                                        nt.pan as f32 * 0.5 + 0.5
                                    };
                                    let top = 3.0 + (1.0 - v) * h;
                                    let col = if lane_sel.contains(n) { c(THEME.pr_note_sel) } else { c(THEME.pr_vel) };
                                    let (y0, y1) = (top.min(base), top.max(base));
                                    let stem = Bounds::new(
                                        point(o.x + px(xx), o.y + px(y0)),
                                        size(px(2.0), px((y1 - y0).max(1.0))),
                                    );
                                    window.paint_quad(fill(stem, col));
                                    let head = Bounds::new(
                                        point(o.x + px(xx - 2.5), o.y + px(top - 2.5)),
                                        size(px(7.0), px(5.0)),
                                    );
                                    window.paint_quad(gpui::quad(head, px(2.0), col, px(0.0), col, Default::default()));
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
    }
}

fn c_(hex: u32) -> gpui::Rgba {
    c(hex)
}

// ---------------------------------------------------------------------------------------------------------------
// The notes as the editors see them (read from the tree) and the note operations

pub mod music {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use lm_port::{NodeId, Value};

    use crate::model;
    use crate::ui::track_color;

    /// 960 ticks per quarter note (the project's ppq default; the editors work in it)
    pub const PPQ: i64 = 960;
    pub const BEAT: i64 = PPQ;
    pub const BAR: i64 = 4 * PPQ;

    pub fn format_bbt(tick: i64) -> String {
        let t = tick.max(0);
        format!("{}.{}.{:03}", t / BAR + 1, t % BAR / BEAT + 1, t % BEAT)
    }

    /// A length as the shortest fraction ("1/16", "3/16", "1/8T")
    pub fn format_len(ticks: i64) -> String {
        for (den, unit) in
            [(1, BAR), (2, BAR / 2), (4, BEAT), (8, BEAT / 2), (16, BEAT / 4), (32, BEAT / 8), (64, BEAT / 16)]
        {
            if ticks % unit == 0 {
                return format!("{}/{den}", ticks / unit);
            }
        }
        for (den, unit) in
            [(6, BAR / 6), (12, BAR / 12), (24, BAR / 24), (48, BAR / 48), (96, BAR / 96), (192, BAR / 192)]
        {
            if ticks % unit == 0 {
                return format!("{}/{}T", ticks / unit, den / 3 * 2);
            }
        }
        format!("{ticks} t")
    }

    /// The legacy's quantization and note lengths: 1/1..1/64 and the triplets 1/3..1/48
    pub const GRIDS: [(&str, i64); 14] = [
        ("1/1", BAR),
        ("1/2", BAR / 2),
        ("1/4", BEAT),
        ("1/8", BEAT / 2),
        ("1/16", BEAT / 4),
        ("1/32", BEAT / 8),
        ("1/64", BEAT / 16),
        ("1/3", BAR / 3),
        ("1/6", BAR / 6),
        ("1/12", BAR / 12),
        ("1/24", BAR / 24),
        ("1/48", BAR / 48),
        ("1/96", BAR / 96),
        ("1/192", BAR / 192),
    ];

    pub fn snap(t: f64, grid: i64) -> i64 {
        if grid <= 1 { t.round() as i64 } else { ((t / grid as f64).round() as i64) * grid }
    }
    pub fn snap_floor(t: f64, grid: i64) -> i64 {
        if grid <= 1 { t.floor() as i64 } else { ((t / grid as f64).floor() as i64) * grid }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Bend {
        pub at: i64,
        pub semitones: f64,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct Note {
        pub key: u8,
        pub start: i64,
        pub length: i64,
        pub velocity: f64,
        /// -1..1, the legacy's per-note panning
        pub pan: f64,
        /// the pitch bend inside the note (P-24), linear between points
        pub bend: Vec<Bend>,
        /// the MPE pressure and timbre curves of the note
        pub mpe: super::super::piano_roll_curves::MpeCurves,
    }

    pub type NoteMap = BTreeMap<u64, Note>;
    pub type NoteChange = (u64, Option<Note>);

    fn real(v: f64) -> Value {
        Value::real(v).unwrap_or(Value::NULL)
    }

    impl Note {
        pub fn new(key: u8, start: i64, length: i64, velocity: f64) -> Note {
            Note { key, start, length, velocity, pan: 0.0, bend: Vec::new(), mpe: Default::default() }
        }
        pub fn end(&self) -> i64 {
            self.start + self.length
        }
        pub fn bend_at(&self, at: i64) -> f64 {
            match self.bend.as_slice() {
                [] => 0.0,
                [b] => b.semitones,
                pts => {
                    if at <= pts[0].at {
                        return pts[0].semitones;
                    }
                    for w in pts.windows(2) {
                        if at <= w[1].at {
                            let f = (at - w[0].at) as f64 / (w[1].at - w[0].at).max(1) as f64;
                            return w[0].semitones + (w[1].semitones - w[0].semitones) * f;
                        }
                    }
                    pts[pts.len() - 1].semitones
                }
            }
        }

        pub fn read(v: &Value) -> Note {
            Note {
                key: model::int(v, "key", 60).clamp(0, 127) as u8,
                start: model::int(v, "start", 0),
                length: model::int(v, "length", 240),
                velocity: model::real(v, "velocity", 0.8),
                pan: model::real(v, "pan", 0.0),
                bend: model::list(v, "bend")
                    .map(|b| Bend { at: model::int(b, "at", 0), semitones: model::real(b, "semitones", 0.0) })
                    .collect(),
                mpe: super::super::piano_roll_curves::MpeCurves::read(v),
            }
        }

        fn bend_value(&self) -> Value {
            Value::list(
                self.bend.iter().map(|b| Value::object([("at", Value::int(b.at)), ("semitones", real(b.semitones))])),
            )
        }

        /// The note as the document's object; `pan` and `bend` only when they say something (optional fields)
        pub fn to_value(&self) -> Value {
            let mut f = vec![
                ("key", Value::int(self.key as i64)),
                ("start", Value::int(self.start.max(0))),
                ("length", Value::int(self.length.max(1))),
                ("velocity", real(self.velocity.clamp(0.0, 1.0))),
            ];
            if self.pan != 0.0 {
                f.push(("pan", real(self.pan.clamp(-1.0, 1.0))));
            }
            if !self.bend.is_empty() {
                f.push(("bend", self.bend_value()));
            }
            f.extend(self.mpe.fields());
            Value::object(f)
        }

        /// The fields that differ from `old`, as (key, value)
        pub fn fields_changed(&self, old: &Note) -> Vec<(&'static str, Value)> {
            let mut v = Vec::new();
            if self.key != old.key {
                v.push(("key", Value::int(self.key as i64)));
            }
            if self.start != old.start {
                v.push(("start", Value::int(self.start.max(0))));
            }
            if self.length != old.length {
                v.push(("length", Value::int(self.length.max(1))));
            }
            if self.velocity != old.velocity {
                v.push(("velocity", real(self.velocity.clamp(0.0, 1.0))));
            }
            if self.pan != old.pan {
                v.push(("pan", real(self.pan.clamp(-1.0, 1.0))));
            }
            if self.bend != old.bend {
                v.push(("bend", self.bend_value()));
            }
            v.extend(self.mpe.changed(&old.mpe));
            v
        }
    }

    /// The clips read from the tree, kept while the track list is the same node: the persistent tree gives the
    /// unchanged list back as the same node, so a mouse event reads nothing again until a change reaches the tracks
    pub struct ClipCache {
        tracks: Option<Value>,
        entries: Vec<(NodeId, Option<Clip>)>,
        cap: usize,
    }

    impl Default for ClipCache {
        fn default() -> Self {
            Self::with_capacity(4)
        }
    }

    impl ClipCache {
        /// A cache of the last `cap` clips read (an editor showing many items asks for more)
        pub fn with_capacity(cap: usize) -> Self {
            ClipCache { tracks: None, entries: Vec::new(), cap: cap.max(1) }
        }

        pub fn get(&mut self, root: &Value, id: NodeId) -> Option<Clip> {
            let tracks = root.get("tracks");
            let same = match (&self.tracks, tracks) {
                (Some(a), Some(b)) => a.same_node(b),
                (None, None) => true,
                _ => false,
            };
            if !same {
                self.entries.clear();
                self.tracks = tracks.cloned();
            }
            if let Some((_, c)) = self.entries.iter().find(|(i, _)| *i == id) {
                return c.clone();
            }
            let c = Clip::read(root, id);
            if self.entries.len() >= self.cap {
                self.entries.remove(0);
            }
            self.entries.push((id, c.clone()));
            c
        }

        /// How many entries are kept (the tests check the reuse)
        #[cfg(test)]
        pub fn len(&self) -> usize {
            self.entries.len()
        }
    }

    /// A MIDI item as the editors see it, by its id in the tree
    #[derive(Clone, Debug)]
    pub struct Clip {
        pub id: NodeId,
        pub track_name: String,
        pub name: String,
        pub color: u32,
        pub start: i64,
        pub length: i64,
        /// the repeating part of a pattern (`loop`), if it repeats
        pub loop_len: Option<i64>,
        pub notes: Arc<NoteMap>,
        /// the step editor's step (`step`, a 16th when absent) and the swing
        pub step: i64,
        pub swing: f64,
    }

    impl Clip {
        pub fn read(root: &Value, id: NodeId) -> Option<Clip> {
            for (ti, t) in model::list(root, "tracks").enumerate() {
                for it in model::list(t, "items") {
                    if model::id_of(it) != Some(id) {
                        continue;
                    }
                    let notes = it
                        .get("notes")
                        .map(|m| {
                            m.entries().filter_map(|(k, v)| Some((k.parse::<u64>().ok()?, Note::read(v)))).collect()
                        })
                        .unwrap_or_default();
                    return Some(Clip {
                        id,
                        track_name: model::text(t, "name").to_string(),
                        name: model::text(it, "name").to_string(),
                        color: track_color(ti),
                        start: model::int(it, "start", 0),
                        length: model::int(it, "length", BAR),
                        loop_len: it.get("loop").and_then(Value::as_int).filter(|l| *l > 0),
                        notes: Arc::new(notes),
                        step: model::int(it, "step", BEAT / 4).max(1),
                        swing: model::real(it, "swing", 0.0),
                    });
                }
            }
            None
        }
    }

    #[cfg(test)]
    mod clip_cache_tests {
        use super::*;
        use lm_port::{CorePort, EditOp, Label};
        use lm_port_fake::{Config, FakePort};

        #[test]
        fn a_clip_is_read_once_until_the_tracks_change() {
            let mut port = FakePort::new(Config { tracks: 3, slow_delay: None, slow_track: usize::MAX }).expect("demo");
            let id = first_midi(port.root()).expect("a MIDI item");
            let mut cache = ClipCache::default();
            let first = cache.get(port.root(), id).expect("the clip");
            let again = cache.get(port.root(), id).expect("the clip");
            // the same notes, not read again
            assert!(Arc::ptr_eq(&first.notes, &again.notes));
            assert_eq!(cache.len(), 1);
            // an edit reaches the tracks: what was kept is dropped
            let track = model::list(port.root(), "tracks").next().and_then(model::id_of).expect("a track");
            let _ = port.apply(Label::new("t"), vec![EditOp::set(track, &["name"], Value::text("renamed"))]);
            let fresh = cache.get(port.root(), id).expect("the clip");
            assert!(!Arc::ptr_eq(&first.notes, &fresh.notes));
        }
    }

    pub fn first_midi(root: &Value) -> Option<NodeId> {
        model::list(root, "tracks")
            .flat_map(|t| model::list(t, "items"))
            .find(|i| i.get("kind").and_then(Value::as_text) != Some("audio"))
            .and_then(model::id_of)
    }

    // --- scales and chords (the legacy's lists, by name) ---------------------------------------------------------

    pub const KEY_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

    /// The legacy's names: key 60 is C5
    pub fn key_name(key: u8) -> String {
        format!("{}{}", KEY_NAMES[key as usize % 12], key / 12)
    }
    pub fn is_black(key: u8) -> bool {
        matches!(key % 12, 1 | 3 | 6 | 8 | 10)
    }

    pub const SCALES: &[(&str, &[u8])] = &[
        ("Major", &[0, 2, 4, 5, 7, 9, 11]),
        ("Natural minor", &[0, 2, 3, 5, 7, 8, 10]),
        ("Harmonic minor", &[0, 2, 3, 5, 7, 8, 11]),
        ("Melodic minor", &[0, 2, 3, 5, 7, 9, 11]),
        ("Dorian", &[0, 2, 3, 5, 7, 9, 10]),
        ("Phrygian", &[0, 1, 3, 5, 7, 8, 10]),
        ("Lydian", &[0, 2, 4, 6, 7, 9, 11]),
        ("Mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
        ("Locrian", &[0, 1, 3, 5, 6, 8, 10]),
        ("Major pentatonic", &[0, 2, 4, 7, 9]),
        ("Minor pentatonic", &[0, 3, 5, 7, 10]),
        ("Blues", &[0, 3, 5, 6, 7, 10]),
        ("Whole tone", &[0, 2, 4, 6, 8, 10]),
        ("Diminished", &[0, 2, 3, 5, 6, 8, 9, 11]),
        ("Chromatic", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
    ];

    pub const CHORDS: &[(&str, &[u8])] = &[
        ("Major", &[0, 4, 7]),
        ("Minor", &[0, 3, 7]),
        ("Diminished", &[0, 3, 6]),
        ("Augmented", &[0, 4, 8]),
        ("Sus2", &[0, 2, 7]),
        ("Sus4", &[0, 5, 7]),
        ("Power", &[0, 7, 12]),
        ("Major 7", &[0, 4, 7, 11]),
        ("Minor 7", &[0, 3, 7, 10]),
        ("Dominant 7", &[0, 4, 7, 10]),
        ("Half-diminished", &[0, 3, 6, 10]),
        ("Diminished 7", &[0, 3, 6, 9]),
        ("Major 6", &[0, 4, 7, 9]),
        ("Minor 6", &[0, 3, 7, 9]),
        ("Add 9", &[0, 4, 7, 14]),
        ("Major 9", &[0, 4, 7, 11, 14]),
        ("Minor 9", &[0, 3, 7, 10, 14]),
        ("Dominant 9", &[0, 4, 7, 10, 14]),
        ("Octave", &[0, 12]),
    ];

    pub fn in_scale(key: u8, root: u8, scale: usize) -> bool {
        SCALES[scale].1.contains(&((key as i32 - root as i32).rem_euclid(12) as u8))
    }

    pub fn to_scale(key: i32, root: u8, scale: usize) -> u8 {
        let key = key.clamp(0, 127);
        for d in 0..12 {
            for k in [key - d, key + d] {
                if (0..=127).contains(&k) && in_scale(k as u8, root, scale) {
                    return k as u8;
                }
            }
        }
        key as u8
    }

    pub fn chord_keys(key: u8, chord: usize) -> Vec<u8> {
        CHORDS[chord]
            .1
            .iter()
            .map(|i| key as i32 + *i as i32)
            .filter(|k| (0..=127).contains(k))
            .map(|k| k as u8)
            .collect()
    }

    // --- note operations: from the notes to the changes --------------------------------------------------------

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum QuantizeWhat {
        Both,
        Starts,
        Lengths,
    }

    pub fn quantize(notes: &NoteMap, which: &[u64], grid: i64, what: QuantizeWhat, strength: f64) -> Vec<NoteChange> {
        let mut out = Vec::new();
        for n in which {
            let Some(x) = notes.get(n) else { continue };
            let mut y = x.clone();
            if what != QuantizeWhat::Lengths {
                y.start = x.start + ((snap(x.start as f64, grid) - x.start) as f64 * strength).round() as i64;
            }
            if what != QuantizeWhat::Starts {
                let l = snap(x.length as f64, grid).max(grid);
                y.length = (x.length + ((l - x.length) as f64 * strength).round() as i64).max(1);
            }
            if y != *x {
                out.push((*n, Some(y)));
            }
        }
        out
    }

    /// Glue: notes of the same key that touch or overlap become one (the first keeps its number)
    pub fn glue(notes: &NoteMap, which: &[u64]) -> Vec<NoteChange> {
        let mut sel: Vec<(u64, &Note)> = which.iter().filter_map(|n| notes.get(n).map(|x| (*n, x))).collect();
        sel.sort_by_key(|(_, x)| (x.key, x.start));
        let mut out = Vec::new();
        let mut i = 0;
        while i < sel.len() {
            let (n, first) = sel[i];
            let mut merged = first.clone();
            let mut j = i + 1;
            while j < sel.len() && sel[j].1.key == merged.key && sel[j].1.start <= merged.end() {
                merged.length = merged.length.max(sel[j].1.end() - merged.start);
                out.push((sel[j].0, None));
                j += 1;
            }
            if j > i + 1 {
                out.push((n, Some(merged)));
            }
            i = j;
        }
        out
    }

    pub fn fill(notes: &NoteMap, which: &[u64], end: i64) -> Vec<NoteChange> {
        let mut starts: Vec<i64> = notes.values().map(|x| x.start).collect();
        starts.sort_unstable();
        starts.dedup();
        which
            .iter()
            .filter_map(|n| {
                let x = notes.get(n)?;
                let next = starts.iter().find(|s| **s > x.start).copied().unwrap_or(end);
                (next - x.start != x.length).then(|| (*n, Some(Note { length: (next - x.start).max(1), ..x.clone() })))
            })
            .collect()
    }

    pub fn cut_overlaps(notes: &NoteMap, which: &[u64]) -> Vec<NoteChange> {
        which
            .iter()
            .filter_map(|n| {
                let x = notes.get(n)?;
                let s = notes.values().filter(|y| y.key == x.key && y.start > x.start).map(|y| y.start).min()?;
                (x.end() > s).then(|| (*n, Some(Note { length: s - x.start, ..x.clone() })))
            })
            .collect()
    }

    pub fn constrain_lengths(notes: &NoteMap, which: &[u64], len: i64, at_least: bool) -> Vec<NoteChange> {
        which
            .iter()
            .filter_map(|n| {
                let x = notes.get(n)?;
                let l = if at_least { x.length.max(len) } else { x.length.min(len) };
                (l != x.length).then(|| (*n, Some(Note { length: l, ..x.clone() })))
            })
            .collect()
    }

    pub fn reverse(notes: &NoteMap, which: &[u64]) -> Vec<NoteChange> {
        let sel: Vec<(u64, &Note)> = which.iter().filter_map(|n| notes.get(n).map(|x| (*n, x))).collect();
        let (Some(a), Some(b)) = (sel.iter().map(|(_, x)| x.start).min(), sel.iter().map(|(_, x)| x.end()).max())
        else {
            return Vec::new();
        };
        sel.into_iter().map(|(n, x)| (n, Some(Note { start: a + (b - x.end()), ..x.clone() }))).collect()
    }

    pub fn split(
        notes: &NoteMap,
        which: &[u64],
        at: i64,
        new_n: &mut impl FnMut() -> u64,
        drop_short: bool,
    ) -> Vec<NoteChange> {
        let mut out = Vec::new();
        for n in which {
            let Some(x) = notes.get(n) else { continue };
            if at <= x.start || at >= x.end() {
                continue;
            }
            let left = Note { length: at - x.start, ..x.clone() };
            let right =
                Note { start: at, length: x.end() - at, bend: Vec::new(), mpe: Default::default(), ..x.clone() };
            if drop_short {
                out.push((*n, Some(if left.length >= right.length { left } else { right })));
            } else {
                out.push((*n, Some(left)));
                out.push((new_n(), Some(right)));
            }
        }
        out
    }

    #[cfg(test)]
    pub fn strum(original: &[(u64, Note)], spread: i64) -> Vec<NoteChange> {
        strum_curved(original, spread, 1.0)
    }

    /// The strum's staircase with a curve: each note's offset is `spread * (i / k) ^ curve` (1: straight)
    pub fn strum_curved(original: &[(u64, Note)], spread: i64, curve: f64) -> Vec<NoteChange> {
        let mut v: Vec<&(u64, Note)> = original.iter().collect();
        v.sort_by_key(|(_, x)| x.key);
        if spread < 0 {
            v.reverse();
        }
        let k = v.len().max(2) - 1;
        v.iter()
            .enumerate()
            .map(|(i, (n, x))| {
                let d = (spread.abs() as f64 * (i as f64 / k as f64).powf(curve.clamp(0.1, 10.0))).round() as i64;
                (*n, Some(Note { start: x.start + d, length: (x.length - d).max(1), ..x.clone() }))
            })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn notes(v: &[(u8, i64, i64)]) -> NoteMap {
            v.iter().enumerate().map(|(i, (k, s, l))| (i as u64 + 1, Note::new(*k, *s, *l, 0.8))).collect()
        }

        #[test]
        fn quantize_glue_split_reverse_strum() {
            let m = notes(&[(60, 230, 470), (60, 700, 240), (60, 940, 100)]);
            let q = quantize(&m, &[1], BEAT / 4, QuantizeWhat::Both, 1.0);
            assert_eq!((q[0].1.as_ref().unwrap().start, q[0].1.as_ref().unwrap().length), (240, 480));
            assert!(glue(&m, &[2, 3]).contains(&(3, None)));
            let mut k = 100;
            let mut next = || {
                k += 1;
                k
            };
            assert_eq!(split(&m, &[1], 300, &mut next, false).len(), 2);
            let r = reverse(&m, &[1, 3]);
            assert_eq!(r.iter().find(|c| c.0 == 3).unwrap().1.as_ref().unwrap().start, 230);
            let chord: Vec<(u64, Note)> = vec![(1, Note::new(60, 0, 960, 1.0)), (2, Note::new(67, 0, 960, 1.0))];
            assert_eq!(strum(&chord, 120)[1].1.as_ref().unwrap().start, 120);
            // a curve above 1 keeps the middle note nearer the start
            let mid = |c: f64| strum_curved(&chord, 120, c)[1].1.as_ref().unwrap().start;
            assert!(mid(2.0) < mid(1.0) || chord.len() < 3);
        }

        #[test]
        fn names_scales_and_values() {
            assert_eq!(key_name(60), "C5", "the legacy's octave numbers");
            assert_eq!(to_scale(70, 9, 1), 69);
            assert_eq!(format_len(BEAT * 3 / 4), "3/16");
            let n = Note::new(60, 0, 240, 0.5);
            assert!(n.to_value().get("pan").is_none(), "an optional field stays out when it says nothing");
            assert_eq!(Note::read(&n.to_value()), n);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The kit the three editors share

pub mod kit {
    use std::rc::Rc;

    use gpui::{
        AnyView, App, AppContext, Context, Div, ElementId, Entity, FontWeight, Hsla, IntoElement, MouseButton,
        MouseMoveEvent, MouseUpEvent, Pixels, Point, Rgba, SharedString, Stateful, Subscription, TextAlign, TextRun,
        Render, WeakEntity, Window, anchored, canvas, deferred, div, prelude::*, px,
    };
    use gpui_component::input::{Input, InputEvent, InputState};
    use gpui_component::tooltip::Tooltip;

    use crate::ui::{THEME, c, tr};

    pub fn with_alpha(col: u32, a: f32) -> Rgba {
        let mut x = c(col);
        x.a = a;
        x
    }

    /// A combo box of the toolbars: the current choice and a ▾
    pub fn dropdown(id: impl Into<ElementId>, text: impl Into<SharedString>, active: bool) -> Stateful<Div> {
        combo_box(id, None, text.into(), active)
    }

    fn combo_box(
        id: impl Into<ElementId>,
        icon: Option<&'static str>,
        text: SharedString,
        active: bool,
    ) -> Stateful<Div> {
        div()
            .id(id.into())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .h(px(20.))
            .px(px(5.))
            .rounded(px(3.))
            .border_1()
            .border_color(c(if active { THEME.menu_border } else { THEME.line2 }))
            .bg(c(THEME.bg3))
            .text_size(px(11.))
            .cursor_pointer()
            .hover(|s| s.bg(c(THEME.bg4)))
            .children(icon.map(crate::ui::icon))
            .when(!text.is_empty(), |d| d.child(text))
            .child(div().text_size(px(8.)).text_color(c(THEME.text2)).child("▾"))
    }

    /// A legacy toolbar button: its icon, pressed (sunken, highlighted) when on
    pub fn icon_button(id: &'static str, icon: &'static str, on: bool, help: &'static str) -> Stateful<Div> {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(24.))
            .rounded(px(3.))
            .border_1()
            .border_color(c(if on { THEME.menu_border } else { THEME.bg2 }))
            .bg(c(if on { THEME.bg1 } else { THEME.bg2 }))
            .hover(|s| s.bg(c(THEME.bg4)))
            .cursor_pointer()
            .tooltip(tip(help))
            .child(crate::ui::icon(icon))
    }

    /// A combo box with the legacy's icon in front of its text
    pub fn dropdown_icon(
        id: impl Into<ElementId>,
        icon: &'static str,
        text: impl Into<SharedString>,
        active: bool,
    ) -> Stateful<Div> {
        combo_box(id, Some(icon), text.into(), active)
    }

    pub fn label(text: impl Into<SharedString>) -> Div {
        div().flex_none().text_size(px(10.5)).text_color(c(THEME.text3)).child(text.into())
    }

    pub fn sep() -> Div {
        div().flex_none().w(px(1.)).h(px(16.)).mx(px(2.)).bg(c(THEME.line2))
    }

    pub fn tip(text: &'static str) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        move |window, cx| {
            if crate::ui::tooltips() { Tooltip::new(tr(text)).build(window, cx) } else { cx.new(|_| NoTip).into() }
        }
    }

    /// What a tooltip is while the preference `interface.tooltips` is off: nothing drawn
    struct NoTip;
    impl Render for NoTip {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[derive(Clone)]
    pub struct MenuItem {
        pub label: SharedString,
        pub checked: bool,
        pub color: Option<u32>,
    }
    impl MenuItem {
        pub fn new(label: impl Into<SharedString>, checked: bool) -> Self {
            MenuItem { label: label.into(), checked, color: None }
        }
        pub fn color(mut self, c: u32) -> Self {
            self.color = Some(c);
            self
        }
    }

    /// A popup list over everything, at `at`; a click outside dismisses it
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
                    .min_w(px(140.))
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .p(px(3.))
                    .rounded(px(4.))
                    .bg(c(THEME.menu_bg))
                    .border_1()
                    .border_color(c(THEME.line2))
                    .shadow_lg()
                    .text_size(px(12.))
                    .on_mouse_down_out(move |_, w, cx| on_dismiss(w, cx))
                    .children(items.into_iter().enumerate().map(|(i, it)| {
                        let pick = pick.clone();
                        div()
                            .id(("item", i))
                            .debug_selector(move || format!("{id}-{i}"))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .px(px(8.))
                            .py(px(2.))
                            .rounded(px(3.))
                            .cursor_pointer()
                            .hover(|s| s.bg(c(THEME.bg4)))
                            .on_click(move |_, w, cx| pick(i, w, cx))
                            .child(div().w(px(10.)).text_color(c(THEME.menu_border)).child(if it.checked {
                                "✓"
                            } else {
                                ""
                            }))
                            .children(it.color.map(|col| div().size(px(8.)).rounded(px(2.)).bg(c(col))))
                            .child(div().when(it.checked, |d| d.font_weight(FontWeight::BOLD)).child(it.label))
                    })),
            ),
        )
        .with_priority(1)
    }

    /// A small field to type a value (a double click), at the mouse; Enter applies, Esc or a click outside closes
    pub struct ValueEntry {
        state: Entity<InputState>,
        at: Point<Pixels>,
        _sub: Subscription,
    }

    impl ValueEntry {
        pub fn new<V: 'static>(
            value: String,
            at: Point<Pixels>,
            window: &mut Window,
            cx: &mut Context<V>,
        ) -> ValueEntry {
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
                        .w(px(110.))
                        .p(px(3.))
                        .rounded(px(4.))
                        .bg(c(THEME.bg2))
                        .border_1()
                        .border_color(c(THEME.menu_border))
                        .shadow_lg()
                        .on_mouse_down_out(move |_, w, cx| close2(w, cx))
                        // keymap-ok: text entry, Enter takes the typed value and Escape drops it
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
                        .child(crate::ui::text_entry(Input::new(&self.state))),
                ),
            )
            .with_priority(2)
        }
    }

    /// Text inside a canvas (one shaped line; GPUI caches the shaping from frame to frame)
    pub fn paint_text(
        window: &mut Window,
        cx: &mut App,
        text: SharedString,
        x: Pixels,
        y: Pixels,
        size: f32,
        color: Rgba,
    ) {
        let font = window.text_style().font();
        let run = TextRun {
            len: text.len(),
            font,
            color: Hsla::from(color),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
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
}
