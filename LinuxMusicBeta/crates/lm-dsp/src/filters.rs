//! Biquads matched to their analog prototypes, and the 4-band EQ of an analog desk's channel (legacy
//! `DeskFilters`).

use crate::math::{Complex, cos, cosh, exp, f64c::PI, hypot, pow, sin};

/// The coefficients of a biquad, `y = b0 x + b1 x1 + b2 x2 - a1 y1 - a2 y2`, designed to match an analog prototype
/// up to Nyquist, unlike the bilinear transform, which cramps responses near it:
/// - second order: Martin Vicanek, "Matched Second Order Digital Filters" (2016): the poles by impulse invariance,
///   the zeros so that the magnitude matches at DC, at the center frequency and at Nyquist;
/// - first order: the magnitude matches exactly at DC, at Nyquist and where the response changes most.
///
/// Frequencies in Hz.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biquad {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Default for Biquad {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// The poles of a second order section, by impulse invariance, and Vicanek's terms for the magnitude matching
struct Matched {
    a1: f64,
    a2: f64,
    a0_term: f64,
    a1_term: f64,
    a2_term: f64,
    phi0: f64,
    phi1: f64,
    phi2: f64,
}

impl Matched {
    fn new(frequency: f64, q: f64, sample_rate: f64) -> Self {
        let w0 = 2.0 * PI * frequency.min(0.4999 * sample_rate) / sample_rate;
        let zeta = 1.0 / (2.0 * q);
        let decay = exp(-zeta * w0);
        let a1 = if zeta <= 1.0 {
            -2.0 * decay * cos(w0 * (1.0 - zeta * zeta).sqrt())
        } else {
            -2.0 * decay * cosh(w0 * (zeta * zeta - 1.0).sqrt())
        };
        let a2 = exp(-2.0 * zeta * w0);
        let s = sin(w0 / 2.0);
        let phi1 = s * s;
        let phi0 = 1.0 - phi1;
        Self {
            a1,
            a2,
            a0_term: (1.0 + a1 + a2) * (1.0 + a1 + a2),
            a1_term: (1.0 - a1 + a2) * (1.0 - a1 + a2),
            a2_term: -4.0 * a2,
            phi0,
            phi1,
            phi2: 4.0 * phi0 * phi1,
        }
    }

    fn sum(&self) -> f64 {
        self.a0_term * self.phi0 + self.a1_term * self.phi1 + self.a2_term * self.phi2
    }
}

/// A first order section matching an analog magnitude exactly at DC, at Nyquist and at @p frequency, where it
/// changes most (three conditions for its three coefficients): with A = (1 + a1)^2, B = (1 - a1)^2, its squared
/// magnitude is (dc^2 A cos^2(w/2) + nyquist^2 B sin^2(w/2)) / (A cos^2(w/2) + B sin^2(w/2))
fn first_order(dc: f64, nyquist: f64, frequency: f64, magnitude: f64, sample_rate: f64) -> Biquad {
    let w = 2.0 * PI * frequency / sample_rate;
    let s = sin(w / 2.0);
    let phi1 = s * s;
    let phi0 = 1.0 - phi1;
    let m2 = magnitude * magnitude;
    let ratio = phi1 * (m2 - nyquist * nyquist) / (phi0 * (dc * dc - m2)); // A / B
    let t = ratio.max(1e-12).sqrt();
    let a1 = (t - 1.0) / (t + 1.0);
    let sum = dc * (1.0 + a1);
    let difference = nyquist * (1.0 - a1);
    Biquad { b0: (sum + difference) / 2.0, b1: (sum - difference) / 2.0, b2: 0.0, a1, a2: 0.0 }
}

/// The magnitude of (s + wz) / (s + wp) at w, all in Hz
fn first_order_magnitude(f: f64, zero: f64, pole: f64) -> f64 {
    hypot(f, zero) / hypot(f, pole)
}

