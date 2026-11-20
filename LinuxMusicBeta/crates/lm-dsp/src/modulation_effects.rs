//! The legacy program's Multitap Echo and Flanger. Each gives the wet signal: dry and wet are the host's.

use crate::basic_filters::OnePole;
use crate::math::{dbfs_to_amp, expf, f32c, f64c, sin};
use crate::oscillator::NoiseRng;

/// The most taps
pub const MAX_STEPS: usize = 32;
/// The most low-pass stages on each tap
pub const MAX_STAGES: usize = 4;

/// The Multitap Echo's knobs and graphs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MultitapSettings {
    /// 4 to 32
    pub steps: usize,
    /// ms, 1 to 500
    pub step_length: f32,
    /// dB, -80 to 20
    pub dry_gain: f32,
    pub swap_inputs: bool,
    /// 1 to 4
    pub stages: usize,
    /// Each tap's level, dB (-60 to 0)
    pub amp: [f32; MAX_STEPS],
    /// Each tap's low-pass, as log10(f / 20 Hz) (0 to 3)
    pub lp: [f32; MAX_STEPS],
}

impl Default for MultitapSettings {
    fn default() -> Self {
        Self {
            steps: 16,
            step_length: 100.0,
            dry_gain: 0.0,
            swap_inputs: false,
            stages: 1,
            amp: [0.0; MAX_STEPS],
            lp: [3.0; MAX_STEPS],
        }
    }
}

/// The Multitap Echo (Vesa Kivimäki's): each tap a copy of the input, low-passed, at its level, one step later
/// than the last, into a ring the length of the longest echo
#[derive(Clone, Debug)]
pub struct MultitapEcho {
    sample_rate: f32,
    sample_ratio: f32,
    /// The frames a call writes and takes (the legacy program's period)
    period: usize,
    ring: Vec<[f32; 2]>,
    position: usize,
    filters: [[OnePole; MAX_STAGES]; MAX_STEPS],
    work: Vec<[f32; 2]>,
}

impl MultitapEcho {
    /// The ring's length, ms
    const RING_MS: f32 = 16100.0;

    /// Prepared for @p sample_rate, called with @p period frames each time
    pub fn new(sample_rate: f32, period: usize) -> Self {
        let size = Self::ms_to_frames(Self::RING_MS, sample_rate) + period;
        Self {
            sample_rate,
            sample_ratio: 1.0 / sample_rate,
            period,
            ring: vec![[0.0; 2]; size],
            position: 0,
            filters: core::array::from_fn(|_| core::array::from_fn(|_| OnePole::default())),
            work: vec![[0.0; 2]; period],
        }
    }

    fn ms_to_frames(ms: f32, sample_rate: f32) -> usize {
        (ms * sample_rate * 0.001).ceil() as usize
    }

    pub fn reset(&mut self) {
        self.ring.fill([0.0; 2]);
        self.position = 0;
        for stage in &mut self.filters {
            for filter in stage {
                filter.clear_history();
            }
        }
    }

    fn write_adding(&mut self, offset: usize, level: f32, swapped: bool, from_work: bool, input: &[[f32; 2]]) {
        let size = self.ring.len();
        // (offset % size first: a huge offset, from an infinite step length, would overflow the sum)
        let mut pos = (self.position + offset % size) % size;
        for i in 0..self.period.min(input.len()) {
            let src = if from_work { self.work[i] } else { input[i] };
            let (l, r) = if swapped { (src[1], src[0]) } else { (src[0], src[1]) };
            self.ring[pos][0] += l * level;
            self.ring[pos][1] += r * level;
            pos += 1;
            if pos == size {
                pos = 0;
            }
        }
    }

    /// Echoes @p frames (exactly the period's) in place
    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &MultitapSettings) {
        let steps = s.steps.min(MAX_STEPS);
        let stages = s.stages.clamp(1, MAX_STAGES);
        // Each tap's low-pass (20 Hz times 10 to its graph's value)
        for (i, stage) in self.filters.iter_mut().enumerate().take(steps) {
            let freq = 20.0 * expf(f32c::LN_10 * s.lp[i]);
            let b1 = expf(-2.0 * f32c::PI * (freq * self.sample_ratio));
            for filter in stage.iter_mut().take(stages) {
                filter.set_coeffs(1.0 - b1, b1);
            }
        }
        self.write_adding(0, dbfs_to_amp(s.dry_gain), false, false, frames);
        let mut offset_ms = s.step_length;
        for i in 0..steps {
            // (each stage filters the input, as the legacy program did: the last one's is what is heard)
            for stage in 0..stages {
                for (w, x) in self.work.iter_mut().zip(frames.iter()) {
                    *w = [self.filters[i][stage].update(x[0], 0), self.filters[i][stage].update(x[1], 1)];
                }
            }
            let offset = Self::ms_to_frames(offset_ms, self.sample_rate);
            self.write_adding(offset, dbfs_to_amp(s.amp[i]), s.swap_inputs, true, frames);
            offset_ms += s.step_length;
        }
        // What is due now comes out, and its place is emptied
        let size = self.ring.len();
        for (n, frame) in frames.iter_mut().enumerate() {
            let at = (self.position + n) % size;
            *frame = self.ring[at];
            self.ring[at] = [0.0; 2];
        }
        self.position = (self.position + self.period) % size;
    }
}

/// A mono delay line with feedback
#[derive(Clone, Debug)]
pub struct MonoDelay {
    buffer: Vec<f32>,
    length: f32,
    feedback: f32,
    write_index: usize,
}

