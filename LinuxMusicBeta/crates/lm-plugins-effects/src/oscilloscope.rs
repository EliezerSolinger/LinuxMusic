//! Oscilloscope (F-FX-022: legacy `plugins/Oscilloscope`, Keratin's): the incoming audio is passed through
//! unchanged and its waveform is published for the window.
//!
//! State, version 1: the legacy knobs, same ranges and defaults: `amp` (0–1000 %, 100: the vertical scale, applied
//! by the window), `length` (1–5000 ms, 500, logarithmic: the window of time shown), `phase` (0–1, 1: where in the
//! history the window ends, 1 is the newest sound; it wraps, as the legacy), and the switches `stereo` (the two
//! sides apart, else their average) and `pause` (the history stops moving).
//!
//! The legacy keeps the last 5 seconds in a ring and the window scans it at every paint. Here the audio side keeps
//! the ring (allocated when built, for the project's rate) and, besides it, a summary of the minimum and maximum of
//! each [`BLOCK`] frames, so that a long window costs a few thousand reads and not a few hundred thousand. When the
//! window is open the audio side reduces the span asked for to [`COLS`] columns of (minimum, maximum), at most
//! [`REFRESH_HZ`] times a second, and publishes them.
//!
//! The window's live frame, little-endian `f32`: [`HEADER`] numbers (the columns used, 1 when a column spans more
//! than one frame (the legacy's high quality drawing) else 0, the span's frames, the frames of a column), then
//! [`COLS`] columns of four: the minimum and maximum of the left side and of the right (stereo), or of the average
//! and zeros (mono). Where a column spans one frame, its minimum is that frame and its maximum the next one (the
//! legacy draws a line from one to the other).

use lm_base::Result;
use lm_plugin::{BuildContext, Descriptor, DisplaySpec, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports, put_f32s};

pub const ID: &str = "oscilloscope";

/// The history, seconds (`OscilloscopeGraph::MaxBufferLengthSeconds`)
pub const MAX_SECONDS: f32 = 5.0;
/// The columns the window is reduced to
pub const COLS: usize = 512;
/// The numbers before the columns
pub const HEADER: usize = 4;
/// The numbers of a frame
pub const FRAME_FLOATS: usize = HEADER + COLS * 4;
/// Frames summarized together
const BLOCK: usize = 32;
/// How many times a second the window's columns are worked out again
const REFRESH_HZ: f64 = 60.0;

const KEYS: [&str; 5] = ["amp", "length", "phase", "stereo", "pause"];
const AMP: usize = 0;
const LENGTH: usize = 1;
const PHASE: usize = 2;
const STEREO: usize = 3;
const PAUSE: usize = 4;

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.oscilloscope.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: Some(DisplaySpec { bytes: FRAME_FLOATS * 4 }),
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    vec![
        NodeSchema::new("plugin:oscilloscope", 1)
            .param(ParamSpec::new("amp", 0.0, 1000.0, 100.0).unit("%"))
            .param(ParamSpec::new("length", 1.0, f64::from(MAX_SECONDS) * 1000.0, 500.0).scale(Scale::Log).unit("ms"))
            .param(ParamSpec::new("phase", 0.0, 1.0, 1.0))
            .param(ParamSpec::new("stereo", 0.0, 1.0, 0.0).scale(Scale::Stepped))
            .param(ParamSpec::new("pause", 0.0, 1.0, 0.0).scale(Scale::Stepped)),
    ]
}

/// One side of the scope: the ring of the last seconds of the left, right and average, and its summary
struct Scope {
    knobs: Knobs<5>,
    rate: f32,
    /// The pair that shows (the first); the others only pass the audio
    active: bool,
    /// Left, right and average of each frame
    ring: Vec<[f32; 3]>,
    /// The minimum and maximum of each channel over each block of the ring
    sums: Vec<[f32; 6]>,
    /// The block being filled
    partial: [f32; 6],
    partial_len: usize,
    /// Where the next frame goes
    write: usize,
    since_refresh: usize,
    /// The frame as last worked out
    cache: Vec<f32>,
    fresh: bool,
}

const EMPTY_PARTIAL: [f32; 6] =
    [f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY];

