//! Loudness and true peak by ITU-R BS.1770-4 and EBU R 128 (EBU Tech 3341 for the meter, Tech 3342 for the
//! loudness range): the master section's meter, and the export's report, so that both read the same.
//!
//! - **K-weighting:** the high shelf and the high pass of BS.1770, designed for the sample rate (the coefficients at
//!   48 kHz are the standard's);
//! - **momentary** (400 ms) and **short-term** (3 s) loudness, kept from 100 ms sub-blocks (the 75 % overlap of the
//!   400 ms gating blocks);
//! - **integrated** loudness: the gating blocks above the absolute gate (−70 LUFS), then above the relative gate (10
//!   LU under their mean); the blocks go into a histogram of fixed size, so a session of any length takes no memory;
//! - **loudness range** (LRA, Tech 3342): the short-term values every 100 ms above −70 LUFS and above 20 LU under
//!   their mean, from the 10th to the 95th percentile;
//! - **true peak:** 4× oversampling through the standard's 48-tap polyphase FIR (Annex 2) below 96 kHz, 2× below
//!   192 kHz, the samples themselves above; the highest since the reset.
//!
//! `new(rate)` allocates everything; `process` never allocates.

use libm::{log10, tan};

/// Loudness below which a block counts as silence (the absolute gate), LUFS
pub const ABSOLUTE_GATE: f64 = -70.0;
/// The relative gate of the integrated loudness, LU under the mean of the blocks above the absolute gate
pub const RELATIVE_GATE: f64 = -10.0;
/// The relative gate of the loudness range, LU
pub const LRA_RELATIVE_GATE: f64 = -20.0;

/// The histogram's span and resolution (Tech 3341 asks ±0.1 LU)
const HIST_LOW: f64 = ABSOLUTE_GATE;
const HIST_HIGH: f64 = 10.0;
const HIST_STEP: f64 = 0.01;
const HIST_BINS: usize = ((HIST_HIGH - HIST_LOW) / HIST_STEP) as usize;
/// Sub-blocks of 100 ms: 4 make the momentary window, 30 the short-term one
const MOMENTARY_SUBS: usize = 4;
const SHORT_SUBS: usize = 30;

/// The true-peak interpolator of BS.1770-4, Annex 2: 4 phases of 12 taps
const TP_TAPS: [[f64; 12]; 4] = [
    [
        0.0017089843750,
        0.0109863281250,
        -0.0196533203125,
        0.0332031250000,
        -0.0594482421875,
        0.1373291015625,
        0.9721679687500,
        -0.1022949218750,
        0.0476074218750,
        -0.0266113281250,
        0.0148925781250,
        -0.0083007812500,
    ],
    [
        -0.0291748046875,
        0.0292968750000,
        -0.0517578125000,
        0.0891113281250,
        -0.1665039062500,
        0.4650878906250,
        0.7797851562500,
        -0.2003173828125,
        0.1015625000000,
        -0.0582275390625,
        0.0330810546875,
        -0.0189208984375,
    ],
    [
        -0.0189208984375,
        0.0330810546875,
        -0.0582275390625,
        0.1015625000000,
        -0.2003173828125,
        0.7797851562500,
        0.4650878906250,
        -0.1665039062500,
        0.0891113281250,
        -0.0517578125000,
        0.0292968750000,
        -0.0291748046875,
    ],
    [
        -0.0083007812500,
        0.0148925781250,
        -0.0266113281250,
        0.0476074218750,
        -0.1022949218750,
        0.9721679687500,
        0.1373291015625,
        -0.0594482421875,
        0.0332031250000,
        -0.0196533203125,
        0.0109863281250,
        0.0017089843750,
    ],
];

/// A biquad, direct form I, in f64 (the energies are summed over minutes)
#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// The K-weighting of BS.1770 for a sample rate: the shelf (stage 1) and the high pass (stage 2)
fn k_weighting(rate: f64) -> (Biquad, Biquad) {
    let pi = core::f64::consts::PI;
    // stage 1: the high shelf, +4 dB above ~1.5 kHz
    let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
    let k = tan(pi * f0 / rate);
    let vh = libm::pow(10.0, g / 20.0);
    let vb = libm::pow(vh, 0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        ..Biquad::default()
    };
    // stage 2: the high pass at ~38 Hz
    let (f0, q) = (38.13547087602444, 0.5003270373238773);
    let k = tan(pi * f0 / rate);
    let a0 = 1.0 + k / q + k * k;
    let high_pass =
        Biquad { b: [1.0, -2.0, 1.0], a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0], ..Biquad::default() };
    (shelf, high_pass)
}

