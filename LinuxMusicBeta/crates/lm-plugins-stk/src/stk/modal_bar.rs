//! The struck bars (`ModalBar` on `Modal`): a strike wave through a one-pole low-pass into four resonators, with a
//! vibrato. Nine presets (marimba to clump).
//!
//! At another rate than 44.1 kHz the bar is the same bar (U36): the strike (the wave, its envelope, the stick's
//! low-pass and gain) is worked out at 44.1 kHz exactly as the library does it and brought to the rate by a
//! band-limited interpolation ([`Strike`]: the library plays the wave with linear interpolation, whose images alias
//! differently at each rate, up to 3 dB on the vibraphone at 96 kHz); the resonators decay as long, their halving of
//! modes above the Nyquist frequency is 44.1 kHz's, and their gain is scaled for the same ringing.

use std::sync::LazyLock;

use super::filters::{BiQuad, Envelope, OnePole, SineWave, WvIn, sine_table};
use super::{ONE_OVER_128, PI, REFERENCE_RATE, Rate, TWO_PI, rawwaves};

/// The modes of a bar
const MODES: usize = 4;

/// `ModalBar::setPreset`'s table: per preset, the modes' ratios (negative: a fixed frequency in Hz), their pole
/// radii, their gains, and the stick's hardness, position and direct gain
#[rustfmt::skip]
static PRESETS: [[[f64; 4]; 4]; 9] = [
    [[1.0, 3.99, 10.65, -2443.0],     // Marimba
     [0.9996, 0.9994, 0.9994, 0.999],
     [0.04, 0.01, 0.01, 0.008],
     [0.429688, 0.445312, 0.093750, 0.0]],
    [[1.0, 2.01, 3.9, 14.37],         // Vibraphone
     [0.99995, 0.99991, 0.99992, 0.9999],
     [0.025, 0.015, 0.015, 0.015],
     [0.390625, 0.570312, 0.078125, 0.0]],
    [[1.0, 4.08, 6.669, -3725.0],     // Agogo
     [0.999, 0.999, 0.999, 0.999],
     [0.06, 0.05, 0.03, 0.02],
     [0.609375, 0.359375, 0.140625, 0.0]],
    [[1.0, 2.777, 7.378, 15.377],     // Wood1
     [0.996, 0.994, 0.994, 0.99],
     [0.04, 0.01, 0.01, 0.008],
     [0.460938, 0.375000, 0.046875, 0.0]],
    [[1.0, 2.777, 7.378, 15.377],     // Reso
     [0.99996, 0.99994, 0.99994, 0.9999],
     [0.02, 0.005, 0.005, 0.004],
     [0.453125, 0.250000, 0.101562, 0.0]],
    [[1.0, 1.777, 2.378, 3.377],      // Wood2
     [0.996, 0.994, 0.994, 0.99],
     [0.04, 0.01, 0.01, 0.008],
     [0.312500, 0.445312, 0.109375, 0.0]],
    [[1.0, 1.004, 1.013, 2.377],      // Beats
     [0.9999, 0.9999, 0.9999, 0.999],
     [0.02, 0.005, 0.005, 0.004],
     [0.398438, 0.296875, 0.070312, 0.0]],
    [[1.0, 4.0, -1320.0, -3960.0],    // 2Fix
     [0.9996, 0.999, 0.9994, 0.999],
     [0.04, 0.01, 0.01, 0.008],
     [0.453125, 0.453125, 0.070312, 0.0]],
    [[1.0, 1.217, 1.475, 1.729],      // Clump
     [0.999, 0.999, 0.999, 0.999],
     [0.03, 0.03, 0.03, 0.03],
     [0.390625, 0.570312, 0.078125, 0.0]],
];

/// Half the band-limited interpolator's taps (reference samples on each side)
const HALF: usize = 16;
const TAPS: usize = 2 * HALF;
/// The interpolator's table resolution (phases per reference sample, linearly interpolated)
const PHASES: usize = 256;
/// The strike's ring of reference samples
const RING: usize = 64;