impl Biquad {
    pub const IDENTITY: Biquad = Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    pub fn low_pass(frequency: f64, q: f64, sample_rate: f64) -> Biquad {
        let m = Matched::new(frequency, q, sample_rate);
        let r1 = m.sum() * q * q;
        let b0_term = m.a0_term;
        let b1_term = ((r1 - b0_term * m.phi0) / m.phi1).max(0.0);
        let b0 = 0.5 * (b0_term.sqrt() + b1_term.sqrt());
        Biquad { b0, b1: b0_term.sqrt() - b0, b2: 0.0, a1: m.a1, a2: m.a2 }
    }

    pub fn high_pass(frequency: f64, q: f64, sample_rate: f64) -> Biquad {
        let m = Matched::new(frequency, q, sample_rate);
        let b0 = q * m.sum().max(0.0).sqrt() / (4.0 * m.phi1);
        Biquad { b0, b1: -2.0 * b0, b2: b0, a1: m.a1, a2: m.a2 }
    }

    /// First order high-pass, s / (s + wp): matched at the corner
    pub fn high_pass1(frequency: f64, sample_rate: f64) -> Biquad {
        let nyquist = sample_rate / 2.0;
        let f = frequency.min(0.45 * sample_rate);
        first_order(0.0, nyquist / hypot(nyquist, f), f, 0.5f64.sqrt(), sample_rate)
    }

    /// First order low-pass, 1 / (1 + s / wp); @p frequency may be above Nyquist: matched where it is down most
    /// below Nyquist, or at the corner
    pub fn low_pass1(frequency: f64, sample_rate: f64) -> Biquad {
        let nyquist = sample_rate / 2.0;
        let matched = frequency.min(0.4 * sample_rate);
        first_order(
            1.0,
            frequency / hypot(nyquist, frequency),
            matched,
            frequency / hypot(matched, frequency),
            sample_rate,
        )
    }

    /// (s^2 + s G/Q + 1) / (s^2 + s/Q + 1): G at the center frequency; a boost (G >= 1)
    pub fn peak(frequency: f64, q: f64, gain: f64, sample_rate: f64) -> Biquad {
        if gain == 1.0 {
            return Biquad::IDENTITY;
        }
        let m = Matched::new(frequency, q, sample_rate);
        let g2 = gain * gain;
        let r1 = m.sum() * g2;
        let r2 = (-m.a0_term + m.a1_term + 4.0 * (m.phi0 - m.phi1) * m.a2_term) * g2;
        let b0_term = m.a0_term;
        let b2_term = (r1 - r2 * m.phi1 - b0_term) / (4.0 * m.phi1 * m.phi1);
        let b1_term = (r2 + b0_term + 4.0 * (m.phi1 - m.phi0) * b2_term).max(0.0);
        let w = 0.5 * (b0_term.sqrt() + b1_term.sqrt());
        let b0 = 0.5 * (w + (w * w + b2_term).max(0.0).sqrt());
        Biquad { b0, b1: 0.5 * (b0_term.sqrt() - b1_term.sqrt()), b2: -b2_term / (4.0 * b0), a1: m.a1, a2: m.a2 }
    }

    /// First order low shelf, as a Baxandall tone control's: (s + wz) / (s + wp), the pole at the corner, the zero G
    /// times higher: G at DC, 3 dB short of it at the corner; a boost
    pub fn low_shelf1(corner: f64, gain: f64, sample_rate: f64) -> Biquad {
        if gain == 1.0 {
            return Biquad::IDENTITY;
        }
        let nyquist = sample_rate / 2.0;
        let zero = corner * gain;
        let middle = (corner * zero).sqrt().min(0.45 * sample_rate);
        first_order(
            gain,
            first_order_magnitude(nyquist, zero, corner),
            middle,
            first_order_magnitude(middle, zero, corner),
            sample_rate,
        )
    }

    /// First order high shelf: G (s + wz) / (s + wp), the pole at the corner, the zero G times lower: G far above it
    pub fn high_shelf1(corner: f64, gain: f64, sample_rate: f64) -> Biquad {
        if gain == 1.0 {
            return Biquad::IDENTITY;
        }
        let nyquist = sample_rate / 2.0;
        let zero = corner / gain;
        let middle = (corner * zero).sqrt().min(0.45 * sample_rate);
        first_order(
            1.0,
            gain * first_order_magnitude(nyquist, zero, corner),
            middle,
            gain * first_order_magnitude(middle, zero, corner),
            sample_rate,
        )
    }

