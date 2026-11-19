//! The legacy program's Dispersion, Wave Shaper and Bitcrush effects. Each gives the wet signal: dry and wet are the
//! host's.

use crate::basic_filters::LinkwitzRiley;
use crate::math::{cosf, dbfs_to_amp, f32c::PI, sinf};
use crate::oscillator::{NoiseRng, lerp};
use crate::small_effects::Ramp;

/// The most all-pass filters the Dispersion chains
pub const MAX_DISPERSION_FILTERS: usize = 999;

/// The Dispersion's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DispersionSettings {
    /// How many all-passes, 0 to 999
    pub amount: usize,
    /// Hz, 20 to 20000
    pub frequency: f32,
    /// 0.01 to 8
    pub resonance: f32,
    /// -1 to 1
    pub feedback: f32,
    /// DC offset removal
    pub dc: bool,
}

impl Default for DispersionSettings {
    fn default() -> Self {
        Self { amount: 0, frequency: 200.0, resonance: 0.707, feedback: 0.0, dc: false }
    }
}

/// The Dispersion: a chain of second order all-passes, which smears the phases (Lost Robot's)
#[derive(Clone, Debug)]
pub struct Dispersion {
    sample_rate: f32,
    amount: usize,
    x0: Vec<f32>,
    x1: Vec<f32>,
    y0: Vec<f32>,
    y1: Vec<f32>,
    feedback: [f32; 2],
    integrator: [f32; 2],
}

impl Dispersion {
    pub fn new(sample_rate: f32) -> Self {
        let n = MAX_DISPERSION_FILTERS * 2;
        Self {
            sample_rate,
            amount: 0,
            x0: vec![0.0; n],
            x1: vec![0.0; n],
            y0: vec![0.0; n],
            y1: vec![0.0; n],
            feedback: [0.0; 2],
            integrator: [0.0; 2],
        }
    }

    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &DispersionSettings) {
        let amount = s.amount.min(MAX_DISPERSION_FILTERS);
        let mut feedback = s.feedback;
        // The all-passes' coefficients
        let w0 = (2.0 * PI / self.sample_rate) * s.frequency;
        let a0 = 1.0 + sinf(w0) / (s.resonance * 2.0);
        let ap1 = (1.0 - (a0 - 1.0)) / a0;
        let ap2 = (-2.0 * cosf(w0)) / a0;
        let dc_coeff = (0.001 * f64::from(44100.0 / self.sample_rate)) as f32;
        if amount != self.amount {
            if amount < self.amount {
                // The filters no longer used are emptied
                for i in amount * 2..self.amount * 2 {
                    self.x0[i] = 0.0;
                    self.x1[i] = 0.0;
                    self.y0[i] = 0.0;
                    self.y1[i] = 0.0;
                }
            }
            self.amount = amount;
        }
        if amount == 0 {
            feedback = 0.0;
            self.feedback = [0.0; 2];
        }
        for frame in frames.iter_mut() {
            let mut x = [frame[0] + self.feedback[0], frame[1] + self.feedback[1]];
            for i in 0..self.amount * 2 {
                let ch = i % 2;
                let input = x[ch];
                let output = ap1 * (input - self.y1[i]) + ap2 * (self.x0[i] - self.y0[i]) + self.x1[i];
                self.x1[i] = self.x0[i];
                self.x0[i] = input;
                self.y1[i] = self.y0[i];
                self.y0[i] = output;
                x[ch] = output;
            }
            self.feedback = [x[0] * feedback, x[1] * feedback];
            if s.dc {
                // DC offset removal
                for ch in 0..2 {
                    self.integrator[ch] = self.integrator[ch] * (1.0 - dc_coeff) + x[ch] * dc_coeff;
                    x[ch] -= self.integrator[ch];
                }
            }
            *frame = x;
        }
    }
}

/// The Wave Shaper's points: its transfer curve, 200 of them from 0 to full scale
pub type WaveShape = [f32; 200];

/// The Wave Shaper's default curve: a straight line
pub fn default_wave_shape() -> WaveShape {
    core::array::from_fn(|i| (i as f32 + 1.0) / 200.0)
}

