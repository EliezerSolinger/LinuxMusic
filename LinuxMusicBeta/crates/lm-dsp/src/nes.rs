//! The legacy program's Nescaline (Vesa Kivimäki's): a NES-like voice of two pulse channels, a triangle and an
//! LFSR noise, with the console's envelopes, sweeps and vibrato, its mixing and its slight output distortion.

use crate::math::{LegacyRand, exp2f, powf};
use crate::oscillator::lerp;

const NES_SIMPLE_FILTER: f32 = 1.0 / 20.0;
const NFB: f32 = 895000.0;
const NOISE_FREQS: [f32; 16] = [
    NFB / 5.0,
    NFB / 9.0,
    NFB / 17.0,
    NFB / 33.0,
    NFB / 65.0,
    NFB / 97.0,
    NFB / 129.0,
    NFB / 161.0,
    NFB / 193.0,
    NFB / 255.0,
    NFB / 381.0,
    NFB / 509.0,
    NFB / 763.0,
    NFB / 1017.0,
    NFB / 2035.0,
    NFB / 4069.0,
];
const DUTY_CYCLE: [f32; 4] = [0.125, 0.25, 0.5, 0.75];
const DITHER_AMP: f32 = 1.0 / 60.0;
const MIN_FREQ: f32 = 10.0;
const TRIANGLE: [i32; 32] =
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0];
const NES_DIST: f32 = 0.9;
const MIXING_12: f32 = 1.0 / 20.0;
const MIXING_34: f32 = 1.0 / 12.0;
const MIN_WLEN: i32 = 4;
/// The release, ms
pub const RELEASE_MS: f32 = 0.2;

/// A pulse channel's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NesPulse {
    pub enabled: bool,
    /// Semitones, -24 to 24
    pub coarse: f32,
    /// 0 to 15
    pub volume: f32,
    pub env_enabled: bool,
    pub env_looped: bool,
    /// 0 to 15
    pub env_length: f32,
    /// 0 to 3: 12.5, 25, 50 or 75 %
    pub duty: usize,
    pub sweep_enabled: bool,
    /// -7 to 7
    pub sweep_amount: f32,
    /// 0 to 7
    pub sweep_rate: f32,
}

/// The Nescaline's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NesSettings {
    pub pulses: [NesPulse; 2],
    pub triangle_enabled: bool,
    pub triangle_coarse: f32,
    pub triangle_volume: f32,
    pub noise_enabled: bool,
    pub noise_volume: f32,
    pub noise_env_enabled: bool,
    pub noise_env_looped: bool,
    pub noise_env_length: f32,
    /// The short (93-step) noise
    pub noise_mode: bool,
    /// The noise's pitch follows the note
    pub noise_freq_mode: bool,
    /// 0 to 15
    pub noise_freq: f32,
    /// -7 to 7
    pub noise_sweep: f32,
    /// The note's noise pitch quantized to the console's
    pub noise_quantize: bool,
    /// 0 to 2
    pub master: f32,
    /// 0 to 15
    pub vibrato: f32,
}

impl Default for NesSettings {
    fn default() -> Self {
        let pulse = |duty| NesPulse {
            enabled: true,
            coarse: 0.0,
            volume: 15.0,
            env_enabled: false,
            env_looped: false,
            env_length: 0.0,
            duty,
            sweep_enabled: false,
            sweep_amount: 0.0,
            sweep_rate: 0.0,
        };
        Self {
            pulses: [pulse(0), pulse(2)],
            triangle_enabled: true,
            triangle_coarse: 0.0,
            triangle_volume: 15.0,
            noise_enabled: false,
            noise_volume: 15.0,
            noise_env_enabled: false,
            noise_env_looped: false,
            noise_env_length: 0.0,
            noise_mode: false,
            noise_freq_mode: false,
            noise_freq: 0.0,
            noise_sweep: 0.0,
            noise_quantize: true,
            master: 1.0,
            vibrato: 0.0,
        }
    }
}

#[inline]
fn signed_powf(v: f32, e: f32) -> f32 {
    powf(v.abs(), e) * if v >= 0.0 { 1.0 } else { -1.0 }
}

fn nearest_noise_freq(f: f32) -> usize {
    let mut n = 15;
    for i in (0..16).rev() {
        if f >= NOISE_FREQS[i] {
            n = i;
        }
    }
    n
}

/// One note of the Nescaline
#[derive(Clone, Debug)]
pub struct NesVoice {
    sample_rate: u32,
    rand: LegacyRand,
    pitch_counter: i32,
    pitch_period: i32,
    counters: [i32; 4],
    env_counters: [i32; 3],
    env_values: [i32; 3],
    sweep_counters: [i32; 3],
    lfsr: u16,
    last_12: f32,
    last_34: f32,
    last_note_freq: f32,
    last_noise_freq: f32,
    max_wlen: i32,
    nsf: f32,
    wlen: [i32; 4],
    vibrato_phase: usize,
}

