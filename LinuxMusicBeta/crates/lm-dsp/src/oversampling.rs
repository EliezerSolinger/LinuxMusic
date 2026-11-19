//! Oversampling by powers of two, with half-band polyphase IIR filters: a port of Laurent de Soras' HIIR (WTFPL),
//! the scalar version, and of the legacy `OversamplingHelpers` that chain its stages.

use crate::math::{cos, f64c::PI, pow, sin, tan};

/// The coefficients of a half-band polyphase IIR filter of @p count coefficients (filter order 2 count + 1) for a
/// transition bandwidth @p transition (relative to the sample rate, in ]0, 1/2[), the stopband attenuation as high
/// as these allow (HIIR's `PolyphaseIir2Designer::compute_coefs_spec_order_tbw`)
pub fn half_band_coefficients(coefficients: &mut [f64], transition: f64) {
    let (k, q) = transition_parameters(transition);
    let order = coefficients.len() as i32 * 2 + 1;
    for (index, c) in coefficients.iter_mut().enumerate() {
        *c = coefficient(index as i32, k, q, order);
    }
}

fn transition_parameters(transition: f64) -> (f64, f64) {
    let mut k = tan((1.0 - transition * 2.0) * PI / 4.0);
    k *= k;
    let kksqrt = pow(1.0 - k * k, 0.25);
    let e = 0.5 * (1.0 - kksqrt) / (1.0 + kksqrt);
    let e2 = e * e;
    let e4 = e2 * e2;
    let q = e * (1.0 + e4 * (2.0 + e4 * (15.0 + 150.0 * e4)));
    (k, q)
}

/// x^n by squaring, as HIIR's `ipowp`
fn ipowp(mut x: f64, mut n: i64) -> f64 {
    let mut z = 1.0;
    while n != 0 {
        if n & 1 != 0 {
            z *= x;
        }
        n >>= 1;
        x *= x;
    }
    z
}

fn coefficient(index: i32, k: f64, q: f64, order: i32) -> f64 {
    let c = index + 1;
    let num = accumulated_numerator(q, order, c) * pow(q, 0.25);
    let den = accumulated_denominator(q, order, c) + 0.5;
    let ww = num / den;
    let wwsq = ww * ww;
    let x = ((1.0 - wwsq * k) * (1.0 - wwsq / k)).sqrt() / (1.0 + wwsq);
    (1.0 - x) / (1.0 + x)
}

fn accumulated_numerator(q: f64, order: i32, c: i32) -> f64 {
    let mut i: i64 = 0;
    let mut j = 1.0;
    let mut acc = 0.0;
    loop {
        let mut q_ii1 = ipowp(q, i * (i + 1));
        q_ii1 *= sin(f64::from((i as i32 * 2 + 1) * c) * PI / f64::from(order)) * j;
        acc += q_ii1;
        j = -j;
        i += 1;
        if q_ii1.abs() <= 1e-100 {
            break;
        }
    }
    acc
}

fn accumulated_denominator(q: f64, order: i32, c: i32) -> f64 {
    let mut i: i64 = 1;
    let mut j = -1.0;
    let mut acc = 0.0;
    loop {
        let mut q_i2 = ipowp(q, i * i);
        q_i2 *= cos(f64::from(i as i32 * 2 * c) * PI / f64::from(order)) * j;
        acc += q_i2;
        j = -j;
        i += 1;
        if q_i2.abs() <= 1e-100 {
            break;
        }
    }
    acc
}

/// The two-path allpass chain of one half-band filter of N coefficients: its coefficients and memories, laid out as
/// HIIR's (two leading slots, then one per coefficient)
#[derive(Clone, Debug)]
struct Stages<const N: usize> {
    coef: [f32; N],
    mem: [f32; 2],
    mems: [f32; N],
}

impl<const N: usize> Default for Stages<N> {
    fn default() -> Self {
        Self { coef: [0.0; N], mem: [0.0; 2], mems: [0.0; N] }
    }
}

impl<const N: usize> Stages<N> {
    fn set_coefficients(&mut self, coefficients: &[f64]) {
        for (c, &v) in self.coef.iter_mut().zip(coefficients) {
            *c = v as f32;
        }
    }

