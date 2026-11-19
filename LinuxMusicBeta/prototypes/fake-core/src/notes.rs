//! Owner: agent B. The fake data and the edit operations of the note editors: the content of the MIDI items (the
//! notes, a persistent map by number `n` inside the item, C1), the automation lanes (points with a shape and a
//! `curve` of -1..1), and what the Piano Roll, the automation editor and the step editor do to them (quantize,
//! glue, knife, strum, scales and chords, flips, scaling...). Positions are musical: ticks at 960 per quarter note
//! (spec item 1), relative to the item's start.
//!
//! In the document: `Doc::music`. Edits go through the session's transactions and gestures like every other field:
//! `Field::Notes` (a whole note map, `Value::Notes`), `Field::Points` (`Value::Points`) and `Field::Clip(key)` for
//! an item's or a lane's own numbers (`Value::Real`). The helpers `Edit::put_notes` / `Edit::put_points` change
//! notes or points by number, so the GUI code reads like the real core's (one note by `n`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Doc, Edit, Error, Field, NodeId, Result, SAMPLE_RATE, Value};

// ---------------------------------------------------------------------------------------------------------------
// Time

/// Ticks per quarter note (spec item 1)
pub const PPQ: i64 = 960;
pub const BEAT: i64 = PPQ;
/// 4/4 in the fake
pub const BAR: i64 = 4 * PPQ;
/// The fake's tempo for the editors' cursor: 120 BPM, so a tick is 25 samples at 48 kHz
pub const SAMPLES_PER_TICK: f64 = SAMPLE_RATE as f64 * 60.0 / 120.0 / PPQ as f64;

pub fn sample_to_tick(s: f64) -> f64 {
    s / SAMPLES_PER_TICK
}
pub fn tick_to_sample(t: f64) -> f64 {
    t * SAMPLES_PER_TICK
}

/// bar.beat.tick (1-based bar and beat), the format of the inspectors
pub fn format_bbt(tick: i64) -> String {
    let t = tick.max(0);
    format!("{}.{}.{:03}", t / BAR + 1, t % BAR / BEAT + 1, t % BEAT)
}

/// A length as bars, beats and ticks, the shortest way: "1/4", "3/16" when it is a simple fraction of the beat
pub fn format_len(ticks: i64) -> String {
    for (den, unit) in [(1, BAR), (2, BAR / 2), (4, BEAT), (8, BEAT / 2), (16, BEAT / 4), (32, BEAT / 8), (64, BEAT / 16)] {
        if ticks % unit == 0 {
            let n = ticks / unit;
            return if den == 1 { format!("{n}/1") } else { format!("{n}/{den}") };
        }
    }
    for (den, unit) in [(6, BAR / 6), (12, BAR / 12), (24, BAR / 24), (48, BAR / 48)] {
        if ticks % unit == 0 {
            return format!("{}/{}T", ticks / unit, den / 3 * 2);
        }
    }
    format!("{ticks} t")
}

/// The snapping grids: the label (shown as is: musical notation) and its size in ticks
pub const GRIDS: [(&str, i64); 11] = [
    ("1/1", BAR),
    ("1/2", BAR / 2),
    ("1/4", BEAT),
    ("1/8", BEAT / 2),
    ("1/16", BEAT / 4),
    ("1/32", BEAT / 8),
    ("1/64", BEAT / 16),
    ("1/4T", BEAT * 2 / 3),
    ("1/8T", BEAT / 3),
    ("1/16T", BEAT / 6),
    ("1/32T", BEAT / 12),
];

pub fn snap(t: f64, grid: i64) -> i64 {
    if grid <= 1 { t.round() as i64 } else { ((t / grid as f64).round() as i64) * grid }
}
pub fn snap_floor(t: f64, grid: i64) -> i64 {
    if grid <= 1 { t.floor() as i64 } else { ((t / grid as f64).floor() as i64) * grid }
}

// ---------------------------------------------------------------------------------------------------------------
// Notes

/// A point of a note's pitch bend: `at` ticks into the note, in semitones (the legacy "detuning" curve)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bend {
    pub at: i64,
    pub semitones: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub key: u8,
    /// ticks from the item's start
    pub start: i64,
    pub length: i64,
    /// 0..1
    pub velocity: f64,
    /// -1 (left) .. +1 (right)
    pub pan: f64,
    /// the pitch bend inside the note; empty = none, one point = a constant detune
    pub bend: Vec<Bend>,
}

impl Note {
    pub fn new(key: u8, start: i64, length: i64, velocity: f64) -> Note {
        Note { key, start, length, velocity, pan: 0.0, bend: Vec::new() }
    }
    pub fn end(&self) -> i64 {
        self.start + self.length
    }
    /// The detune at `at` ticks into the note (linear between the bend points)
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
}

/// The notes of an item by number `n` (C1). Persistent in the real core; an `Arc` here, so an undo step or a
/// gesture's "before" costs a pointer
pub type NoteMap = BTreeMap<u64, Note>;

/// A change to one note by number: `Some` puts it (new or changed), `None` removes it
pub type NoteChange = (u64, Option<Note>);

/// A piece of a drum kit: a row of the step editor (the name is an English key, translated by the GUI)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KitPiece {
    pub key: u8,
    pub name: &'static str,
}

/// The AVL kit's pieces as the instrument declares them (General MIDI keys)
pub const AVL_KIT: &[KitPiece] = &[
    KitPiece { key: 36, name: "Kick" },
    KitPiece { key: 38, name: "Snare" },
    KitPiece { key: 37, name: "Side stick" },
    KitPiece { key: 39, name: "Clap" },
    KitPiece { key: 42, name: "Hi-hat closed" },
    KitPiece { key: 44, name: "Hi-hat pedal" },
    KitPiece { key: 46, name: "Hi-hat open" },
    KitPiece { key: 41, name: "Floor tom" },
    KitPiece { key: 45, name: "Low tom" },
    KitPiece { key: 48, name: "High tom" },
    KitPiece { key: 49, name: "Crash" },
    KitPiece { key: 51, name: "Ride" },
    KitPiece { key: 53, name: "Ride bell" },
    KitPiece { key: 56, name: "Cowbell" },
];