impl NesVoice {
    /// A note at @p frequency; the dither's randomness from @p seed
    pub fn new(s: &NesSettings, frequency: f32, sample_rate: u32, seed: u64) -> Self {
        let mut v = Self {
            sample_rate,
            rand: LegacyRand::new(seed),
            pitch_counter: 0,
            pitch_period: 0,
            counters: [0; 4],
            env_counters: [0; 3],
            env_values: [15; 3],
            sweep_counters: [0; 3],
            lfsr: 1,
            last_12: 0.0,
            last_34: 0.0,
            last_note_freq: 0.0,
            last_noise_freq: -1.0,
            max_wlen: 0,
            nsf: (f64::from(NES_SIMPLE_FILTER) * (f64::from(sample_rate) / 44100.0)) as f32,
            wlen: [0; 4],
            vibrato_phase: 0,
        };
        v.pitch_period = v.wavelength(60.0);
        v.max_wlen = v.wavelength(MIN_FREQ);
        v.update_pitch(s, frequency);
        v
    }

    #[inline]
    fn wavelength(&self, freq: f32) -> i32 {
        (self.sample_rate as f32 / freq) as i32
    }

    fn update_lfsr(&mut self, mode: bool) {
        let x = if mode { (self.lfsr & (1 << 8)) << 6 } else { (self.lfsr & (1 << 13)) << 1 };
        self.lfsr ^= x;
        self.lfsr <<= 1;
        if self.lfsr & (1 << 15) != 0 {
            self.lfsr += 1;
        }
    }

    fn update_pitch(&mut self, s: &NesSettings, note: f32) {
        let mut freq = note;
        if s.vibrato > 0.0 {
            let amount = s.vibrato.floor() / 15.0;
            self.vibrato_phase = (self.vibrato_phase + 1) % 32;
            freq *= 1.0 + (TRIANGLE[self.vibrato_phase] as f32 * 0.02 * amount);
        }
        if freq != self.last_note_freq {
            self.wlen[0] = self.wavelength(freq * exp2f(s.pulses[0].coarse / 12.0));
            self.wlen[1] = self.wavelength(freq * exp2f(s.pulses[1].coarse / 12.0));
            self.wlen[2] = self.wavelength(freq * exp2f(s.triangle_coarse / 12.0));
        }
        if s.noise_freq_mode && freq != self.last_note_freq {
            let mut f = freq * 2.0;
            if s.noise_quantize {
                f = NOISE_FREQS[nearest_noise_freq(f)];
            }
            self.wlen[3] = self.wavelength(f);
        }
        if !s.noise_freq_mode && self.last_noise_freq != s.noise_freq {
            self.wlen[3] = self.wavelength(NOISE_FREQS[15 - s.noise_freq as usize]);
            self.last_noise_freq = s.noise_freq;
        }
        self.last_note_freq = freq;
    }

    fn dither(&mut self) -> f32 {
        -1.0 + self.rand.inclusive(2.0)
    }

