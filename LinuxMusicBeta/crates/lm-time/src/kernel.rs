//! The exact arithmetic inside one segment of the tempo map.
//!
//! Every segment is anchored at an exact pair (anchor tick, anchor sample). Inside it, the *real* sample offset
//! of a tick offset `dt` is `y(dt)`, an increasing function; the map's two conversions are:
//!
//! - `sample_offset(dt) = round_half_even(y(dt))`;
//! - `tick_offset(r) = max { dt : sample_offset(dt) <= r }`, i.e. the last tick that has started at sample `r`.
//!
//! With `F = y⁻¹` (ticks as a function of the real sample offset), `round_half_even(y) <= r` holds exactly when
//! `y < r + ½`, or `y == r + ½` and `r` is even. So `tick_offset(r)` is `floor(F(r + ½))` when `r` is even and
//! `ceil(F(r + ½)) - 1` when it is odd. `F(r + ½)` is a rational number computed exactly, so no float is ever
//! involved.
//!
//! - **Constant tempo:** `y(dt) = dt · a / b` with `a = 60 · sr · 10⁶`, `b = µbpm · ppq` (reduced); all in `i128`.
//! - **Linear ramp** (the tempo changes linearly *in time* from `bpm0` to `bpm1` over the segment's `L` ticks):
//!   the segment lasts `Y = 120 · sr · L · 10⁶ / (ppq · (m0 + m1))` samples (`m` = µbpm), a rational, and
//!   `F(y) = (c1 · y + c2 · y²) / den` with `c1 = 240 · sr · L · 10⁶ · ppq · m0`, `c2 = ppq² · (m1² − m0²)`,
//!   `den = 14400 · sr² · L · 10¹²`. With the crate's bounds every intermediate value fits in 230 bits, so the
//!   ramp is computed exactly in 256-bit integers. The inverse (`y` from a tick) is found by an exact search on
//!   the integers (`F` is increasing on `[0, Y]`), started from a float guess that only speeds it up.

use crate::wide::I256;

use crate::Bpm;

const SCALE: i128 = Bpm::SCALE as i128;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Kernel {
    Constant(Constant),
    Ramp(Ramp),
}

impl Kernel {
    /// The sample offset where the tick at offset `dt` starts
    pub(crate) fn sample_offset(&self, dt: i128) -> i128 {
        match self {
            Kernel::Constant(k) => k.sample_offset(dt),
            Kernel::Ramp(k) => k.sample_offset(dt),
        }
    }

    /// The last tick offset that has started at sample offset `r`
    pub(crate) fn tick_offset(&self, r: i128) -> i128 {
        match self {
            Kernel::Constant(k) => k.tick_offset(r),
            Kernel::Ramp(k) => k.tick_offset(r),
        }
    }
}

/// Constant tempo: `a / b` samples per tick, reduced, both positive
#[derive(Clone, Copy, Debug)]
pub(crate) struct Constant {
    a: i128,
    b: i128,
}

impl Constant {
    pub(crate) fn new(sample_rate: u32, ppq: u32, bpm: Bpm) -> Self {
        let a = 60 * i128::from(sample_rate) * SCALE;
        let b = i128::from(bpm.micro()) * i128::from(ppq);
        let g = gcd_i128(a, b);
        Self { a: a / g, b: b / g }
    }

    pub(crate) fn sample_offset(&self, dt: i128) -> i128 {
        round_half_even_div(dt * self.a, self.b)
    }

    pub(crate) fn tick_offset(&self, r: i128) -> i128 {
        // F(r + ½) = (2r + 1) · b / (2a)
        let num = (2 * r + 1) * self.b;
        let den = 2 * self.a;
        if r.rem_euclid(2) == 0 { num.div_euclid(den) } else { ceil_div(num, den) - 1 }
    }

    /// Whether ticks are no finer than samples here (`a / b >= 1`), so every tick starts on its own sample
    pub(crate) fn ticks_are_coarser(&self) -> bool {
        self.a >= self.b
    }
}

/// A linear tempo ramp over `length` ticks (see the module docs for the formulas)
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ramp {
    c1: I256,
    c2: I256,
    den: I256,
    /// `Y = y_num / y_den`, the exact duration in samples
    y_num: i128,
    y_den: i128,
    /// `floor(Y)`
    floor_y: i128,
    from: Bpm,
    to: Bpm,
}