    pub fn is_identity(&self) -> bool {
        *self == Biquad::IDENTITY
    }

    /// The complex response at @p frequency
    pub fn response(&self, frequency: f64, sample_rate: f64) -> Complex {
        let z = Complex::polar(1.0, -2.0 * PI * frequency / sample_rate); // z^-1
        let num = z * (z * self.b2 + self.b1) + self.b0;
        let den = z * (z * self.a2 + self.a1) + 1.0;
        num / den
    }

    /// The magnitude at @p frequency
    pub fn magnitude(&self, frequency: f64, sample_rate: f64) -> f64 {
        self.response(frequency, sample_rate).abs()
    }
}

impl core::ops::Mul<f64> for Complex {
    type Output = Complex;
    fn mul(self, o: f64) -> Complex {
        Complex::new(self.re * o, self.im * o)
    }
}

/// A stereo biquad (transposed direct form II) whose coefficients move linearly from those it had to new ones
/// over a stretch of samples (the caller's `progress`, 0 to 1), so that changing them does not click.
#[derive(Clone, Debug, Default)]
pub struct RampedBiquad {
    from: Biquad,
    to: Biquad,
    z1: [f64; 2],
    z2: [f64; 2],
}

impl RampedBiquad {
    /// The coefficients to reach by the end of this stretch
    pub fn set_target(&mut self, target: Biquad) {
        self.from = self.to;
        self.to = target;
    }
    /// Ends the stretch: the target is reached
    pub fn finish(&mut self) {
        self.from = self.to;
    }
    pub fn reset(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
    }
    pub fn is_identity(&self) -> bool {
        self.from.is_identity() && self.to.is_identity()
    }
    pub fn target(&self) -> &Biquad {
        &self.to
    }

    #[inline]
    pub fn process(&mut self, x: f64, channel: usize, progress: f64) -> f64 {
        let c = self.at(progress);
        let y = c.b0 * x + self.z1[channel];
        self.advance(&c, x, y, channel);
        y
    }

    /// The coefficients @p progress of the way (for filters in a delay-free loop: the output is b0 times the input
    /// plus what the state gives)
    #[inline]
    pub fn at(&self, progress: f64) -> Biquad {
        if progress >= 1.0 || self.from == self.to {
            return self.to;
        }
        let mix = |a: f64, b: f64| a + (b - a) * progress;
        Biquad {
            b0: mix(self.from.b0, self.to.b0),
            b1: mix(self.from.b1, self.to.b1),
            b2: mix(self.from.b2, self.to.b2),
            a1: mix(self.from.a1, self.to.a1),
            a2: mix(self.from.a2, self.to.a2),
        }
    }
    #[inline]
    pub fn state(&self, channel: usize) -> f64 {
        self.z1[channel]
    }
    #[inline]
    pub fn advance(&mut self, c: &Biquad, x: f64, y: f64, channel: usize) {
        self.z1[channel] = c.b1 * x - c.a1 * y + self.z2[channel];
        self.z2[channel] = c.b2 * x - c.a2 * y;
    }
}

/// The bands of [`DeskEq`]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Low = 0,
    LowMid = 1,
    HighMid = 2,
    High = 3,
}

/// The 4-band EQ of a Soundcraft or Mackie desk's channel: first order shelves at 80 Hz and 12 kHz and peaks at
/// 400 Hz and 2.5 kHz, +-15 dB, as in their active circuits:
/// - the peaks narrow as their gain grows (proportional Q): 2.5 octaves wide at +-3 dB, one at +-15 dB;
/// - the boosts act in parallel and the cuts in the amplifier's feedback, H = (1 + sum of boosts) / (1 + sum of
///   cuts): alone, a band boosts or cuts as it says, while neighbouring bands interact, overlapping boosts or cuts
///   adding up to less than their sum in dB, as on the desk. (The feedback is a delay-free loop, solved exactly.)
///
/// A band that changes from boost to cut goes through flat over a stretch, so that it does not click.
#[derive(Clone, Debug, Default)]
pub struct DeskEq {
    filters: [RampedBiquad; 4],
    cut: [bool; 4],
}

