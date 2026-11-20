//! The banded waveguides (`BandedWG`): one delay line and band-pass per mode, plucked or bowed. Four presets
//! (uniform bar, tuned bar, glass, Tibetan bowl).
//!
//! At another rate than 44.1 kHz each line is as long in time as the library makes it at 44.1 kHz (a whole number of
//! samples there, a fraction here): the library's lengths are whole samples at the instance's rate, so its modes
//! were tuned, and the top ones left out, differently at each rate (up to 17 dB and 30 cents apart).
//!
//! The library's lines are 4096 samples long, which at 44.1 kHz holds any note above 10.8 Hz; a lower note could not
//! set its first line (a warning) and played at the line's previous length. Here the frequency is held at the lowest
//! the first line takes at 44.1 kHz ([`BandedWG::lowest_frequency`]), and each line holds, at any rate, the longest
//! delay its mode needs above that.

use super::filters::{Adsr, BiQuad, BowTable, DelayL};
use super::{ONE_OVER_128, PI, Rate};

/// `MAX_BANDED_MODES`
pub const MAX_MODES: usize = 20;
/// The library's longest delay (`DelayL`'s default `maxDelay`), at 44.1 kHz
const LIBRARY_MAX_DELAY: f64 = 4095.0;
/// The highest frequency (`setFrequency`)
const HIGHEST: f64 = 1568.0;

/// A preset's modes: their ratios, their loop gains and their excitations
#[derive(Clone, Copy, Debug, PartialEq)]
struct Preset {
    modes: usize,
    ratios: [f64; MAX_MODES],
    basegains: [f64; MAX_MODES],
    excitation: [f64; MAX_MODES],
}

/// `BandedWG::setPreset`'s values
fn preset(preset: i32) -> Preset {
    let mut p =
        Preset { modes: 4, ratios: [0.0; MAX_MODES], basegains: [0.0; MAX_MODES], excitation: [0.0; MAX_MODES] };
    let pow = lm_dsp::math::pow;
    match preset {
        1 => {
            // Tuned bar
            p.modes = 4;
            p.ratios[..4].copy_from_slice(&[1.0, 4.0198391420, 10.7184986595, 18.0697050938]);
            for i in 0..p.modes {
                p.basegains[i] = pow(0.999, i as f64 + 1.0);
                p.excitation[i] = 1.0;
            }
        }
        2 => {
            // Glass harmonica
            p.modes = 5;
            p.ratios[..5].copy_from_slice(&[1.0, 2.32, 4.25, 6.63, 9.38]);
            for i in 0..p.modes {
                p.basegains[i] = pow(0.999, i as f64 + 1.0);
                p.excitation[i] = 1.0;
            }
        }
        3 => {
            // Tibetan prayer bowl
            p.modes = 12;
            #[rustfmt::skip]
            let (r, g, e): ([f64; 12], [f64; 12], [f64; 12]) = (
                [0.996108344, 1.0038916562, 2.979178, 2.99329767, 5.704452, 5.704452, 8.9982, 9.01549726, 12.83303,
                 12.807382, 17.2808219, 21.97602739726],
                [0.999925960128219, 0.999925960128219, 0.999982774366897, 0.999982774366897, 1.0, 1.0, 1.0, 1.0,
                 0.999965497558225, 0.999965497558225, 0.9999999999999999999965497558225,
                 0.999999999999999965497558225],
                [11.900357 / 10.0, 11.900357 / 10., 10.914886 / 10., 10.914886 / 10., 42.995041 / 10.,
                 42.995041 / 10., 40.063034 / 10., 40.063034 / 10., 7.063034 / 10., 7.063034 / 10.,
                 57.063034 / 10., 57.063034 / 10.],
            );
            p.ratios[..12].copy_from_slice(&r);
            p.basegains[..12].copy_from_slice(&g);
            p.excitation[..12].copy_from_slice(&e);
        }
        _ => {
            // Uniform bar
            p.modes = 4;
            p.ratios[..4].copy_from_slice(&[1.0, 2.756, 5.404, 8.933]);
            for i in 0..p.modes {
                p.basegains[i] = pow(0.9, i as f64 + 1.0);
                p.excitation[i] = 1.0;
            }
        }
    }
    p
}

