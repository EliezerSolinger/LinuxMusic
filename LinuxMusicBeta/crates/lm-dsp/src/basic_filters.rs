//! The legacy program's general filters (legacy `BasicFilters.h`): the filter of every instrument's sound shaping and
//! of the DualFilter, with its 22 types, and the Linkwitz-Riley crossover and the one-pole the plugins use.

use crate::math::{cosf, expf, f32c::PI, sinf, tan};
use crate::oscillator::lerp;

/// The values below which a one-pole or a formant filter stops computing (the legacy `F_EPSILON`)
const F_EPSILON: f32 = 1.0e-10;

/// A 4th-order Linkwitz-Riley filter (low- or high-pass), in double precision, stereo.
///
/// The legacy program's response (the bilinear transform with the cutoff prewarped), as two identical 2nd-order
/// Butterworth sections in transposed direct form II: a Linkwitz-Riley filter is a Butterworth one squared. The
/// legacy computed it as one 4th-order section in direct form II, whose inner state grows to about (rate / cutoff)^4
/// times the signal: with a low cutoff, any change of the cutoff while it plays released that state as a burst
/// (a crossover swept from 50 Hz to 20 kHz gave 1e37). The two forms are the same filter; they differ by rounding
/// only (the A/B still nulls), and a change of cutoff here never adds more than the signal's own size
#[derive(Clone, Debug)]
pub struct LinkwitzRiley {
    sample_rate: f32,
    /// b0, b1, b2, a1, a2 of each section (both the same)
    b: [f64; 3],
    a: [f64; 2],
    /// Per section and channel, the two state values of the transposed form
    s: [[[f64; 2]; 2]; 2],
}

impl LinkwitzRiley {
    pub fn new(sample_rate: f32) -> Self {
        Self { sample_rate, b: [0.0; 3], a: [0.0; 2], s: [[[0.0; 2]; 2]; 2] }
    }

    pub fn clear_history(&mut self) {
        self.s = [[[0.0; 2]; 2]; 2];
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    /// The Butterworth section's denominator and its prewarped constants: (wc, k, 1 / (k² + √2 wc k + wc²))
    fn section(&mut self, freq: f32) -> (f64, f64, f64) {
        use core::f64::consts::{PI, SQRT_2};
        let wc = 2.0 * PI * f64::from(freq);
        let k = wc / tan(PI * f64::from(freq) / f64::from(self.sample_rate));
        let (wc2, k2) = (wc * wc, k * k);
        let norm = 1.0 / (k2 + SQRT_2 * wc * k + wc2);
        self.a = [2.0 * (wc2 - k2) * norm, (k2 - SQRT_2 * wc * k + wc2) * norm];
        (wc, k, norm)
    }

    pub fn set_lowpass(&mut self, freq: f32) {
        let (wc, _, norm) = self.section(freq);
        let b0 = wc * wc * norm;
        self.b = [b0, 2.0 * b0, b0];
    }

    pub fn set_highpass(&mut self, freq: f32) {
        let (_, k, norm) = self.section(freq);
        let b0 = k * k * norm;
        self.b = [b0, -2.0 * b0, b0];
    }

    #[inline]
    pub fn update(&mut self, input: f32, ch: usize) -> f32 {
        let [b0, b1, b2] = self.b;
        let [a1, a2] = self.a;
        let mut x = f64::from(input);
        for section in &mut self.s {
            let state = &mut section[ch];
            let y = b0 * x + state[0];
            state[0] = b1 * x - a1 * y + state[1];
            state[1] = b2 * x - a2 * y;
            x = y;
        }
        x as f32
    }
}

/// A stereo one-pole filter: y = a0 x + b1 y1
#[derive(Clone, Debug)]
pub struct OnePole {
    a0: f32,
    b1: f32,
    z1: [f32; 2],
}

impl Default for OnePole {
    fn default() -> Self {
        Self { a0: 1.0, b1: 0.0, z1: [0.0; 2] }
    }
}

impl OnePole {
    pub fn clear_history(&mut self) {
        self.z1 = [0.0; 2];
    }