    fn clear(&mut self) {
        self.mem = [0.0; 2];
        self.mems = [0.0; N];
    }

    /// The memory at HIIR's index @p i (0 and 1: the leading slots)
    #[inline]
    fn get(&self, i: usize) -> f32 {
        if i < 2 { self.mem[i] } else { self.mems[i - 2] }
    }
    #[inline]
    fn set(&mut self, i: usize, v: f32) {
        if i < 2 { self.mem[i] = v } else { self.mems[i - 2] = v }
    }

    /// HIIR's `StageProcTpl::process_sample_pos`, unrolled into a loop: the allpasses in pairs, one on each path
    #[inline]
    fn process(&mut self, s0: &mut f32, s1: &mut f32) {
        let mut remaining = N;
        while remaining >= 2 {
            let cnt = N + 2 - remaining;
            let mut tmp0 = *s0;
            tmp0 -= self.get(cnt);
            tmp0 *= self.coef[cnt - 2];
            tmp0 += self.get(cnt - 2);
            let mut tmp1 = *s1;
            tmp1 -= self.get(cnt + 1);
            tmp1 *= self.coef[cnt - 1];
            tmp1 += self.get(cnt - 1);
            self.set(cnt - 2, *s0);
            self.set(cnt - 1, *s1);
            *s0 = tmp0;
            *s1 = tmp1;
            remaining -= 2;
        }
        if remaining == 1 {
            let cnt = N + 1;
            let mut tmp0 = *s0;
            tmp0 -= self.get(cnt);
            tmp0 *= self.coef[cnt - 2];
            tmp0 += self.get(cnt - 2);
            self.set(cnt - 2, *s0);
            self.set(cnt - 1, *s1);
            self.set(cnt, tmp0);
            *s0 = tmp0;
        } else {
            let cnt = N + 2;
            self.set(cnt - 2, *s0);
            self.set(cnt - 1, *s1);
        }
    }

    /// Upsamples one sample into two
    #[inline]
    fn up(&mut self, input: f32) -> (f32, f32) {
        let mut even = input;
        let mut odd = input;
        self.process(&mut even, &mut odd);
        (even, odd)
    }

    /// Downsamples two samples into one
    #[inline]
    fn down(&mut self, pair: [f32; 2]) -> f32 {
        let mut s0 = pair[1];
        let mut s1 = pair[0];
        self.process(&mut s0, &mut s1);
        0.5 * (s0 + s1)
    }
}

/// The passband kept by default, Hz
pub const DEFAULT_PASSBAND: f32 = 19600.0;

/// The transition bandwidths of each stage: the first, the tightest, from the passband; each next one halfway to 1/2
fn transitions(sample_rate: f32, passband: f32) -> [f32; 3] {
    let first = 0.5 - passband / sample_rate;
    let second = (first + 0.5) * 0.5;
    let rest = (second + 0.5) * 0.5;
    [first, second, rest]
}

/// Oversampling by 2^@p STAGES (1 to 3: 2, 4 or 8 times), one sample in, 2^STAGES out: the first stage with 8
/// coefficients, the second with 4, the third with 2 (the legacy `Upsampler<MaxStages, 8>`)
#[derive(Clone, Debug)]
pub struct Upsampler<const STAGES: usize> {
    first: Stages<8>,
    second: Stages<4>,
    rest: Stages<2>,
}

impl<const STAGES: usize> Upsampler<STAGES> {
    pub const FACTOR: usize = 1 << STAGES;

    pub fn new(sample_rate: f32) -> Self {
        Self::with_passband(sample_rate, DEFAULT_PASSBAND)
    }

    pub fn with_passband(sample_rate: f32, passband: f32) -> Self {
        const { assert!(STAGES >= 1 && STAGES <= 3) };
        let t = transitions(sample_rate, passband);
        let mut s = Self { first: Stages::default(), second: Stages::default(), rest: Stages::default() };
        let mut c8 = [0.0; 8];
        half_band_coefficients(&mut c8, f64::from(t[0]));
        s.first.set_coefficients(&c8);
        let mut c4 = [0.0; 4];
        half_band_coefficients(&mut c4, f64::from(t[1]));
        s.second.set_coefficients(&c4);
        let mut c2 = [0.0; 2];
        half_band_coefficients(&mut c2, f64::from(t[2]));
        s.rest.set_coefficients(&c2);
        s
    }