/// The interpolator's windowed sinc (Blackman-Harris over 32 taps), per phase: `(PHASES + 1) * TAPS` values; tap
/// `j` of phase `p` weighs the reference sample `n0 + j - (HALF - 1)` for the time `n0 + p / PHASES`
pub(crate) fn kernel() -> &'static [f64] {
    static K: LazyLock<Box<[f64]>> = LazyLock::new(|| {
        let pi = core::f64::consts::PI;
        let mut k = vec![0.0; (PHASES + 1) * TAPS];
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            for j in 0..TAPS {
                let d = j as f64 - (HALF as f64 - 1.0) - frac;
                let sinc = if d == 0.0 { 1.0 } else { lm_dsp::math::sin(pi * d) / (pi * d) };
                // (the window spans −HALF to HALF)
                let x = (d + HALF as f64) / (2.0 * HALF as f64);
                let w = 0.35875 - 0.48829 * lm_dsp::math::cos(2.0 * pi * x) + 0.14128 * lm_dsp::math::cos(4.0 * pi * x)
                    - 0.01168 * lm_dsp::math::cos(6.0 * pi * x);
                k[p * TAPS + j] = sinc * w;
            }
        }
        k.into_boxed_slice()
    });
    &K
}

/// The strike as the library makes it at 44.1 kHz: the wave, its envelope, the stick's low-pass and gain
/// (`masterGain * onepole(wave * envelope)`)
#[derive(Clone, Copy, Debug)]
struct Exciter {
    envelope: Envelope,
    wave: WvIn,
    onepole: OnePole,
    master_gain: f64,
}

impl Exciter {
    #[inline]
    fn tick(&mut self) -> f64 {
        self.master_gain * self.onepole.tick(self.wave.tick() * self.envelope.tick())
    }
}

/// The strike at the instrument's rate: the exciter itself at 44.1 kHz, or brought to another rate by the
/// band-limited interpolator (the exciter runs `HALF` samples ahead: it depends on nothing after the note on, so the
/// interpolation adds no delay)
#[derive(Clone, Copy, Debug)]
struct Strike {
    exciter: Exciter,
    /// The reference samples, by index modulo `RING`
    ring: [f64; RING],
    /// The next reference sample to make
    made: i64,
    /// The time of the next output, in reference samples
    time: f64,
    /// Reference samples per output sample (`44100 / rate`)
    step: f64,
    /// The last reference samples made that were exactly 0 in a row (all `TAPS` of them: the strike is over)
    zeros: usize,
    reference: bool,
}

impl Strike {
    /// Starts the interpolation (after the exciter's note on)
    fn start(&mut self) {
        self.ring = [0.0; RING];
        self.made = 0;
        self.time = 0.0;
        self.zeros = 0;
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        if self.reference {
            return self.exciter.tick();
        }
        let n0 = self.time.floor() as i64;
        while self.made <= n0 + HALF as i64 {
            let x = self.exciter.tick();
            self.zeros = if x == 0.0 { self.zeros + 1 } else { 0 };
            self.ring[(self.made as usize) % RING] = x;
            self.made += 1;
        }
        self.time += self.step;
        if self.zeros > TAPS {
            // (the strike is over: its tail is exact zeros, and stays so)
            return 0.0;
        }
        let frac = (self.time - self.step) - n0 as f64;
        let pos = frac * PHASES as f64;
        let p = (pos as usize).min(PHASES - 1);
        let a = pos - p as f64;
        let k = kernel();
        let (k0, k1) = (&k[p * TAPS..(p + 1) * TAPS], &k[(p + 1) * TAPS..(p + 2) * TAPS]);
        let mut y = 0.0;
        for j in 0..TAPS {
            let n = n0 + j as i64 - (HALF as i64 - 1);
            if n < 0 {
                continue;
            }
            let w = k0[j] + a * (k1[j] - k0[j]);
            y += w * self.ring[(n as usize) % RING];
        }
        y
    }
}