    pub fn set_coeffs(&mut self, a0: f32, b1: f32) {
        self.a0 = a0;
        self.b1 = b1;
    }

    #[inline]
    pub fn update(&mut self, s: f32, ch: usize) -> f32 {
        if s.abs() < F_EPSILON && self.z1[ch].abs() < F_EPSILON {
            return 0.0;
        }
        self.z1[ch] = s * self.a0 + self.z1[ch] * self.b1;
        self.z1[ch]
    }
}

/// A stereo biquad in single precision, the legacy `BiQuad` (the same as [`crate::rbj::StereoBiquad`])
pub use crate::rbj::StereoBiquad;

/// The filter types. Their keys are saved with projects and never change (U30).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FilterType {
    LowPass,
    HiPass,
    BandPassCsg,
    BandPassCzpg,
    Notch,
    AllPass,
    Moog,
    DoubleLowPass,
    LowpassRc12,
    BandpassRc12,
    HighpassRc12,
    LowpassRc24,
    BandpassRc24,
    HighpassRc24,
    Formant,
    DoubleMoog,
    LowpassSv,
    BandpassSv,
    HighpassSv,
    NotchSv,
    FastFormant,
    Tripole,
}

impl FilterType {
    /// In the legacy program's order
    pub const ALL: [FilterType; 22] = [
        FilterType::LowPass,
        FilterType::HiPass,
        FilterType::BandPassCsg,
        FilterType::BandPassCzpg,
        FilterType::Notch,
        FilterType::AllPass,
        FilterType::Moog,
        FilterType::DoubleLowPass,
        FilterType::LowpassRc12,
        FilterType::BandpassRc12,
        FilterType::HighpassRc12,
        FilterType::LowpassRc24,
        FilterType::BandpassRc24,
        FilterType::HighpassRc24,
        FilterType::Formant,
        FilterType::DoubleMoog,
        FilterType::LowpassSv,
        FilterType::BandpassSv,
        FilterType::HighpassSv,
        FilterType::NotchSv,
        FilterType::FastFormant,
        FilterType::Tripole,
    ];

    pub fn key(self) -> &'static str {
        match self {
            FilterType::LowPass => "low-pass",
            FilterType::HiPass => "high-pass",
            FilterType::BandPassCsg => "band-pass-csg",
            FilterType::BandPassCzpg => "band-pass-czpg",
            FilterType::Notch => "notch",
            FilterType::AllPass => "all-pass",
            FilterType::Moog => "moog",
            FilterType::DoubleLowPass => "double-low-pass",
            FilterType::LowpassRc12 => "rc-low-pass-12",
            FilterType::BandpassRc12 => "rc-band-pass-12",
            FilterType::HighpassRc12 => "rc-high-pass-12",
            FilterType::LowpassRc24 => "rc-low-pass-24",
            FilterType::BandpassRc24 => "rc-band-pass-24",
            FilterType::HighpassRc24 => "rc-high-pass-24",
            FilterType::Formant => "formant",
            FilterType::DoubleMoog => "double-moog",
            FilterType::LowpassSv => "sv-low-pass",
            FilterType::BandpassSv => "sv-band-pass",
            FilterType::HighpassSv => "sv-high-pass",
            FilterType::NotchSv => "sv-notch",
            FilterType::FastFormant => "fast-formant",
            FilterType::Tripole => "tripole",
        }
    }

    pub fn from_key(key: &str) -> Option<FilterType> {
        Self::ALL.into_iter().find(|t| t.key() == key)
    }
}

/// The lowest frequency, Hz
pub const MIN_FREQ: f32 = 5.0;
/// The lowest resonance
pub const MIN_Q: f32 = 0.01;

/// The formants of a, e, i, o, u, a
const FORMANTS: [[f32; 2]; 6] =
    [[1000.0, 1400.0], [500.0, 2300.0], [320.0, 3200.0], [500.0, 1000.0], [320.0, 800.0], [1000.0, 1400.0]];

