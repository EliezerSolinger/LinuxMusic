//! Modulators (F-CTL; decisoes-autonomas, "O que deixa de seguir o legado", item 1): sources that move a parameter
//! around the value its automation or the document gives it. They live with the owner of the parameter (a track, a
//! channel or a plugin) in the document and compile to the fixed table of the program: no allocation on the audio
//! thread.
//!
//! Five kinds: LFO (the legacy controller's math), envelope (ADSR, triggered by the notes a node plays), peak follower
//! (the legacy Peak Controller's math, measuring the output of a track or channel), random (sample and hold, with
//! optional smoothing) and macro (one knob). Each gives `b`, from -1 to 1 (the LFO and random) or from 0 to 1. A
//! target's value is `clamp(n + amount * b, min, max)`, `n` being the parameter's position (0 to 1, by its range's
//! law) before modulation.
//!
//! Everything is a pure function of the timeline sample (the engine's frame while stopped), the parameters and the
//! modulator's seed, evaluated on the automation grid ([`crate::AUTOMATION_GRID`] samples of the engine's frame
//! count), so the events fall on the same samples whatever the block size

use std::sync::Arc;

use crate::ids::{NodeIndex, ParamSlot};
use crate::midimap::ParamRange;

/// What a modulator does
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModKind {
    Lfo,
    Envelope,
    Peak,
    Random,
    Macro,
}

/// The parameters of a modulator, by index. The keys are the document's (`MOD_KEYS`, in this order)
pub const MOD_KEYS: [&str; MOD_PARAMS] = [
    "speed",
    "phase",
    "wave",
    "multiplier",
    "sync",
    "syncNumerator",
    "syncDenominator",
    "depth",
    "attack",
    "decay",
    "sustain",
    "release",
    "peakAttack",
    "peakDecay",
    "threshold",
    "gain",
    "abs",
    "rate",
    "smooth",
    "value",
];
pub const MOD_PARAMS: usize = 20;
pub const P_SPEED: usize = 0;
pub const P_PHASE: usize = 1;
pub const P_WAVE: usize = 2;
pub const P_MULTIPLIER: usize = 3;
pub const P_SYNC: usize = 4;
pub const P_SYNC_NUM: usize = 5;
pub const P_SYNC_DEN: usize = 6;
pub const P_DEPTH: usize = 7;
pub const P_ATTACK: usize = 8;
pub const P_DECAY: usize = 9;
pub const P_SUSTAIN: usize = 10;
pub const P_RELEASE: usize = 11;
pub const P_PEAK_ATTACK: usize = 12;
pub const P_PEAK_DECAY: usize = 13;
pub const P_THRESHOLD: usize = 14;
pub const P_GAIN: usize = 15;
pub const P_ABS: usize = 16;
pub const P_RATE: usize = 17;
pub const P_SMOOTH: usize = 18;
pub const P_VALUE: usize = 19;

/// The defaults of [`MOD_KEYS`] (the document's schema has the same)
pub const MOD_DEFAULTS: [f32; MOD_PARAMS] =
    [2.0, 0.0, 0.0, 0.0, 0.0, 1.0, 4.0, 1.0, 10.0, 200.0, 0.5, 300.0, 0.0, 0.0, 0.0, 1.0, 1.0, 4.0, 0.0, 0.0];

/// One target of a modulator: a parameter slot, its range (the law that gives its position `n`), the `amount`
/// (-1 to 1) and the window `min`..`max` (0 to 1 of the range) the result stays in
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModTarget {
    pub slot: ParamSlot,
    pub range: ParamRange,
    pub amount: f32,
    pub min: f32,
    pub max: f32,
}

/// A modulator for the program builder
#[derive(Clone, Debug)]
pub struct ModulatorSpec {
    pub kind: ModKind,
    /// The random's and the noise wave's seed (the compiler hashes the project seed and the node's id, C13)
    pub seed: u64,
    /// One cycle of the LFO's "user" wave
    pub user_wave: Option<Arc<[f32]>>,
    /// The parameters' values now (the tree's, S20)
    pub initial: [f32; MOD_PARAMS],
    /// The peak follower measures this node's output
    pub input: Option<NodeIndex>,
    /// The envelope starts with the notes this node plays
    pub trigger: Option<NodeIndex>,
    pub targets: Vec<ModTarget>,
}