/// A struck bar
#[derive(Clone, Copy, Debug)]
pub struct ModalBar {
    rate: Rate,
    strike: Strike,
    filters: [BiQuad; MODES],
    /// Each mode's gain (`setModeGain`) and the frequency its resonator was last set to
    mode_gains: [f64; MODES],
    mode_freqs: [f64; MODES],
    vibrato: SineWave,
    ratios: [f64; MODES],
    radii: [f64; MODES],
    vibrato_gain: f64,
    direct_gain: f64,
    stick_hardness: f64,
    strike_position: f64,
    base_frequency: f64,
}

impl ModalBar {
    /// As the library builds one (`Modal()`, then `ModalBar()`)
    pub fn new(rate: Rate) -> Self {
        let sr = rate.hz;
        let mut filters = [BiQuad::default(); MODES];
        for f in &mut filters {
            f.set_equal_gain_zeroes();
        }
        let mut vibrato = SineWave::default();
        vibrato.set_frequency(6.0, sr);
        // (the strike is made at 44.1 kHz at any rate)
        let mut wave = WvIn::open(rawwaves::marmstk1(), REFERENCE_RATE);
        wave.set_rate(0.5 * 22050.0 / REFERENCE_RATE);
        let exciter = Exciter { envelope: Envelope::default(), wave, onepole: OnePole::default(), master_gain: 1.0 };
        let strike = Strike {
            exciter,
            ring: [0.0; RING],
            made: 0,
            time: 0.0,
            step: rate.k,
            zeros: 0,
            reference: rate.is_reference(),
        };
        let mut bar = Self {
            rate,
            strike,
            filters,
            mode_gains: [1.0; MODES],
            mode_freqs: [0.0; MODES],
            vibrato,
            ratios: [0.0; MODES],
            radii: [0.0; MODES],
            vibrato_gain: 0.0,
            direct_gain: 0.0,
            stick_hardness: 0.5,
            strike_position: 0.561,
            base_frequency: 440.0,
        };
        bar.clear();
        bar.set_preset(0);
        bar
    }

    fn clear(&mut self) {
        self.strike.exciter.onepole.clear();
        for f in &mut self.filters {
            f.clear();
        }
    }

    /// The scale of a resonator's gain at `freq` Hz (see the module): its ringing after the strike is the strike's
    /// spectrum (the rate over 44.1 kHz times the reference's) over `sin(2π f / rate)`; 1 at 44.1 kHz
    fn gain_scale(&self, freq: f64) -> f64 {
        if self.rate.is_reference() {
            return 1.0;
        }
        let at_reference = lm_dsp::math::sin(TWO_PI * freq / REFERENCE_RATE);
        let here = lm_dsp::math::sin(TWO_PI * freq / self.rate.hz);
        if at_reference > 1.0e-6 && here > 0.0 { self.rate.k * here / at_reference } else { self.rate.k * self.rate.k }
    }

    /// `filters_[i]->setResonance(freq, radius)`, with the gain for that frequency
    fn resonate(&mut self, i: usize, freq: f64, radius: f64) {
        self.filters[i].set_resonance(freq, self.rate.radius(radius), false, self.rate.hz);
        self.mode_freqs[i] = freq;
        self.filters[i].set_gain(self.mode_gains[i] * self.gain_scale(freq));
    }

    pub fn set_frequency(&mut self, frequency: f64) {
        self.base_frequency = frequency;
        for i in 0..MODES {
            self.set_ratio_and_radius(i, self.ratios[i], self.radii[i]);
        }
    }

    fn set_ratio_and_radius(&mut self, i: usize, ratio: f64, radius: f64) {
        // (the modes halved are 44.1 kHz's at any rate above it)
        let nyquist = (self.rate.hz / 2.0).min(REFERENCE_RATE / 2.0);
        if ratio * self.base_frequency < nyquist {
            self.ratios[i] = ratio;
        } else {
            let mut temp = ratio;
            while temp * self.base_frequency > nyquist {
                temp *= 0.5;
            }
            self.ratios[i] = temp;
        }
        self.radii[i] = radius;
        let temp = if ratio < 0.0 { -ratio } else { ratio * self.base_frequency };
        self.resonate(i, temp, radius);
    }