/// A mode's line: the library's `DelayL` at 44.1 kHz; at another rate a whole-sample line and a first-order
/// all-pass for the fraction, its delay exact at the mode's frequency (the line's linear interpolation would damp
/// the high modes on every pass: up to 1.3 dB per pass for an 8 kHz mode at 48 kHz)
#[derive(Debug)]
struct Line {
    delay: DelayL,
    /// The line's length as the library makes it at 44.1 kHz (whole samples)
    reference: f64,
    allpass: bool,
    eta: f64,
    x1: f64,
    y1: f64,
    last: f64,
}

impl Clone for Line {
    fn clone(&self) -> Self {
        Self { delay: self.delay.clone(), ..*self }
    }

    fn clone_from(&mut self, source: &Self) {
        self.delay.clone_from(&source.delay);
        let delay = std::mem::replace(&mut self.delay, DelayL::empty());
        *self = Self { delay, ..*source };
    }
}

impl Line {
    fn new(capacity: usize) -> Self {
        Self { delay: DelayL::new(capacity), reference: 0.0, allpass: false, eta: 0.0, x1: 0.0, y1: 0.0, last: 0.0 }
    }

    /// The library's `setDelay(length)` and `clear()` at 44.1 kHz; at `rate`, the same time for a mode at `freq` Hz.
    /// False (the line kept, cleared) when it does not fit
    fn set_and_clear(&mut self, length: f64, rate: Rate, freq: f64) -> bool {
        self.x1 = 0.0;
        self.y1 = 0.0;
        self.last = 0.0;
        if rate.is_reference() || rate.k > 1.0 {
            self.allpass = false;
            let ok = self.delay.set_delay_and_clear(length);
            if ok {
                self.reference = length;
            } else {
                self.delay.clear();
            }
            return ok;
        }
        // (the loop is the line and one sample more: the line's output is read a sample later)
        let total = (length + 1.0) / rate.k - 1.0;
        let whole = (total - 0.618).floor().max(0.0);
        let d = total - whole;
        let w = super::TWO_PI * freq / rate.hz;
        let den = lm_dsp::math::sin((1.0 + d) * w / 2.0);
        if !self.delay.set_delay_and_clear(whole) {
            self.delay.clear();
            return false;
        }
        self.reference = length;
        self.allpass = true;
        self.eta = if den.abs() > 1.0e-12 { lm_dsp::math::sin((1.0 - d) * w / 2.0) / den } else { 0.0 };
        true
    }

    fn last_out(&self) -> f64 {
        self.last
    }

    #[inline]
    fn tick(&mut self, input: f64) -> f64 {
        let v = self.delay.tick(input);
        self.last = if self.allpass {
            let y = self.eta * v + self.x1 - self.eta * self.y1;
            self.x1 = v;
            self.y1 = y;
            y
        } else {
            v
        };
        self.last
    }
}

/// A banded waveguide
#[derive(Debug)]
pub struct BandedWG {
    rate: Rate,
    do_pluck: bool,
    track_velocity: bool,
    modes: usize,
    preset: Preset,
    bow_table: BowTable,
    adsr: Adsr,
    bandpass: [BiQuad; MAX_MODES],
    delay: Box<[Line]>,
    max_velocity: f64,
    frequency: f64,
    base_gain: f64,
    gains: [f64; MAX_MODES],
    integration_constant: f64,
    velocity_input: f64,
    bow_velocity: f64,
    bow_target: f64,
}

impl Clone for BandedWG {
    fn clone(&self) -> Self {
        Self { delay: self.delay.clone(), ..*self }
    }