impl Ramp {
    /// `length` must be positive and `from != to`
    pub(crate) fn new(sample_rate: u32, ppq: u32, from: Bpm, to: Bpm, length: i64) -> Self {
        debug_assert!(length > 0 && from != to);
        let sr = I256::from(i128::from(sample_rate));
        let ppq_w = I256::from(i128::from(ppq));
        let l = I256::from(i128::from(length));
        let q = I256::from(SCALE);
        let m0 = I256::from(i128::from(from.micro()));
        let m1 = I256::from(i128::from(to.micro()));
        let c1 = I256::from(240_i128) * sr * l * q * ppq_w * m0;
        let c2 = ppq_w * ppq_w * (m1 * m1 - m0 * m0);
        let den = I256::from(14_400_i128) * sr * sr * l * q * q;
        let g = gcd_i256(gcd_i256(c1, c2), den);
        let y_num = 120 * i128::from(sample_rate) * i128::from(length) * SCALE;
        let y_den = i128::from(ppq) * (i128::from(from.micro()) + i128::from(to.micro()));
        let gy = gcd_i128(y_num, y_den);
        let (y_num, y_den) = (y_num / gy, y_den / gy);
        Self { c1: c1 / g, c2: c2 / g, den: den / g, y_num, y_den, floor_y: y_num / y_den, from, to }
    }

    pub(crate) fn from(&self) -> Bpm {
        self.from
    }

    pub(crate) fn to(&self) -> Bpm {
        self.to
    }

    /// `4 · den · F(h / 2)`: the ticks at the half-sample offset `h / 2`, scaled to an integer
    fn scaled_ticks(&self, h: i128) -> I256 {
        let h = I256::from(h);
        I256::from(2_i128) * self.c1 * h + self.c2 * h * h
    }

    /// The exact duration in samples as an `f64`, for display
    pub(crate) fn duration_f64(&self) -> f64 {
        self.y_num as f64 / self.y_den as f64
    }

    /// `dt` in `0..=length`
    pub(crate) fn sample_offset(&self, dt: i128) -> i128 {
        debug_assert!(dt >= 0);
        let target = I256::from(4_i128) * self.den * I256::from(dt);
        let n = self.floor_root(target);
        // The real offset y* is in [n, n + 1). Round half to even by looking at n + ½, unless that is past Y
        if (2 * n + 1) * self.y_den > 2 * self.y_num {
            return n;
        }
        match self.scaled_ticks(2 * n + 1).cmp(&target) {
            std::cmp::Ordering::Less => n + 1,
            std::cmp::Ordering::Greater => n,
            std::cmp::Ordering::Equal => n + (n & 1),
        }
    }

    /// `r` in `0..` with `r + ½ <= Y` (the sample is inside the segment)
    pub(crate) fn tick_offset(&self, r: i128) -> i128 {
        debug_assert!(r >= 0 && (2 * r + 1) * self.y_den <= 2 * self.y_num);
        let num = self.scaled_ticks(2 * r + 1);
        let den = I256::from(4_i128) * self.den;
        let floor = num / den; // both non-negative
        let exact = floor * den == num;
        let v = if r & 1 == 0 || !exact { floor } else { floor - I256::ONE };
        v.as_i128()
    }

