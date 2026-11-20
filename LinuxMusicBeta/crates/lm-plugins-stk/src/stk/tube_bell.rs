//! The tubular bells (`TubeBell` on `FM`): four operators in two pairs (`1` modulates `0`, `3` modulates `2` and
//! itself through a two-zero filter), crossfaded, with an amplitude vibrato.

use super::filters::{Adsr, FileLoop, SineWave, TwoZero, sine_table};
use super::{ONE_OVER_128, Rate, rawwaves};

const OPERATORS: usize = 4;

/// Tubular bells
#[derive(Clone, Copy, Debug)]
pub struct TubeBell {
    rate: Rate,
    adsr: [Adsr; OPERATORS],
    waves: [FileLoop; OPERATORS],
    vibrato: SineWave,
    twozero: TwoZero,
    base_frequency: f64,
    ratios: [f64; OPERATORS],
    gains: [f64; OPERATORS],
    mod_depth: f64,
    control1: f64,
    control2: f64,
    fm_gains: [f64; 100],
}

impl TubeBell {
    /// As the library builds one (`FM(4)`, then `TubeBell()`)
    pub fn new(rate: Rate) -> Self {
        let sr = rate.hz;
        let mut twozero = TwoZero::default();
        twozero.set_b2(-1.0);
        twozero.set_gain(0.0);
        let mut vibrato = SineWave::default();
        vibrato.set_frequency(6.0, sr);
        let mut fm_gains = [0.0; 100];
        let mut temp = 1.0;
        for i in (0..100).rev() {
            fm_gains[i] = temp;
            temp *= 0.933033;
        }
        let sine = FileLoop::open(rawwaves::sinewave(), sr);
        let mut bell = Self {
            rate,
            adsr: [Adsr::default(); OPERATORS],
            waves: [sine, sine, sine, FileLoop::open(rawwaves::fwavblnk(), sr)],
            vibrato,
            twozero,
            base_frequency: 440.0,
            ratios: [1.0; OPERATORS],
            gains: [1.0; OPERATORS],
            mod_depth: 0.0,
            control1: 1.0,
            control2: 1.0,
            fm_gains,
        };
        bell.set_ratio(0, 1.0 * 0.995);
        bell.set_ratio(1, 1.414 * 0.995);
        bell.set_ratio(2, 1.0 * 1.005);
        bell.set_ratio(3, 1.414 * 1.000);
        bell.gains = [bell.fm_gains[94], bell.fm_gains[76], bell.fm_gains[99], bell.fm_gains[71]];
        bell.adsr[0].set_all_times(0.005, 4.0, 0.0, 0.04, sr);
        bell.adsr[1].set_all_times(0.005, 4.0, 0.0, 0.04, sr);
        bell.adsr[2].set_all_times(0.001, 2.0, 0.0, 0.04, sr);
        bell.adsr[3].set_all_times(0.004, 4.0, 0.0, 0.04, sr);
        // (operator 3's feedback differentiates over two samples, a smaller step the higher the rate: scaled back)
        bell.twozero.set_gain(if rate.is_reference() { 0.5 } else { 0.5 / rate.k });
        bell.vibrato.set_frequency(2.0, sr);
        bell
    }

    fn set_ratio(&mut self, i: usize, ratio: f64) {
        self.ratios[i] = ratio;
        if ratio > 0.0 {
            self.waves[i].set_frequency(self.base_frequency * ratio, self.rate.hz);
        } else {
            self.waves[i].set_frequency(ratio, self.rate.hz);
        }
    }

    pub fn set_frequency(&mut self, frequency: f64) {
        self.base_frequency = frequency;
        for i in 0..OPERATORS {
            self.waves[i].set_frequency(self.base_frequency * self.ratios[i], self.rate.hz);
        }
    }

    /// `FM::controlChange` (2 modulator, 4 crossfade, 11 LFO speed, 1 LFO depth, 128 ADSR)
    pub fn control_change(&mut self, number: i32, value: f64) {
        let nv = value * ONE_OVER_128;
        match number {
            2 => self.control1 = nv * 2.0,
            4 => self.control2 = nv * 2.0,
            11 => self.vibrato.set_frequency(nv * 12.0, self.rate.hz),
            1 => self.mod_depth = nv,
            128 => {
                self.adsr[1].set_target(nv);
                self.adsr[3].set_target(nv);
            }
            _ => {}
        }
    }

    pub fn note_on(&mut self, frequency: f64, amplitude: f64) {
        self.gains[0] = amplitude * self.fm_gains[94];
        self.gains[1] = amplitude * self.fm_gains[76];
        self.gains[2] = amplitude * self.fm_gains[99];
        self.gains[3] = amplitude * self.fm_gains[71];
        self.set_frequency(frequency);
        for a in &mut self.adsr {
            a.key_on();
        }
    }

    #[inline]
    pub fn tick(&mut self) -> f64 {
        let mut temp = self.gains[1] * self.adsr[1].tick() * self.waves[1].tick();
        temp *= self.control1;
        self.waves[0].add_phase_offset(temp);
        self.waves[3].add_phase_offset(self.twozero.last_out());
        temp = self.gains[3] * self.adsr[3].tick() * self.waves[3].tick();
        self.twozero.tick(temp);
        self.waves[2].add_phase_offset(temp);
        temp = (1.0 - (self.control2 * 0.5)) * self.gains[0] * self.adsr[0].tick() * self.waves[0].tick();
        temp += self.control2 * 0.5 * self.gains[2] * self.adsr[2].tick() * self.waves[2].tick();
        let temp2 = self.vibrato.tick(sine_table()) * self.mod_depth;
        temp *= 1.0 + temp2;
        temp * 0.5
    }
}
