//! Mono bass: the lows made mono, leaving the image above them (legacy `MonoBass`).

use crate::math::{f32c::PI, tanf};

/// A state variable filter, Butterworth, giving its low-pass and high-pass outputs (Zavalishin's topology-preserving
/// transform)
#[derive(Clone, Copy, Debug, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    #[inline]
    fn process(&mut self, input: f32, g: f32) -> (f32, f32) {
        // Butterworth: 1/Q = sqrt(2)
        const K: f32 = core::f32::consts::SQRT_2;
        let a1 = 1.0 / (1.0 + g * (g + K));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = input - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, input - K * v1 - v2)
    }
}

/// A 4th-order Linkwitz-Riley crossover: two Butterworth sections in a row on each side of the split
#[derive(Clone, Copy, Debug, Default)]
struct Crossover {
    low1: Svf,
    low2: Svf,
    high1: Svf,
    high2: Svf,
}

impl Crossover {
    #[inline]
    fn process(&mut self, input: f32, g: f32) -> (f32, f32) {
        let low = self.low2.process(self.low1.process(input, g).0, g).0;
        let high = self.high2.process(self.high1.process(input, g).1, g).1;
        (low, high)
    }
}

/// Below a frequency, narrows the side of a mid/side signal (M = (L+R)/2, S = (L-R)/2) to a width, 0 being mono.
/// Used by the master's stereo section and by the channels' strips.
///
/// A Linkwitz-Riley crossover (24 dB/oct) splits the side, S' = HP(S) + width LP(S), and the mid goes through the
/// same crossover, LP(M) + HP(M), an allpass of flat gain: mid and side get the same phase, so the image above the
/// frequency stays where it was. (Subtracting a low-pass from the side would not do: the low-pass lags in phase, so
/// the lows would not cancel.) The crossover is built from state variable filters, whose frequency glides sample by
/// sample without clicks; the width is ramped over each stretch, and switching on or off crossfades between the
/// crossover and the dry signal over a stretch.
///
/// Each stretch: begin(), then process() for every frame if begin() returned true, then end().
#[derive(Clone, Debug)]
pub struct MonoBass {
    /// For the mid and the side
    crossover: [Crossover; 2],
    /// The frequency warped for the filters, at the end of the last stretch; 0 = afresh
    g: f32,
    target_g: f32,
    /// At the end of the last stretch
    width: f32,
    target_width: f32,
    /// Of the crossover against the dry signal, at the end of the last stretch
    mix: f32,
    target_mix: f32,
}

impl Default for MonoBass {
    fn default() -> Self {
        Self {
            crossover: [Crossover::default(); 2],
            g: 0.0,
            target_g: 0.0,
            width: 1.0,
            target_width: 1.0,
            mix: 0.0,
            target_mix: 0.0,
        }
    }
}

impl MonoBass {
    /// Starts a stretch with @p frequency (Hz) and @p width (of the side below it: 1 = off, 0 = mono).
    /// Returns whether the stretch needs processing: on, or fading out
    pub fn begin(&mut self, frequency: f32, width: f32, sample_rate: f32) -> bool {
        self.target_width = width.clamp(0.0, 1.0);
        self.target_mix = if self.target_width < 1.0 { 1.0 } else { 0.0 };
        if self.target_mix == 0.0 && self.mix == 0.0 {
            self.g = 0.0;
            self.width = 1.0;
            return false;
        }

        // The frequency warped for the filters (bilinear transform); it glides over the stretch
        self.target_g = tanf(PI * frequency.min(0.45 * sample_rate) / sample_rate);
        if self.g == 0.0 {
            // Starting afresh, faded in
            self.crossover = [Crossover::default(); 2];
            self.g = self.target_g;
        }
        true
    }

    /// One frame, in mid/side, @p progress of the way through the stretch (up to 1 at its last frame)
    #[inline]
    pub fn process(&mut self, mid: &mut f32, side: &mut f32, progress: f32) {
        let g = self.g + (self.target_g - self.g) * progress;
        let width = self.width + (self.target_width - self.width) * progress;
        let (mid_low, mid_high) = self.crossover[0].process(*mid, g);
        let (side_low, side_high) = self.crossover[1].process(*side, g);
        let mix = self.mix + (self.target_mix - self.mix) * progress;
        *mid += (mid_low + mid_high - *mid) * mix;
        *side += (side_high + width * side_low - *side) * mix;
    }

    pub fn end(&mut self) {
        self.g = if self.target_mix > 0.0 { self.target_g } else { 0.0 };
        self.width = self.target_width;
        self.mix = self.target_mix;
    }

    /// Forgets the crossover's state, e.g. while the channel is muted
    pub fn reset(&mut self) {
        self.crossover = [Crossover::default(); 2];
        self.g = 0.0;
        self.width = 1.0;
        self.mix = 0.0;
    }
}