    /// The next @p out frames of the note at @p frequency (its release: [`crate::instrument::apply_release`])
    pub fn render(&mut self, out: &mut [[f32; 2]], s: &NesSettings, frequency: f32) {
        let env_len = |x: f32| self.wavelength(((240.0 / f64::from(x + 1.0)) as f32).floor());
        let env_lens = [env_len(s.pulses[0].env_length), env_len(s.pulses[1].env_length), env_len(s.noise_env_length)];
        let sweep_rate = |x: f32| self.wavelength(((120.0 / f64::from(x + 1.0)) as f32).floor());
        let sweep_rates = [
            sweep_rate(s.pulses[0].sweep_rate),
            sweep_rate(s.pulses[1].sweep_rate),
            self.wavelength((60.0 / (8.0 - s.noise_sweep.abs())).floor()),
        ];
        let sweep = |amount: f32| {
            let n = -f64::from(amount) as i32;
            if n > 0 {
                8 - n
            } else if n < 0 {
                -8 - n
            } else {
                0
            }
        };
        let sweeps = [sweep(s.pulses[0].sweep_amount), sweep(s.pulses[1].sweep_amount)];
        let noise_sweep = if s.noise_sweep == 0.0 {
            0
        } else if s.noise_sweep > 0.0 {
            -1
        } else {
            1
        };

        for frame in out.iter_mut() {
            self.pitch_counter += 1;
            if self.pitch_counter >= self.pitch_period {
                self.update_pitch(s, frequency);
                self.pitch_counter = 0;
            }
            // The two pulses
            let mut pulse = [0i32; 2];
            let mut pulse_level = [0i32; 2];
            for c in 0..2 {
                let p = &s.pulses[c];
                let w = self.wlen[c];
                if w <= self.max_wlen && w >= MIN_WLEN && p.enabled {
                    pulse_level[c] = if p.env_enabled {
                        (f64::from(p.volume * self.env_values[c] as f32) / 15.0) as i32
                    } else {
                        p.volume as i32
                    };
                    pulse[c] =
                        if self.counters[c] as f32 > w as f32 * DUTY_CYCLE[p.duty.min(3)] { 0 } else { pulse_level[c] };
                }
                self.sweep_counters[c] += 1;
                if self.sweep_counters[c] >= sweep_rates[c] {
                    self.sweep_counters[c] = 0;
                    if p.sweep_enabled && w <= self.max_wlen && w >= MIN_WLEN {
                        if sweeps[c] > 0 {
                            self.wlen[c] += self.wlen[c] >> sweeps[c].abs();
                        }
                        if sweeps[c] < 0 {
                            self.wlen[c] -= self.wlen[c] >> sweeps[c].abs();
                            // (one less on the first channel only, as the console)
                            if c == 0 {
                                self.wlen[c] -= 1;
                            }
                        }
                    }
                }
                self.counters[c] += 1;
                self.counters[c] = if self.wlen[c] != 0 { self.counters[c] % self.wlen[c] } else { 0 };
                self.env_counters[c] += 1;
                if self.env_counters[c] >= env_lens[c] {
                    self.env_counters[c] = 0;
                    self.env_values[c] -= 1;
                    if self.env_values[c] < 0 {
                        self.env_values[c] = if p.env_looped { 15 } else { 0 };
                    }
                }
            }

            // The triangle
            let w3 = self.wlen[2];
            self.counters[2] = if w3 != 0 { self.counters[2] % w3 } else { 0 };
            let (tri, tri_level) = if w3 <= self.max_wlen && s.triangle_enabled {
                let level = s.triangle_volume as i32;
                let t = if w3 != 0 { TRIANGLE[((self.counters[2] * 32) / w3) as usize] } else { 0 };
                ((t * level) / 15, level)
            } else {
                (0, 0)
            };
            self.counters[2] += 1;

            // The noise
            let (noise, noise_level) = if s.noise_enabled {
                let level = if s.noise_env_enabled {
                    (s.noise_volume as i32 * self.env_values[2]) / 15
                } else {
                    s.noise_volume as i32
                };
                (if self.lfsr & (1 << 14) != 0 { level } else { 0 }, level)
            } else {
                (0, 0)
            };
            self.counters[3] += 1;
            if self.counters[3] >= self.wlen[3] {
                self.counters[3] = 0;
                self.update_lfsr(s.noise_mode);
            }
            self.env_counters[2] += 1;
            if self.env_counters[2] >= env_lens[2] {
                self.env_counters[2] = 0;
                self.env_values[2] -= 1;
                if self.env_values[2] < 0 {
                    self.env_values[2] = if s.noise_env_looped { 15 } else { 0 };
                }
            }
            self.sweep_counters[2] += 1;
            if self.sweep_counters[2] >= sweep_rates[2] {
                self.sweep_counters[2] = 0;
                if noise_sweep != 0 {
                    let current = nearest_noise_freq(self.sample_rate as f32 / self.wlen[3] as f32) as i32;
                    let mut n = (current + noise_sweep).clamp(0, 15) as usize;
                    self.wlen[3] = self.wavelength(NOISE_FREQS[n]);
                    // (a sweep up stuck on no wavelength goes on to the next that has one)
                    if self.wlen[3] == 0 && noise_sweep == 1 {
                        while self.wlen[3] == 0 && n < 15 {
                            n += 1;
                            self.wlen[3] = self.wavelength(NOISE_FREQS[n]);
                        }
                    }
                }
            }

            // The console's mixing: two pins, each its own low-pass and distortion
            let mut pin1 = (pulse[0] + pulse[1]) as f32;
            pin1 = (f64::from(pin1) * (1.0 + f64::from(self.dither() * DITHER_AMP))) as f32;
            pin1 /= 30.0;
            pin1 = signed_powf(pin1, NES_DIST);
            pin1 = pin1 * 2.0 - 1.0;
            pin1 = lerp(pin1, self.last_12, self.nsf);
            self.last_12 = pin1;
            pin1 += 1.0 - signed_powf((pulse_level[0] + pulse_level[1]) as f32 / 30.0, NES_DIST);
            pin1 *= MIXING_12;

            let mut pin2 = (tri + noise) as f32;
            pin2 = (f64::from(pin2) * (1.0 + f64::from(self.dither() * DITHER_AMP))) as f32;
            pin2 /= 30.0;
            pin2 = signed_powf(pin2, NES_DIST);
            pin2 = pin2 * 2.0 - 1.0;
            pin2 = lerp(pin2, self.last_34, self.nsf);
            self.last_34 = pin2;
            pin2 += 1.0 - signed_powf((tri_level + noise_level) as f32 / 30.0, NES_DIST);
            pin2 *= MIXING_34;

            let mix = (pin1 + pin2) * (1.0 / (MIXING_12 + MIXING_34)) * s.master;
            *frame = [mix, mix];
        }
    }
}