/// The state of one stage of a [`BasicFilter`]
#[derive(Clone, Debug)]
struct Stage {
    biquad: StereoBiquad,
    // The Moog's and the tripole's coefficients
    r: f32,
    p: f32,
    k: f32,
    // The RC filters'
    rca: f32,
    rcb: f32,
    rcc: f32,
    rcq: f32,
    // The formant filters'
    vfa: [f32; 4],
    vfb: [f32; 4],
    vfc: [f32; 4],
    vfq: f32,
    // The state variable filters'
    svf1: f32,
    svf2: f32,
    svq: f32,
    // History
    y1: [f32; 2],
    y2: [f32; 2],
    y3: [f32; 2],
    y4: [f32; 2],
    oldx: [f32; 2],
    oldy1: [f32; 2],
    oldy2: [f32; 2],
    oldy3: [f32; 2],
    last: [f32; 2],
    rcbp0: [f32; 2],
    rclp0: [f32; 2],
    rchp0: [f32; 2],
    rclast0: [f32; 2],
    rcbp1: [f32; 2],
    rclp1: [f32; 2],
    rchp1: [f32; 2],
    rclast1: [f32; 2],
    vfbp: [[f32; 2]; 6],
    vfhp: [[f32; 2]; 6],
    vflast: [[f32; 2]; 6],
    delay1: [f32; 2],
    delay2: [f32; 2],
    delay3: [f32; 2],
    delay4: [f32; 2],
    kind: FilterType,
}

impl Stage {
    fn new() -> Self {
        Self {
            biquad: StereoBiquad::default(),
            r: 0.0,
            p: 0.0,
            k: 0.0,
            rca: 0.0,
            rcb: 0.0,
            rcc: 0.0,
            rcq: 0.0,
            vfa: [0.0; 4],
            vfb: [0.0; 4],
            vfc: [0.0; 4],
            vfq: 0.0,
            svf1: 0.0,
            svf2: 0.0,
            svq: 0.0,
            y1: [0.0; 2],
            y2: [0.0; 2],
            y3: [0.0; 2],
            y4: [0.0; 2],
            oldx: [0.0; 2],
            oldy1: [0.0; 2],
            oldy2: [0.0; 2],
            oldy3: [0.0; 2],
            last: [0.0; 2],
            rcbp0: [0.0; 2],
            rclp0: [0.0; 2],
            rchp0: [0.0; 2],
            rclast0: [0.0; 2],
            rcbp1: [0.0; 2],
            rclp1: [0.0; 2],
            rchp1: [0.0; 2],
            rclast1: [0.0; 2],
            vfbp: [[0.0; 2]; 6],
            vfhp: [[0.0; 2]; 6],
            vflast: [[0.0; 2]; 6],
            delay1: [0.0; 2],
            delay2: [0.0; 2],
            delay3: [0.0; 2],
            delay4: [0.0; 2],
            kind: FilterType::LowPass,
        }
    }

    fn clear_history(&mut self) {
        self.biquad.clear();
        let kind = self.kind;
        let biquad = self.biquad;
        let (r, p, k, rca, rcb, rcc, rcq) = (self.r, self.p, self.k, self.rca, self.rcb, self.rcc, self.rcq);
        let (vfa, vfb, vfc, vfq, svf1, svf2, svq) =
            (self.vfa, self.vfb, self.vfc, self.vfq, self.svf1, self.svf2, self.svq);
        *self = Self { kind, biquad, r, p, k, rca, rcb, rcc, rcq, vfa, vfb, vfc, vfq, svf1, svf2, svq, ..Self::new() };
    }