    fn set_mode_gain(&mut self, i: usize, gain: f64) {
        self.mode_gains[i] = gain;
        self.filters[i].set_gain(gain * self.gain_scale(self.mode_freqs[i]));
    }

    fn set_stick_hardness(&mut self, hardness: f64) {
        if !(0.0..=1.0).contains(&hardness) {
            return;
        }
        self.stick_hardness = hardness;
        self.strike.exciter.wave.set_rate(0.25 * lm_dsp::math::pow(4.0, self.stick_hardness));
        self.strike.exciter.master_gain = 0.1 + (1.8 * self.stick_hardness);
    }

    fn set_strike_position(&mut self, position: f64) {
        if !(0.0..=1.0).contains(&position) {
            return;
        }
        self.strike_position = position;
        let temp2 = position * PI;
        let temp = lm_dsp::math::sin(temp2);
        self.set_mode_gain(0, 0.12 * temp);
        let temp = lm_dsp::math::sin(0.05 + (3.9 * temp2));
        self.set_mode_gain(1, -0.03 * temp);
        let temp = lm_dsp::math::sin(-0.05 + (11.0 * temp2));
        self.set_mode_gain(2, 0.11 * temp);
    }

    fn set_preset(&mut self, preset: i32) {
        let temp = preset.rem_euclid(9) as usize;
        let p = &PRESETS[temp];
        #[allow(clippy::needless_range_loop)] // (the library's loop: the mode's index, its ratio and gain by index)
        for i in 0..MODES {
            self.set_ratio_and_radius(i, p[0][i], p[1][i]);
            self.set_mode_gain(i, p[2][i]);
        }
        self.set_stick_hardness(p[3][0]);
        self.set_strike_position(p[3][1]);
        self.direct_gain = p[3][2];
        self.vibrato_gain = if temp == 1 { 0.2 } else { 0.0 };
    }

    /// `controlChange` (2 hardness, 4 position, 16 preset, 8 vibrato gain, 1 stick mix, 11 vibrato frequency, 128
    /// volume)
    pub fn control_change(&mut self, number: i32, value: f64) {
        let nv = value * ONE_OVER_128;
        match number {
            2 => self.set_stick_hardness(nv),
            4 => self.set_strike_position(nv),
            16 => self.set_preset(value as i32),
            8 => self.vibrato_gain = nv * 0.3,
            1 => self.direct_gain = nv,
            11 => self.vibrato.set_frequency(nv * 12.0, self.rate.hz),
            128 => self.strike.exciter.envelope.set_target(nv),
            _ => {}
        }
    }

    fn strike(&mut self, amplitude: f64) {
        let e = &mut self.strike.exciter;
        e.envelope.set_rate(1.0);
        e.envelope.set_target(amplitude);
        e.onepole.set_pole(1.0 - amplitude);
        e.envelope.tick();
        e.wave.reset();
        for i in 0..MODES {
            let temp = if self.ratios[i] < 0.0 { -self.ratios[i] } else { self.ratios[i] * self.base_frequency };
            self.resonate(i, temp, self.radii[i]);
        }
        self.strike.start();
    }

    pub fn note_on(&mut self, frequency: f64, amplitude: f64) {
        self.strike(amplitude);
        self.set_frequency(frequency);
    }

    #[inline]
    pub fn tick(&mut self) -> f64 {
        let mut temp = self.strike.tick();
        let mut temp2 = 0.0;
        for f in &mut self.filters {
            temp2 += f.tick(temp);
        }
        temp2 -= temp2 * self.direct_gain;
        temp2 += self.direct_gain * temp;
        if self.vibrato_gain != 0.0 {
            temp = 1.0 + (self.vibrato.tick(sine_table()) * self.vibrato_gain);
            temp2 *= temp;
        }
        temp2
    }
}