/// The Wave Shaper: an input gain, a drawn transfer curve (mirrored for the negative side), an output gain
pub fn wave_shaper(frames: &mut [[f32; 2]], shape: &WaveShape, input: Ramp, output: Ramp, clip: bool) {
    let n = frames.len();
    for (f, frame) in frames.iter_mut().enumerate() {
        let (gain_in, gain_out) = (input.at(f, n), output.at(f, n));
        let mut x = [frame[0] * gain_in, frame[1] * gain_in];
        if clip {
            x = [x[0].clamp(-1.0, 1.0), x[1].clamp(-1.0, 1.0)];
        }
        for v in &mut x {
            let lookup = (v.abs() * 200.0) as i32;
            let position = v.abs() * 200.0;
            let frac = position - position.trunc();
            let sign = if *v < 0.0 { -1.0 } else { 1.0 };
            *v = if lookup < 1 {
                frac * shape[0] * sign
            } else if lookup < 200 {
                lerp(shape[lookup as usize - 1], shape[lookup as usize], frac) * sign
            } else {
                *v * shape[199]
            };
        }
        *frame = [x[0] * gain_out, x[1] * gain_out];
    }
}

const OS_RATE: usize = 5;
const OS_RATIO: f32 = 1.0 / OS_RATE as f32;
const CUTOFF_RATIO: f32 = 0.353_553_391;
const SILENCE_FRAMES: i32 = 10;
const OS_RESAMPLE: [f32; 5] = [
    0.000_149_006_288_396_411_2,
    0.164_597_837_676_399_2,
    0.670_506_312_070_408_8,
    0.164_597_837_676_399_2,
    0.000_149_006_288_396_411_2,
];

/// The Bitcrush's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BitcrushSettings {
    /// dB, -20 to 20
    pub in_gain: f32,
    /// %, 0 to 100
    pub in_noise: f32,
    pub out_gain: f32,
    pub out_clip: f32,
    /// Hz, 20 to 44100
    pub rate: f32,
    /// %, -50 to 50
    pub stereo_diff: f32,
    /// 1 to 256
    pub levels: f32,
    pub rate_enabled: bool,
    pub depth_enabled: bool,
}

impl Default for BitcrushSettings {
    fn default() -> Self {
        Self {
            in_gain: 0.0,
            in_noise: 0.0,
            out_gain: 0.0,
            out_clip: 0.0,
            rate: 44100.0,
            stereo_diff: 0.0,
            levels: 256.0,
            rate_enabled: true,
            depth_enabled: true,
        }
    }
}

/// The Bitcrush (Vesa Kivimäki's): sample rate and bit depth crushing, 5 times oversampled
#[derive(Clone, Debug)]
pub struct Bitcrush {
    sample_rate: f32,
    settings: Option<BitcrushSettings>,
    buffer: Vec<[f32; 2]>,
    filter: LinkwitzRiley,
    bit_counter: [f32; 2],
    rate_coeff: [f32; 2],
    held: [f32; 2],
    levels: i32,
    levels_ratio: f32,
    in_gain: f32,
    out_gain: f32,
    out_clip: f32,
    silence_counter: i32,
    noise: NoiseRng,
}

impl Bitcrush {
    /// Prepared for @p sample_rate and calls of up to @p max_period frames; @p seed is the noise's
    pub fn new(sample_rate: f32, max_period: usize, seed: u64) -> Self {
        let mut filter = LinkwitzRiley::new(sample_rate);
        filter.set_lowpass(sample_rate * (CUTOFF_RATIO * OS_RATIO));
        Self {
            sample_rate,
            settings: None,
            buffer: vec![[0.0; 2]; max_period * OS_RATE],
            filter,
            bit_counter: [0.0; 2],
            rate_coeff: [0.0; 2],
            held: [0.0; 2],
            levels: 256,
            levels_ratio: 1.0 / 256.0,
            in_gain: 1.0,
            out_gain: 1.0,
            out_clip: 1.0,
            silence_counter: 0,
            noise: NoiseRng::new(seed),
        }
    }

    pub fn reset(&mut self) {
        self.filter.clear_history();
        self.bit_counter = [0.0; 2];
        self.held = [0.0; 2];
    }