impl DeskEq {
    pub const BANDS: usize = 4;
    pub const LOW_FREQUENCY: f64 = 80.0;
    pub const LOW_MID_FREQUENCY: f64 = 400.0;
    pub const HIGH_MID_FREQUENCY: f64 = 2500.0;
    pub const HIGH_FREQUENCY: f64 = 12000.0;
    /// dB
    pub const MAX_GAIN: f64 = 15.0;

    /// The peaks' bandwidth in octaves, at half their gain in dB, for a gain in dB: 2.5 octaves for gentle settings,
    /// narrowing to one when pushed all the way
    pub fn peak_bandwidth(gain: f64) -> f64 {
        let g = gain.abs().clamp(3.0, Self::MAX_GAIN);
        2.5 - 1.5 * (g - 3.0) / (Self::MAX_GAIN - 3.0)
    }

    /// The peaks' Q for a gain in dB, giving that bandwidth: at half the gain in dB, where |H|^2 = G,
    /// (1 - w^2) Q = sqrt(G) w, with w = 2^(-bw/2)
    pub fn peak_q(gain: f64) -> f64 {
        let g = pow(10.0, gain.abs() / 20.0);
        let w = pow(2.0, -Self::peak_bandwidth(gain) / 2.0);
        g.sqrt() * w / (1.0 - w * w)
    }

    /// Each band as a boost; a cut is the inverse of the boost of the same amount, in the feedback
    fn design(band: usize, gain: f64, frequency_factor: f64, sample_rate: f64) -> Biquad {
        let boost = pow(10.0, gain.abs() / 20.0);
        match band {
            0 => Biquad::low_shelf1(Self::LOW_FREQUENCY * frequency_factor, boost, sample_rate),
            1 => Biquad::peak(Self::LOW_MID_FREQUENCY * frequency_factor, Self::peak_q(gain), boost, sample_rate),
            2 => Biquad::peak(Self::HIGH_MID_FREQUENCY * frequency_factor, Self::peak_q(gain), boost, sample_rate),
            _ => Biquad::high_shelf1(Self::HIGH_FREQUENCY * frequency_factor, boost, sample_rate),
        }
    }

    /// Sets this stretch's gains in dB, low to high.
    /// @p frequencies and @p gain_factors are the channel's tolerances on the bands' frequencies and gains
    pub fn begin(&mut self, gains: [f32; 4], sample_rate: f64, frequencies: [f64; 4], gain_factors: [f64; 4]) {
        for band in 0..Self::BANDS {
            let gain = f64::from(gains[band]) * gain_factors[band];
            let cut = gain < 0.0;
            let filter = &mut self.filters[band];
            // Changing sides: flat first, then the other side from the next stretch on
            if cut != self.cut[band] && gain != 0.0 {
                if !filter.target().is_identity() {
                    filter.set_target(Biquad::IDENTITY);
                    continue;
                }
                self.cut[band] = cut;
            }
            filter.set_target(Self::design(band, gain, frequencies[band], sample_rate));
        }
    }

    /// Whether it does anything this stretch
    pub fn is_active(&self) -> bool {
        self.filters.iter().any(|f| !f.is_identity())
    }

    #[inline]
    pub fn process(&mut self, x: f64, channel: usize, progress: f64) -> f64 {
        // The boosts, in parallel: 1 + sum of (H - 1)
        let mut boosted = x;
        for band in 0..Self::BANDS {
            if self.cut[band] || self.filters[band].is_identity() {
                continue;
            }
            boosted += self.filters[band].process(x, channel, progress) - x;
        }

        // The cuts, in the feedback: y = boosted - sum of (H - 1)(y), with H(y) = b0 y + state, solved for y
        let mut states = 0.0;
        let mut slope = 1.0;
        let mut coefficients = [Biquad::IDENTITY; 4];
        for band in 0..Self::BANDS {
            let filter = &self.filters[band];
            if !self.cut[band] || filter.is_identity() {
                continue;
            }
            coefficients[band] = filter.at(progress);
            states += filter.state(channel);
            slope += coefficients[band].b0 - 1.0;
        }
        if slope == 1.0 && states == 0.0 {
            return boosted;
        }
        let y = (boosted - states) / slope;
        for band in 0..Self::BANDS {
            if !self.cut[band] || self.filters[band].is_identity() {
                continue;
            }
            let c = coefficients[band];
            let output = c.b0 * y + self.filters[band].state(channel);
            self.filters[band].advance(&c, y, output, channel);
        }
        y
    }