    /// Without allocating when both have lines of the same capacities (the voices of one instrument)
    fn clone_from(&mut self, source: &Self) {
        if self.delay.len() != source.delay.len() {
            *self = source.clone();
            return;
        }
        for (d, s) in self.delay.iter_mut().zip(source.delay.iter()) {
            d.clone_from(s);
        }
        let delay = std::mem::take(&mut self.delay);
        *self = Self { delay, ..*source };
    }
}

impl BandedWG {
    /// The lowest frequency a preset plays (see the module): its first line at the library's longest delay at
    /// 44.1 kHz
    pub fn lowest_frequency(preset_index: i32) -> f64 {
        let p = preset(preset_index);
        super::REFERENCE_RATE / (LIBRARY_MAX_DELAY * p.ratios[0])
    }

    /// The delay each line may need at `rate`, over every preset, from its lowest frequency
    fn capacities(rate: Rate) -> [usize; MAX_MODES] {
        let mut c = [2usize; MAX_MODES];
        for index in 0..4 {
            let p = preset(index);
            let lowest = Self::lowest_frequency(index);
            for (c, ratio) in c.iter_mut().zip(&p.ratios[..p.modes]) {
                let longest = (rate.hz / lowest / ratio).floor() as usize;
                *c = (*c).max(longest + 2);
            }
        }
        c
    }

    /// As the legacy Mallets builds one (`BandedWG()`, with the bow at rest: `BandedWGAtRest`), its lines allocated
    pub fn new(rate: Rate) -> Self {
        let capacities = Self::capacities(rate);
        let mut adsr = Adsr::default();
        adsr.set_all_times(0.02, 0.005, 0.9, 0.01, rate.hz);
        let mut bow_table = BowTable::default();
        bow_table.set_slope(3.0);
        let mut wg = Self {
            rate,
            do_pluck: true,
            track_velocity: false,
            modes: 4,
            preset: preset(0),
            bow_table,
            adsr,
            bandpass: [BiQuad::default(); MAX_MODES],
            delay: capacities.iter().map(|&c| Line::new(c)).collect(),
            max_velocity: 0.0,
            frequency: 220.0,
            base_gain: 0.999,
            gains: [0.0; MAX_MODES],
            integration_constant: 0.0,
            velocity_input: 0.0,
            bow_velocity: 0.0,
            bow_target: 0.0,
        };
        wg.set_preset(0);
        wg
    }

    /// Back to a new voice's state (the library's constructor), without allocating
    pub fn reset_from(&mut self, fresh: &Self) {
        self.clone_from(fresh);
    }

    pub fn set_preset(&mut self, index: i32) {
        self.preset = preset(index);
        self.modes = self.preset.modes;
        self.set_frequency(self.frequency);
    }

    pub fn set_frequency(&mut self, frequency: f64) {
        let lowest = super::REFERENCE_RATE / (LIBRARY_MAX_DELAY * self.preset.ratios[0]);
        let mut frequency = frequency.max(lowest);
        if frequency > HIGHEST {
            frequency = HIGHEST;
        }
        // (the lines are whole samples long at 44.1 kHz, as the library makes them there, and as long in time at
        // any rate: the same modes, tuned the same, with the same ones left out at the top)
        let base = super::REFERENCE_RATE.min(self.rate.hz) / frequency;
        for i in 0..self.preset.modes {
            let length = ((base / self.preset.ratios[i]) as i32) as f64;
            if length > 2.0 {
                // (a line that cannot take it keeps its length, as the library's warning does)
                self.delay[i].set_and_clear(length, self.rate, frequency * self.preset.ratios[i]);
                self.gains[i] = self.preset.basegains[i];
            } else {
                self.modes = i;
                break;
            }
            let mut radius = 1.0 - PI * 32.0 / self.rate.hz;
            if radius < 0.0 {
                radius = 0.0;
            }
            self.bandpass[i].set_resonance(frequency * self.preset.ratios[i], radius, true, self.rate.hz);
            self.bandpass[i].clear();
        }
    }

