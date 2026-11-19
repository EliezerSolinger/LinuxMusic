//! Single precision biquads designed with Robert Bristow-Johnson's cookbook, and the legacy `EqFilter`'s way of
//! changing them: two filters, the old and the new, crossfaded over a stretch.

use crate::math::{cosf, exp, f32c::PI, f64c::LN_10, sinf};

/// A stereo biquad in single precision (transposed direct form II), the legacy `StereoBiQuad`
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StereoBiquad {
    a1: f32,
    a2: f32,
    b0: f32,
    b1: f32,
    b2: f32,
    z1: [f32; 2],
    z2: [f32; 2],
}

impl StereoBiquad {
    pub fn set_coefficients(&mut self, a1: f32, a2: f32, b0: f32, b1: f32, b2: f32) {
        self.a1 = a1;
        self.a2 = a2;
        self.b0 = b0;
        self.b1 = b1;
        self.b2 = b2;
    }

    pub fn clear(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
    }

    #[inline]
    pub fn process(&mut self, input: f32, channel: usize) -> f32 {
        let out = self.z1[channel] + self.b0 * input;
        self.z1[channel] = self.b1 * input + self.z2[channel] - self.a1 * out;
        self.z2[channel] = self.b2 * input - self.a2 * out;
        out
    }
}

/// What a cookbook filter is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    HighPass,
    LowPass,
    LowShelf,
    HighShelf,
}

/// The legacy `EqFilter`: a cookbook filter of a shape, whose coefficients are recomputed when its parameters
/// change, the old filter crossfaded into the new over a stretch (the caller's progress, 0 to 1).
#[derive(Clone, Debug)]
pub struct CrossfadedBiquad {
    shape: Shape,
    sample_rate: f32,
    frequency: f32,
    q: f32,
    gain: f32,
    initial: StereoBiquad,
    target: StereoBiquad,
}

impl CrossfadedBiquad {
    pub fn new(shape: Shape) -> Self {
        Self {
            shape,
            sample_rate: 0.0,
            frequency: 0.0,
            q: 0.0,
            gain: 0.0,
            initial: StereoBiquad::default(),
            target: StereoBiquad::default(),
        }
    }

    /// Sets the parameters (@p gain in dB, for the shelves); the coefficients are recomputed if any changed
    pub fn set_parameters(&mut self, sample_rate: f32, frequency: f32, q: f32, gain: f32) {
        if sample_rate != self.sample_rate || frequency != self.frequency || q != self.q || gain != self.gain {
            self.sample_rate = sample_rate;
            self.frequency = frequency;
            self.q = q;
            self.gain = gain;
            self.compute();
        }
    }

    /// Forgets the filter's past input
    pub fn clear(&mut self) {
        self.initial.clear();
        self.target.clear();
    }

    /// One sample, crossfading the old filter into the new one @p progress of the way
    #[inline]
    pub fn process(&mut self, input: f32, channel: usize, progress: f32) -> f32 {
        let initial = self.initial.process(input, channel);
        let target = self.target.process(input, channel);
        if progress > 0.99999 {
            self.initial = self.target;
        }
        (1.0 - progress) * initial + progress * target
    }

    fn compute(&mut self) {
        let w0 = 2.0 * PI * self.frequency / self.sample_rate;
        let c = cosf(w0);
        let s = sinf(w0);
        let (b0, b1, b2, a0, a1, a2) = match self.shape {
            Shape::HighPass => {
                let alpha = s / (2.0 * self.q);
                ((1.0 + c) * 0.5, -(1.0 + c), (1.0 + c) * 0.5, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
            }
            Shape::LowPass => {
                let alpha = s / (2.0 * self.q);
                ((1.0 - c) * 0.5, 1.0 - c, (1.0 - c) * 0.5, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
            }
            Shape::LowShelf => {
                // The gain times a double: the legacy computed this power in double precision
                let a = exp(LN_10 * (f64::from(self.gain) * 0.025)) as f32;
                let beta = a.sqrt() / self.q;
                (
                    a * ((a + 1.0) - (a - 1.0) * c + beta * s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                    a * ((a + 1.0) - (a - 1.0) * c - beta * s),
                    (a + 1.0) + (a - 1.0) * c + beta * s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * c),
                    (a + 1.0) + (a - 1.0) * c - beta * s,
                )
            }
            Shape::HighShelf => {
                // The gain times a double: the legacy computed this power in double precision
                let a = exp(LN_10 * (f64::from(self.gain) * 0.025)) as f32;
                let beta = a.sqrt() / self.q;
                (
                    a * ((a + 1.0) + (a - 1.0) * c + beta * s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                    a * ((a + 1.0) + (a - 1.0) * c - beta * s),
                    (a + 1.0) - (a - 1.0) * c + beta * s,
                    2.0 * ((a - 1.0) - (a + 1.0) * c),
                    (a + 1.0) - (a - 1.0) * c - beta * s,
                )
            }
        };
        self.target.set_coefficients(a1 / a0, a2 / a0, b0 / a0, b1 / a0, b2 / a0);
    }
}