/// Loudness of a mean square (the channels' sum), LUFS
fn lufs(mean_square: f64) -> f64 {
    if mean_square <= 0.0 { f64::NEG_INFINITY } else { -0.691 + 10.0 * log10(mean_square) }
}

fn bin_of(l: f64) -> Option<usize> {
    if l <= HIST_LOW || !l.is_finite() {
        return None;
    }
    Some((((l - HIST_LOW) / HIST_STEP) as usize).min(HIST_BINS - 1))
}

fn bin_loudness(i: usize) -> f64 {
    HIST_LOW + (i as f64 + 0.5) * HIST_STEP
}

/// The gated blocks of a measure: their count and energy per loudness bin
#[derive(Clone, Debug)]
struct Histogram {
    count: Vec<u64>,
    energy: Vec<f64>,
    total_count: u64,
    total_energy: f64,
}

impl Histogram {
    fn new() -> Self {
        Self { count: vec![0; HIST_BINS], energy: vec![0.0; HIST_BINS], total_count: 0, total_energy: 0.0 }
    }

    fn add(&mut self, mean_square: f64) {
        if let Some(i) = bin_of(lufs(mean_square)) {
            self.count[i] += 1;
            self.energy[i] += mean_square;
            self.total_count += 1;
            self.total_energy += mean_square;
        }
    }

    fn clear(&mut self) {
        self.count.fill(0);
        self.energy.fill(0.0);
        self.total_count = 0;
        self.total_energy = 0.0;
    }

    /// The first bin above the relative gate (`offset` LU under the mean of everything above the absolute gate)
    fn relative_start(&self, offset: f64) -> Option<usize> {
        if self.total_count == 0 {
            return None;
        }
        let gate = lufs(self.total_energy / self.total_count as f64) + offset;
        Some(if gate <= HIST_LOW { 0 } else { (((gate - HIST_LOW) / HIST_STEP) as usize).min(HIST_BINS) })
    }
}

/// The meter: feed it the left and right channels as they play
#[derive(Clone, Debug)]
pub struct Loudness {
    rate: f64,
    filters: [(Biquad, Biquad); 2],
    /// samples per 100 ms sub-block, and how far the current one is
    sub_len: usize,
    sub_pos: usize,
    sub_energy: f64,
    /// the last sub-blocks' summed energies (a ring of 30)
    subs: [f64; SHORT_SUBS],
    sub_count: usize,
    sub_next: usize,
    integrated: Histogram,
    range: Histogram,
    /// the true-peak interpolator's history per channel, its factor, and the highest absolute value since the reset
    tp_history: [[f64; 12]; 2],
    tp_factor: usize,
    true_peak: f64,
}

impl Loudness {
    pub fn new(rate: u32) -> Self {
        let rate = f64::from(rate.max(8000));
        let k = k_weighting(rate);
        Self {
            rate,
            filters: [k, k],
            sub_len: ((rate / 10.0).round() as usize).max(1),
            sub_pos: 0,
            sub_energy: 0.0,
            subs: [0.0; SHORT_SUBS],
            sub_count: 0,
            sub_next: 0,
            integrated: Histogram::new(),
            range: Histogram::new(),
            tp_history: [[0.0; 12]; 2],
            tp_factor: if rate < 96_000.0 {
                4
            } else if rate < 192_000.0 {
                2
            } else {
                1
            },
            true_peak: 0.0,
        }
    }

    /// The rate it was made for
    pub fn rate(&self) -> u32 {
        self.rate as u32
    }

    /// Starts the measure again: the integrated loudness, the range and the true-peak hold
    pub fn reset(&mut self) {
        let k = k_weighting(self.rate);
        self.filters = [k, k];
        self.sub_pos = 0;
        self.sub_energy = 0.0;
        self.subs = [0.0; SHORT_SUBS];
        self.sub_count = 0;
        self.sub_next = 0;
        self.integrated.clear();
        self.range.clear();
        self.tp_history = [[0.0; 12]; 2];
        self.true_peak = 0.0;
    }

