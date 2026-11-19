//! The legacy program's Delay effect: a stereo delay line with feedback, its length swept by a sine LFO (legacy
//! `plugins/Delay`: `StereoDelay`, `Lfo` and `DelayEffect::process`).

use crate::math::{dbfs_to_amp, f32c, sin};

/// The longest delay, s
pub const MAX_TIME: f32 = 20.0;

/// A stereo delay line with feedback, of up to [`MAX_TIME`]
#[derive(Clone, Debug)]
pub struct StereoDelay {
    buffer: Vec<[f32; 2]>,
    /// Frames, the delay now
    length: f32,
    feedback: f32,
    write_index: usize,
}

impl StereoDelay {
    /// Prepared for @p sample_rate: allocates the whole line
    pub fn new(sample_rate: f32) -> Self {
        let frames = ((sample_rate * MAX_TIME) as usize).max(1);
        Self { buffer: vec![[0.0; 2]; frames], length: frames as f32, feedback: 0.0, write_index: 0 }
    }

    /// The delay in frames; ignored outside 0 to the line's length
    pub fn set_length(&mut self, length: f32) {
        if length <= self.buffer.len() as f32 && length >= 0.0 {
            self.length = length;
        }
    }

    pub fn set_feedback(&mut self, feedback: f32) {
        self.feedback = feedback;
    }

    /// One frame: what was written `length` frames ago comes out, and the input plus the feedback goes in
    #[inline]
    pub fn tick(&mut self, frame: &mut [f32; 2]) {
        let max = self.buffer.len();
        self.write_index = (self.write_index + 1) % max;
        let mut read = self.write_index as i64 - self.length as i64;
        if read < 0 {
            read += max as i64;
        }
        let out = self.buffer[read as usize];
        self.buffer[self.write_index] = [frame[0] + out[0] * self.feedback, frame[1] + out[1] * self.feedback];
        *frame = out;
    }

    pub fn clear(&mut self) {
        self.buffer.fill([0.0; 2]);
        self.write_index = 0;
    }
}

/// A sine LFO
#[derive(Clone, Debug)]
pub struct Lfo {
    /// Hz; NaN until set
    frequency: f64,
    phase: f64,
    increment: f64,
    sample_rate: f32,
    two_pi_over_sr: f64,
}

impl Lfo {
    pub fn new(sample_rate: f32) -> Self {
        // (The legacy program left the frequency, the phase and the increment unset until the first frequency and
        // a reset: here they start at the wave's start)
        Self {
            frequency: f64::NAN,
            phase: 0.0,
            increment: 0.0,
            sample_rate,
            two_pi_over_sr: f64::from(2.0 * f32c::PI / sample_rate),
        }
    }

    /// Ignored below 0 and above Nyquist
    pub fn set_frequency(&mut self, frequency: f64) {
        if frequency < 0.0 || frequency > f64::from(self.sample_rate) / 2.0 || frequency == self.frequency {
            return;
        }
        self.frequency = frequency;
        self.increment = self.frequency * self.two_pi_over_sr;
        self.phase %= f64::from(2.0 * f32c::PI);
    }

    /// From the start of its wave
    pub fn restart(&mut self) {
        self.phase = 0.0;
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        let output = sin(self.phase) as f32;
        self.phase += self.increment;
        output
    }
}

/// The Delay's controls
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DelaySettings {
    /// s, 0.01 to 5
    pub time: f32,
    /// 0 to 1
    pub feedback: f32,
    /// s, the LFO's period, 0.01 to 5
    pub lfo_time: f32,
    /// s of sweep, 0 to 0.5
    pub lfo_amount: f32,
    /// dB, -60 to +20
    pub out_gain: f32,
}

impl Default for DelaySettings {
    fn default() -> Self {
        Self { time: 0.5, feedback: 0.0, lfo_time: 2.0, lfo_amount: 0.0, out_gain: 0.0 }
    }
}

/// The Delay: the delay line swept by the LFO, and its output gain. The wet signal only: dry and wet are mixed by the
/// host, as for every effect
#[derive(Clone, Debug)]
pub struct Delay {
    sample_rate: f32,
    line: StereoDelay,
    lfo: Lfo,
}

impl Delay {
    pub fn new(sample_rate: f32) -> Self {
        Self { sample_rate, line: StereoDelay::new(sample_rate), lfo: Lfo::new(sample_rate) }
    }

    pub fn reset(&mut self) {
        self.line.clear();
        self.lfo.restart();
    }

    /// Delays @p frames in place; returns the wet signal's peak, for the meters
    pub fn process(&mut self, frames: &mut [[f32; 2]], settings: &DelaySettings) -> [f32; 2] {
        let sr = self.sample_rate;
        let amplitude = settings.lfo_amount * sr;
        // (A double kept in a float, as the legacy program did)
        let lfo_frequency = f64::from((1.0 / f64::from(settings.lfo_time)) as f32);
        let out_gain = dbfs_to_amp(settings.out_gain);
        let length = (settings.time * sr) as i32;
        let mut peak = [0.0f32; 2];
        for frame in frames.iter_mut() {
            self.line.set_feedback(settings.feedback);
            self.lfo.set_frequency(lfo_frequency);
            self.line.set_length(length as f32 + amplitude * self.lfo.tick());
            self.line.tick(frame);
            frame[0] *= out_gain;
            frame[1] *= out_gain;
            peak = [peak[0].max(frame[0].abs()), peak[1].max(frame[1].abs())];
        }
        peak
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_impulse_comes_back_after_the_delay_and_fades() {
        let mut delay = Delay::new(1000.0);
        let settings = DelaySettings { time: 0.1, feedback: 0.5, ..DelaySettings::default() };
        let mut frames = vec![[0.0f32; 2]; 400];
        frames[0] = [1.0, 1.0];
        delay.process(&mut frames, &settings);
        assert_eq!(frames[100], [1.0, 1.0]);
        assert_eq!(frames[200], [0.5, 0.5]);
        assert_eq!(frames[300], [0.25, 0.25]);
    }
}