impl ModulatorSpec {
    pub fn new(kind: ModKind, seed: u64) -> Self {
        Self { kind, seed, user_wave: None, initial: MOD_DEFAULTS, input: None, trigger: None, targets: Vec::new() }
    }
}

/// `x - floor(x)`
pub fn frac(x: f64) -> f64 {
    x - libm::floor(x)
}

/// The sync notes' length in beats: double whole, whole, half, quarter, eighth, sixteenth, thirty-second
const SYNC_BEATS: [f64; 7] = [8.0, 4.0, 2.0, 1.0, 0.5, 0.25, 0.125];

/// The LFO's phase in cycles (not wrapped) at a timeline sample: free (`sync` 0), from the sample, with the speed in
/// seconds per cycle times the multiplier (x1, /100, x100: the legacy `updateDuration`); or tempo synced, from the
/// tick, a cycle being a note (or `syncNumerator/syncDenominator` of a whole note) in beats
pub fn lfo_phase(p: &[f32; MOD_PARAMS], sample_rate: u32, sample: i64, tick: i64, ppq: u32) -> f64 {
    let offset = f64::from(p[P_PHASE]) / 360.0;
    let sync = p[P_SYNC].round() as usize;
    let cycles = if sync == 0 {
        let mut seconds = f64::from(p[P_SPEED]).max(1e-6);
        match p[P_MULTIPLIER].round() as i64 {
            1 => seconds /= 100.0,
            2 => seconds *= 100.0,
            _ => {}
        }
        sample as f64 / (seconds * f64::from(sample_rate))
    } else {
        let beats = match sync {
            1..=7 => SYNC_BEATS[sync - 1],
            _ => 4.0 * f64::from(p[P_SYNC_NUM].max(1.0)) / f64::from(p[P_SYNC_DEN].max(1.0)),
        };
        tick as f64 / (beats * f64::from(ppq.max(1)))
    };
    cycles + offset
}