    pub fn reset(&mut self) {
        self.first.clear();
        self.second.clear();
        self.rest.clear();
    }

    /// One sample in, `FACTOR` out (only the first `FACTOR` of @p out are written)
    #[inline]
    pub fn process(&mut self, out: &mut [f32; 8], input: f32) {
        let total = Self::FACTOR;
        let gap1 = total / 2;
        let (a, b) = self.first.up(input);
        out[0] = a;
        out[gap1] = b;
        if STAGES >= 2 {
            let gap2 = gap1 / 2;
            let (a, b) = self.second.up(out[0]);
            out[0] = a;
            out[gap2] = b;
            let (a, b) = self.second.up(out[gap1]);
            out[gap1] = a;
            out[gap1 + gap2] = b;
        }
        if STAGES >= 3 {
            let count = 4;
            let gap = total / count;
            for j in 0..count {
                let at = j * gap;
                let (a, b) = self.rest.up(out[at]);
                out[at] = a;
                out[at + gap / 2] = b;
            }
        }
    }
}

/// The inverse of [`Upsampler`]: 2^@p STAGES samples in, one out
#[derive(Clone, Debug)]
pub struct Downsampler<const STAGES: usize> {
    first: Stages<8>,
    second: Stages<4>,
    rest: Stages<2>,
}

impl<const STAGES: usize> Downsampler<STAGES> {
    pub const FACTOR: usize = 1 << STAGES;

    pub fn new(sample_rate: f32) -> Self {
        Self::with_passband(sample_rate, DEFAULT_PASSBAND)
    }

    pub fn with_passband(sample_rate: f32, passband: f32) -> Self {
        const { assert!(STAGES >= 1 && STAGES <= 3) };
        let t = transitions(sample_rate, passband);
        let mut s = Self { first: Stages::default(), second: Stages::default(), rest: Stages::default() };
        let mut c8 = [0.0; 8];
        half_band_coefficients(&mut c8, f64::from(t[0]));
        s.first.set_coefficients(&c8);
        let mut c4 = [0.0; 4];
        half_band_coefficients(&mut c4, f64::from(t[1]));
        s.second.set_coefficients(&c4);
        let mut c2 = [0.0; 2];
        half_band_coefficients(&mut c2, f64::from(t[2]));
        s.rest.set_coefficients(&c2);
        s
    }

    pub fn reset(&mut self) {
        self.first.clear();
        self.second.clear();
        self.rest.clear();
    }

    /// `FACTOR` samples in (@p input, which is used as scratch), one out
    #[inline]
    pub fn process(&mut self, input: &mut [f32; 8]) -> f32 {
        if STAGES >= 3 {
            for j in 0..4 {
                input[j] = self.rest.down([input[j * 2], input[j * 2 + 1]]);
            }
        }
        if STAGES >= 2 {
            for j in 0..2 {
                input[j] = self.second.down([input[j * 2], input[j * 2 + 1]]);
            }
        }
        input[0] = self.first.down([input[0], input[1]]);
        input[0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coefficients_are_allpass_coefficients() {
        let mut c = [0.0; 8];
        half_band_coefficients(&mut c, 0.5 - 19600.0 / 48000.0);
        for (i, &v) in c.iter().enumerate() {
            assert!(v > 0.0 && v < 1.0, "{i}: {v}");
        }
        assert!(c.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn a_round_trip_keeps_a_low_sine() {
        let rate = 48000.0f32;
        let mut up = Upsampler::<2>::new(rate);
        let mut down = Downsampler::<2>::new(rate);
        let mut peak = 0.0f32;
        let mut buffer = [0.0f32; 8];
        for n in 0..9600 {
            let x = crate::math::sinf(2.0 * core::f32::consts::PI * 1000.0 * n as f32 / rate);
            up.process(&mut buffer, x);
            let y = down.process(&mut buffer);
            if n > 4800 {
                peak = peak.max(y.abs());
            }
        }
        assert!((peak - 1.0).abs() < 0.01, "{peak}");
    }
}
