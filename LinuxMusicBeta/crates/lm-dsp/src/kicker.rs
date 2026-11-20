//! The legacy program's Kicker: a sine swept from a start to an end frequency, with noise, a click (the phase it
//! starts at), a distortion that can move over the note, and a short attack.

use crate::math::{cos, f32c, f64c, sinf};
use crate::oscillator::{NoiseRng, lerp};

/// The legacy `fastPow`: Martin Ankerl's approximation, the exponent's bits scaled (the Kicker's curves are its)
#[inline]
pub fn fast_pow(a: f64, b: f64) -> f64 {
    let hi = (a.to_bits() >> 32) as i32;
    let hi = (b * f64::from(hi.wrapping_sub(1_072_632_447)) + 1_072_632_447.0) as i32;
    f64::from_bits(u64::from(hi as u32) << 32)
}

/// The legacy `DspEffectLibrary::Distortion`: a soft saturation, harder with the threshold
#[derive(Clone, Copy, Debug)]
pub struct SoftDistortion {
    pub threshold: f32,
    pub gain: f32,
}

impl SoftDistortion {
    #[inline]
    pub fn next(&self, x: f32) -> f32 {
        self.gain * (x * (x.abs() + self.threshold) / (x * x + (self.threshold - 1.0) * x.abs() + 1.0))
    }
}

/// The Kicker's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KickerSettings {
    /// Hz, 5 to 1000 (or the note's, with `start_from_note`)
    pub start_freq: f32,
    /// Hz, 5 to 1000 (or the note's, with `end_to_note`)
    pub end_freq: f32,
    /// ms, 5 to 5000: the note's length
    pub length: f32,
    /// 0 to 100: the distortion at the start
    pub dist_start: f32,
    /// 0 to 100: at the end
    pub dist_end: f32,
    /// 0.1 to 5
    pub gain: f32,
    /// 0.01 to 1: the volume envelope's slope
    pub env_slope: f32,
    /// 0 to 1
    pub noise: f32,
    /// 0 to 1: where in its period the sine starts
    pub click: f32,
    /// 0.001 to 1: the frequency sweep's slope
    pub freq_slope: f32,
    /// ms, 0.1 to 20
    pub attack: f32,
    pub start_from_note: bool,
    pub end_to_note: bool,
}

impl Default for KickerSettings {
    fn default() -> Self {
        Self {
            start_freq: 150.0,
            end_freq: 40.0,
            length: 440.0,
            dist_start: 0.8,
            dist_end: 0.8,
            gain: 1.0,
            env_slope: 0.163,
            noise: 0.0,
            click: 0.4,
            freq_slope: 0.06,
            attack: 1.0,
            start_from_note: true,
            end_to_note: false,
        }
    }
}

/// The release, ms: a short linear fade
pub const RELEASE_MS: f32 = 12.0;

/// One note of the Kicker
#[derive(Clone, Debug)]
pub struct KickerVoice {
    phase: f32,
    start_freq: f32,
    end_freq: f32,
    noise: f32,
    slope: f32,
    env: f32,
    dist_start: f32,
    dist_end: f32,
    has_dist_env: bool,
    /// Frames
    length: f32,
    /// Frames
    attack: f32,
    fx: [SoftDistortion; 2],
    counter: u64,
    freq: f64,
    rng: NoiseRng,
}

impl KickerVoice {
    /// A note of @p note_freq Hz at @p sample_rate; @p seed is its noise's
    pub fn new(s: &KickerSettings, note_freq: f32, sample_rate: f32, seed: u64) -> Self {
        let start = if s.start_from_note { note_freq } else { s.start_freq };
        let end = if s.end_to_note { note_freq } else { s.end_freq };
        Self::from_parts(
            start,
            end,
            s.noise * s.noise,
            s.click * 0.25,
            s.freq_slope,
            s.env_slope,
            s.dist_start,
            s.dist_end,
            s.gain,
            s.length * sample_rate / 1000.0,
            s.attack * sample_rate / 1000.0,
            seed,
        )
    }

    /// From the values the legacy oscillator is built with (the note's frames of length and attack)
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        start: f32,
        end: f32,
        noise: f32,
        offset: f32,
        slope: f32,
        env: f32,
        dist_start: f32,
        dist_end: f32,
        gain: f32,
        length: f32,
        attack: f32,
        seed: u64,
    ) -> Self {
        let fx = SoftDistortion { threshold: dist_start, gain };
        Self {
            phase: offset,
            start_freq: start,
            end_freq: end,
            noise,
            slope,
            env,
            dist_start,
            dist_end,
            has_dist_env: dist_start != dist_end,
            length,
            attack: attack.max(1.0),
            fx: [fx, fx],
            counter: 0,
            freq: f64::from(start),
            rng: NoiseRng::new(seed),
        }
    }

    /// The note's length, frames: it is let go after it
    pub fn length(&self) -> f32 {
        self.length
    }

    /// The next @p out frames
    pub fn render(&mut self, out: &mut [[f32; 2]], sample_rate: f32) {
        for frame in out.iter_mut() {
            let position = self.counter as f32 / self.length;
            let within = (self.counter as f32) < self.length;
            let gain = 1.0 - fast_pow(f64::from(if within { position } else { 1.0 }), f64::from(self.env));
            let noise = if self.noise != 0.0 { f64::from(self.rng.sample()) } else { 0.0 };
            let sine = sinf(self.phase * 2.0 * f32c::PI);
            let s = (f64::from(sine * (1.0 - self.noise)) + noise * gain * gain * f64::from(self.noise)) as f32;
            let v = (f64::from(s) * gain) as f32;
            *frame = [v, v];
            // The distortion moves over the note
            if self.has_dist_env && within {
                let threshold = lerp(self.dist_start, self.dist_end, position);
                self.fx[0].threshold = threshold;
                self.fx[1].threshold = threshold;
            }
            frame[0] = self.fx[0].next(frame[0]);
            frame[1] = self.fx[1].next(frame[1]);
            // The attack: from silence (the click starts the sine off zero)
            if (self.counter as f32) < self.attack {
                let rise = (0.5 - 0.5 * cos(f64c::PI * self.counter as f64 / f64::from(self.attack))) as f32;
                frame[0] *= rise;
                frame[1] *= rise;
            }
            self.phase = (f64::from(self.phase) + self.freq / f64::from(sample_rate)) as f32;
            let change = if within {
                f64::from(self.start_freq - self.end_freq)
                    * (1.0 - fast_pow(f64::from(position), f64::from(self.slope)))
            } else {
                0.0
            };
            self.freq = f64::from(self.end_freq) + change;
            self.counter += 1;
        }
    }
}

/// The release's fade on @p out: from frame @p from (where the release starts in these frames), the release
/// @p done frames in already, over @p release frames in all
pub fn release_fade(out: &mut [[f32; 2]], from: usize, mut done: u64, release: f32) {
    for frame in out.iter_mut().skip(from) {
        let attenuation = if (done as f32) < release { 1.0 - (done as f32 / release) } else { 0.0 };
        frame[0] *= attenuation;
        frame[1] *= attenuation;
        done += 1;
    }
}
