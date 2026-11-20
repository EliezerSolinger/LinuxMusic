//! The legacy program's small effects: Amplifier, Stereo Matrix, Stereo Enhancer and Bass Booster (with the
//! `DspEffectLibrary` pieces they are made of). Each gives the wet signal: dry and wet are the host's.

use crate::math::{f32c::PI, sinf};
use crate::oscillator::lerp;

/// A knob over one call: where it was and where it goes. The legacy program's sample-exact knobs ramped linearly
/// over the period in which they changed (`ValueBuffer::interpolate`); a knob that did not change is the same at
/// both ends
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ramp {
    pub from: f32,
    pub to: f32,
}

impl Ramp {
    pub fn constant(value: f32) -> Self {
        Self { from: value, to: value }
    }

    /// Its value at frame @p f of a call of @p length frames
    #[inline]
    pub fn at(&self, f: usize, length: usize) -> f32 {
        if self.from == self.to { self.to } else { lerp(self.from, self.to, f as f32 / length as f32) }
    }
}

/// The Amplifier's knobs, in % (sample-exact)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmplifierSettings {
    /// 0 to 200
    pub volume: Ramp,
    /// -100 to 100
    pub pan: Ramp,
    /// 0 to 200
    pub left: Ramp,
    pub right: Ramp,
}

impl Default for AmplifierSettings {
    fn default() -> Self {
        Self {
            volume: Ramp::constant(100.0),
            pan: Ramp::constant(0.0),
            left: Ramp::constant(100.0),
            right: Ramp::constant(100.0),
        }
    }
}

/// The Amplifier: volume, pan and each side's gain
pub fn amplifier(frames: &mut [[f32; 2]], s: &AmplifierSettings) {
    let n = frames.len();
    for (f, frame) in frames.iter_mut().enumerate() {
        let volume = s.volume.at(f, n) * 0.01;
        let pan = s.pan.at(f, n) * 0.01;
        let left = s.left.at(f, n) * 0.01;
        let right = s.right.at(f, n) * 0.01;
        let pan_left = (1.0 - pan).min(1.0);
        let pan_right = (1.0 + pan).min(1.0);
        frame[0] = frame[0] * (left * pan_left) * volume;
        frame[1] = frame[1] * (right * pan_right) * volume;
    }
}

/// The Stereo Matrix's knobs, -1 to 1: how much of each side goes to each side
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StereoMatrixSettings {
    pub left_to_left: f32,
    pub left_to_right: f32,
    pub right_to_left: f32,
    pub right_to_right: f32,
}

impl Default for StereoMatrixSettings {
    fn default() -> Self {
        Self { left_to_left: 1.0, left_to_right: 0.0, right_to_left: 0.0, right_to_right: 1.0 }
    }
}

/// The Stereo Matrix
pub fn stereo_matrix(frames: &mut [[f32; 2]], s: &StereoMatrixSettings) {
    for frame in frames.iter_mut() {
        let (l, r) = (frame[0], frame[1]);
        frame[0] = s.left_to_left * l + s.right_to_left * r;
        frame[1] = s.left_to_right * l + s.right_to_right * r;
    }
}

/// The frames the Stereo Enhancer delays its right side by, at most
pub const ENHANCER_BUFFER: usize = 256;

/// The Stereo Enhancer: the right side delayed by the width (in frames), then each side mixed with the other by
/// the sine of half the width in degrees
#[derive(Clone, Debug)]
pub struct StereoEnhancer {
    delay: Box<[[f32; 2]; ENHANCER_BUFFER]>,
    current: usize,
    width: f32,
}

impl Default for StereoEnhancer {
    fn default() -> Self {
        Self { delay: Box::new([[0.0; 2]; ENHANCER_BUFFER]), current: 0, width: 0.0 }
    }
}

impl StereoEnhancer {
    pub fn reset(&mut self) {
        self.delay.fill([0.0; 2]);
        self.current = 0;
    }

    /// @p width: 0 to 180
    pub fn process(&mut self, frames: &mut [[f32; 2]], width: f32) {
        self.width = width;
        let to_rad = PI / 180.0;
        for frame in frames.iter_mut() {
            self.delay[self.current] = *frame;
            // (truncated toward zero, as the legacy program did; then wrapped around the ring, for a width outside
            // 0 to 180 too: the legacy program added the ring's length once, enough only within its range)
            let index = ((self.current as f32 - self.width) as i32).rem_euclid(ENHANCER_BUFFER as i32);
            let (mut l, mut r) = (frame[0], self.delay[index as usize][1]);
            let tmp = l;
            l += r * sinf(self.width * to_rad * 0.5);
            r -= tmp * sinf(self.width * to_rad * 0.5);
            *frame = [l, r];
            self.current = (self.current + 1) % ENHANCER_BUFFER;
        }
    }
}

/// A fast bass boost: a one-pole low-pass added back, then a gain (`DspEffectLibrary::FastBassBoost`)
#[derive(Clone, Copy, Debug)]
pub struct FastBassBoost {
    frequency: f32,
    gain1: f32,
    gain2: f32,
    ratio: f32,
    cap: f32,
}

impl FastBassBoost {
    pub fn new(frequency: f32, gain: f32, ratio: f32) -> Self {
        let frequency = frequency.max(10.0);
        Self { frequency, gain1: (1.0 / (f64::from(frequency) + 1.0)) as f32, gain2: gain, ratio, cap: 0.0 }
    }

    #[inline]
    pub fn next_sample(&mut self, input: f32) -> f32 {
        self.cap = (input + self.cap * self.frequency) * self.gain1;
        (input + self.cap * self.ratio) * self.gain2
    }

    pub fn clear_state(&mut self) {
        self.cap = 0.0;
    }

    pub fn set_frequency(&mut self, frequency: f32) {
        self.frequency = frequency;
        self.gain1 = (1.0 / (f64::from(frequency) + 1.0)) as f32;
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.gain2 = gain;
    }

    pub fn set_ratio(&mut self, ratio: f32) {
        self.ratio = ratio;
    }
}

/// The Bass Booster's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BassBoosterSettings {
    /// Hz, 50 to 200 (at 44.1 kHz; scaled with the rate)
    pub frequency: f32,
    /// 0.1 to 5 (sample-exact)
    pub gain: Ramp,
    /// 0.1 to 10
    pub ratio: f32,
}

impl Default for BassBoosterSettings {
    fn default() -> Self {
        Self { frequency: 100.0, gain: Ramp::constant(1.0), ratio: 2.0 }
    }
}

/// The Bass Booster: a fast bass boost on each side
#[derive(Clone, Debug)]
pub struct BassBooster {
    sample_rate: f32,
    fx: [FastBassBoost; 2],
}

impl BassBooster {
    pub fn new(sample_rate: f32) -> Self {
        let fx = FastBassBoost::new(70.0, 1.0, 2.8);
        Self { sample_rate, fx: [fx, fx] }
    }

    pub fn reset(&mut self) {
        for fx in &mut self.fx {
            fx.clear_state();
        }
    }

    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &BassBoosterSettings) {
        let fac = self.sample_rate / 44100.0;
        let n = frames.len();
        for fx in &mut self.fx {
            fx.set_frequency(s.frequency * fac);
            fx.set_ratio(s.ratio);
        }
        for (f, frame) in frames.iter_mut().enumerate() {
            let gain = s.gain.at(f, n);
            for (ch, fx) in self.fx.iter_mut().enumerate() {
                fx.set_gain(gain);
                frame[ch] = fx.next_sample(frame[ch]);
            }
        }
    }
}
