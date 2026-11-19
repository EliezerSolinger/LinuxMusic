//! The tolerances of a desk channel's components, and the output transformer (legacy `DeskTolerance`).

use crate::filters::{Biquad, RampedBiquad};
use crate::math::f64c::PI;

/// The tolerances of a channel's components: no two channels of an analog desk are quite alike, which is part of
/// how a desk sounds when they are summed. Each channel's are fixed, drawn from its number, as a physical desk's
/// channel 3 is always the same channel 3, in every project and every version of the program (see
/// [`ChannelTolerance::for_channel`]), so that one learns one's desk: within the usual tolerances of resistors,
/// capacitors and pots (uniform): +-0.1 dB of gain, +-1% on the filters' and the EQ's frequencies, +-2% on the EQ's
/// gains, +-3% on where the gain stage saturates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelTolerance {
    /// dB
    pub gain: f32,
    /// Frequency factors
    pub low_cut: f32,
    pub high_pass: f32,
    pub low_pass: f32,
    pub eq_frequency: [f64; 4],
    pub eq_gain: [f64; 4],
    /// A factor on the gain stage's input, undone at its output
    pub drive: f32,
}

impl Default for ChannelTolerance {
    fn default() -> Self {
        Self::NONE
    }
}

/// The tolerances are a contract with the people who mix on the desk: channel 3 has the same character in every
/// project and in every version of the program, so that they can learn their desk ("channel 3 is a little
/// brighter"). So each tolerance is drawn from its own stream, keyed by the channel's number and the tolerance's own
/// number below, by integer arithmetic only (the same on every platform). Never change the seed, the mixing or these
/// numbers; a new tolerance takes a new number, which leaves the others as they are (the frozen values are tested).
#[derive(Clone, Copy)]
enum Part {
    Gain = 1,
    LowCut = 2,
    HighPass = 3,
    LowPass = 4,
    EqLowFrequency = 5,
    EqLowMidFrequency = 6,
    EqHighMidFrequency = 7,
    EqHighFrequency = 8,
    EqLowGain = 9,
    EqLowMidGain = 10,
    EqHighMidGain = 11,
    EqHighGain = 12,
    Drive = 13,
}

const SEED: u64 = 0x4c69_6e75_784d_7573; // "LinuxMus"