/// The splitmix64 finalizer: a stateless random from a seed and a counter
pub fn hash(seed: u64, n: u64) -> u64 {
    let mut z = seed.wrapping_add(n.wrapping_mul(0x9E37_79B9_7F4A_7C15)).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Uniform in -1..1 from a seed and a counter
pub fn bipolar(seed: u64, n: i64) -> f32 {
    ((hash(seed, n as u64) >> 40) as f32) * (2.0 / (1u64 << 24) as f32) - 1.0
}

/// The legacy oscillator's shapes at a phase in cycles (`Oscillator::*Sample`): 0 sine, 1 triangle, 2 saw, 3 square,
/// 4 Moog saw, 5 exponential, 6 white noise (held for a cycle), 7 the user wave (linear between its frames)
pub fn lfo_shape(wave: usize, phase: f64, seed: u64, user: Option<&[f32]>) -> f32 {
    let ph = frac(phase) as f32;
    match wave {
        0 => libm::sin(phase * std::f64::consts::TAU) as f32,
        1 => {
            if ph <= 0.25 {
                ph * 4.0
            } else if ph <= 0.75 {
                2.0 - ph * 4.0
            } else {
                ph * 4.0 - 4.0
            }
        }
        2 => -1.0 + ph * 2.0,
        3 => {
            if ph > 0.5 {
                -1.0
            } else {
                1.0
            }
        }
        4 => {
            if ph < 0.5 {
                -1.0 + ph * 4.0
            } else {
                1.0 - 2.0 * ph
            }
        }
        5 => {
            let p = if ph > 0.5 { 1.0 - ph } else { ph };
            -1.0 + 8.0 * p * p
        }
        6 => bipolar(seed, libm::floor(phase) as i64),
        _ => match user {
            Some(t) if !t.is_empty() => {
                let pos = frac(phase) * t.len() as f64;
                let i = pos as usize;
                let a = t[i.min(t.len() - 1)];
                let b = t[(i + 1) % t.len()];
                a + (b - a) * (pos - i as f64) as f32
            }
            _ => 0.0,
        },
    }
}

/// The LFO's `b` (-1 to 1) at a phase: the shape times `depth`
pub fn lfo_value(p: &[f32; MOD_PARAMS], phase: f64, seed: u64, user: Option<&[f32]>) -> f32 {
    (lfo_shape(p[P_WAVE].round().max(0.0) as usize, phase, seed, user) * p[P_DEPTH]).clamp(-1.0, 1.0)
}

/// The random's `b` (-1 to 1) at a timeline sample: a new value every `1 / rate` seconds, held, or glided to over the
/// first `smooth` of the cycle
pub fn random_value(p: &[f32; MOD_PARAMS], sample_rate: u32, sample: i64, seed: u64) -> f32 {
    let cycles = sample as f64 * f64::from(p[P_RATE].max(0.001)) / f64::from(sample_rate.max(1));
    let n = libm::floor(cycles) as i64;
    let cur = bipolar(seed, n);
    let smooth = p[P_SMOOTH].clamp(0.0, 1.0);
    if smooth <= 0.0 {
        return cur;
    }
    let prev = bipolar(seed, n - 1);
    let u = (frac(cycles) as f32 / smooth).min(1.0);
    prev + (cur - prev) * (u * u * (3.0 - 2.0 * u))
}

/// The envelope's stages
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// An ADSR (times in ms, `sustain` 0 to 1), advanced in frames. Linear segments; a gate on restarts from the level
/// it has
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Envelope {
    pub stage: Stage,
    pub level: f32,
}

impl Envelope {
    pub const fn new() -> Self {
        Self { stage: Stage::Idle, level: 0.0 }
    }

    pub fn gate(&mut self, on: bool) {
        if on {
            self.stage = Stage::Attack;
        } else if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    pub fn advance(&mut self, p: &[f32; MOD_PARAMS], sample_rate: u32, frames: u32) {
        let per_ms = f64::from(sample_rate) / 1000.0;
        let step = |ms: f32| -> f32 {
            let n = f64::from(ms.max(0.0)) * per_ms;
            if n < 1.0 { 1.0 } else { (f64::from(frames) / n) as f32 }
        };
        let sustain = p[P_SUSTAIN].clamp(0.0, 1.0);
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += step(p[P_ATTACK]);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level -= step(p[P_DECAY]) * (1.0 - sustain);
                if self.level <= sustain {
                    self.level = sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => self.level = sustain,
            Stage::Release => {
                self.level -= step(p[P_RELEASE]) * self.level.max(1e-3).max(sustain.min(0.05));
                if self.level <= 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
    }
}

impl Default for Envelope {
    fn default() -> Self {
        Self::new()
    }
}

/// The sum the legacy peak controller measures over a stretch of a stereo signal (`PeakControllerEffect::process`):
/// the sum of squares with `abs`, signed squares without
pub fn peak_sum(left: &[f32], right: &[f32], absolute: bool) -> f64 {
    let mut sum = 0.0f64;
    for (l, r) in left.iter().zip(right) {
        if absolute {
            sum += f64::from(l * l + r * r);
        } else {
            sum += f64::from(l * l.abs() + r * r.abs());
        }
    }
    sum
}

/// The rest of the measurement for `frames` frames summing to `sum`: the RMS, the threshold, then
/// `clamp(amount * rms, 0, 1)` with `amount` the gain parameter
pub fn peak_finish(sum: f64, frames: usize, p: &[f32; MOD_PARAMS]) -> f32 {
    let v = sum / frames.max(1) as f64;
    // sqrt_neg
    let rms = if v < 0.0 { -libm::sqrt(-v) } else { libm::sqrt(v) } as f32;
    let rms = if rms.abs() < p[P_THRESHOLD] { 0.0 } else { rms };
    (p[P_GAIN] * rms).clamp(0.0, 1.0)
}

/// The legacy measurement of one block of a stereo signal
pub fn peak_measure(left: &[f32], right: &[f32], p: &[f32; MOD_PARAMS]) -> f32 {
    peak_finish(peak_sum(left, right, p[P_ABS] >= 0.5), left.len(), p)
}

/// How far behind the signal the peak follower reads, in frames. A window of the measured node is only complete
/// after the pass that renders it, and the modulation of a pass is computed before the pass renders, so a reading
/// that is the same for every slicing of the stream must come from a window older than the longest pass: this is
/// that margin, a constant of the program, never of the device (any `max_block` up to it gives the same output)
pub const PEAK_LAG_FRAMES: u64 = 4096;

/// The grid cells of [`PEAK_LAG_FRAMES`]
const PEAK_LAG_CELLS: u64 = PEAK_LAG_FRAMES / crate::AUTOMATION_GRID;

/// The measurement and smoothing of one peak follower, a function of the frame number alone (R-1). The measured
/// node's output is cut in windows of one grid cell ([`crate::AUTOMATION_GRID`] frames, aligned to frame 0); the
/// sum of a window grows in an accumulator that survives passes, so where the host cuts the stream does not change
/// it. The follower then moves one whole cell at a time toward the window `PEAK_LAG_FRAMES` older, and every step
/// of a cell reads the value at the cell's start
#[derive(Debug)]
pub struct PeakTracker {
    follower: PeakFollower,
    /// The cell whose start `follower.current` is, `u64::MAX` before the first step
    cell: u64,
    /// The window in progress: its sum, its frames, whether it began on its cell's first frame and the next frame
    /// it expects (a gap or a late start drops the window rather than measuring part of it)
    acc: f64,
    acc_signed: f64,
    acc_frames: u32,
    valid: bool,
    next_frame: u64,
    /// The closed windows, by cell number modulo the ring (`ring_cell` tells which cell a slot holds)
    ring_cell: Box<[u64]>,
    /// the sum of a closed window, absolute and signed: the measurement is finished with the parameters of the
    /// grid step that reads it (an automated gain, threshold or `abs` is sampled on the grid, not at the pass)
    ring_abs: Box<[f64]>,
    ring_signed: Box<[f64]>,
    first_closed: u64,
    newest: (f64, f64),
}

impl PeakTracker {
    /// `max_block` is the largest pass: the ring holds the lag, that pass and some slack
    pub fn new(max_block: usize) -> Self {
        let cells = PEAK_LAG_CELLS as usize + max_block / crate::AUTOMATION_GRID as usize + 8;
        Self {
            follower: PeakFollower::new(),
            cell: u64::MAX,
            acc: 0.0,
            acc_signed: 0.0,
            acc_frames: 0,
            valid: false,
            next_frame: 0,
            ring_cell: vec![u64::MAX; cells].into_boxed_slice(),
            ring_abs: vec![0.0; cells].into_boxed_slice(),
            ring_signed: vec![0.0; cells].into_boxed_slice(),
            first_closed: u64::MAX,
            newest: (0.0, 0.0),
        }
    }

    /// The node rendered `left.len()` frames starting at engine frame `frame`
    pub fn accumulate(&mut self, frame: u64, left: &[f32], right: &[f32]) {
        let grid = crate::AUTOMATION_GRID;
        let n = left.len().min(right.len());
        let mut i = 0;
        if frame != self.next_frame {
            self.valid = false;
            self.acc_frames = 0;
        }
        while i < n {
            let f = frame + i as u64;
            let into = f % grid;
            if into == 0 {
                self.valid = true;
                self.acc = 0.0;
                self.acc_signed = 0.0;
                self.acc_frames = 0;
            } else if self.acc_frames == 0 {
                self.valid = false;
            }
            let take = ((grid - into) as usize).min(n - i);
            if self.valid {
                self.acc += peak_sum(&left[i..i + take], &right[i..i + take], true);
                self.acc_signed += peak_sum(&left[i..i + take], &right[i..i + take], false);
                self.acc_frames += take as u32;
            }
            i += take;
            if into + take as u64 == grid {
                if self.valid && u64::from(self.acc_frames) == grid {
                    let w = f / grid;
                    let slot = (w % self.ring_cell.len() as u64) as usize;
                    self.ring_cell[slot] = w;
                    self.ring_abs[slot] = self.acc;
                    self.ring_signed[slot] = self.acc_signed;
                    self.first_closed = self.first_closed.min(w);
                    self.newest = (self.acc, self.acc_signed);
                }
                self.acc_frames = 0;
                self.valid = false;
            }
        }
        self.next_frame = frame + n as u64;
    }

    /// The window `cell` was measured from, finished with `p`, or the nearest honest answer: silence before the
    /// first window ever measured, the newest one when the pass is longer than the lag
    fn window(&self, cell: i64, p: &[f32; MOD_PARAMS]) -> f32 {
        if cell < 0 {
            return 0.0;
        }
        let cell = cell as u64;
        let slot = (cell % self.ring_cell.len() as u64) as usize;
        let (abs, signed) = if self.ring_cell[slot] == cell {
            (self.ring_abs[slot], self.ring_signed[slot])
        } else if self.first_closed == u64::MAX || cell < self.first_closed {
            return 0.0;
        } else {
            self.newest
        };
        let sum = if p[P_ABS] >= 0.5 { abs } else { signed };
        peak_finish(sum, crate::AUTOMATION_GRID as usize, p)
    }

    /// The follower's value for a step that starts in `cell`: it first moves, a whole cell at a time, to the start
    /// of that cell
    pub fn value_at(&mut self, cell: u64, p: &[f32; MOD_PARAMS], sample_rate: u32) -> f32 {
        if self.cell == u64::MAX {
            self.cell = cell;
        }
        // A gap this long (never in a running engine) is settled already
        if cell.saturating_sub(self.cell) > 4096 {
            self.cell = cell - 4096;
        }
        while self.cell < cell {
            self.follower.target = self.window(self.cell as i64 - PEAK_LAG_CELLS as i64, p);
            self.follower.advance(p, sample_rate, crate::AUTOMATION_GRID as u32);
            self.cell += 1;
        }
        self.follower.current
    }
}

/// The peak follower's smoothing (`PeakController::updateValueBuffer`): per frame, the value goes to the measured
/// target by the attack coefficient when rising, the decay coefficient when falling
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeakFollower {
    pub target: f32,
    pub current: f32,
}

impl PeakFollower {
    pub const fn new() -> Self {
        Self { target: 0.0, current: 0.0 }
    }

    /// `1 - 2^(-0.3 (1 - k) 44100 / rate)`, the legacy coefficient of an attack or decay `k` (0 to 0.999)
    pub fn coefficient(k: f32, sample_rate: u32) -> f32 {
        let ratio = 44100.0 / sample_rate.max(1) as f32;
        1.0 - libm::powf(2.0, -0.3 * (1.0 - k.clamp(0.0, 0.999)) * ratio)
    }

    /// `frames` frames of the legacy per-frame loop, in closed form: the distance to the target shrinks by `1 - c`
    /// every frame
    pub fn advance(&mut self, p: &[f32; MOD_PARAMS], sample_rate: u32, frames: u32) {
        if self.current == self.target {
            return;
        }
        let k = if self.current < self.target { p[P_PEAK_ATTACK] } else { p[P_PEAK_DECAY] };
        let c = Self::coefficient(k, sample_rate).clamp(0.0, 1.0);
        let left = libm::powf(1.0 - c, frames as f32);
        self.current = self.target - (self.target - self.current) * left;
    }
}

impl Default for PeakFollower {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn lfo_is_a_function_of_the_sample() {
        let p = MOD_DEFAULTS;
        let a = lfo_phase(&p, 48000, 24000, 0, 960);
        assert!((a - 0.25).abs() < 1e-9, "speed 2 s: a quarter cycle at 0.5 s");
        assert!((lfo_value(&p, a, 1, None) - 1.0).abs() < 1e-6);
        let mut q = p;
        q[P_SYNC] = 4.0;
        // a quarter note: one cycle per 960 ticks
        assert!((lfo_phase(&q, 48000, 0, 480, 960) - 0.5).abs() < 1e-9);
        q[P_PHASE] = 90.0;
        assert!((lfo_phase(&q, 48000, 0, 0, 960) - 0.25).abs() < 1e-9);
        q[P_SYNC] = 8.0;
        q[P_SYNC_NUM] = 3.0;
        q[P_SYNC_DEN] = 8.0;
        assert!((lfo_phase(&q, 48000, 0, 1440, 960) - 1.0 - 0.25).abs() < 1e-9);
    }

    #[test]
    fn lfo_shapes_match_the_legacy() {
        assert_eq!(lfo_shape(1, 0.25, 0, None), 1.0);
        assert_eq!(lfo_shape(1, 0.75, 0, None), -1.0);
        assert_eq!(lfo_shape(2, 0.5, 0, None), 0.0);
        assert_eq!(lfo_shape(3, 0.25, 0, None), 1.0);
        assert_eq!(lfo_shape(3, 0.75, 0, None), -1.0);
        assert_eq!(lfo_shape(4, 0.25, 0, None), 0.0);
        assert_eq!(lfo_shape(5, 0.0, 0, None), -1.0);
        assert_eq!(lfo_shape(6, 3.2, 9, None), lfo_shape(6, 3.9, 9, None), "noise holds for a cycle");
        assert_ne!(lfo_shape(6, 3.2, 9, None), lfo_shape(6, 4.2, 9, None));
        let t = [0.0, 1.0];
        assert_eq!(lfo_shape(7, 0.25, 0, Some(&t)), 0.5);
        assert_eq!(lfo_shape(7, 0.25, 0, None), 0.0);
    }

    #[test]
    fn random_is_deterministic_and_in_range() {
        let mut p = MOD_DEFAULTS;
        for s in (0..200_000).step_by(997) {
            let v = random_value(&p, 48000, s, 5);
            assert!((-1.0..=1.0).contains(&v));
            assert_eq!(v, random_value(&p, 48000, s, 5));
        }
        p[P_SMOOTH] = 1.0;
        let a = random_value(&p, 48000, 100, 5);
        let b = random_value(&p, 48000, 101, 5);
        assert!((a - b).abs() < 0.01, "smoothed values are continuous");
    }

    #[test]
    fn envelope_runs_its_stages() {
        let p = MOD_DEFAULTS;
        let mut e = Envelope::new();
        e.gate(true);
        for _ in 0..10 {
            e.advance(&p, 48000, 32);
        }
        assert!(e.level > 0.0 && e.level <= 1.0);
        for _ in 0..2000 {
            e.advance(&p, 48000, 32);
        }
        assert_eq!(e.stage, Stage::Sustain);
        assert_eq!(e.level, 0.5);
        e.gate(false);
        for _ in 0..5000 {
            e.advance(&p, 48000, 32);
        }
        assert_eq!(e.stage, Stage::Idle);
        assert_eq!(e.level, 0.0);
    }

    #[test]
    fn peak_follows_the_legacy_math() {
        let mut p = MOD_DEFAULTS;
        let l = [0.5f32; 8];
        let r = [0.5f32; 8];
        // abs: sqrt((0.25 + 0.25)) = 0.7071
        assert!((peak_measure(&l, &r, &p) - 0.707_106_8).abs() < 1e-5);
        p[P_THRESHOLD] = 0.8;
        assert_eq!(peak_measure(&l, &r, &p), 0.0);
        p[P_THRESHOLD] = 0.0;
        p[P_GAIN] = 4.0;
        assert_eq!(peak_measure(&l, &r, &p), 1.0);
        // attack 0: c = 1 - 2^-0.3
        let c = PeakFollower::coefficient(0.0, 44100);
        assert!((c - (1.0 - libm::powf(2.0, -0.3))).abs() < 1e-6);
        let mut f = PeakFollower { target: 1.0, current: 0.0 };
        f.advance(&p, 44100, 1);
        assert!((f.current - c).abs() < 1e-6);
        let mut g = PeakFollower { target: 1.0, current: 0.0 };
        for _ in 0..32 {
            g.advance(&p, 44100, 1);
        }
        let mut h = PeakFollower { target: 1.0, current: 0.0 };
        h.advance(&p, 44100, 32);
        assert!((g.current - h.current).abs() < 1e-4);
    }

    #[test]
    fn the_peak_tracker_finishes_with_the_parameters_of_the_reading_step() {
        let grid = crate::AUTOMATION_GRID as usize;
        let mut t = PeakTracker::new(256);
        let l = vec![0.5f32; grid];
        // a stretch long enough for the lag, cell by cell
        for c in 0..(PEAK_LAG_CELLS as usize + 4) {
            t.accumulate((c * grid) as u64, &l, &l);
        }
        let mut p = MOD_DEFAULTS;
        p[P_PEAK_ATTACK] = 0.0;
        let quiet = {
            let mut q = p;
            q[P_GAIN] = 0.0;
            q
        };
        // the same closed window read with two gains: the measurement follows the reading step's parameters
        let cell = PEAK_LAG_CELLS + 2;
        assert_eq!(t.window(cell as i64 - PEAK_LAG_CELLS as i64, &quiet), 0.0);
        assert!(t.window(cell as i64 - PEAK_LAG_CELLS as i64, &p) > 0.0);
    }
}