    /// Ends the stretch
    pub fn finish(&mut self) {
        for filter in &mut self.filters {
            filter.finish();
            // A flat band starts afresh when it is used again
            if filter.is_identity() {
                filter.reset();
            }
        }
    }

    pub fn reset(&mut self) {
        for filter in &mut self.filters {
            filter.reset();
        }
    }

    /// The EQ's magnitude at @p frequency with its current settings
    pub fn magnitude(&self, frequency: f64, sample_rate: f64) -> f64 {
        let mut boosts = Complex::ONE;
        let mut cuts = Complex::ONE;
        for band in 0..Self::BANDS {
            let c = self.filters[band].target();
            if c.is_identity() {
                continue;
            }
            let r = c.response(frequency, sample_rate) - 1.0;
            if self.cut[band] { cuts += r } else { boosts += r }
        }
        (boosts / cuts).abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48000.0;

    fn db(x: f64) -> f64 {
        20.0 * crate::math::log(x) / core::f64::consts::LN_10
    }

    #[test]
    fn matched_filters_have_their_analog_corners() {
        assert!((db(Biquad::low_pass(1000.0, 0.5f64.sqrt(), RATE).magnitude(1000.0, RATE)) + 3.01).abs() < 0.05);
        assert!((db(Biquad::high_pass(100.0, 0.5f64.sqrt(), RATE).magnitude(100.0, RATE)) + 3.01).abs() < 0.05);
        assert!((db(Biquad::high_pass1(100.0, RATE).magnitude(100.0, RATE)) + 3.01).abs() < 0.01);
        assert!((db(Biquad::low_pass(1000.0, 0.5f64.sqrt(), RATE).magnitude(10.0, RATE))).abs() < 0.01);
        // Unlike the bilinear transform, a high corner is not cramped toward Nyquist
        assert!(db(Biquad::low_pass(15000.0, 0.5f64.sqrt(), RATE).magnitude(15000.0, RATE)) > -3.6);
    }

    #[test]
    fn peaks_and_shelves_reach_their_gain() {
        let g = crate::math::pow(10.0, 12.0 / 20.0);
        assert!((db(Biquad::peak(1000.0, 1.0, g, RATE).magnitude(1000.0, RATE)) - 12.0).abs() < 0.01);
        assert!((db(Biquad::low_shelf1(80.0, g, RATE).magnitude(5.0, RATE)) - 12.0).abs() < 0.3);
        assert!((db(Biquad::high_shelf1(12000.0, g, RATE).magnitude(23999.0, RATE)) - 12.0).abs() < 1.0);
    }

    #[test]
    fn eq_boosts_and_cuts_are_mirror_images() {
        let mut boost = DeskEq::default();
        boost.begin([0.0, 9.0, 0.0, 0.0], RATE, [1.0; 4], [1.0; 4]);
        let mut cut = DeskEq::default();
        cut.begin([0.0, -9.0, 0.0, 0.0], RATE, [1.0; 4], [1.0; 4]);
        for f in [100.0, 400.0, 1000.0] {
            let up = db(boost.magnitude(f, RATE));
            let down = db(cut.magnitude(f, RATE));
            assert!((up + down).abs() < 1e-9, "{f}: {up} {down}");
        }
        assert!((db(boost.magnitude(400.0, RATE)) - 9.0).abs() < 0.01);
    }

    #[test]
    fn neutral_eq_is_inactive_and_transparent() {
        let mut eq = DeskEq::default();
        eq.begin([0.0; 4], RATE, [1.0; 4], [1.0; 4]);
        assert!(!eq.is_active());
        assert_eq!(eq.process(0.25, 0, 0.5), 0.25);
    }
}