impl Scope {
    fn new(knobs: Knobs<5>, rate: f32, active: bool) -> Self {
        let n = if active { (((rate * MAX_SECONDS) as usize).div_ceil(BLOCK) * BLOCK).max(BLOCK * 4) } else { BLOCK };
        Self {
            knobs,
            rate,
            active,
            ring: vec![[0.0; 3]; n],
            sums: vec![[0.0; 6]; n / BLOCK],
            partial: EMPTY_PARTIAL,
            partial_len: 0,
            write: 0,
            since_refresh: 0,
            cache: vec![0.0; if active { FRAME_FLOATS } else { 0 }],
            fresh: false,
        }
    }

    fn push(&mut self, frames: &[[f32; 2]]) {
        let n = self.ring.len();
        for f in frames {
            let v = [f[0], f[1], 0.5 * (f[0] + f[1])];
            self.ring[self.write] = v;
            for (c, x) in v.iter().enumerate() {
                self.partial[2 * c] = self.partial[2 * c].min(*x);
                self.partial[2 * c + 1] = self.partial[2 * c + 1].max(*x);
            }
            self.write += 1;
            if self.write == n {
                self.write = 0;
            }
            self.partial_len += 1;
            if self.partial_len == BLOCK {
                let block = ((self.write + n - BLOCK) % n) / BLOCK;
                self.sums[block] = self.partial;
                self.partial = EMPTY_PARTIAL;
                self.partial_len = 0;
            }
        }
        self.since_refresh += frames.len();
    }