/// A pads instrument of samples (P-23: beats with samples are an instrument played by the step editor); the names
/// are the samples' files
pub const PADS_KIT: &[KitPiece] = &[
    KitPiece { key: 60, name: "kick-909.wav" },
    KitPiece { key: 61, name: "snare-909.wav" },
    KitPiece { key: 62, name: "clap-808.wav" },
    KitPiece { key: 63, name: "hat-closed.wav" },
    KitPiece { key: 64, name: "hat-open.wav" },
    KitPiece { key: 65, name: "rim.wav" },
    KitPiece { key: 66, name: "shaker.wav" },
    KitPiece { key: 67, name: "vox-chop.wav" },
];

/// The content of a MIDI item, by the item's id (the arrangement's item: same id, start and length)
#[derive(Clone, Debug)]
pub struct Clip {
    pub id: NodeId,
    pub track: NodeId,
    pub track_name: String,
    pub name: String,
    pub color: u32,
    /// on the timeline, in ticks
    pub start: i64,
    /// how long the item lasts on the timeline (all its repeats), in ticks
    pub length: i64,
    /// the repeating part (a "pattern"): the notes live in 0..loop_len and play again until `length`
    pub loop_len: i64,
    pub notes: Arc<NoteMap>,
    /// 0..1: how late the off-beat steps play (applied when the item is compiled, not written into the notes)
    pub swing: f64,
    /// the step editor's step, in ticks
    pub step: i64,
    /// a drum instrument's pieces: the step editor shows one row per piece
    pub kit: Option<&'static [KitPiece]>,
}

