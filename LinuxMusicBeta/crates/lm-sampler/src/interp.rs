//! Interpolation of a sample read at a fractional position. The engine reads with a windowed sinc (the default),
//! a four-point Hermite cubic or a straight line; the sinc's table is built once, off the audio thread.

use std::f64::consts::PI;
use std::sync::Arc;

use crate::pcm::Pcm;

/// How a sample is read between its frames
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Quality {
    Linear,
    Cubic,
    /// A Kaiser-windowed sinc of [`SINC_TAPS`] taps: the high-quality default
    #[default]
    Sinc,
}

impl Quality {
    pub fn key(self) -> &'static str {
        match self {
            Quality::Linear => "linear",
            Quality::Cubic => "cubic",
            Quality::Sinc => "sinc",
        }
    }

    pub fn from_key(key: &str) -> Option<Quality> {
        match key {
            "linear" => Some(Quality::Linear),
            "cubic" => Some(Quality::Cubic),
            "sinc" => Some(Quality::Sinc),
            _ => None,
        }
    }
}

/// Taps of the sinc (8 each side of the position's frame, 16 in all): stop-band about -90 dB
pub const SINC_TAPS: usize = 16;
/// Sub-sample positions the table holds; a position takes the two nearest and blends them
const PHASES: usize = 512;

/// The sinc's coefficients for `PHASES + 1` fractional positions
#[derive(Debug)]
pub struct SincTable {
    coefficients: Box<[f32]>,
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..40 {
        term *= q / (f64::from(k) * f64::from(k));
        sum += term;
        if term < 1.0e-14 * sum {
            break;
        }
    }
    sum
}

impl SincTable {
    pub fn new() -> Arc<Self> {
        const BETA: f64 = 9.0;
        let half = (SINC_TAPS / 2) as i64;
        let norm = bessel_i0(BETA);
        let mut c = vec![0.0f32; (PHASES + 1) * SINC_TAPS];
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            let mut taps = [0.0f64; SINC_TAPS];
            let mut sum = 0.0;
            for (t, slot) in taps.iter_mut().enumerate() {
                // The tap reads frame `i - half + 1 + t`, at distance `x` from the position `i + frac`
                let x = (t as i64 - half + 1) as f64 - frac;
                let w = {
                    let r = x / half as f64;
                    if r.abs() >= 1.0 { 0.0 } else { bessel_i0(BETA * libm::sqrt(1.0 - r * r)) / norm }
                };
                let s = if x.abs() < 1.0e-12 { 1.0 } else { libm::sin(PI * x) / (PI * x) };
                *slot = s * w;
                sum += *slot;
            }
            // (unity gain at DC at every phase)
            for (t, v) in taps.iter().enumerate() {
                c[p * SINC_TAPS + t] = (v / sum) as f32;
            }
        }
        Arc::new(Self { coefficients: c.into_boxed_slice() })
    }

    #[inline]
    fn phase(&self, p: usize) -> &[f32] {
        &self.coefficients[p * SINC_TAPS..(p + 1) * SINC_TAPS]
    }
}

/// How the reader finds frame `i` of the sample: inside a loop the frames past its end are the ones after its start
#[derive(Clone, Copy, Debug)]
pub struct Wrap {
    pub loop_start: i64,
    pub loop_end: i64,
    pub active: bool,
}

impl Wrap {
    pub const NONE: Wrap = Wrap { loop_start: 0, loop_end: 0, active: false };

    #[inline]
    fn index(&self, i: i64) -> i64 {
        if self.active {
            let len = self.loop_end - self.loop_start;
            if len > 0 && i >= self.loop_end {
                return self.loop_start + (i - self.loop_start) % len;
            }
        }
        i
    }
}

/// The largest position, in frames, that [`read`] indexes with
const POSITION_LIMIT: f64 = 1.0e15;

/// The frame at `position` (frames, fractional) of `pcm`
#[inline]
pub fn read(pcm: &Pcm, position: f64, quality: Quality, wrap: Wrap, table: &SincTable) -> [f32; 2] {
    read_with(|i| pcm.at(i), position, quality, wrap, table)
}