    /// The minimum and maximum of channel `k` over the ring's frames `from..=from + span` (wrapping)
    fn range(&self, k: usize, from: usize, span: usize) -> (f32, f32) {
        let n = self.ring.len();
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        if span >= 2 * BLOCK {
            let blocks = n / BLOCK;
            let filling = self.write / BLOCK;
            for b in from / BLOCK..=(from + span) / BLOCK {
                let b = b % blocks;
                let s = if b == filling && self.partial_len > 0 { &self.partial } else { &self.sums[b] };
                lo = lo.min(s[2 * k]);
                hi = hi.max(s[2 * k + 1]);
            }
        } else {
            for i in from..=from + span {
                let x = self.ring[i % n][k];
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
        (lo, hi)
    }

    /// The legacy's `paintEvent` scan of the history, reduced to the columns
    fn compute(&mut self) {
        let n = self.ring.len();
        let length_ms = f64::from(self.knobs.get(LENGTH));
        let mut win = ((f64::from(self.rate) * length_ms / 1000.0) as usize).clamp(2, n);
        let offset = ((f64::from(self.knobs.get(PHASE)) * n as f64) as usize) % n;
        let mut start = (self.write + n - win + offset) % n;
        let per_col = (win / COLS).max(1);
        // Quantized so that the picture does not flicker; rounded up so that it never goes below the newest frame
        let adjust = per_col - start % per_col;
        start += adjust;
        win = win.saturating_sub(adjust);
        let hq = per_col > 1;
        let stereo = self.knobs.on(STEREO);
        let mut cols = 0;
        let mut offset = 0;
        while offset + per_col < win && cols < COLS {
            let from = start + offset;
            let out = HEADER + cols * 4;
            if hq {
                if stereo {
                    let (a, b) = self.range(0, from, per_col);
                    let (c, d) = self.range(1, from, per_col);
                    self.cache[out..out + 4].copy_from_slice(&[a, b, c, d]);
                } else {
                    let (a, b) = self.range(2, from, per_col);
                    self.cache[out..out + 4].copy_from_slice(&[a, b, 0.0, 0.0]);
                }
            } else {
                let now = self.ring[from % n];
                let next = self.ring[(from + per_col) % n];
                if stereo {
                    self.cache[out..out + 4].copy_from_slice(&[now[0], next[0], now[1], next[1]]);
                } else {
                    self.cache[out..out + 4].copy_from_slice(&[now[2], next[2], 0.0, 0.0]);
                }
            }
            cols += 1;
            offset += per_col;
        }
        self.cache[HEADER + cols * 4..].fill(0.0);
        self.cache[..HEADER].copy_from_slice(&[cols as f32, if hq { 1.0 } else { 0.0 }, win as f32, per_col as f32]);
        self.fresh = true;
        self.since_refresh = 0;
    }
}

impl Effect for Scope {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
        // The window's span, offset or channels changed: the next frame is worked out at once
        if index != AMP && index != PAUSE {
            self.fresh = false;
        }
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        if self.active && !self.knobs.on(PAUSE) {
            self.push(frames);
        }
    }

    fn reset(&mut self) {
        self.ring.fill([0.0; 3]);
        self.sums.fill([0.0; 6]);
        self.partial = EMPTY_PARTIAL;
        self.partial_len = 0;
        self.fresh = false;
    }

    fn adopt(&mut self, old: &mut Self) -> bool {
        if self.ring.len() != old.ring.len() {
            return false;
        }
        std::mem::swap(&mut self.ring, &mut old.ring);
        std::mem::swap(&mut self.sums, &mut old.sums);
        std::mem::swap(&mut self.partial, &mut old.partial);
        std::mem::swap(&mut self.partial_len, &mut old.partial_len);
        self.write = old.write;
        self.fresh = false;
        true
    }

    fn display(&mut self, frame: &mut [u8]) {
        if !self.fresh || self.since_refresh as f64 >= f64::from(self.rate) / REFRESH_HZ {
            self.compute();
        }
        put_f32s(frame, &self.cache);
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let knobs = Knobs::from_state(state, &KEYS)?;
    let rate = cx.spec.sample_rate as f32;
    let mut first = true;
    EffectProcessor::build(cx, || {
        let active = std::mem::take(&mut first);
        Ok(Scope::new(knobs, rate, active))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn knobs(length: f32, stereo: bool) -> Knobs<5> {
        let mut k = Knobs::<5>::from_values([100.0, 500.0, 1.0, 0.0, 0.0]);
        k.set(LENGTH, length);
        k.set(STEREO, if stereo { 1.0 } else { 0.0 });
        k
    }

    fn column(scope: &Scope, i: usize) -> [f32; 4] {
        let at = HEADER + i * 4;
        [scope.cache[at], scope.cache[at + 1], scope.cache[at + 2], scope.cache[at + 3]]
    }

    #[test]
    fn the_newest_second_of_a_ramp_shows_its_extremes() {
        let mut scope = Scope::new(knobs(1000.0, true), 48_000.0, true);
        let frames: Vec<[f32; 2]> = (0..48_000).map(|i| [i as f32 / 48_000.0, -(i as f32) / 48_000.0]).collect();
        scope.push(&frames);
        scope.compute();
        assert!(scope.cache[0] > 400.0, "the columns used");
        let first = column(&scope, 0);
        let last = column(&scope, scope.cache[0] as usize - 1);
        assert!(first[0] < 0.01 && last[1] > 0.98, "left runs from 0 to 1: {first:?} {last:?}");
        assert!(first[3] > -0.01 && last[2] < -0.98, "right runs from 0 to -1: {first:?} {last:?}");
    }

    #[test]
    fn mono_shows_the_average_and_leaves_the_second_side_empty() {
        let mut scope = Scope::new(knobs(100.0, false), 48_000.0, true);
        scope.push(&vec![[1.0, 0.0]; 12_000]);
        scope.compute();
        let c = column(&scope, 3);
        assert!(c[0] > 0.49 && c[1] < 0.51 && c[2] == 0.0 && c[3] == 0.0, "{c:?}");
    }

    #[test]
    fn a_short_window_draws_lines_between_frames() {
        let mut scope = Scope::new(knobs(1.0, true), 48_000.0, true);
        scope.push(&(0..4800).map(|i| [(i % 2) as f32, 0.0]).collect::<Vec<_>>());
        scope.compute();
        assert_eq!(scope.cache[1], 0.0, "not high quality");
        let c = column(&scope, 0);
        assert!((c[0] - c[1]).abs() == 1.0, "one frame to the next: {c:?}");
    }

    #[test]
    fn pause_stops_the_history_and_only_active_pairs_record() {
        let mut k = knobs(100.0, true);
        k.set(PAUSE, 1.0);
        let mut scope = Scope::new(k, 48_000.0, true);
        scope.process(&mut vec![[1.0, 1.0]; 480]);
        assert_eq!(scope.write, 0);
        let mut quiet = Scope::new(knobs(100.0, true), 48_000.0, false);
        quiet.process(&mut vec![[1.0, 1.0]; 480]);
        assert_eq!(quiet.write, 0);
    }

    #[test]
    fn the_long_window_uses_the_summary() {
        let mut scope = Scope::new(knobs(5000.0, true), 48_000.0, true);
        let mut frames = vec![[0.0f32; 2]; 240_000];
        frames[100_000] = [0.9, -0.9];
        scope.push(&frames);
        scope.compute();
        let peak = (0..scope.cache[0] as usize).map(|i| column(&scope, i)[1]).fold(0.0f32, f32::max);
        assert!(peak > 0.89, "the peak survives the reduction: {peak}");
    }
}
