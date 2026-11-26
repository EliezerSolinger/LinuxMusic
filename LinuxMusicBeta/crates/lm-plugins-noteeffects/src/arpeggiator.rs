//! The arpeggiator (legacy `InstrumentFunctionArpeggio`, F-IJN-015, moved out of the instruments): each held note
//! plays the steps of a chord or scale — up, down, up and down, down and up or at random, over `range` octaves, with
//! repeats, a cycle, a chance to skip a step or to miss it (a random one), a step `time` (free, in ms, or synced to
//! the tempo) and a `gate` (how long each step sounds, as a share of the step).
//!
//! The held note itself is silent (the legacy master note). The three modes: **free** (each held note has its own
//! steps, from the moment it was struck), **sync** (all of them step together, from the oldest) and **sort** (the
//! held notes, sorted, are the chord's roots, and only one step plays at a time).
//!
//! Steps and gate ends are placed on the engine's frames (`transport.frame`, which never jumps), so they fall on
//! the exact sample and do not depend on the block size. The sustain pedal holds the notes' arpeggios until it is
//! released. The steps are notes of the effect's own, numbered from [`lm_plugin::note_effect::GENERATED_IDS`].

use lm_base::Result;
use lm_plugin::instrument::{SYNC_KEYS, sync_beats};
use lm_plugin::note_effect::{BlockCx, Expressions, IdCounter, NoteEffect, NoteFx};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::{EventKind, NoteId, NoteOut, Processor};
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::table::{CHORD_KEYS, chord};

pub const ID: &str = "arpeggiator";
/// Held notes followed at once, and steps sounding at once
pub const MAX_BASES: usize = 128;
pub const MAX_STEPS: usize = 1024;

pub static DIRECTIONS: [&str; 5] = ["up", "down", "up-down", "down-up", "random"];
pub static MODES: [&str; 3] = ["free", "sort", "sync"];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.arpeggiator.name",
    kind: PluginKind::NoteEffect,
    schemas,
    ports: crate::no_ports,
    velocity: Velocity::Ignored,
    display: None,
    build,
    tests: TestHints::new(),
};

/// The parameters, by index
mod p {
    pub const RANGE: usize = 0;
    pub const REPEATS: usize = 1;
    pub const CYCLE: usize = 2;
    pub const SKIP: usize = 3;
    pub const MISS: usize = 4;
    pub const TIME: usize = 5;
    pub const GATE: usize = 6;
    pub const COUNT: usize = 7;
}