    /// One sample through this stage; `Some` when the type's output is final (the legacy returned before the second
    /// stage), `None` with the output in @p out otherwise
    #[inline]
    fn update(&mut self, input: f32, ch: usize, out: &mut f32) -> Option<f32> {
        let clamp = |x: f32, limit: f32| x.clamp(-limit, limit);
        match self.kind {
            FilterType::Moog => {
                let x = input - self.r * self.y4[ch];
                // Four cascaded one-pole filters (bilinear transform)
                self.y1[ch] = clamp((x + self.oldx[ch]) * self.p - self.k * self.y1[ch], 10.0);
                self.y2[ch] = clamp((self.y1[ch] + self.oldy1[ch]) * self.p - self.k * self.y2[ch], 10.0);
                self.y3[ch] = clamp((self.y2[ch] + self.oldy2[ch]) * self.p - self.k * self.y3[ch], 10.0);
                self.y4[ch] = clamp((self.y3[ch] + self.oldy3[ch]) * self.p - self.k * self.y4[ch], 10.0);
                self.oldx[ch] = x;
                self.oldy1[ch] = self.y1[ch];
                self.oldy2[ch] = self.y2[ch];
                self.oldy3[ch] = self.y3[ch];
                *out = self.y4[ch] - self.y4[ch] * self.y4[ch] * self.y4[ch] * (1.0 / 6.0);
                None
            }
            FilterType::Tripole => {
                // Three one-poles, 4 times oversampled: the input interpolated linearly, the output averaged
                let mut result = 0.0f32;
                let mut ip = 0.0f32;
                for _ in 0..4 {
                    ip += 0.25;
                    let x = lerp(self.last[ch], input, ip) - self.r * self.y3[ch];
                    self.y1[ch] = clamp((x + self.oldx[ch]) * self.p - self.k * self.y1[ch], 10.0);
                    self.y2[ch] = clamp((self.y1[ch] + self.oldy1[ch]) * self.p - self.k * self.y2[ch], 10.0);
                    self.y3[ch] = clamp((self.y2[ch] + self.oldy2[ch]) * self.p - self.k * self.y3[ch], 10.0);
                    self.oldx[ch] = x;
                    self.oldy1[ch] = self.y1[ch];
                    self.oldy2[ch] = self.y2[ch];
                    result += self.y3[ch] - self.y3[ch] * self.y3[ch] * self.y3[ch] * (1.0 / 6.0);
                }
                result *= 0.25;
                self.last[ch] = input;
                Some(result)
            }
            // Hal Chamberlin's state variable filters, 2 times oversampled (after Nekobee)
            FilterType::LowpassSv | FilterType::BandpassSv => {
                for _ in 0..2 {
                    self.delay2[ch] += self.svf1 * self.delay1[ch];
                    let highpass = input - self.delay2[ch] - self.svq * self.delay1[ch];
                    self.delay1[ch] += self.svf1 * highpass;
                    self.delay4[ch] += self.svf2 * self.delay3[ch];
                    let highpass = self.delay2[ch] - self.delay4[ch] - self.svq * self.delay3[ch];
                    self.delay3[ch] += self.svf2 * highpass;
                }
                Some(if self.kind == FilterType::LowpassSv { self.delay4[ch] } else { self.delay3[ch] })
            }
            FilterType::HighpassSv => {
                let mut hp = 0.0;
                for _ in 0..2 {
                    self.delay2[ch] += self.svf1 * self.delay1[ch];
                    hp = input - self.delay2[ch] - self.svq * self.delay1[ch];
                    self.delay1[ch] += self.svf1 * hp;
                }
                Some(hp)
            }
            FilterType::NotchSv => {
                let mut hp1 = 0.0;
                for _ in 0..2 {
                    self.delay2[ch] += self.svf1 * self.delay1[ch];
                    hp1 = input - self.delay2[ch] - self.svq * self.delay1[ch];
                    self.delay1[ch] += self.svf1 * hp1;
                    self.delay4[ch] += self.svf2 * self.delay3[ch];
                    let hp2 = self.delay2[ch] - self.delay4[ch] - self.svq * self.delay3[ch];
                    self.delay3[ch] += self.svf2 * hp2;
                }
                Some(self.delay4[ch] + hp1)
            }
            // A 4 times oversampled simulation of an active RC network (S. Fendt, 1998-2009, GPL v2 or later): it can
            // be driven to self-oscillation; the limits stay
            FilterType::LowpassRc12 => {
                let mut lp = 0.0;
                for _ in 0..4 {
                    let x = clamp(input + self.rcbp0[ch] * self.rcq, 1.0);
                    lp = clamp(x * self.rcb + self.rclp0[ch] * self.rca, 1.0);
                    let hp = clamp(self.rcc * (self.rchp0[ch] + x - self.rclast0[ch]), 1.0);
                    let bp = clamp(hp * self.rcb + self.rcbp0[ch] * self.rca, 1.0);
                    self.rclast0[ch] = x;
                    self.rclp0[ch] = lp;
                    self.rchp0[ch] = hp;
                    self.rcbp0[ch] = bp;
                }
                Some(lp)
            }
            FilterType::HighpassRc12 | FilterType::BandpassRc12 => {
                let (mut hp, mut bp) = (0.0, 0.0);
                for _ in 0..4 {
                    let x = clamp(input + self.rcbp0[ch] * self.rcq, 1.0);
                    hp = clamp(self.rcc * (self.rchp0[ch] + x - self.rclast0[ch]), 1.0);
                    bp = clamp(hp * self.rcb + self.rcbp0[ch] * self.rca, 1.0);
                    self.rclast0[ch] = x;
                    self.rchp0[ch] = hp;
                    self.rcbp0[ch] = bp;
                }
                Some(if self.kind == FilterType::HighpassRc12 { hp } else { bp })
            }
            FilterType::LowpassRc24 => {
                let mut lp = 0.0;
                for _ in 0..4 {
                    // The first stage, as the 12 dB one
                    let mut x = clamp(input + self.rcbp0[ch] * self.rcq, 1.0);
                    lp = clamp(x * self.rcb + self.rclp0[ch] * self.rca, 1.0);
                    let mut hp = clamp(self.rcc * (self.rchp0[ch] + x - self.rclast0[ch]), 1.0);
                    let mut bp = clamp(hp * self.rcb + self.rcbp0[ch] * self.rca, 1.0);
                    self.rclast0[ch] = x;
                    self.rclp0[ch] = lp;
                    self.rcbp0[ch] = bp;
                    self.rchp0[ch] = hp;
                    // The second, from the first's output
                    x = clamp(lp + self.rcbp1[ch] * self.rcq, 1.0);
                    lp = clamp(x * self.rcb + self.rclp1[ch] * self.rca, 1.0);
                    hp = clamp(self.rcc * (self.rchp1[ch] + x - self.rclast1[ch]), 1.0);
                    bp = clamp(hp * self.rcb + self.rcbp1[ch] * self.rca, 1.0);
                    self.rclast1[ch] = x;
                    self.rclp1[ch] = lp;
                    self.rcbp1[ch] = bp;
                    self.rchp1[ch] = hp;
                }
                Some(lp)
            }
            FilterType::HighpassRc24 | FilterType::BandpassRc24 => {
                let (mut hp, mut bp) = (0.0, 0.0);
                for _ in 0..4 {
                    let mut x = clamp(input + self.rcbp0[ch] * self.rcq, 1.0);
                    hp = clamp(self.rcc * (self.rchp0[ch] + x - self.rclast0[ch]), 1.0);
                    bp = clamp(hp * self.rcb + self.rcbp0[ch] * self.rca, 1.0);
                    self.rclast0[ch] = x;
                    self.rchp0[ch] = hp;
                    self.rcbp0[ch] = bp;
                    x = if self.kind == FilterType::HighpassRc24 {
                        hp + self.rcbp1[ch] * self.rcq
                    } else {
                        bp + self.rcbp1[ch] * self.rcq
                    };
                    x = clamp(x, 1.0);
                    hp = clamp(self.rcc * (self.rchp1[ch] + x - self.rclast1[ch]), 1.0);
                    bp = clamp(hp * self.rcb + self.rcbp1[ch] * self.rca, 1.0);
                    self.rclast1[ch] = x;
                    self.rchp1[ch] = hp;
                    self.rcbp1[ch] = bp;
                }
                Some(if self.kind == FilterType::HighpassRc24 { hp } else { bp })
            }
            FilterType::Formant | FilterType::FastFormant => {
                // Too small to matter: nothing to compute
                if input.abs() < F_EPSILON && self.vflast[0][ch].abs() < F_EPSILON {
                    return Some(0.0);
                }
                let os = if self.kind == FilterType::FastFormant { 1 } else { 4 };
                let mut result = 0.0f32;
                for _ in 0..os {
                    for formant in 0..2 {
                        // Three band-passes in a row, at the formant's frequency
                        let mut x = input;
                        for (stage, index) in [formant, formant + 2, formant + 4].into_iter().enumerate() {
                            let feed = if stage == 0 { self.vfbp[0][ch] } else { self.vfbp[index][ch] };
                            x = clamp(x + feed * self.vfq, 1.0);
                            let hp =
                                clamp(self.vfc[formant] * (self.vfhp[index][ch] + x - self.vflast[index][ch]), 1.0);
                            let bp = clamp(hp * self.vfb[formant] + self.vfbp[index][ch] * self.vfa[formant], 1.0);
                            self.vflast[index][ch] = x;
                            self.vfhp[index][ch] = hp;
                            self.vfbp[index][ch] = bp;
                            x = bp;
                        }
                        result += x;
                    }
                }
                Some(if self.kind == FilterType::FastFormant { result * 2.0 } else { result * 0.5 })
            }
            _ => {
                *out = self.biquad.process(input, ch);
                None
            }
        }
    }
}