    fn start_bowing(&mut self, amplitude: f64, rate: f64) {
        // (the attack's rate is per sample: scaled to last as long at any rate)
        self.adsr.set_attack_rate(self.rate.per_sample(rate));
        self.adsr.key_on();
        self.max_velocity = 0.03 + (0.1 * amplitude);
    }

    fn pluck(&mut self, amplitude: f64) {
        if self.modes == 0 {
            return;
        }
        // (the pulse is as long in time as the library's at 44.1 kHz)
        let min_len = self.delay[self.modes - 1].reference;
        for i in 0..self.modes {
            let n = (self.delay[i].reference / min_len) as i32;
            let n = if self.rate.is_reference() { n } else { (f64::from(n) / self.rate.k).round() as i32 };
            for _ in 0..n {
                self.delay[i].tick(self.preset.excitation[i] * amplitude / self.modes as f64);
            }
        }
    }

    pub fn note_on(&mut self, frequency: f64, amplitude: f64) {
        self.set_frequency(frequency);
        if self.do_pluck {
            self.pluck(amplitude);
        } else {
            self.start_bowing(amplitude, amplitude * 0.001);
        }
    }

    /// `controlChange` (2 pressure, 128 bow speed, 1 loop gain, 11 integration, 64 bowed, 65 velocity tracking,
    /// 16 preset; 4 and 8 are not sent by the legacy Mallets and not ported)
    pub fn control_change(&mut self, number: i32, value: f64) {
        let nv = value * ONE_OVER_128;
        match number {
            2 => {
                if nv == 0.0 {
                    self.do_pluck = true;
                } else {
                    self.do_pluck = false;
                    self.bow_table.set_slope(10.0 - (9.0 * nv));
                }
            }
            128 => {
                self.track_velocity = false;
                self.max_velocity = 0.13 * nv;
                self.adsr.set_target(nv);
            }
            1 => {
                self.base_gain = 0.8999999999999999 + (0.1 * nv);
                for i in 0..self.modes {
                    self.gains[i] = self.preset.basegains[i] * self.base_gain;
                }
            }
            11 => self.integration_constant = nv,
            64 => self.do_pluck = value < 65.0,
            65 => self.track_velocity = value >= 65.0,
            16 => self.set_preset(value as i32),
            _ => {}
        }
    }

    #[inline]
    pub fn tick(&mut self) -> f64 {
        let mut input = 0.0;
        if !self.do_pluck {
            let c = self.integration_constant;
            // (the bow's velocity integrator leaks per sample: at another rate, the same time constant and the same
            // gain at low frequencies)
            let (leak, scale) = if self.rate.is_reference() || c == 0.0 || c >= 1.0 {
                (c, 1.0)
            } else {
                let leak = lm_dsp::math::pow(c, self.rate.k);
                (leak, (1.0 - leak) / (1.0 - c))
            };
            if c == 0.0 {
                self.velocity_input = 0.0;
            } else {
                self.velocity_input *= leak;
            }
            for k in 0..self.modes {
                if scale == 1.0 {
                    self.velocity_input += self.base_gain * self.delay[k].last_out();
                } else {
                    self.velocity_input += scale * (self.base_gain * self.delay[k].last_out());
                }
            }
            if self.track_velocity {
                self.bow_velocity *= 0.9995;
                self.bow_velocity += self.bow_target;
                self.bow_target *= 0.995;
            } else {
                self.bow_velocity = self.adsr.tick() * self.max_velocity;
            }
            input = self.bow_velocity - self.velocity_input;
            input *= self.bow_table.tick(input);
            input /= self.modes as f64;
        }
        let mut data = 0.0;
        for k in 0..self.modes {
            self.bandpass[k].tick(input + self.gains[k] * self.delay[k].last_out());
            self.delay[k].tick(self.bandpass[k].last_out());
            data += self.bandpass[k].last_out();
        }
        data * 4.0
    }
}