fn schemas() -> Vec<NodeSchema> {
    vec![
        NodeSchema::new("plugin:arpeggiator", 1)
            .field(FieldSpec::choice("chord", &CHORD_KEYS, "octave"))
            .field(FieldSpec::choice("direction", &DIRECTIONS, "up"))
            .field(FieldSpec::choice("mode", &MODES, "free"))
            .field(FieldSpec::choice("sync", &SYNC_KEYS, "off"))
            .param(ParamSpec::new("range", 1.0, 9.0, 1.0).scale(Scale::Stepped))
            .param(ParamSpec::new("repeats", 1.0, 8.0, 1.0).scale(Scale::Stepped))
            .param(ParamSpec::new("cycle", 0.0, 6.0, 0.0).scale(Scale::Stepped))
            .param(ParamSpec::new("skip", 0.0, 100.0, 0.0).unit("%"))
            .param(ParamSpec::new("miss", 0.0, 100.0, 0.0).unit("%"))
            .param(ParamSpec::new("time", 25.0, 2000.0, 200.0).unit("ms"))
            .param(ParamSpec::new("gate", 1.0, 200.0, 100.0).unit("%")),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
    UpDown,
    DownUp,
    Random,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Free,
    Sort,
    Sync,
}

fn build(state: &Value, _: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let v = StateView::new(state);
    let direction = v.choice("direction", |k| {
        Some(match k {
            "up" => Direction::Up,
            "down" => Direction::Down,
            "up-down" => Direction::UpDown,
            "down-up" => Direction::DownUp,
            "random" => Direction::Random,
            _ => return None,
        })
    })?;
    let mode = v.choice("mode", |k| {
        Some(match k {
            "free" => Mode::Free,
            "sort" => Mode::Sort,
            "sync" => Mode::Sync,
            _ => return None,
        })
    })?;
    let fx = Arpeggiator {
        chord: v.choice("chord", chord)?,
        direction,
        mode,
        sync: v.choice("sync", |s| SYNC_KEYS.contains(&s).then(|| sync_beats(s)))?,
        values: [0.0; p::COUNT],
        bases: Vec::with_capacity(MAX_BASES),
        steps: Vec::with_capacity(MAX_STEPS),
        expr: Expressions::new(MAX_BASES),
        ids: IdCounter::default(),
        sustain: false,
    };
    Ok(Box::new(NoteFx::new(fx, &v, &DESCRIPTOR.schema().params)?))
}

#[derive(Clone, Copy, Debug)]
struct Base {
    id: NoteId,
    channel: u8,
    key: u8,
    velocity: f32,
    /// The engine frame it was struck at
    start: u64,
    /// The next step (free mode)
    step: u64,
    /// Let go while the pedal was down
    sustained: bool,
}

/// A step sounding
#[derive(Clone, Copy, Debug)]
struct Step {
    id: NoteId,
    base: NoteId,
    channel: u8,
    key: u8,
    /// The engine frame its gate ends at
    end: u64,
}

/// The arpeggiator
#[derive(Debug)]
pub struct Arpeggiator {
    chord: &'static [u8],
    direction: Direction,
    mode: Mode,
    /// Beats of a synced step, or `None` (free: `time`)
    sync: Option<f64>,
    values: [f32; p::COUNT],
    bases: Vec<Base>,
    steps: Vec<Step>,
    expr: Expressions,
    ids: IdCounter,
    sustain: bool,
}

impl Arpeggiator {
    fn step_frames(&self, cx: &BlockCx<'_>) -> u64 {
        match self.sync {
            Some(beats) => cx.beats_to_frames(beats),
            None => cx.ms_to_frames(f64::from(self.values[p::TIME])),
        }
    }

    fn release_steps(&mut self, base: NoteId, at: u32, out: &mut NoteOut) {
        let mut i = 0;
        while i < self.steps.len() {
            if self.steps[i].base == base {
                let s = self.steps.swap_remove(i);
                out.push(at, EventKind::NoteOff { note: s.id, channel: s.channel, key: s.key, velocity: 0.0 });
            } else {
                i += 1;
            }
        }
    }

    fn end_base(&mut self, i: usize, at: u32, out: &mut NoteOut) {
        let b = self.bases.swap_remove(i);
        self.expr.end(b.id);
        self.release_steps(b.id, at, out);
    }

    /// The steps and gate ends due at frame `now`; `at` is its offset in the block
    fn timed(&mut self, now: u64, at: u32, cx: &mut BlockCx<'_>, out: &mut NoteOut) {
        let mut i = 0;
        while i < self.steps.len() {
            if self.steps[i].end <= now {
                let s = self.steps.swap_remove(i);
                out.push(at, EventKind::NoteOff { note: s.id, channel: s.channel, key: s.key, velocity: 0.0 });
            } else {
                i += 1;
            }
        }
        if self.bases.is_empty() {
            return;
        }
        let af = self.step_frames(cx);
        match self.mode {
            Mode::Free => {
                for j in 0..self.bases.len() {
                    let b = self.bases[j];
                    let mut step = b.step;
                    while b.start + step * af <= now {
                        if b.start + step * af == now {
                            self.play(cx, out, &b, step, af, now, at);
                        }
                        step += 1;
                    }
                    self.bases[j].step = step;
                }
            }
            Mode::Sync | Mode::Sort => {
                let Some(anchor) = self.bases.iter().map(|b| b.start).min() else { return };
                let since = now.saturating_sub(anchor);
                if !since.is_multiple_of(af) {
                    return;
                }
                let n = since / af;
                if self.mode == Mode::Sync {
                    for j in 0..self.bases.len() {
                        let b = self.bases[j];
                        self.play(cx, out, &b, n, af, now, at);
                    }
                } else if let Some(lowest) = self.bases.iter().min_by_key(|b| (b.key, b.start)).copied() {
                    self.play(cx, out, &lowest, n, af, now, at);
                }
            }
        }
    }

    /// The next frame after `now` at which a step or a gate end falls
    fn next_timed(&self, now: u64, cx: &BlockCx<'_>) -> u64 {
        let mut next = self.steps.iter().map(|s| s.end).filter(|e| *e > now).min().unwrap_or(u64::MAX);
        if !self.bases.is_empty() {
            let af = self.step_frames(cx);
            match self.mode {
                Mode::Free => {
                    for b in &self.bases {
                        let mut t = b.start + b.step * af;
                        if t <= now {
                            t = b.start + ((now - b.start) / af + 1) * af;
                        }
                        next = next.min(t);
                    }
                }
                Mode::Sync | Mode::Sort => {
                    if let Some(anchor) = self.bases.iter().map(|b| b.start).min() {
                        next = next.min(anchor + ((now.saturating_sub(anchor)) / af + 1) * af);
                    }
                }
            }
        }
        next
    }

    /// Plays step `n` of a base's arpeggio (legacy `InstrumentFunctionArpeggio::processNote`, one step)
    #[allow(clippy::too_many_arguments)]
    fn play(&mut self, cx: &mut BlockCx<'_>, out: &mut NoteOut, base: &Base, n: u64, af: u64, now: u64, at: u32) {
        let chord = self.chord;
        let cs = chord.len() as i64;
        let repeats = (self.values[p::REPEATS] as i64).max(1);
        let single = cs * (self.values[p::RANGE] as i64).max(1) * repeats;
        let sort = self.mode == Mode::Sort;
        let mut keys = [0u8; MAX_BASES];
        let mut count = 0usize;
        if sort {
            for b in &self.bases {
                keys[count] = b.key;
                count += 1;
            }
            keys[..count].sort_unstable();
        }
        let range = if sort { single * count.max(1) as i64 } else { single };
        let skip = self.values[p::SKIP];
        if skip > 0.0 && cx.rng.next_f32() * 100.0 <= skip {
            return;
        }
        let mut dir = self.direction;
        let miss = self.values[p::MISS];
        if miss > 0.0 && cx.rng.next_f32() * 100.0 <= miss {
            dir = Direction::Random;
        }
        let n = n as i64;
        let mut idx = match dir {
            Direction::Up | Direction::Down => n % range,
            Direction::UpDown | Direction::DownUp if range > 1 => {
                let period = (range * 2 - 2 * repeats).max(1);
                let mut i = n % period;
                if i >= range {
                    i = range - i % (range - 1) - repeats;
                }
                i
            }
            Direction::UpDown | Direction::DownUp => 0,
            Direction::Random => cx.rng.below(range.max(1) as u64) as i64,
        };
        idx /= repeats;
        let cycle = self.values[p::CYCLE] as i64;
        let per = (range / repeats).max(1);
        if cycle > 0 && dir != Direction::Random {
            idx = (idx * (cycle + 1)) % per;
        }
        if matches!(dir, Direction::Down | Direction::DownUp) {
            idx = per - idx - 1;
        }
        let idx = idx.max(0);
        let key = if sort {
            let count = count.max(1) as i64;
            let total = cs * count;
            let octave = idx / total;
            let rem = idx % total;
            let arp_index = (rem % count) as usize;
            let chord_index = (rem / count) as usize;
            i64::from(keys[arp_index]) + i64::from(chord[chord_index.min(chord.len() - 1)]) + octave * 12
        } else {
            i64::from(base.key) + (idx / cs) * 12 + i64::from(chord[(idx % cs) as usize])
        };
        if !(0..128).contains(&key) || self.steps.len() == self.steps.capacity() {
            return;
        }
        let gated = ((f64::from(self.values[p::GATE]) * af as f64 / 100.0) as u64).max(1);
        let id = self.ids.next();
        let key = key as u8;
        self.steps.push(Step { id, base: base.id, channel: base.channel, key, end: now + gated });
        out.push(at, EventKind::NoteOn { note: id, channel: base.channel, key, velocity: base.velocity });
        self.expr.apply(base.id, id, at, out);
    }
}

impl NoteEffect for Arpeggiator {
    const PARAMS: usize = p::COUNT;

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(v) = self.values.get_mut(index) {
            *v = value;
        }
    }

    fn event(&mut self, at: u32, kind: EventKind, cx: &mut BlockCx<'_>, out: &mut NoteOut) {
        match kind {
            EventKind::NoteOn { note, channel, key, velocity } => {
                if self.bases.len() < self.bases.capacity() && !self.bases.iter().any(|b| b.id == note) {
                    self.bases.push(Base {
                        id: note,
                        channel,
                        key,
                        velocity,
                        start: cx.frame_at(at),
                        step: 0,
                        sustained: false,
                    });
                    self.expr.begin(note);
                }
            }
            EventKind::NoteOff { note, .. } => {
                if let Some(i) = self.bases.iter().position(|b| b.id == note) {
                    if self.sustain {
                        self.bases[i].sustained = true;
                    } else {
                        self.end_base(i, at, out);
                    }
                }
            }
            EventKind::NoteExpression { note, expression, value, ramp } => {
                self.expr.set(note, expression, value);
                for s in self.steps.iter().filter(|s| s.base == note) {
                    out.push(at, EventKind::NoteExpression { note: s.id, expression, value, ramp });
                }
            }
            EventKind::Midi { bytes } => {
                if bytes[0] & 0xF0 == 0xB0 && bytes[1] == 64 {
                    self.sustain = bytes[2] >= 64;
                    if !self.sustain {
                        while let Some(i) = self.bases.iter().position(|b| b.sustained) {
                            self.end_base(i, at, out);
                        }
                    }
                }
                out.push(at, kind);
            }
            EventKind::AllNotesRelease | EventKind::AllSoundOff => {
                self.bases.clear();
                self.steps.clear();
                self.expr.clear();
                self.sustain = false;
                out.push(at, kind);
            }
            kind => {
                out.push(at, kind);
            }
        }
    }

    fn run(&mut self, from: u32, to: u32, cx: &mut BlockCx<'_>, out: &mut NoteOut) {
        let base = cx.frame_at(0);
        let end = base + u64::from(to);
        let mut now = base + u64::from(from);
        loop {
            self.timed(now, (now - base) as u32, cx, out);
            let next = self.next_timed(now, cx);
            if next >= end {
                break;
            }
            now = next;
        }
    }

    fn reset(&mut self) {
        self.bases.clear();
        self.steps.clear();
        self.expr.clear();
        self.sustain = false;
    }

    fn take_state(&mut self, old: &mut Self) {
        std::mem::swap(&mut self.bases, &mut old.bases);
        std::mem::swap(&mut self.steps, &mut old.steps);
        std::mem::swap(&mut self.expr, &mut old.expr);
        self.ids = old.ids;
        self.sustain = old.sustain;
    }
}