/// SplitMix64's finalizer: a well mixed 64-bit hash
const fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Channel @p channel's draw for @p part, uniform in [-1, 1]
fn draw(channel: i32, part: Part) -> f64 {
    // As the legacy's static_cast<uint64_t>(int): a negative number is sign-extended
    let z = mix(mix(SEED ^ (i64::from(channel) as u64)) ^ part as u64);
    (z >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

impl ChannelTolerance {
    /// None: an ideal channel
    pub const NONE: ChannelTolerance = ChannelTolerance {
        gain: 0.0,
        low_cut: 1.0,
        high_pass: 1.0,
        low_pass: 1.0,
        eq_frequency: [1.0; 4],
        eq_gain: [1.0; 4],
        drive: 1.0,
    };

    /// Channel @p channel's
    pub fn for_channel(channel: i32) -> ChannelTolerance {
        let frequencies =
            [Part::EqLowFrequency, Part::EqLowMidFrequency, Part::EqHighMidFrequency, Part::EqHighFrequency];
        let gains = [Part::EqLowGain, Part::EqLowMidGain, Part::EqHighMidGain, Part::EqHighGain];
        let mut tolerance = ChannelTolerance {
            gain: (0.1 * draw(channel, Part::Gain)) as f32,
            low_cut: (1.0 + 0.01 * draw(channel, Part::LowCut)) as f32,
            high_pass: (1.0 + 0.01 * draw(channel, Part::HighPass)) as f32,
            low_pass: (1.0 + 0.01 * draw(channel, Part::LowPass)) as f32,
            drive: (1.0 + 0.03 * draw(channel, Part::Drive)) as f32,
            ..Self::NONE
        };
        for band in 0..4 {
            tolerance.eq_frequency[band] = 1.0 + 0.01 * draw(channel, frequencies[band]);
            tolerance.eq_gain[band] = 1.0 + 0.02 * draw(channel, gains[band]);
        }
        tolerance
    }
}

/// What makes an output transformer
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformerCore {
    /// Hz
    pub low_corner: f64,
    /// Full scale times seconds
    pub saturation_flux: f64,
    /// Hz, first order
    pub high_corner: f64,
}

impl Default for TransformerCore {
    /// The master's
    fn default() -> Self {
        Self { low_corner: 3.0, saturation_flux: 0.008, high_corner: 60000.0 }
    }
}

/// The output transformer of a Neve desk's master, after its fader: the source driving the magnetizing inductance of
/// the core, whose flux (the integral of the voltage) saturates, so that it thickens the lows and does nothing to
/// the mids:
/// - the magnetizing current is linear plus a cubic term, (phi + p^3 / phi_s^2) / Lm, where p follows the flux
///   with a dead band (a play operator): the core's hysteresis, lagging the saturation behind the flux;
/// - the source resistance over the inductance puts the low corner at 3 Hz; the saturation, about 0.5% at 30 Hz
///   near full scale, falls with the square of the frequency, nothing at 1 kHz;
/// - the leakage inductance and capacitances roll the highs off: -0.5 dB at 20 kHz.
///
/// Solved implicitly at each sample (Newton's method).
#[derive(Clone, Debug)]
pub struct OutputTransformer {
    core: TransformerCore,
    period: f64,
    /// 2 pi low corner: the source resistance over the inductance
    corner: f64,
    flux: [f64; 2],
    lag: [f64; 2],
    highs: RampedBiquad,
}

impl OutputTransformer {
    /// The dead band's half width (its coercivity), of the saturation flux
    pub const HYSTERESIS: f64 = 0.02;

    /// Prepared for @p sample_rate
    pub fn new(core: TransformerCore, sample_rate: f64) -> Self {
        let mut highs = RampedBiquad::default();
        highs.set_target(Biquad::low_pass1(core.high_corner, sample_rate));
        highs.finish();
        Self {
            core,
            period: 1.0 / sample_rate,
            corner: 2.0 * PI * core.low_corner,
            flux: [0.0; 2],
            lag: [0.0; 2],
            highs,
        }
    }

    pub fn reset(&mut self) {
        self.flux = [0.0; 2];
        self.lag = [0.0; 2];
        self.highs.reset();
    }

    #[inline]
    pub fn process(&mut self, input: f32, channel: usize) -> f32 {
        // The voltage across the core, v = input - corner (phi + p^3 / phi_s^2), with phi = flux + v T (backward
        // Euler), solved for v
        let source = f64::from(input);
        let band = Self::HYSTERESIS * self.core.saturation_flux;
        let s2 = self.core.saturation_flux * self.core.saturation_flux;
        let mut v = source;
        for _ in 0..8 {
            let flux = self.flux[channel] + v * self.period;
            let lag = self.lag[channel].clamp(flux - band, flux + band);
            let moving = lag != self.lag[channel];
            let f = v - source + self.corner * (flux + lag * lag * lag / s2);
            let slope = 1.0 + self.corner * self.period * (1.0 + if moving { 3.0 * lag * lag / s2 } else { 0.0 });
            let step = f / slope;
            v -= step;
            if step.abs() < 1e-12 {
                break;
            }
        }
        let flux = self.flux[channel] + v * self.period;
        self.lag[channel] = self.lag[channel].clamp(flux - band, flux + band);
        self.flux[channel] = flux;
        self.highs.process(v, channel, 1.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerances_are_within_their_ranges_and_differ() {
        for channel in 1..64 {
            let t = ChannelTolerance::for_channel(channel);
            assert!(t.gain.abs() <= 0.1);
            assert!((t.drive - 1.0).abs() <= 0.03);
            for band in 0..4 {
                assert!((t.eq_frequency[band] - 1.0).abs() <= 0.01);
                assert!((t.eq_gain[band] - 1.0).abs() <= 0.02);
            }
        }
        assert_ne!(ChannelTolerance::for_channel(1), ChannelTolerance::for_channel(2));
        assert_eq!(ChannelTolerance::for_channel(3), ChannelTolerance::for_channel(3));
    }

    /// A contract with the people who mix on the desk: each channel keeps its character in every version, so that
    /// they learn their desk. These are the legacy program's values, and they must never change
    #[test]
    fn tolerances_are_frozen() {
        struct Frozen {
            channel: i32,
            gain: f32,
            low_cut: f32,
            drive: f32,
            high_pass: f32,
            high_frequency: f64,
            high_mid_gain: f64,
        }
        #[rustfmt::skip]
        let frozen = [
            Frozen { channel: 1, gain: -0.00362166832, low_cut: 0.994946778, drive: 0.975250304, high_pass: 1.00620019, high_frequency: 1.0039064650250091, high_mid_gain: 1.0012884893891381 },
            Frozen { channel: 2, gain: 0.0190470163, low_cut: 1.00957358, drive: 0.99104774, high_pass: 1.00720668, high_frequency: 0.99157399934457169, high_mid_gain: 0.98573882847934868 },
            Frozen { channel: 3, gain: -0.0585210323, low_cut: 0.998126507, drive: 0.973794281, high_pass: 0.991076529, high_frequency: 0.99133357141275358, high_mid_gain: 0.99252185458568676 },
            Frozen { channel: 8, gain: -0.0315597095, low_cut: 1.00288808, drive: 0.985630751, high_pass: 1.00447083, high_frequency: 1.0093123677569744, high_mid_gain: 1.0061400838803543 },
            Frozen { channel: 32, gain: -0.00284665497, low_cut: 0.995807171, drive: 1.01793587, high_pass: 0.997933328, high_frequency: 1.0021337474165282, high_mid_gain: 0.9813826111870485 },
        ];
        for f in frozen {
            let t = ChannelTolerance::for_channel(f.channel);
            assert_eq!(t.gain, f.gain, "channel {}", f.channel);
            assert_eq!(t.low_cut, f.low_cut);
            assert_eq!(t.drive, f.drive);
            assert_eq!(t.high_pass, f.high_pass);
            assert!((t.eq_frequency[3] - f.high_frequency).abs() < 1e-15);
            assert!((t.eq_gain[2] - f.high_mid_gain).abs() < 1e-15);
        }
    }

    #[test]
    fn transformer_passes_the_mids() {
        let rate = 48000.0;
        let mut t = OutputTransformer::new(TransformerCore::default(), rate);
        let mut peak = 0.0f32;
        for n in 0..48000 {
            let x = 0.5 * crate::math::sin(2.0 * PI * 1000.0 * n as f64 / rate) as f32;
            let y = t.process(x, 0);
            if n > 24000 {
                peak = peak.max(y.abs());
            }
        }
        assert!((peak - 0.5).abs() < 0.005, "{peak}");
    }
}