/// The frame at `position` of a sample whose frame `i` is `frame(i)` (a streamed sample's head and blocks)
#[inline]
pub fn read_with(
    mut frame: impl FnMut(i64) -> [f32; 2],
    position: f64,
    quality: Quality,
    wrap: Wrap,
    table: &SincTable,
) -> [f32; 2] {
    // A position that is not finite or beyond any sample would overflow the integer indices below
    let position = if position.is_finite() { position.clamp(-POSITION_LIMIT, POSITION_LIMIT) } else { 0.0 };
    let i = position.floor();
    let frac = (position - i) as f32;
    let i = i as i64;
    match quality {
        Quality::Linear => {
            let a = frame(wrap.index(i));
            let b = frame(wrap.index(i + 1));
            [a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]
        }
        Quality::Cubic => {
            let p = [
                frame(wrap.index(i - 1)),
                frame(wrap.index(i)),
                frame(wrap.index(i + 1)),
                frame(wrap.index(i + 2)),
            ];
            let mut out = [0.0; 2];
            for (ch, o) in out.iter_mut().enumerate() {
                let (y0, y1, y2, y3) = (p[0][ch], p[1][ch], p[2][ch], p[3][ch]);
                let c0 = y1;
                let c1 = 0.5 * (y2 - y0);
                let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
                let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
                *o = ((c3 * frac + c2) * frac + c1) * frac + c0;
            }
            out
        }
        Quality::Sinc => {
            let scaled = f64::from(frac) * PHASES as f64;
            let p0 = (scaled as usize).min(PHASES - 1);
            let mix = (scaled - p0 as f64) as f32;
            let (c0, c1) = (table.phase(p0), table.phase(p0 + 1));
            let first = i - (SINC_TAPS as i64 / 2) + 1;
            let mut acc = [0.0f32; 2];
            for t in 0..SINC_TAPS {
                let k = c0[t] + (c1[t] - c0[t]) * mix;
                let f = frame(wrap.index(first + t as i64));
                acc[0] += k * f[0];
                acc[1] += k * f[1];
            }
            acc
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp() -> Pcm {
        Pcm::mono(1000, (0..64).map(|i| i as f32).collect())
    }

    #[test]
    fn absurd_positions_do_not_overflow() {
        let table = SincTable::new();
        let p = ramp();
        for q in [Quality::Linear, Quality::Cubic, Quality::Sinc] {
            for pos in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN, 1.0e300, -1.0e300, 9.3e18] {
                let _ = read(&p, pos, q, Wrap::NONE, &table);
            }
        }
    }

    #[test]
    fn every_quality_hits_the_frames_exactly() {
        let table = SincTable::new();
        let p = ramp();
        for q in [Quality::Linear, Quality::Cubic, Quality::Sinc] {
            let v = read(&p, 30.0, q, Wrap::NONE, &table);
            assert!((v[0] - 30.0).abs() < 1.0e-3, "{q:?}: {}", v[0]);
        }
    }

    #[test]
    fn between_frames_a_ramp_is_interpolated() {
        let table = SincTable::new();
        let p = ramp();
        for q in [Quality::Linear, Quality::Cubic, Quality::Sinc] {
            let v = read(&p, 30.5, q, Wrap::NONE, &table);
            assert!((v[0] - 30.5).abs() < 0.05, "{q:?}: {}", v[0]);
        }
    }

    #[test]
    fn a_loop_wraps_the_taps() {
        let table = SincTable::new();
        let p = Pcm::mono(1000, vec![0.0, 1.0, 1.0, 1.0, 5.0]);
        let w = Wrap { loop_start: 1, loop_end: 4, active: true };
        // Past the end the frames are the loop's: a flat 1.0 there
        let v = read(&p, 5.0, Quality::Linear, w, &table);
        assert!((v[0] - 1.0).abs() < 1.0e-6);
    }
}