/// The legacy program's general filter, stereo: one of 22 types; the double types run a second stage, made with
/// the filter, so that choosing any type allocates nothing
#[derive(Clone, Debug)]
pub struct BasicFilter {
    stage: Stage,
    second: Box<Stage>,
    double: bool,
    sample_rate: f32,
    sample_ratio: f32,
}

impl BasicFilter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            stage: Stage::new(),
            second: Box::new(Stage::new()),
            double: false,
            sample_rate,
            sample_ratio: 1.0 / sample_rate,
        }
    }

    pub fn set_filter_type(&mut self, kind: FilterType) {
        self.double = kind == FilterType::DoubleLowPass || kind == FilterType::DoubleMoog;
        if !self.double {
            self.stage.kind = kind;
            return;
        }
        // The double types: the single type in both stages
        self.stage.kind = if kind == FilterType::DoubleLowPass { FilterType::LowPass } else { FilterType::Moog };
        self.second.kind = self.stage.kind;
    }

    pub fn clear_history(&mut self) {
        self.second.clear_history();
        self.stage.clear_history();
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.sample_ratio = 1.0 / sample_rate;
    }

    #[inline]
    pub fn update(&mut self, input: f32, ch: usize) -> f32 {
        let mut out = 0.0;
        if let Some(done) = self.stage.update(input, ch, &mut out) {
            return done;
        }
        if self.double {
            let mut second = 0.0;
            return self.second.update(out, ch, &mut second).unwrap_or(second);
        }
        out
    }

    pub fn calc_filter_coeffs(&mut self, mut freq: f32, q: f32) {
        let q = q.max(MIN_Q);
        let s = &mut self.stage;
        match s.kind {
            FilterType::LowpassRc12
            | FilterType::BandpassRc12
            | FilterType::HighpassRc12
            | FilterType::LowpassRc24
            | FilterType::BandpassRc24
            | FilterType::HighpassRc24 => {
                freq = freq.clamp(50.0, 20000.0);
                let sr = self.sample_ratio * 0.25;
                let f = 1.0 / (freq * 2.0 * PI);
                s.rca = 1.0 - sr / (f + sr);
                s.rcb = 1.0 - s.rca;
                s.rcc = f / (f + sr);
                // Q stretched: self-oscillation starts reliably at about 2.5
                s.rcq = q * 0.25;
            }
            FilterType::Formant | FilterType::FastFormant => {
                freq = freq.clamp(MIN_FREQ, 20000.0);
                let freq_ratio = 4.0f32 / 14000.0;
                s.vfq = q * 0.25;
                // 1 Hz to 14 kHz is a to u, then on to the last a at 17.5 kHz; above it, that a
                let vowelf = (freq * freq_ratio).min(5.0);
                let vowel = (vowelf as i32).min(4) as usize;
                let fract = vowelf - vowel as f32;
                let f0 = 1.0 / (lerp(FORMANTS[vowel][0], FORMANTS[vowel + 1][0], fract) * 2.0 * PI);
                let f1 = 1.0 / (lerp(FORMANTS[vowel][1], FORMANTS[vowel + 1][1], fract) * 2.0 * PI);
                // The rate's coefficient depends on the oversampling
                let sr = if s.kind == FilterType::FastFormant { self.sample_ratio } else { self.sample_ratio * 0.25 };
                s.vfa[0] = 1.0 - sr / (f0 + sr);
                s.vfb[0] = 1.0 - s.vfa[0];
                s.vfc[0] = f0 / (f0 + sr);
                s.vfa[1] = 1.0 - sr / (f1 + sr);
                s.vfb[1] = 1.0 - s.vfa[1];
                s.vfc[1] = f1 / (f1 + sr);
            }
            FilterType::Moog => {
                let f = freq.clamp(MIN_FREQ, 20000.0) * self.sample_ratio;
                // Empirical tuning
                s.p = (3.6 - 3.2 * f) * f;
                s.k = 2.0 * s.p - 1.0;
                s.r = q * expf((1.0 - s.p) * 1.386_249);
                if self.double {
                    self.second.r = s.r;
                    self.second.p = s.p;
                    self.second.k = s.k;
                }
            }
            FilterType::Tripole => {
                let f = freq.clamp(20.0, 20000.0) * self.sample_ratio * 0.25;
                s.p = (3.6 - 3.2 * f) * f;
                s.k = 2.0 * s.p - 1.0;
                s.r = q * 0.1 * expf((1.0 - s.p) * 1.386_249);
            }
            FilterType::LowpassSv | FilterType::BandpassSv | FilterType::HighpassSv | FilterType::NotchSv => {
                let f = sinf(MIN_FREQ.max(freq) * self.sample_ratio * PI);
                s.svf1 = f.min(0.825);
                s.svf2 = (f * 2.0).min(0.825);
                s.svq = 0.0001f32.max(2.0 - q * 0.1995);
            }
            kind => {
                freq = freq.clamp(MIN_FREQ, 20000.0);
                let omega = 2.0 * PI * freq * self.sample_ratio;
                let tsin = sinf(omega) * 0.5;
                let tcos = cosf(omega);
                let alpha = tsin / q;
                let a0 = 1.0 / (1.0 + alpha);
                let a1 = -2.0 * tcos * a0;
                let a2 = (1.0 - alpha) * a0;
                match kind {
                    FilterType::LowPass => {
                        let b1 = (1.0 - tcos) * a0;
                        let b0 = b1 * 0.5;
                        s.biquad.set_coefficients(a1, a2, b0, b1, b0);
                    }
                    FilterType::HiPass => {
                        let b1 = (-1.0 - tcos) * a0;
                        let b0 = b1 * -0.5;
                        s.biquad.set_coefficients(a1, a2, b0, b1, b0);
                    }
                    FilterType::BandPassCsg => {
                        let b0 = tsin * a0;
                        s.biquad.set_coefficients(a1, a2, b0, 0.0, -b0);
                    }
                    FilterType::BandPassCzpg => {
                        let b0 = alpha * a0;
                        s.biquad.set_coefficients(a1, a2, b0, 0.0, -b0);
                    }
                    FilterType::Notch => s.biquad.set_coefficients(a1, a2, a0, a1, a0),
                    FilterType::AllPass => s.biquad.set_coefficients(a1, a2, a2, a1, 1.0),
                    _ => {}
                }
                if self.double {
                    self.second.biquad.copy_coefficients(&s.biquad);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_is_finite_with_a_loud_input() {
        for kind in FilterType::ALL {
            let mut f = BasicFilter::new(48000.0);
            f.set_filter_type(kind);
            f.calc_filter_coeffs(800.0, 4.0);
            let mut peak = 0.0f32;
            for n in 0..4800 {
                let x = if n % 50 < 25 { 0.9 } else { -0.9 };
                let y = f.update(x, 0);
                assert!(y.is_finite(), "{kind:?}");
                peak = peak.max(y.abs());
            }
            assert!(peak > 0.0, "{kind:?}");
        }
    }

    #[test]
    fn keys_round_trip() {
        for kind in FilterType::ALL {
            assert_eq!(FilterType::from_key(kind.key()), Some(kind));
        }
    }
}
