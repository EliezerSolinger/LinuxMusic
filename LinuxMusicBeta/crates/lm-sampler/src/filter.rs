//! The voice filter: a two-pole biquad (RBJ cookbook), low-pass, high-pass or band-pass, whose cutoff the voice moves
//! by the key, the velocity and the filter envelope. Coefficients are recomputed every sub-block, not every sample.

use crate::model::FilterKind;

#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: [f32; 2],
    z2: [f32; 2],
}

impl Biquad {
    /// New coefficients for a cutoff (Hz) and resonance (dB), keeping the state
    pub fn set(&mut self, kind: FilterKind, cutoff: f32, resonance_db: f32, rate: f32) {
        let f = cutoff.clamp(20.0, rate * 0.45);
        let w = 2.0 * std::f32::consts::PI * f / rate;
        let (sin, cos) = (libm::sinf(w), libm::cosf(w));
        // Resonance in dB above a flat Butterworth Q
        let q = std::f32::consts::FRAC_1_SQRT_2 * libm::powf(10.0, resonance_db.clamp(-20.0, 40.0) / 20.0);
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = match kind {
            FilterKind::LowPass => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0),
            FilterKind::HighPass => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0),
            FilterKind::BandPass => (alpha, 0.0, -alpha),
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = -2.0 * cos / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    pub fn reset(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
    }

    /// One frame, both channels (transposed direct form II)
    #[inline]
    pub fn process(&mut self, x: [f32; 2]) -> [f32; 2] {
        let mut y = [0.0; 2];
        for ch in 0..2 {
            y[ch] = self.b0 * x[ch] + self.z1[ch];
            self.z1[ch] = self.b1 * x[ch] - self.a1 * y[ch] + self.z2[ch];
            self.z2[ch] = self.b2 * x[ch] - self.a2 * y[ch];
        }
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain_at(kind: FilterKind, cutoff: f32, hz: f32) -> f32 {
        let rate = 48_000.0;
        let mut b = Biquad::default();
        b.set(kind, cutoff, 0.0, rate);
        let mut peak = 0.0f32;
        for n in 0..9600 {
            let x = libm::sinf(2.0 * std::f32::consts::PI * hz * n as f32 / rate);
            let y = b.process([x, x]);
            if n > 4800 {
                peak = peak.max(y[0].abs());
            }
        }
        peak
    }

    #[test]
    fn low_pass_passes_low_and_stops_high() {
        assert!(gain_at(FilterKind::LowPass, 2000.0, 100.0) > 0.95);
        assert!(gain_at(FilterKind::LowPass, 2000.0, 15000.0) < 0.05);
    }

    #[test]
    fn high_pass_is_the_opposite() {
        assert!(gain_at(FilterKind::HighPass, 2000.0, 100.0) < 0.05);
        assert!(gain_at(FilterKind::HighPass, 2000.0, 15000.0) > 0.95);
    }
}