    /// The largest integer `n` in `[0, floor(Y)]` with `F(n) <= target / (4 · den)`
    fn floor_root(&self, target: I256) -> i128 {
        let fits = |n: i128| self.scaled_ticks(2 * n) <= target;
        let hi_limit = self.floor_y;
        // Float guess of the root of c2·h² + 2·c1·h − target = 0 (h = 2y), in the cancellation-free form
        let (c1, c2, t) = (self.c1.as_f64(), self.c2.as_f64(), target.as_f64());
        let h = t / (c1 + (c1 * c1 + c2 * t).max(0.0).sqrt());
        let guess = if h.is_finite() { (h / 2.0).floor().clamp(0.0, hi_limit as f64) as i128 } else { 0 };
        let guess = guess.clamp(0, hi_limit);
        // Invariant once bracketed: fits(lo), and hi is past the answer (!fits(hi) or hi == hi_limit + 1)
        let (mut lo, mut hi);
        if fits(guess) {
            lo = guess;
            let mut step = 1_i128;
            loop {
                let c = lo + step;
                if c > hi_limit {
                    hi = hi_limit + 1;
                    break;
                }
                if fits(c) {
                    lo = c;
                    step *= 2;
                } else {
                    hi = c;
                    break;
                }
            }
        } else {
            hi = guess;
            let mut step = 1_i128;
            loop {
                let c = hi - step;
                if c <= 0 {
                    lo = 0; // F(0) = 0 <= target
                    break;
                }
                if fits(c) {
                    lo = c;
                    break;
                }
                hi = c;
                step *= 2;
            }
        }
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if fits(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// Whether ticks are no finer than samples anywhere in the ramp (the faster end has at least one sample per tick)
    pub(crate) fn ticks_are_coarser(&self, sample_rate: u32, ppq: u32) -> bool {
        let fastest = self.from.max(self.to);
        Constant::new(sample_rate, ppq, fastest).ticks_are_coarser()
    }

    /// Whether ticks are no coarser than samples anywhere in the ramp (the slower end has at most one sample per tick)
    pub(crate) fn ticks_are_finer(&self, sample_rate: u32, ppq: u32) -> bool {
        let slowest = self.from.min(self.to);
        let k = Constant::new(sample_rate, ppq, slowest);
        k.a <= k.b
    }

    /// The tempo at the real sample offset `y`, for display
    pub(crate) fn bpm_at(&self, y: f64) -> f64 {
        let (b0, b1) = (self.from.to_f64(), self.to.to_f64());
        let u = (y / self.duration_f64()).clamp(0.0, 1.0);
        b0 + (b1 - b0) * u
    }
}

impl Constant {
    /// Whether ticks are no coarser than samples (`a / b <= 1`), so every sample starts some tick
    pub(crate) fn ticks_are_finer(&self) -> bool {
        self.a <= self.b
    }
}

/// `round_half_even(n / d)` for `d > 0`
pub(crate) fn round_half_even_div(n: i128, d: i128) -> i128 {
    debug_assert!(d > 0);
    let q = n.div_euclid(d);
    let r = n.rem_euclid(d);
    match (2 * r).cmp(&d) {
        std::cmp::Ordering::Less => q,
        std::cmp::Ordering::Greater => q + 1,
        std::cmp::Ordering::Equal => q + (q & 1),
    }
}

/// `ceil(n / d)` for `d > 0`
fn ceil_div(n: i128, d: i128) -> i128 {
    -((-n).div_euclid(d))
}

fn gcd_i128(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn gcd_i256(a: I256, b: I256) -> I256 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != I256::ZERO {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_half_even_on_ties_and_negatives() {
        assert_eq!(round_half_even_div(5, 2), 2);
        assert_eq!(round_half_even_div(7, 2), 4);
        assert_eq!(round_half_even_div(-5, 2), -2);
        assert_eq!(round_half_even_div(-7, 2), -4);
        assert_eq!(round_half_even_div(-1, 2), 0);
        assert_eq!(round_half_even_div(1, 3), 0);
        assert_eq!(round_half_even_div(2, 3), 1);
        assert_eq!(round_half_even_div(-2, 3), -1);
    }

    #[test]
    fn constant_120_bpm_at_48k_is_25_samples_per_tick() {
        let k = Constant::new(48_000, 960, Bpm::default());
        assert_eq!((k.a, k.b), (25, 1));
        assert_eq!(k.sample_offset(3), 75);
        assert_eq!(k.tick_offset(74), 2);
        assert_eq!(k.tick_offset(75), 3);
        assert_eq!(k.tick_offset(-1), -1);
        assert_eq!(k.tick_offset(-25), -1);
        assert_eq!(k.tick_offset(-26), -2);
    }

    #[test]
    fn constant_tick_offset_is_the_galois_inverse() {
        for (sr, ppq, micro) in [(44_100, 960, 119_880_000), (8_000, 960, 1_000_000_000), (48_000, 7, 1_234_567)] {
            let k = Constant::new(sr, ppq, Bpm::from_micro(micro).unwrap());
            for r in -500..500 {
                let t = k.tick_offset(r);
                assert!(k.sample_offset(t) <= r && k.sample_offset(t + 1) > r, "sr {sr} r {r}");
            }
        }
    }

    #[test]
    fn ramp_ends_where_its_duration_says() {
        // 120 → 60 BPM over one bar of 4/4 at 960 PPQ: Y = 120·48000·3840·10⁶ / (960·180·10⁶) = 128000 samples
        let r = Ramp::new(48_000, 960, Bpm::default(), Bpm::from_integer(60).unwrap(), 3840);
        assert_eq!((r.y_num, r.y_den), (128_000, 1));
        assert_eq!(r.sample_offset(3840), 128_000);
        assert_eq!(r.sample_offset(0), 0);
        // The first tick lasts about 25 samples (120 BPM), the last about 50 (60 BPM)
        assert_eq!(r.sample_offset(1), 25);
        assert!((r.sample_offset(3840) - r.sample_offset(3839) - 50).abs() <= 1);
        for s in 0..128_000 {
            let t = r.tick_offset(s);
            assert!(r.sample_offset(t) <= s && r.sample_offset(t + 1) > s, "s {s}");
        }
    }
}