impl Clip {
    pub fn repeats(&self) -> bool {
        self.loop_len < self.length
    }
    /// The notes in time order (the compiler's order; the map is by number)
    pub fn sorted(&self) -> Vec<(u64, &Note)> {
        let mut v: Vec<(u64, &Note)> = self.notes.iter().map(|(n, x)| (*n, x)).collect();
        v.sort_by_key(|(n, x)| (x.start, x.key, *n));
        v
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Scales and chords (the legacy's lists, by name; names are English keys)

pub const KEY_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// C4 = 60 (scientific pitch)
pub fn key_name(key: u8) -> String {
    format!("{}{}", KEY_NAMES[key as usize % 12], key as i32 / 12 - 1)
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

/// Whether `key` is in the scale `scale` (an index of SCALES) on `root` (0 = C)
pub fn in_scale(key: u8, root: u8, scale: usize) -> bool {
    let k = (key as i32 - root as i32).rem_euclid(12) as u8;
    SCALES[scale].1.contains(&k)
}

/// The nearest key of the scale (down first when it is in the middle)
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

/// The keys of `chord` with `key` as the root, inside 0..=127
pub fn chord_keys(key: u8, chord: usize) -> Vec<u8> {
    CHORDS[chord].1.iter().map(|i| key as i32 + *i as i32).filter(|k| (0..=127).contains(k)).map(|k| k as u8).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Note operations: pure functions from the notes to the changes (the GUI puts the changes in a transaction)

/// What quantizing moves
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuantizeWhat {
    Both,
    Starts,
    Lengths,
}

/// Quantize `which` notes to `grid`; `strength` 0..1 (1 = all the way)
pub fn quantize(notes: &NoteMap, which: &[u64], grid: i64, what: QuantizeWhat, strength: f64) -> Vec<NoteChange> {
    let mut out = Vec::new();
    for n in which {
        let Some(x) = notes.get(n) else { continue };
        let mut y = x.clone();
        if what != QuantizeWhat::Lengths {
            let s = snap(x.start as f64, grid);
            y.start = x.start + ((s - x.start) as f64 * strength).round() as i64;
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

/// Fill: each note lasts until the next note's start (of any key), the last until `end`
pub fn fill(notes: &NoteMap, which: &[u64], end: i64) -> Vec<NoteChange> {
    let mut starts: Vec<i64> = notes.values().map(|x| x.start).collect();
    starts.sort();
    starts.dedup();
    let mut out = Vec::new();
    for n in which {
        let Some(x) = notes.get(n) else { continue };
        let next = starts.iter().find(|s| **s > x.start).copied().unwrap_or(end);
        if next - x.start != x.length {
            out.push((*n, Some(Note { length: (next - x.start).max(1), ..x.clone() })));
        }
    }
    out
}

/// Cut overlaps: a note that runs into the next note of the same key ends where that one starts
pub fn cut_overlaps(notes: &NoteMap, which: &[u64]) -> Vec<NoteChange> {
    let mut out = Vec::new();
    for n in which {
        let Some(x) = notes.get(n) else { continue };
        let next = notes.values().filter(|y| y.key == x.key && y.start > x.start).map(|y| y.start).min();
        if let Some(s) = next {
            if x.end() > s {
                out.push((*n, Some(Note { length: s - x.start, ..x.clone() })));
            }
        }
    }
    out
}

/// Lengths no shorter (`at_least`) or no longer (`!at_least`) than `len`
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

/// Reverse in time, inside the span of the notes
pub fn reverse(notes: &NoteMap, which: &[u64]) -> Vec<NoteChange> {
    let sel: Vec<(u64, &Note)> = which.iter().filter_map(|n| notes.get(n).map(|x| (*n, x))).collect();
    let (Some(a), Some(b)) = (sel.iter().map(|(_, x)| x.start).min(), sel.iter().map(|(_, x)| x.end()).max()) else {
        return Vec::new();
    };
    sel.into_iter().map(|(n, x)| (n, Some(Note { start: a + (b - x.end()), ..x.clone() }))).collect()
}

/// Knife: split each note crossed at `at` (ticks) into two; the right part gets `new_n`'s numbers. With
/// `drop_short`, the shorter piece is removed (the legacy's Shift)
pub fn split(notes: &NoteMap, which: &[u64], at: i64, new_n: &mut impl FnMut() -> u64, drop_short: bool) -> Vec<NoteChange> {
    let mut out = Vec::new();
    for n in which {
        let Some(x) = notes.get(n) else { continue };
        if at <= x.start || at >= x.end() {
            continue;
        }
        let left = Note { length: at - x.start, ..x.clone() };
        let right = Note { start: at, length: x.end() - at, bend: Vec::new(), ..x.clone() };
        if drop_short {
            if left.length >= right.length {
                out.push((*n, Some(left)));
            } else {
                out.push((*n, Some(right)));
            }
        } else {
            out.push((*n, Some(left)));
            out.push((new_n(), Some(right)));
        }
    }
    out
}

/// Strum: the notes that start together are spread, low to high (or high to low with a negative `spread`), by
/// `spread` ticks in all from the first to the last
pub fn strum(original: &[(u64, Note)], spread: i64) -> Vec<NoteChange> {
    let mut v: Vec<&(u64, Note)> = original.iter().collect();
    v.sort_by_key(|(_, x)| x.key);
    if spread < 0 {
        v.reverse();
    }
    let k = v.len().max(2) - 1;
    v.iter()
        .enumerate()
        .map(|(i, (n, x))| {
            let d = spread.abs() * i as i64 / k as i64;
            (*n, Some(Note { start: x.start + d, length: (x.length - d).max(1), ..x.clone() }))
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Automation

/// How a point goes to the next one
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// holds its value until the next point (the legacy's "discrete")
    Step,
    /// a line bent by `curve` (-1..1; 0 = straight: the legacy's "linear")
    Curve,
    /// a smooth spline through the points, with the lane's tension (the legacy's "cubic Hermite")
    Smooth,
}

pub const SHAPES: [(Shape, &str); 3] = [(Shape::Step, "Step"), (Shape::Curve, "Curve"), (Shape::Smooth, "Smooth")];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub tick: i64,
    /// in the parameter's own units (dB, Hz, %...)
    pub value: f64,
    pub shape: Shape,
    /// -1..1, for `Shape::Curve`
    pub curve: f64,
    /// the value the segment leaves from, when it differs from `value` (a jump at the point: the legacy's
    /// "out value")
    pub jump: Option<f64>,
}

impl Point {
    pub fn new(tick: i64, value: f64, shape: Shape) -> Point {
        Point { tick, value, shape, curve: 0.0, jump: None }
    }
    pub fn out(&self) -> f64 {
        self.jump.unwrap_or(self.value)
    }
}

pub type PointMap = BTreeMap<u64, Point>;
pub type PointChange = (u64, Option<Point>);

/// How a parameter's values are laid out on the lane's height
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Linear,
    /// frequencies
    Log,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamInfo {
    /// the permanent key (`vol`, `cutoff`...)
    pub key: &'static str,
    /// English label key
    pub name: &'static str,
    pub unit: &'static str,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub scale: Scale,
    /// a `Choice` parameter: never interpolated (spec item 1); the values are the indices
    pub choices: &'static [&'static str],
    /// a `Stepped` parameter's step (0 = continuous)
    pub step: f64,
}

impl ParamInfo {
    pub fn discrete(&self) -> bool {
        !self.choices.is_empty() || self.step > 0.0
    }
    /// value -> 0..1 of the lane's height
    pub fn norm(&self, v: f64) -> f64 {
        let v = v.clamp(self.min, self.max);
        match self.scale {
            Scale::Linear => (v - self.min) / (self.max - self.min),
            Scale::Log => (v / self.min).ln() / (self.max / self.min).ln(),
        }
    }
    pub fn denorm(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        let v = match self.scale {
            Scale::Linear => self.min + t * (self.max - self.min),
            Scale::Log => self.min * (self.max / self.min).powf(t),
        };
        self.quantize(v)
    }
    pub fn quantize(&self, v: f64) -> f64 {
        let v = v.clamp(self.min, self.max);
        if !self.choices.is_empty() {
            v.round()
        } else if self.step > 0.0 {
            (v / self.step).round() * self.step
        } else {
            v
        }
    }
    /// The value as shown: the choice's name key, or the number and its unit
    pub fn format(&self, v: f64) -> String {
        if !self.choices.is_empty() {
            return self.choices[(v.round() as usize).min(self.choices.len() - 1)].to_string();
        }
        let a = v.abs();
        let digits = if self.step >= 1.0 || a >= 1000.0 { 0 } else if a >= 100.0 { 1 } else { 2 };
        if self.unit.is_empty() { format!("{v:.digits$}") } else { format!("{v:.digits$} {}", self.unit) }
    }
}

pub const PARAM_VOLUME: ParamInfo = ParamInfo {
    key: "vol",
    name: "Volume",
    unit: "dB",
    min: -60.0,
    max: 6.0,
    default: 0.0,
    scale: Scale::Linear,
    choices: &[],
    step: 0.0,
};
pub const PARAM_PAN: ParamInfo =
    ParamInfo { key: "pan", name: "Pan", unit: "%", min: -100.0, max: 100.0, default: 0.0, scale: Scale::Linear, choices: &[], step: 0.0 };
pub const PARAM_CUTOFF: ParamInfo = ParamInfo {
    key: "cutoff",
    name: "Filter cutoff",
    unit: "Hz",
    min: 20.0,
    max: 20000.0,
    default: 2000.0,
    scale: Scale::Log,
    choices: &[],
    step: 0.0,
};
pub const PARAM_WAVE: ParamInfo = ParamInfo {
    key: "wave1",
    name: "Oscillator 1 wave",
    unit: "",
    min: 0.0,
    max: 5.0,
    default: 0.0,
    scale: Scale::Linear,
    choices: &["Sine", "Triangle", "Saw", "Square", "Moog saw", "Noise"],
    step: 0.0,
};
pub const PARAM_COARSE: ParamInfo = ParamInfo {
    key: "crs1",
    name: "Oscillator 1 coarse",
    unit: "st",
    min: -24.0,
    max: 24.0,
    default: 0.0,
    scale: Scale::Linear,
    choices: &[],
    step: 1.0,
};
pub const PARAM_SEND: ParamInfo = ParamInfo {
    key: "send.reverb",
    name: "Send to Reverb",
    unit: "dB",
    min: -60.0,
    max: 6.0,
    default: -12.0,
    scale: Scale::Linear,
    choices: &[],
    step: 0.0,
};

/// An automation lane: the curve of one parameter, owned by the parameter's owner (spec item 2: a desk channel's
/// lanes live in the channel, a track's and its instrument's in the track)
#[derive(Clone, Debug)]
pub struct AutoLane {
    pub id: NodeId,
    pub owner: NodeId,
    pub owner_name: String,
    pub color: u32,
    pub param: ParamInfo,
    pub points: Arc<PointMap>,
    /// 0..1, for the `Smooth` segments
    pub tension: f64,
    /// recording armed: a touched parameter writes points while the transport records (I-6.4)
    pub armed: bool,
    /// a shared control (P-33): the other parameters this curve moves too, as "owner · parameter"
    pub also: Vec<String>,
}

impl AutoLane {
    /// The points in time order
    pub fn sorted(&self) -> Vec<(u64, Point)> {
        sorted_points(&self.points)
    }
    pub fn value_at(&self, tick: f64) -> f64 {
        value_at(&self.sorted(), tick, self.tension, &self.param)
    }
}

pub fn sorted_points(points: &PointMap) -> Vec<(u64, Point)> {
    let mut v: Vec<(u64, Point)> = points.iter().map(|(n, p)| (*n, *p)).collect();
    v.sort_by_key(|(n, p)| (p.tick, *n));
    v
}

/// The bend of a `Curve` segment: t 0..1 -> 0..1; curve > 0 rises fast then slows, < 0 the opposite
pub fn bend(t: f64, curve: f64) -> f64 {
    if curve.abs() < 1e-6 {
        return t;
    }
    let e = 4f64.powf(-curve * 1.5);
    t.powf(e)
}

/// The lane's value at `tick` (points sorted by tick)
pub fn value_at(pts: &[(u64, Point)], tick: f64, tension: f64, param: &ParamInfo) -> f64 {
    if pts.is_empty() {
        return param.default;
    }
    if tick < pts[0].1.tick as f64 {
        return pts[0].1.value;
    }
    for i in 0..pts.len() {
        let a = pts[i].1;
        let Some(&(_, b)) = pts.get(i + 1) else { return a.out() };
        if tick < b.tick as f64 {
            if param.discrete() || a.shape == Shape::Step {
                return a.out();
            }
            let t = (tick - a.tick as f64) / (b.tick - a.tick).max(1) as f64;
            let (va, vb) = (param.norm(a.out()), param.norm(b.value));
            let v = match a.shape {
                Shape::Step => va,
                Shape::Curve => va + (vb - va) * bend(t, a.curve),
                Shape::Smooth => {
                    // cubic Hermite with Cardinal tangents, in normalized units
                    let prev = if i > 0 { Some(pts[i - 1].1) } else { None };
                    let next = pts.get(i + 2).map(|p| p.1);
                    let span = (b.tick - a.tick).max(1) as f64;
                    let m = |p0: Option<Point>, p1: Point, p2: Option<Point>, v1: f64| -> f64 {
                        let (t0, v0) = p0.map_or((p1.tick as f64 - span, v1), |p| (p.tick as f64, param.norm(p.out())));
                        let (t2, v2) = p2.map_or((p1.tick as f64 + span, v1), |p| (p.tick as f64, param.norm(p.value)));
                        (1.0 - tension) * (v2 - v0) / (t2 - t0).max(1.0) * span
                    };
                    let ma = m(prev, a, Some(b), va);
                    let mb = m(Some(a), b, next, vb);
                    let (t2, t3) = (t * t, t * t * t);
                    (2.0 * t3 - 3.0 * t2 + 1.0) * va + (t3 - 2.0 * t2 + t) * ma + (-2.0 * t3 + 3.0 * t2) * vb + (t3 - t2) * mb
                }
            };
            return param.denorm(v.clamp(0.0, 1.0));
        }
    }
    pts[pts.len() - 1].1.out()
}

/// Flip in time (inside the points' span) or in value (inside the lane's range)
pub fn flip_points(points: &PointMap, which: &[u64], param: &ParamInfo, in_time: bool) -> Vec<PointChange> {
    let sel: Vec<(u64, Point)> = which.iter().filter_map(|n| points.get(n).map(|p| (*n, *p))).collect();
    if sel.is_empty() {
        return Vec::new();
    }
    let (a, b) = (sel.iter().map(|(_, p)| p.tick).min().unwrap(), sel.iter().map(|(_, p)| p.tick).max().unwrap());
    sel.into_iter()
        .map(|(n, p)| {
            let q = if in_time {
                Point { tick: a + b - p.tick, curve: -p.curve, ..p }
            } else {
                let f = |v: f64| param.denorm(1.0 - param.norm(v));
                Point { value: f(p.value), jump: p.jump.map(f), curve: -p.curve, ..p }
            };
            (n, Some(q))
        })
        .collect()
}

/// Scale the values of the points around `center` (normalized 0..1) by `factor`, then add `offset` (normalized):
/// the "value scaling" of a selection
pub fn scale_points(original: &[(u64, Point)], param: &ParamInfo, center: f64, factor: f64, offset: f64) -> Vec<PointChange> {
    let f = |v: f64| param.denorm(center + (param.norm(v) - center) * factor + offset);
    original.iter().map(|(n, p)| (*n, Some(Point { value: f(p.value), jump: p.jump.map(f), ..*p }))).collect()
}

/// Thin a freehand stroke: keep a point only where the line from the last kept one stops explaining it
pub fn thin(stroke: &[(i64, f64)], tolerance: f64) -> Vec<(i64, f64)> {
    if stroke.len() <= 2 {
        return stroke.to_vec();
    }
    let mut out = vec![stroke[0]];
    let mut last = 0;
    for i in 2..stroke.len() {
        let (t0, v0) = stroke[last];
        let (t1, v1) = stroke[i];
        let bad = stroke[last + 1..i].iter().any(|(t, v)| {
            let f = (*t - t0) as f64 / (t1 - t0).max(1) as f64;
            (v0 + (v1 - v0) * f - v).abs() > tolerance
        });
        if bad {
            last = i - 1;
            out.push(stroke[last]);
        }
    }
    out.push(stroke[stroke.len() - 1]);
    out
}

// ---------------------------------------------------------------------------------------------------------------
// In the document

/// The note editors' part of the document
#[derive(Clone, Debug, Default)]
pub struct Music {
    pub clips: Vec<Clip>,
    pub lanes: Vec<AutoLane>,
}

impl Music {
    pub fn clip(&self, id: NodeId) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == id)
    }
    pub fn lane(&self, id: NodeId) -> Option<&AutoLane> {
        self.lanes.iter().find(|l| l.id == id)
    }
    pub fn ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.clips.iter().map(|c| c.id).chain(self.lanes.iter().map(|l| l.id))
    }
}

/// `Edit::set` for the fields of this module
pub(crate) fn set(m: &mut Music, id: NodeId, field: Field, v: Value) -> Result<()> {
    if let Some(c) = m.clips.iter_mut().find(|c| c.id == id) {
        match (field, v) {
            (Field::Notes, Value::Notes(n)) => c.notes = n,
            (Field::Clip("length"), Value::Real(x)) => c.length = (x as i64).max(c.step.max(1)),
            (Field::Clip("loop"), Value::Real(x)) => c.loop_len = (x as i64).max(c.step.max(1)),
            (Field::Clip("swing"), Value::Real(x)) => c.swing = x.clamp(0.0, 1.0),
            (Field::Clip("step"), Value::Real(x)) => c.step = (x as i64).max(1),
            _ => return Err(Error::WrongType),
        }
        return Ok(());
    }
    if let Some(l) = m.lanes.iter_mut().find(|l| l.id == id) {
        match (field, v) {
            (Field::Points, Value::Points(p)) => l.points = p,
            (Field::Clip("tension"), Value::Real(x)) => l.tension = x.clamp(0.0, 1.0),
            (Field::Clip("armed"), Value::Bool(b)) => l.armed = b,
            _ => return Err(Error::WrongType),
        }
        return Ok(());
    }
    Err(Error::NoSuchNode(id))
}

/// `Edit::get` for the fields of this module
pub(crate) fn get(m: &Music, id: NodeId, field: Field) -> Result<Value> {
    if let Some(c) = m.clip(id) {
        return Ok(match field {
            Field::Notes => Value::Notes(c.notes.clone()),
            Field::Clip("length") => Value::Real(c.length as f64),
            Field::Clip("loop") => Value::Real(c.loop_len as f64),
            Field::Clip("swing") => Value::Real(c.swing),
            Field::Clip("step") => Value::Real(c.step as f64),
            _ => return Err(Error::WrongType),
        });
    }
    if let Some(l) = m.lane(id) {
        return Ok(match field {
            Field::Points => Value::Points(l.points.clone()),
            Field::Clip("tension") => Value::Real(l.tension),
            Field::Clip("armed") => Value::Bool(l.armed),
            _ => return Err(Error::WrongType),
        });
    }
    Err(Error::NoSuchNode(id))
}

impl Edit<'_> {
    /// Puts or removes notes of a MIDI item by number (C1)
    pub fn put_notes(&mut self, clip: NodeId, changes: impl IntoIterator<Item = NoteChange>) -> Result<()> {
        let Value::Notes(cur) = self.get(clip, Field::Notes)? else { return Err(Error::WrongType) };
        let mut map = (*cur).clone();
        for (n, note) in changes {
            match note {
                Some(mut x) => {
                    x.key = x.key.min(127);
                    x.start = x.start.max(0);
                    x.length = x.length.max(1);
                    x.velocity = x.velocity.clamp(0.0, 1.0);
                    x.pan = x.pan.clamp(-1.0, 1.0);
                    map.insert(n, x);
                }
                None => {
                    map.remove(&n);
                }
            }
        }
        self.set(clip, Field::Notes, Value::Notes(Arc::new(map)))
    }

    /// Puts or removes points of an automation lane by number
    pub fn put_points(&mut self, lane: NodeId, changes: impl IntoIterator<Item = PointChange>) -> Result<()> {
        let Value::Points(cur) = self.get(lane, Field::Points)? else { return Err(Error::WrongType) };
        let param = self.doc.music.lane(lane).map(|l| l.param).ok_or(Error::NoSuchNode(lane))?;
        let mut map = (*cur).clone();
        for (n, p) in changes {
            match p {
                Some(mut p) => {
                    p.tick = p.tick.max(0);
                    p.value = param.quantize(p.value);
                    p.jump = p.jump.map(|j| param.quantize(j));
                    p.curve = p.curve.clamp(-1.0, 1.0);
                    map.insert(n, p);
                }
                None => {
                    map.remove(&n);
                }
            }
        }
        self.set(lane, Field::Points, Value::Points(Arc::new(map)))
    }
}

/// The global counter of note numbers (S12: an undo never gives a number back). In the real core, the session's
static NEXT_N: AtomicU64 = AtomicU64::new(1 << 41);

/// The CorePort's note edits (I-15): `AddNote` (a new number from the counter; the note is `Value::Note`),
/// `SetNote` (`Field::Note`, the whole note) and `RemoveNote`. A note added earlier in the same list is named by
/// its temp number. The new note's number is reported as a created node of kind `n`.
pub(crate) fn apply_op(e: &mut Edit, op: &crate::port::EditOp, created: &[(u32, NodeId)]) -> Option<Result<Option<(u32, NodeId)>>> {
    use crate::port::{EditOp, NoteRef, Ref};
    let item = |r: &Ref| match r {
        Ref::Id(id) => Ok(*id),
        Ref::Temp(t) => created.iter().find(|(k, _)| k == t).map(|(_, id)| *id).ok_or(Error::WrongType),
    };
    let number = |r: &NoteRef| match r {
        NoteRef::N(n) => Ok(*n),
        NoteRef::Temp(t) => created.iter().find(|(k, id)| k == t && id.kind == b'n').map(|(_, id)| id.number).ok_or(Error::WrongType),
    };
    match op {
        EditOp::AddNote { temp, item: it, note: Value::Note(x) } => Some((|| {
            let id = item(it)?;
            let n = NEXT_N.fetch_add(1, Ordering::Relaxed);
            e.put_notes(id, [(n, Some(x.clone()))])?;
            Ok(Some((*temp, NodeId { kind: b'n', number: n })))
        })()),
        EditOp::SetNote { item: it, n, field: Field::Note, value: Value::Note(x) } => Some((|| {
            let (id, n) = (item(it)?, number(n)?);
            e.put_notes(id, [(n, Some(x.clone()))])?;
            Ok(None)
        })()),
        EditOp::RemoveNote { item: it, n } => Some((|| {
            let (id, n) = (item(it)?, number(n)?);
            e.put_notes(id, [(n, None)])?;
            Ok(None)
        })()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The fake content

/// Numbers of the fake content's notes and points: above anything the session's counter reaches in a run
const DEMO_N: u64 = 1 << 40;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() % 10_000) as f64 / 10_000.0
    }
}

fn map(notes: Vec<Note>, n: &mut u64) -> Arc<NoteMap> {
    Arc::new(
        notes
            .into_iter()
            .map(|x| {
                *n += 1;
                (*n, x)
            })
            .collect(),
    )
}

/// A rock beat with fills, `bars` long
fn drum_notes(bars: i64) -> Vec<Note> {
    let s = BEAT / 4; // a 16th
    let mut v = Vec::new();
    for bar in 0..bars {
        let o = bar * BAR;
        for i in 0..16 {
            let t = o + i * s;
            if i % 2 == 0 {
                let hat = if bar == bars - 1 && i == 14 { 46 } else { 42 };
                v.push(Note::new(hat, t, s, if i % 4 == 0 { 0.82 } else { 0.55 }));
            }
            if [0, 6, 8].contains(&i) || (bar % 2 == 1 && i == 11) {
                v.push(Note::new(36, t, s, if i == 0 { 1.0 } else { 0.85 }));
            }
            if i == 4 || i == 12 {
                v.push(Note::new(38, t, s, 0.92));
            }
        }
        if bar == 0 {
            v.push(Note::new(49, o, BEAT, 0.9));
        }
        if bar == bars - 1 {
            for (k, key) in [48u8, 48, 45, 41].iter().enumerate() {
                v.push(Note::new(*key, o + 12 * s + k as i64 * s, s, 0.7 + 0.05 * k as f64));
            }
            v.push(Note::new(38, o + 15 * s, s / 2, 0.45)); // a ghost note
        }
    }
    v
}

/// A bass line on Am - F - C - G, with a slide (pitch bend) into each bar
fn bass_notes(bars: i64) -> Vec<Note> {
    let roots = [45u8, 41, 48, 43];
    let s = BEAT / 2;
    let mut v = Vec::new();
    for bar in 0..bars {
        let r = roots[bar as usize % 4];
        for (i, (d, len, vel)) in [(0i8, 3, 0.95), (0, 1, 0.6), (12, 1, 0.7), (0, 2, 0.8), (7, 1, 0.65)].iter().enumerate() {
            let at = [0, 3, 4, 5, 7][i];
            let mut n = Note::new((r as i8 + d) as u8, bar * BAR + at * s, len * s - 40, *vel);
            if i == 0 && bar > 0 {
                n.bend = vec![Bend { at: 0, semitones: -2.0 }, Bend { at: 120, semitones: 0.0 }];
            }
            v.push(n);
        }
    }
    v
}

/// Pads and a melody over the same progression, panned a little
fn keys_notes(bars: i64, seed: u64) -> Vec<Note> {
    let chords: [[u8; 4]; 4] = [[57, 60, 64, 69], [53, 57, 60, 65], [55, 60, 64, 67], [55, 59, 62, 67]];
    let mut rng = Rng(seed | 1);
    let mut v = Vec::new();
    for bar in 0..bars {
        let ch = chords[bar as usize % 4];
        for (j, k) in ch.iter().enumerate() {
            let mut n = Note::new(*k, bar * BAR, BAR - 60, 0.55 + 0.05 * j as f64);
            n.pan = -0.3 + 0.2 * j as f64;
            v.push(n);
        }
        let mut t = 0;
        while t < BAR {
            let len = [BEAT / 2, BEAT / 4, BEAT, BEAT / 2][(rng.next() % 4) as usize];
            if rng.unit() > 0.25 {
                let key = to_scale(72 + (rng.next() % 12) as i32, 9, 1);
                v.push(Note::new(key, bar * BAR + t, len - 30, 0.5 + 0.45 * rng.unit()));
            }
            t += len;
        }
    }
    v
}

/// The stress case: about ten thousand 1/64 notes, an arpeggio over 160 bars
fn arpeggio_notes(bars: i64) -> Vec<Note> {
    let s = BEAT / 16;
    let pattern = [0i32, 3, 7, 12, 15, 12, 7, 3];
    let mut v = Vec::new();
    for i in 0..bars * BAR / s {
        let bar = i * s / BAR;
        let root = [57, 53, 48, 55][bar as usize % 4];
        let key = root + pattern[i as usize % 8] + if (i / 64) % 2 == 1 { 12 } else { 0 };
        v.push(Note::new(key as u8, i * s, s - 4, 0.4 + 0.5 * ((i % 16) as f64 / 15.0)));
    }
    v
}

fn lane(id: u64, owner: NodeId, owner_name: &str, color: u32, param: ParamInfo, pts: Vec<Point>, n: &mut u64) -> AutoLane {
    AutoLane {
        id: NodeId { kind: b'a', number: id },
        owner,
        owner_name: owner_name.to_string(),
        color,
        param,
        points: Arc::new(
            pts.into_iter()
                .map(|p| {
                    *n += 1;
                    (*n, p)
                })
                .collect(),
        ),
        tension: 0.5,
        armed: false,
        also: Vec::new(),
    }
}

impl Music {
    /// The fake content, on the document's tracks and channels: a drum pattern on the AVL kit, a bass pattern,
    /// keys, a 10 000-note arpeggio (the dense case), and lanes of every kind (dB, pan, Hz on a log scale, a
    /// choice that never interpolates, a stepped parameter)
    pub fn demo(doc: &Doc) -> Music {
        let mut n = DEMO_N;
        let mut m = Music::default();
        let track = |i: usize| doc.tracks.get(i.min(doc.tracks.len().saturating_sub(1)));
        let mut clip = |id: u64, ti: usize, name: &str, start_bar: i64, bars: i64, loop_bars: i64, notes: Vec<Note>, kit: bool, n: &mut u64| {
            let Some(t) = track(ti) else { return };
            m.clips.push(Clip {
                id: NodeId { kind: b'c', number: id },
                track: t.id,
                track_name: t.name.clone(),
                name: name.to_string(),
                color: t.color,
                start: start_bar * BAR,
                length: bars * BAR,
                loop_len: loop_bars * BAR,
                notes: map(notes, n),
                swing: if kit { 0.18 } else { 0.0 },
                step: BEAT / 4,
                kit: kit.then_some(AVL_KIT),
            });
        };
        clip(7001, 1, "Groove A", 0, 8, 2, drum_notes(2), true, &mut n);
        clip(7002, 1, "Groove B (fill)", 8, 4, 1, drum_notes(1), true, &mut n);
        clip(7003, 4, "Bass line", 0, 16, 4, bass_notes(4), false, &mut n);
        clip(7004, 5, "Keys", 0, 8, 8, keys_notes(8, 7), false, &mut n);
        clip(7005, 13, "Arpeggio (10 000 notes)", 0, 160, 160, arpeggio_notes(160), false, &mut n);
        let pads: Vec<Note> = (0..16)
            .flat_map(|i| {
                let t = i * BEAT / 4;
                let mut v = vec![Note::new(63, t, BEAT / 4, if i % 4 == 2 { 0.9 } else { 0.5 })];
                if i % 4 == 0 {
                    v.push(Note::new(60, t, BEAT / 4, 1.0));
                }
                if i == 4 || i == 12 {
                    v.push(Note::new(62, t, BEAT / 4, 0.85));
                }
                if i == 14 {
                    v.push(Note::new(67, t, BEAT / 2, 0.7));
                }
                v
            })
            .collect();
        clip(7006, 7, "Pads (samples)", 0, 8, 1, pads, true, &mut n);
        if let Some(c) = m.clips.last_mut() {
            c.kit = Some(PADS_KIT);
            c.swing = 0.0;
        }

        let t = |i: usize| track(i).map(|t| (t.id, t.name.clone(), t.color));
        let ch = |i: usize| doc.channels.get(i).map(|c| (c.id, c.name.clone()));
        if let Some((id, name, color)) = t(2) {
            let pts = (0..16)
                .map(|k| {
                    let mut p = Point::new(k * BAR, if k % 4 == 3 { -12.0 } else { -3.0 + 2.0 * ((k as f64) * 0.8).sin() }, Shape::Curve);
                    p.curve = [0.0, 0.5, -0.6, 0.0][k as usize % 4];
                    p
                })
                .collect();
            m.lanes.push(lane(9001, id, &name, color, PARAM_VOLUME, pts, &mut n));
        }
        if let Some((id, name, color)) = t(5) {
            let pts = vec![
                Point::new(0, 300.0, Shape::Smooth),
                Point::new(2 * BAR, 2400.0, Shape::Smooth),
                Point::new(4 * BAR, 800.0, Shape::Smooth),
                Point::new(6 * BAR, 9000.0, Shape::Curve),
                Point { curve: -0.7, ..Point::new(7 * BAR, 1200.0, Shape::Curve) },
                Point::new(8 * BAR, 500.0, Shape::Step),
            ];
            let mut l = lane(9002, id, &name, color, PARAM_CUTOFF, pts, &mut n);
            l.also = vec![format!("{name} · Filter 2 cutoff"), "Bass line · Filter cutoff".to_string()];
            m.lanes.push(l);
            let pts = vec![Point::new(0, 2.0, Shape::Step), Point::new(4 * BAR, 3.0, Shape::Step), Point::new(6 * BAR, 0.0, Shape::Step)];
            m.lanes.push(lane(9003, id, &name, color, PARAM_WAVE, pts, &mut n));
            let pts = vec![Point::new(0, 0.0, Shape::Step), Point::new(2 * BAR, 7.0, Shape::Step), Point::new(3 * BAR, 12.0, Shape::Step), Point::new(4 * BAR, 0.0, Shape::Step)];
            m.lanes.push(lane(9004, id, &name, color, PARAM_COARSE, pts, &mut n));
        }
        if let Some((id, name)) = ch(0) {
            let color = doc.tracks.first().map_or(0, |t| t.color);
            let pts = (0..9).map(|k| Point { curve: 0.4, ..Point::new(k * BAR, if k % 2 == 0 { -60.0 } else { 60.0 }, Shape::Curve) }).collect();
            m.lanes.push(lane(9005, id, &name, color, PARAM_PAN, pts, &mut n));
            let pts = vec![
                Point::new(0, -30.0, Shape::Curve),
                Point { jump: Some(-6.0), ..Point::new(4 * BAR, -24.0, Shape::Curve) },
                Point::new(8 * BAR, -18.0, Shape::Curve),
            ];
            m.lanes.push(lane(9006, id, &name, color, PARAM_SEND, pts, &mut n));
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes(v: &[(u8, i64, i64)]) -> NoteMap {
        v.iter().enumerate().map(|(i, (k, s, l))| (i as u64 + 1, Note::new(*k, *s, *l, 0.8))).collect()
    }

    #[test]
    fn quantize_moves_starts_and_lengths_to_the_grid() {
        let m = notes(&[(60, 230, 470), (62, 500, 100)]);
        let c = quantize(&m, &[1, 2], BEAT / 4, QuantizeWhat::Both, 1.0);
        assert_eq!(c[0].1.as_ref().unwrap().start, 240);
        assert_eq!(c[0].1.as_ref().unwrap().length, 480);
        assert_eq!(c[1].1.as_ref().unwrap().length, 240, "at least one grid");
        let half = quantize(&m, &[1], BEAT / 4, QuantizeWhat::Starts, 0.5);
        assert_eq!(half[0].1.as_ref().unwrap().start, 235);
    }

    #[test]
    fn glue_split_reverse_and_strum() {
        let m = notes(&[(60, 0, 240), (60, 240, 240), (60, 600, 100), (64, 0, 10)]);
        let g = glue(&m, &[1, 2, 3, 4]);
        assert!(g.contains(&(2, None)));
        assert_eq!(g.iter().find(|c| c.0 == 1).unwrap().1.as_ref().unwrap().length, 480);
        let mut next = 100;
        let s = split(&m, &[1], 100, &mut || {
            next += 1;
            next
        }, false);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].1.as_ref().unwrap().start, 100);
        let r = reverse(&m, &[1, 3]);
        assert_eq!(r.iter().find(|c| c.0 == 3).unwrap().1.as_ref().unwrap().start, 0);
        let chord: Vec<(u64, Note)> = [(1, Note::new(60, 0, 960, 1.0)), (2, Note::new(67, 0, 960, 1.0)), (3, Note::new(64, 0, 960, 1.0))].to_vec();
        let st = strum(&chord, 120);
        assert_eq!(st.iter().find(|c| c.0 == 2).unwrap().1.as_ref().unwrap().start, 120, "the highest last");
    }

    #[test]
    fn scales_and_chords() {
        assert!(in_scale(69, 9, 1)); // A in A minor
        assert!(!in_scale(70, 9, 1));
        assert_eq!(to_scale(70, 9, 1), 69);
        assert_eq!(chord_keys(60, 7), vec![60, 64, 67, 71]);
        assert_eq!(key_name(60), "C4");
    }

    #[test]
    fn automation_shapes() {
        let p = PARAM_PAN;
        let pts = sorted_points(&[(1, Point::new(0, -100.0, Shape::Curve)), (2, Point::new(960, 100.0, Shape::Step))].into_iter().collect());
        assert!((value_at(&pts, 480.0, 0.5, &p)).abs() < 1e-9, "straight line");
        assert_eq!(value_at(&pts, 2000.0, 0.5, &p), 100.0);
        let bent = sorted_points(&[(1, Point { curve: 0.8, ..Point::new(0, -100.0, Shape::Curve) }), (2, Point::new(960, 100.0, Shape::Curve))].into_iter().collect());
        assert!(value_at(&bent, 480.0, 0.5, &p) > 0.0, "a positive curve rises fast");
        // a choice never interpolates
        let w = sorted_points(&[(1, Point::new(0, 0.0, Shape::Curve)), (2, Point::new(960, 4.0, Shape::Curve))].into_iter().collect());
        assert_eq!(value_at(&w, 900.0, 0.5, &PARAM_WAVE), 0.0);
        // the log scale puts 632 Hz in the middle of 20..20000
        assert!((PARAM_CUTOFF.denorm(0.5) - 632.4).abs() < 1.0);
        let thinned = thin(&(0..100).map(|i| (i * 10, 0.5)).collect::<Vec<_>>(), 0.01);
        assert_eq!(thinned.len(), 2, "a flat stroke is two points");
    }

    #[test]
    fn lengths_print_as_fractions() {
        assert_eq!(format_len(BEAT / 4), "1/16");
        assert_eq!(format_len(BEAT * 3 / 4), "3/16");
        assert_eq!(format_len(BEAT / 3), "1/8T");
        assert_eq!(format_bbt(BAR + BEAT + 5), "2.2.005");
    }
}