    /// The next samples of the two channels (mono: the same slice twice is not mono by BS.1770, which sums the
    /// channels; give the mono signal as `l` and silence as `r`)
    pub fn process(&mut self, l: &[f32], r: &[f32]) {
        let n = l.len().min(r.len());
        for i in 0..n {
            let (x0, x1) = (f64::from(l[i]), f64::from(r[i]));
            self.peak(0, x0);
            self.peak(1, x1);
            let (s0, h0) = &mut self.filters[0];
            let y0 = h0.run(s0.run(x0));
            let (s1, h1) = &mut self.filters[1];
            let y1 = h1.run(s1.run(x1));
            self.sub_energy += y0 * y0 + y1 * y1;
            self.sub_pos += 1;
            if self.sub_pos == self.sub_len {
                self.end_sub_block();
            }
        }
    }

    fn peak(&mut self, ch: usize, x: f64) {
        let h = &mut self.tp_history[ch];
        h.copy_within(0..11, 1);
        h[0] = x;
        let mut top = self.true_peak;
        match self.tp_factor {
            4 => {
                for taps in &TP_TAPS {
                    let y: f64 = taps.iter().zip(h.iter()).map(|(t, s)| t * s).sum();
                    top = top.max(y.abs());
                }
            }
            2 => {
                for taps in [&TP_TAPS[0], &TP_TAPS[2]] {
                    let y: f64 = taps.iter().zip(h.iter()).map(|(t, s)| t * s).sum();
                    top = top.max(y.abs());
                }
            }
            _ => top = top.max(x.abs()),
        }
        self.true_peak = top;
    }

    fn end_sub_block(&mut self) {
        self.subs[self.sub_next] = self.sub_energy / self.sub_len as f64;
        self.sub_next = (self.sub_next + 1) % SHORT_SUBS;
        self.sub_count += 1;
        self.sub_energy = 0.0;
        self.sub_pos = 0;
        // every 100 ms: a 400 ms gating block (75 % overlap), and a short-term value for the range
        if self.sub_count >= MOMENTARY_SUBS {
            self.integrated.add(self.window(MOMENTARY_SUBS));
        }
        if self.sub_count >= SHORT_SUBS {
            self.range.add(self.window(SHORT_SUBS));
        }
    }

    /// The mean square of the last `subs` sub-blocks
    fn window(&self, subs: usize) -> f64 {
        let mut sum = 0.0;
        for k in 1..=subs {
            sum += self.subs[(self.sub_next + SHORT_SUBS - k) % SHORT_SUBS];
        }
        sum / subs as f64
    }

    /// Momentary loudness (400 ms), LUFS; −∞ before 400 ms have passed
    pub fn momentary(&self) -> f64 {
        if self.sub_count < MOMENTARY_SUBS { f64::NEG_INFINITY } else { lufs(self.window(MOMENTARY_SUBS)) }
    }

    /// Short-term loudness (3 s), LUFS; −∞ before 3 s have passed
    pub fn short_term(&self) -> f64 {
        if self.sub_count < SHORT_SUBS { f64::NEG_INFINITY } else { lufs(self.window(SHORT_SUBS)) }
    }

    /// Integrated loudness since the reset, LUFS; −∞ while no block has passed the gates
    pub fn integrated(&self) -> f64 {
        let h = &self.integrated;
        let Some(start) = h.relative_start(RELATIVE_GATE) else { return f64::NEG_INFINITY };
        let (count, energy) = (start..HIST_BINS).fold((0u64, 0.0), |(c, e), i| (c + h.count[i], e + h.energy[i]));
        if count == 0 { f64::NEG_INFINITY } else { lufs(energy / count as f64) }
    }

    /// Loudness range since the reset (Tech 3342), LU; 0 while there is too little
    pub fn range(&self) -> f64 {
        let h = &self.range;
        let Some(start) = h.relative_start(LRA_RELATIVE_GATE) else { return 0.0 };
        let total: u64 = h.count[start..].iter().sum();
        if total == 0 {
            return 0.0;
        }
        let percentile = |p: f64| {
            let want = (p * total as f64).ceil().max(1.0) as u64;
            let mut seen = 0;
            for i in start..HIST_BINS {
                seen += h.count[i];
                if seen >= want {
                    return bin_loudness(i);
                }
            }
            bin_loudness(HIST_BINS - 1)
        };
        (percentile(0.95) - percentile(0.10)).max(0.0)
    }