impl MonoDelay {
    /// Of up to @p max_seconds at @p sample_rate
    pub fn new(max_seconds: f32, sample_rate: f32) -> Self {
        let max = ((sample_rate as i32 as f32 * max_seconds) as usize).max(1);
        Self { buffer: vec![0.0; max], length: max as f32, feedback: 0.0, write_index: 0 }
    }

    pub fn set_length(&mut self, length: f32) {
        if length <= self.buffer.len() as f32 && length >= 0.0 {
            self.length = length;
        }
    }

    pub fn set_feedback(&mut self, feedback: f32) {
        self.feedback = feedback;
    }

    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
    }

    #[inline]
    pub fn tick(&mut self, sample: &mut f32) {
        let max = self.buffer.len();
        self.write_index = (self.write_index + 1) % max;
        let mut read = self.write_index as i64 - self.length as i64;
        if read < 0 {
            read += max as i64;
        }
        let out = self.buffer[read as usize];
        self.buffer[self.write_index] = *sample + out * self.feedback;
        *sample = out;
    }
}

/// Two sines a quarter period (or an offset) apart
#[derive(Clone, Debug)]
pub struct QuadratureLfo {
    frequency: f64,
    phase: f64,
    increment: f64,
    two_pi_over_sr: f64,
    offset: f64,
    sample_rate: i32,
}

impl QuadratureLfo {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate as i32;
        Self {
            frequency: 0.0,
            phase: 0.0,
            increment: 0.0,
            two_pi_over_sr: f64::from(2.0 * f32c::PI / sample_rate as f32),
            offset: f64c::PI * 0.5,
            sample_rate,
        }
    }

    pub fn set_frequency(&mut self, frequency: f64) {
        if frequency < 0.0 || frequency > f64::from(self.sample_rate) / 2.0 || frequency == self.frequency {
            return;
        }
        self.frequency = frequency;
        self.increment = self.frequency * self.two_pi_over_sr;
    }

    pub fn restart(&mut self) {
        self.phase = 0.0;
    }

    pub fn set_offset(&mut self, offset: f64) {
        self.offset = offset;
    }

    #[inline]
    pub fn tick(&mut self) -> (f32, f32) {
        let l = sin(self.phase) as f32;
        let r = sin(self.phase + self.offset) as f32;
        self.phase += self.increment;
        self.phase %= 2.0 * f64c::PI;
        (l, r)
    }
}

/// The Flanger's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlangerSettings {
    /// s, 0.0001 to 0.05
    pub delay: f32,
    /// s: the LFO's period, 0.01 to 60
    pub lfo_period: f32,
    /// s of sweep, 0 to 0.0025
    pub lfo_amount: f32,
    /// Degrees, 0 to 360: the right side's LFO behind the left's
    pub stereo_phase: f32,
    /// -1 to 1
    pub feedback: f32,
    /// 0 to 0.05
    pub noise: f32,
    pub invert_feedback: bool,
}

impl Default for FlangerSettings {
    fn default() -> Self {
        Self {
            delay: 0.001,
            lfo_period: 0.25,
            lfo_amount: 0.0,
            stereo_phase: 90.0,
            feedback: 0.0,
            noise: 0.0,
            invert_feedback: false,
        }
    }
}

/// The Flanger (Dave French's): two short delays swept by a quadrature LFO, with feedback, crossed when inverted
#[derive(Clone, Debug)]
pub struct Flanger {
    sample_rate: f32,
    lfo: QuadratureLfo,
    left: MonoDelay,
    right: MonoDelay,
    noise: NoiseRng,
}

impl Flanger {
    /// Prepared for @p sample_rate; @p seed is its noise's
    pub fn new(sample_rate: f32, seed: u64) -> Self {
        Self {
            sample_rate,
            lfo: QuadratureLfo::new(sample_rate),
            left: MonoDelay::new(1.0, sample_rate),
            right: MonoDelay::new(1.0, sample_rate),
            noise: NoiseRng::new(seed),
        }
    }

    pub fn reset(&mut self) {
        self.left.clear();
        self.right.clear();
        self.lfo.restart();
    }

    /// The LFO from its start (the interface's button)
    pub fn restart_lfo(&mut self) {
        self.lfo.restart();
    }

    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &FlangerSettings) {
        let length = s.delay * self.sample_rate;
        let amplitude = s.lfo_amount * self.sample_rate;
        self.lfo.set_frequency(1.0 / f64::from(s.lfo_period));
        self.lfo.set_offset(f64::from(s.stereo_phase / 180.0) * f64c::PI);
        self.left.set_feedback(s.feedback);
        self.right.set_feedback(s.feedback);
        for frame in frames.iter_mut() {
            if s.noise != 0.0 {
                frame[0] += self.noise.sample() * s.noise;
                frame[1] += self.noise.sample() * s.noise;
            }
            let (l, r) = self.lfo.tick();
            self.left.set_length((f64::from(length) + f64::from(amplitude) * (f64::from(l) + 1.0)) as f32);
            self.right.set_length((f64::from(length) + f64::from(amplitude) * (f64::from(r) + 1.0)) as f32);
            if s.invert_feedback {
                self.left.tick(&mut frame[1]);
                self.right.tick(&mut frame[0]);
            } else {
                self.left.tick(&mut frame[0]);
                self.right.tick(&mut frame[1]);
            }
        }
    }
}
