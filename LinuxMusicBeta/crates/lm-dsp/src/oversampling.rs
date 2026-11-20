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

/// The most stages: 32 times
pub const MAX_STAGES: usize = 5;
/// The samples of the most stages
pub const MAX_FACTOR: usize = 1 << MAX_STAGES;

/// The half-band filters of each stage: the first with 8 coefficients, the second with 4, the others with 2; each
/// stage's transition bandwidth halfway from the last one's to 1/2 (the first, the tightest, from the passband)
#[derive(Clone, Debug)]
struct Chain {
    stages: usize,
    first: Stages<8>,
    second: Stages<4>,
    rest: [Stages<2>; MAX_STAGES - 2],
}

impl Chain {
    fn new(stages: usize, sample_rate: f32, passband: f32) -> Self {
        assert!(stages <= MAX_STAGES, "at most {MAX_STAGES} oversampling stages");
        let mut c = Self { stages, first: Stages::default(), second: Stages::default(), rest: Default::default() };
        let mut transition = 0.5 - passband / sample_rate;
        let mut c8 = [0.0; 8];
        half_band_coefficients(&mut c8, f64::from(transition));
        c.first.set_coefficients(&c8);
        transition = (transition + 0.5) * 0.5;
        let mut c4 = [0.0; 4];
        half_band_coefficients(&mut c4, f64::from(transition));
        c.second.set_coefficients(&c4);
        transition = (transition + 0.5) * 0.5;
        for rest in c.rest.iter_mut().take(stages.saturating_sub(2)) {
            let mut c2 = [0.0; 2];
            half_band_coefficients(&mut c2, f64::from(transition));
            rest.set_coefficients(&c2);
            transition = (transition + 0.5) * 0.5;
        }
        c
    }

    fn clear(&mut self) {
        self.first.clear();
        self.second.clear();
        for rest in &mut self.rest {
            rest.clear();
        }
    }

    /// One sample in, 2^stages out (into the first 2^stages of @p out)
    #[inline]
    fn up(&mut self, out: &mut [f32], input: f32) {
        let total = 1 << self.stages;
        out[0] = input;
        let gap1 = total / 2;
        if self.stages >= 1 {
            (out[0], out[gap1]) = self.first.up(out[0]);
        }
        if self.stages >= 2 {
            let gap2 = gap1 / 2;
            (out[0], out[gap2]) = self.second.up(out[0]);
            (out[gap1], out[gap1 + gap2]) = self.second.up(out[gap1]);
        }
        for i in 2..self.stages {
            let count = 1 << i;
            let gap = total / count;
            for j in 0..count {
                let at = j * gap;
                (out[at], out[at + gap / 2]) = self.rest[i - 2].up(out[at]);
            }
        }
    }

    /// 2^stages samples in (@p input, used as scratch), one out
    #[inline]
    fn down(&mut self, input: &mut [f32]) -> f32 {
        for i in (2..self.stages).rev() {
            for j in 0..1 << i {
                input[j] = self.rest[i - 2].down([input[j * 2], input[j * 2 + 1]]);
            }
        }
        if self.stages >= 2 {
            for j in 0..2 {
                input[j] = self.second.down([input[j * 2], input[j * 2 + 1]]);
            }
        }
        if self.stages >= 1 {
            input[0] = self.first.down([input[0], input[1]]);
        }
        input[0]
    }
}

/// Oversampling by 2^stages (0 to 5 stages: 1 to 32 times), one sample in, 2^stages out (the legacy
/// `Upsampler<MaxStages, 8>`)
#[derive(Clone, Debug)]
pub struct VariableUpsampler {
    chain: Chain,
}

impl VariableUpsampler {
    pub fn new(stages: usize, sample_rate: f32) -> Self {
        Self { chain: Chain::new(stages, sample_rate, DEFAULT_PASSBAND) }
    }

    pub fn stages(&self) -> usize {
        self.chain.stages
    }

    pub fn reset(&mut self) {
        self.chain.clear();
    }

    /// One sample in, 2^stages out (only the first 2^stages of @p out are written)
    #[inline]
    pub fn process(&mut self, out: &mut [f32], input: f32) {
        self.chain.up(out, input);
    }
}

/// The inverse of [`VariableUpsampler`]: 2^stages samples in, one out
#[derive(Clone, Debug)]
pub struct VariableDownsampler {
    chain: Chain,
}

impl VariableDownsampler {
    pub fn new(stages: usize, sample_rate: f32) -> Self {
        Self { chain: Chain::new(stages, sample_rate, DEFAULT_PASSBAND) }
    }

    pub fn stages(&self) -> usize {
        self.chain.stages
    }

    pub fn reset(&mut self) {
        self.chain.clear();
    }

    /// 2^stages samples in (@p input, used as scratch), one out
    #[inline]
    pub fn process(&mut self, input: &mut [f32]) -> f32 {
        self.chain.down(input)
    }
}

/// Oversampling by 2^@p STAGES (1 to 3: 2, 4 or 8 times), fixed when it is built: [`VariableUpsampler`]'s
#[derive(Clone, Debug)]
pub struct Upsampler<const STAGES: usize> {
    chain: Chain,
}

impl<const STAGES: usize> Upsampler<STAGES> {
    pub const FACTOR: usize = 1 << STAGES;

    pub fn new(sample_rate: f32) -> Self {
        Self::with_passband(sample_rate, DEFAULT_PASSBAND)
    }

    pub fn with_passband(sample_rate: f32, passband: f32) -> Self {
        const { assert!(STAGES >= 1 && STAGES <= 3) };
        Self { chain: Chain::new(STAGES, sample_rate, passband) }
    }

    pub fn reset(&mut self) {
        self.chain.clear();
    }

    /// One sample in, `FACTOR` out (only the first `FACTOR` of @p out are written)
    #[inline]
    pub fn process(&mut self, out: &mut [f32; 8], input: f32) {
        self.chain.up(out, input);
    }
}

/// The inverse of [`Upsampler`]: 2^@p STAGES samples in, one out
#[derive(Clone, Debug)]
pub struct Downsampler<const STAGES: usize> {
    chain: Chain,
}

impl<const STAGES: usize> Downsampler<STAGES> {
    pub const FACTOR: usize = 1 << STAGES;

    pub fn new(sample_rate: f32) -> Self {
        Self::with_passband(sample_rate, DEFAULT_PASSBAND)
    }

    pub fn with_passband(sample_rate: f32, passband: f32) -> Self {
        const { assert!(STAGES >= 1 && STAGES <= 3) };
        Self { chain: Chain::new(STAGES, sample_rate, passband) }
    }

    pub fn reset(&mut self) {
        self.chain.clear();
    }

    /// `FACTOR` samples in (@p input, which is used as scratch), one out
    #[inline]
    pub fn process(&mut self, input: &mut [f32; 8]) -> f32 {
        self.chain.down(input)
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