    /// The highest true peak since the reset, dBTP; −∞ in silence
    pub fn true_peak(&self) -> f64 {
        if self.true_peak <= 0.0 { f64::NEG_INFINITY } else { 20.0 * log10(self.true_peak) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// A stereo sine of `dbfs` peak in both channels, for `seconds`
    fn sine(m: &mut Loudness, freq: f64, dbfs: f64, seconds: f64, phase: f64) {
        let a = libm::pow(10.0, dbfs / 20.0);
        let n = (seconds * f64::from(m.rate())) as usize;
        let mut buf = vec![0.0f32; 4800];
        let mut t = 0usize;
        while t < n {
            let len = buf.len().min(n - t);
            for (i, x) in buf[..len].iter_mut().enumerate() {
                let s = (t + i) as f64 / f64::from(m.rate());
                *x = (a * libm::sin(2.0 * core::f64::consts::PI * freq * s + phase)) as f32;
            }
            m.process(&buf[..len], &buf[..len]);
            t += len;
        }
    }

    fn near(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    /// EBU Tech 3341, cases 1 and 2: a 1 kHz stereo sine at −23 and −33 dBFS reads −23 and −33 LUFS (M, S, I)
    #[test]
    fn a_sine_reads_its_level() {
        for level in [-23.0, -33.0] {
            let mut m = Loudness::new(RATE);
            sine(&mut m, 1000.0, level, 20.0, 0.0);
            assert!(near(m.momentary(), level, 0.1), "M {}", m.momentary());
            assert!(near(m.short_term(), level, 0.1), "S {}", m.short_term());
            assert!(near(m.integrated(), level, 0.1), "I {}", m.integrated());
        }
    }

    /// Tech 3341, cases 3 and 4: the gates leave the quiet parts out of the integrated loudness
    #[test]
    fn the_gates() {
        let mut m = Loudness::new(RATE);
        for (level, s) in [(-36.0, 10.0), (-23.0, 60.0), (-36.0, 10.0)] {
            sine(&mut m, 1000.0, level, s, 0.0);
        }
        assert!(near(m.integrated(), -23.0, 0.1), "case 3: {}", m.integrated());
        let mut m = Loudness::new(RATE);
        for (level, s) in [(-72.0, 10.0), (-36.0, 10.0), (-23.0, 60.0), (-36.0, 10.0), (-72.0, 10.0)] {
            sine(&mut m, 1000.0, level, s, 0.0);
        }
        assert!(near(m.integrated(), -23.0, 0.1), "case 4: {}", m.integrated());
    }

    /// The other rates give the same loudness (the K-weighting is designed for the rate)
    #[test]
    fn other_rates() {
        for rate in [44_100, 96_000] {
            let mut m = Loudness::new(rate);
            sine(&mut m, 1000.0, -23.0, 10.0, 0.0);
            assert!(near(m.integrated(), -23.0, 0.1), "{rate}: {}", m.integrated());
        }
    }

    /// Tech 3342, case 1: −20 then −30 dBFS, 20 s each: a range of 10 LU
    #[test]
    fn the_loudness_range() {
        let mut m = Loudness::new(RATE);
        sine(&mut m, 1000.0, -20.0, 20.0, 0.0);
        sine(&mut m, 1000.0, -30.0, 20.0, 0.0);
        assert!(near(m.range(), 10.0, 1.0), "LRA {}", m.range());
    }

    /// True peak: a sine at a quarter of the rate, sampled 45° off its crests, peaks 3 dB above its samples
    #[test]
    fn the_true_peak_sees_between_the_samples() {
        let mut m = Loudness::new(RATE);
        sine(&mut m, f64::from(RATE) / 4.0, -6.0, 1.0, core::f64::consts::PI / 4.0);
        assert!(near(m.true_peak(), -6.0, 0.6), "TP {}", m.true_peak());
        let mut m = Loudness::new(RATE);
        sine(&mut m, 997.0, -12.0, 1.0, 0.0);
        assert!(near(m.true_peak(), -12.0, 0.2), "TP {}", m.true_peak());
    }

    /// Silence reads −∞ and no range; a reset starts again
    #[test]
    fn silence_and_reset() {
        let mut m = Loudness::new(RATE);
        m.process(&[0.0; 48_000], &[0.0; 48_000]);
        assert_eq!(m.integrated(), f64::NEG_INFINITY);
        assert_eq!(m.true_peak(), f64::NEG_INFINITY);
        sine(&mut m, 1000.0, -23.0, 5.0, 0.0);
        assert!(m.integrated().is_finite());
        m.reset();
        assert_eq!(m.integrated(), f64::NEG_INFINITY);
        assert_eq!(m.momentary(), f64::NEG_INFINITY);
    }
}