    /// The rate crushing's counters from zero again, as when the rate changes
    pub fn reset_counters(&mut self) {
        self.bit_counter = [0.0; 2];
    }

    fn depth_crush(&self, x: f32) -> f32 {
        // (C's roundf: halves away from zero)
        (x * self.levels as f32).round() * self.levels_ratio
    }

    /// The input with its noise: -amount to +amount at random (a seeded generator, item 9)
    fn noise(&mut self, amount: f32) -> f32 {
        if amount == 0.0 { 0.0 } else { -amount + (self.noise.sample() + 1.0) * 0.5 * (2.0 * amount) }
    }

    fn crush(&mut self, x: f32, noise: f32, depth: bool) -> f32 {
        let n = self.noise(x * noise);
        let v = x * self.in_gain + n;
        if depth { self.depth_crush(v) } else { v }
    }

    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &BitcrushSettings) {
        let old = self.settings;
        let changed = |f: fn(&BitcrushSettings) -> f32| old.is_none_or(|o| f(&o) != f(s));
        let flag = |f: fn(&BitcrushSettings) -> bool| old.is_none_or(|o| f(&o) != f(s));
        if flag(|s| s.rate_enabled) {
            self.bit_counter = [0.0; 2];
        }
        if changed(|s| s.rate) || changed(|s| s.stereo_diff) {
            let diff = (f64::from(s.stereo_diff) * 0.005 * f64::from(s.rate)) as f32;
            let os_rate = self.sample_rate * OS_RATE as f32;
            self.rate_coeff = [os_rate / (s.rate - diff), os_rate / (s.rate + diff)];
            self.bit_counter = [0.0; 2];
        }
        if changed(|s| s.levels) {
            self.levels = s.levels as i32;
            self.levels_ratio = 1.0 / self.levels as f32;
        }
        if changed(|s| s.in_gain) {
            self.in_gain = dbfs_to_amp(s.in_gain);
        }
        if changed(|s| s.out_gain) {
            self.out_gain = dbfs_to_amp(s.out_gain);
        }
        if changed(|s| s.out_clip) {
            self.out_clip = dbfs_to_amp(s.out_clip);
        }
        self.settings = Some(*s);
        let noise = s.in_noise * 0.01;
        let depth = s.depth_enabled;
        let n = frames.len();

        // Into the oversampled buffer
        for f in 0..n {
            let input = frames[f];
            for o in 0..OS_RATE {
                let at = f * OS_RATE + o;
                if s.rate_enabled {
                    self.buffer[at] = self.held;
                    for ch in 0..2 {
                        self.bit_counter[ch] += 1.0;
                        if self.bit_counter[ch] > self.rate_coeff[ch] {
                            self.bit_counter[ch] -= self.rate_coeff[ch];
                            self.held[ch] = self.crush(input[ch], noise, depth);
                        }
                    }
                } else {
                    // Rate crushing off: oversampled by holding
                    let l = self.crush(input[0], noise, depth);
                    let r = self.crush(input[1], noise, depth);
                    self.buffer[at] = [l, r];
                }
            }
        }

        // Filtered, against aliasing
        for at in 0..n * OS_RATE {
            let v = self.buffer[at];
            if v[0].abs().max(v[1].abs()) >= 1.0e-10 {
                self.silence_counter = 0;
                self.buffer[at] = [self.filter.update(v[0], 0), self.filter.update(v[1], 1)];
            } else if self.silence_counter > SILENCE_FRAMES {
                self.buffer[at] = [0.0; 2];
            } else {
                self.silence_counter += 1;
                self.buffer[at] = [self.filter.update(v[0], 0), self.filter.update(v[1], 1)];
            }
        }

        // Down to the rate again
        for (f, frame) in frames.iter_mut().enumerate() {
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for o in 0..OS_RATE {
                l += self.buffer[f * OS_RATE + o][0] * OS_RESAMPLE[o];
                r += self.buffer[f * OS_RATE + o][1] * OS_RESAMPLE[o];
            }
            *frame = [
                l.clamp(-self.out_clip, self.out_clip) * self.out_gain,
                r.clamp(-self.out_clip, self.out_clip) * self.out_gain,
            ];
        }
    }
}
