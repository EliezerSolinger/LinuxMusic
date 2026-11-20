//! The legacy program's Monstro (Vesa Kivimäki's): three oscillators (a band-limited pulse with sync out, a
//! multi-wave one, and one crossfading two waves), two envelopes and two LFOs, and a modulation matrix of volume,
//! phase, pitch, pulse width and sub mix; the second oscillator mixed with, or modulating, the third.

use std::sync::OnceLock;

use crate::band_limited_wave::{BandLimitedWaveform, MAX_TABLE, TABLE_LENGTHS, oscillate as bl, table, table_for};
use crate::kicker::fast_pow;
use crate::math::{LegacyRand, cosf, exp2f, expf, f32c, f64c, sin};
use crate::oscillator::{
    abs_fraction, exp_sample, lerp, moog_saw_sample, saw_sample, sin_sample, square_sample, triangle_sample,
};

const MODCLIP: f32 = 2.0;
const MIN_FREQ: f32 = 18.0;
const MAX_FREQ: f32 = 48000.0;
const INTEGRATOR: f32 = 3.0 / 7.0;
const FM_AMOUNT: f32 = 0.25;
/// %: the pulse width's range
pub const PW_MIN: f32 = 0.25;
pub const PW_MAX: f32 = 100.0 - PW_MIN;
/// The periods a period holds at most
pub const MAX_PERIOD: usize = 2048;

/// The oscillators' waves, in the order of their menus
pub mod wave {
    pub const SINE: usize = 0;
    pub const TRI: usize = 1;
    pub const SAW: usize = 2;
    pub const RAMP: usize = 3;
    pub const SQR: usize = 4;
    pub const MOOG: usize = 5;
    pub const SQRSOFT: usize = 6;
    pub const SINABS: usize = 7;
    pub const EXP: usize = 8;
    pub const NOISE: usize = 9;
    pub const TRI_D: usize = 10;
    pub const SAW_D: usize = 11;
    pub const RAMP_D: usize = 12;
    pub const SQR_D: usize = 13;
    pub const MOOG_D: usize = 14;
    /// The LFOs': in place of the noise, random steps and random smooth
    pub const RANDOM: usize = 9;
    pub const RANDOM_SMOOTH: usize = 10;
}

/// How the second oscillator works with the third
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum O23Mod {
    Mix,
    Am,
    Fm,
    Pm,
}

impl O23Mod {
    pub const ALL: [O23Mod; 4] = [O23Mod::Mix, O23Mod::Am, O23Mod::Fm, O23Mod::Pm];
}

/// A modulation's amounts from envelope 1, envelope 2, LFO 1 and LFO 2 (each -1 to 1)
pub type Mod = [f32; 4];

/// An envelope's knobs, ms (sustain 0 to 1, slope -1 to 1)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonstroEnvelope {
    pub pre: f32,
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub slope: f32,
}

/// An LFO's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonstroLfo {
    /// Index of its wave (see [`wave`])
    pub wave: usize,
    /// ms
    pub attack: f32,
    /// ms: its period
    pub rate: f32,
    /// Degrees
    pub phase: f32,
}

/// The Monstro's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonstroSettings {
    /// Each oscillator's volume (0 to 200) and panning (-100 to 100)
    pub volume: [f32; 3],
    pub pan: [f32; 3],
    /// Semitones
    pub coarse: [f32; 3],
    /// Cents, left and right, of oscillators 1 and 2
    pub fine: [[f32; 2]; 2],
    /// Degrees: the stereo phase offsets
    pub stereo_phase: [f32; 3],
    /// %
    pub pulse_width: f32,
    pub sync_on_rise: bool,
    pub sync_on_fall: bool,
    pub osc2_wave: usize,
    /// Hard and reverse sync of oscillators 2 and 3
    pub sync_hard: [bool; 2],
    pub sync_reverse: [bool; 2],
    pub osc3_waves: [usize; 2],
    /// -100 to 100
    pub osc3_sub: f32,
    pub lfos: [MonstroLfo; 2],
    pub envelopes: [MonstroEnvelope; 2],
    pub o23: O23Mod,
    pub vol_mod: [Mod; 3],
    pub phase_mod: [Mod; 3],
    pub pitch_mod: [Mod; 3],
    pub pw_mod: Mod,
    pub sub_mod: Mod,
}

impl Default for MonstroSettings {
    fn default() -> Self {
        let env =
            MonstroEnvelope { pre: 0.0, attack: 0.0, hold: 0.0, decay: 0.0, sustain: 1.0, release: 0.0, slope: 0.0 };
        let lfo = MonstroLfo { wave: 0, attack: 0.0, rate: 1.0, phase: 0.0 };
        Self {
            volume: [10.0; 3],
            pan: [0.0; 3],
            coarse: [0.0; 3],
            fine: [[0.0; 2]; 2],
            stereo_phase: [0.0; 3],
            pulse_width: 50.0,
            sync_on_rise: false,
            sync_on_fall: false,
            osc2_wave: 0,
            sync_hard: [false; 2],
            sync_reverse: [false; 2],
            osc3_waves: [0; 2],
            osc3_sub: 0.0,
            lfos: [lfo; 2],
            envelopes: [env; 2],
            o23: O23Mod::Mix,
            vol_mod: [[0.0; 4]; 3],
            phase_mod: [[0.0; 4]; 3],
            pitch_mod: [[0.0; 4]; 3],
            pw_mod: [0.0; 4],
            sub_mod: [0.0; 4],
        }
    }
}

/// The values the knobs make, worked out when they change (the legacy instrument's update slots)
#[derive(Clone, Copy, Debug)]
struct Derived {
    vol: [[f32; 2]; 3],
    freq: [[f32; 2]; 3],
    po: [[f32; 2]; 3],
    env_pre: [f32; 2],
    env_att: [f32; 2],
    env_hold: [f32; 2],
    env_dec: [f32; 2],
    env_rel: [f32; 2],
    slope: [f32; 2],
    lfo_att: [i64; 2],
    integrator: f32,
    fm_correction: f32,
    counter_max: i32,
}

fn left_ch(vol: f32, pan: f32) -> f32 {
    ((if pan <= 0.0 { 1.0 } else { 1.0 - (f64::from(pan) / 100.0) }) * f64::from(vol) / 100.0) as f32
}

fn right_ch(vol: f32, pan: f32) -> f32 {
    ((if pan >= 0.0 { 1.0 } else { 1.0 + (f64::from(pan) / 100.0) }) * f64::from(vol) / 100.0) as f32
}

fn derive(s: &MonstroSettings, sample_rate: u32) -> Derived {
    let sr = sample_rate as f32;
    let step = |ms: f32| if ms == 0.0 { 1.0 } else { 1.0 / (ms / 1000.0) / sr };
    let env = |f: fn(&MonstroEnvelope) -> f32| [step(f(&s.envelopes[0])), step(f(&s.envelopes[1]))];
    let po = |spo: f32| [spo / 720.0, ((-f64::from(spo)) / 720.0) as f32];
    let coarse = |c: f32| exp2f(c / 12.0);
    Derived {
        vol: core::array::from_fn(|i| [left_ch(s.volume[i], s.pan[i]), right_ch(s.volume[i], s.pan[i])]),
        freq: [
            [coarse(s.coarse[0]) * exp2f(s.fine[0][0] / 1200.0), coarse(s.coarse[0]) * exp2f(s.fine[0][1] / 1200.0)],
            [coarse(s.coarse[1]) * exp2f(s.fine[1][0] / 1200.0), coarse(s.coarse[1]) * exp2f(s.fine[1][1] / 1200.0)],
            [coarse(s.coarse[2]); 2],
        ],
        po: [po(s.stereo_phase[0]), po(s.stereo_phase[1]), po(s.stereo_phase[2])],
        env_pre: env(|e| e.pre),
        env_att: env(|e| e.attack),
        env_hold: env(|e| e.hold),
        env_dec: env(|e| e.decay),
        env_rel: env(|e| e.release),
        slope: [expf(f32c::LN_10 * -s.envelopes[0].slope), expf(f32c::LN_10 * -s.envelopes[1].slope)],
        lfo_att: [(s.lfos[0].attack * sr / 1000.0) as i64, (s.lfos[1].attack * sr / 1000.0) as i64],
        integrator: 0.5 - (0.5 - INTEGRATOR) * 44100.0 / sr,
        fm_correction: 44100.0 / sr * FM_AMOUNT,
        counter_max: (sample_rate as i32 * 5) / 44100,
    }
}

/// The scale of the band-limited saw in each table (its first harmonic, 1 in its sum, gives it back)
fn saw_scales() -> &'static [f32; MAX_TABLE + 1] {
    static SCALES: OnceLock<[f32; MAX_TABLE + 1]> = OnceLock::new();
    SCALES.get_or_init(|| {
        core::array::from_fn(|t| {
            let saw = table(BandLimitedWaveform::Saw, t);
            let length = TABLE_LENGTHS[t];
            let mut first = 0.0f64;
            for (ph, &s) in saw.iter().enumerate() {
                first += f64::from(s) * sin(2.0 * f64c::PI * ph as f64 / length as f64);
            }
            first *= 2.0 / length as f64;
            if first != 0.0 { (1.0 / first.abs()) as f32 } else { 1.0 }
        })
    })
}

/// A band-limited pulse, 1 from @p phase 0 to @p width and -1 after, of @p wavelength samples: two band-limited
/// saws a width apart, one from the other, and the pulse's mean added back
#[inline]
fn band_limited_pulse(phase: f32, width: f32, wavelength: f32) -> f32 {
    let t = table_for(wavelength);
    let saw = |at: f32| bl(abs_fraction(at), wavelength, BandLimitedWaveform::Saw);
    (saw(phase - width) - saw(phase)) * saw_scales()[t] * (2.0 / f32c::PI) + (2.0 * width - 1.0)
}

#[inline]
fn fraction(x: f32) -> f32 {
    x - x.trunc()
}

/// One note of the Monstro
#[derive(Clone, Debug)]
pub struct MonstroVoice {
    sample_rate: u32,
    d: Derived,
    settings: MonstroSettings,
    rand: LegacyRand,
    osc_phase: [[f32; 2]; 3],
    ph_last: [[f32; 2]; 2],
    env_phase: [f32; 2],
    lfo_phase: [f32; 2],
    lfo_last: [f32; 2],
    lfo_next: [f32; 2],
    osc1_last: [f32; 2],
    out_last: [f32; 2],
    invert: [[bool; 2]; 2],
    counter: [[i32; 2]; 2],
    lfo: Box<[[f32; MAX_PERIOD]; 2]>,
    env: Box<[[f32; MAX_PERIOD]; 2]>,
}

impl MonstroVoice {
    /// A note of @p settings at @p sample_rate; its randomness from @p seed
    pub fn new(settings: &MonstroSettings, sample_rate: u32, seed: u64) -> Self {
        saw_scales();
        let mut rand = LegacyRand::new(seed);
        let lfo_next = [-1.0 + rand.inclusive(2.0), -1.0 + rand.inclusive(2.0)];
        Self {
            sample_rate,
            d: derive(settings, sample_rate),
            settings: *settings,
            rand,
            osc_phase: [[0.0; 2]; 3],
            ph_last: [[0.0; 2]; 2],
            env_phase: [0.0; 2],
            lfo_phase: [0.0; 2],
            lfo_last: [0.0; 2],
            lfo_next,
            osc1_last: [0.0; 2],
            out_last: [0.0; 2],
            invert: [[false; 2]; 2],
            counter: [[0; 2]; 2],
            lfo: Box::new([[0.0; MAX_PERIOD]; 2]),
            env: Box::new([[0.0; MAX_PERIOD]; 2]),
        }
    }

    /// The knobs for the next period (what they make is worked out again when they change)
    pub fn set(&mut self, settings: &MonstroSettings) {
        if *settings != self.settings {
            self.settings = *settings;
            self.d = derive(settings, self.sample_rate);
        }
    }

    /// The longest release of the two envelopes, ms (at least 1.5): the note's
    pub fn release_ms(&self) -> f32 {
        self.settings.envelopes[0].release.max(self.settings.envelopes[1].release).max(1.5)
    }

    fn noise(&mut self) -> f32 {
        -1.0 + self.rand.inclusive(2.0)
    }

    fn osc(&mut self, w: usize, ph: f32, len: f32) -> f32 {
        match w {
            wave::SINE => sin_sample(ph),
            wave::TRI => bl(ph, len, BandLimitedWaveform::Triangle),
            wave::SAW => bl(ph, len, BandLimitedWaveform::Saw),
            wave::RAMP => -bl(ph, len, BandLimitedWaveform::Saw),
            wave::SQR => bl(ph, len, BandLimitedWaveform::Square),
            wave::SQRSOFT => soft_square(ph),
            wave::MOOG => bl(ph, len, BandLimitedWaveform::Moog),
            wave::SINABS => sin_sample(ph).abs(),
            wave::EXP => exp_sample(ph),
            wave::NOISE => self.noise(),
            wave::TRI_D => triangle_sample(ph),
            wave::SAW_D => saw_sample(ph),
            wave::RAMP_D => -saw_sample(ph),
            wave::SQR_D => square_sample(ph),
            wave::MOOG_D => moog_saw_sample(ph),
            _ => 0.0,
        }
    }

    fn calc_slope(&self, i: usize, s: f32) -> f32 {
        let slope = self.d.slope[i];
        if slope == 1.0 || s == 0.0 { s } else { fast_pow(f64::from(s), f64::from(slope)) as f32 }
    }

    /// The envelopes and LFOs of @p frames frames: the note @p played frames in, let go from frame @p release of
    /// these (if it is)
    fn update_modulators(&mut self, frames: usize, played: i64, release: Option<usize>) {
        let s = self.settings;
        for i in 0..2 {
            let rate = s.lfos[i].rate * 0.001 * self.sample_rate as f32;
            let inc = 1.0 / rate;
            for f in 0..frames {
                let ph = self.lfo_phase[i];
                self.lfo[i][f] = match s.lfos[i].wave {
                    wave::SINE => sin_sample(ph),
                    wave::TRI => triangle_sample(ph),
                    wave::SAW => saw_sample(ph),
                    wave::RAMP => -saw_sample(ph),
                    wave::SQR => square_sample(ph),
                    wave::SQRSOFT => soft_square(ph),
                    wave::MOOG => moog_saw_sample(ph),
                    wave::SINABS => sin_sample(ph).abs(),
                    wave::EXP => exp_sample(ph),
                    wave::RANDOM => {
                        if (played + f as i64) % i64::from(rate as i32) == 0 {
                            self.lfo_last[i] = self.noise();
                        }
                        self.lfo_last[i]
                    }
                    wave::RANDOM_SMOOTH => {
                        let tm = (played + f as i64) % i64::from(rate as i32);
                        if tm == 0 {
                            self.lfo_last[i] = self.lfo_next[i];
                            self.lfo_next[i] = self.noise();
                        }
                        let x = tm as f32 / rate;
                        let k = (1.0 - cosf(x * f32c::PI)) * 0.5;
                        k * (self.lfo_next[i] - self.lfo_last[i]) + self.lfo_last[i]
                    }
                    _ => self.lfo[i][f],
                };
                self.lfo_phase[i] += inc;
            }
            // The LFO's attack (by the frames played before this period, as the legacy program)
            let att = self.d.lfo_att[i] as i32;
            for f in 0..frames {
                if played + (f as i64) < i64::from(att) {
                    self.lfo[i][f] *= played as f32 / att as f32;
                }
            }

            let sus = s.envelopes[i].sustain;
            for f in 0..frames {
                let phase = &mut self.env_phase[i];
                if *phase < 4.0 && release.is_some_and(|r| f >= r) {
                    *phase = if *phase < 1.0 {
                        5.0
                    } else if *phase < 2.0 {
                        5.0 - fraction(*phase)
                    } else if *phase < 3.0 {
                        4.0
                    } else {
                        4.0 + fraction(*phase)
                    };
                }
                let p = *phase;
                let value;
                if p < 1.0 {
                    value = 0.0;
                    self.env_phase[i] = (p + self.d.env_pre[i]).min(1.0);
                } else if p < 2.0 {
                    value = self.calc_slope(i, fraction(p));
                    self.env_phase[i] = (p + self.d.env_att[i]).min(2.0);
                } else if p < 3.0 {
                    value = 1.0;
                    self.env_phase[i] = (p + self.d.env_hold[i]).min(3.0);
                } else if p < 4.0 {
                    let v = self.calc_slope(i, 1.0 - fraction(p));
                    if v <= sus {
                        value = sus;
                    } else {
                        value = v;
                        let mut next = (p + self.d.env_dec[i]).min(4.0 - sus);
                        // (no sustain: on past the release, which would click)
                        if next == 4.0 {
                            next = 5.0;
                        }
                        self.env_phase[i] = next;
                    }
                } else if p < 5.0 {
                    value = self.calc_slope(i, 1.0 - fraction(p));
                    self.env_phase[i] = p + self.d.env_rel[i];
                } else {
                    value = 0.0;
                }
                self.env[i][f] = value;
            }
        }
    }

    /// The next @p out frames (at most [`MAX_PERIOD`]) of the note at @p frequency, @p played frames in, let go
    /// from frame @p release of these if it is (its fade-in is the host's)
    pub fn render(&mut self, out: &mut [[f32; 2]], frequency: f32, played: i64, release: Option<usize>) {
        let s = self.settings;
        let d = self.d;
        let frames = out.len().min(MAX_PERIOD);
        let sr = self.sample_rate as f32;

        let lfo_po = [s.lfos[0].phase / 360.0, s.lfos[1].phase / 360.0];
        for i in 0..2 {
            self.lfo_phase[i] = abs_fraction(self.lfo_phase[i] + lfo_po[i]);
        }
        self.update_modulators(frames, played, release);

        let half = |m: Mod| [m[0], m[1], m[2] * 0.5, m[3] * 0.5];
        let pitch = |m: Mod| [m[0] * 2.0, m[1] * 2.0, m[2], m[3]];
        let any = |m: &Mod| m.iter().any(|v| *v != 0.0);
        let pw_m = half(s.pw_mod);
        let ph_m = [half(s.phase_mod[0]), half(s.phase_mod[1]), half(s.phase_mod[2])];
        let pit_m = [pitch(s.pitch_mod[0]), pitch(s.pitch_mod[1]), pitch(s.pitch_mod[2])];
        let vol_m = s.vol_mod;
        let sub_m = half(s.sub_mod);
        let o3sub = (s.osc3_sub + 100.0) / 200.0;
        let base = [
            [d.freq[0][0] * frequency, d.freq[0][1] * frequency],
            [d.freq[1][0] * frequency, d.freq[1][1] * frequency],
            [d.freq[2][0] * frequency; 2],
        ];

        let mut p = [[0.0f32; 2]; 3];
        for o in 0..3 {
            for c in 0..2 {
                p[o][c] = self.osc_phase[o][c] + d.po[o][c];
            }
        }

        for f in 0..frames {
            let (e1, e2, l1, l2) = (self.env[0][f], self.env[1][f], self.lfo[0][f], self.lfo[1][f]);
            let modfreq = |car: f32, m: &Mod| {
                let mut t = 0.0f32;
                if m[0] != 0.0 {
                    t += e1 * m[0];
                }
                if m[1] != 0.0 {
                    t += e2 * m[1];
                }
                if m[2] != 0.0 {
                    t += l1 * m[2];
                }
                if m[3] != 0.0 {
                    t += l2 * m[3];
                }
                (car * exp2f(t)).clamp(MIN_FREQ, MAX_FREQ)
            };
            let modabs = |mut car: f32, m: &Mod| {
                if m[0] != 0.0 {
                    car += e1 * m[0];
                }
                if m[1] != 0.0 {
                    car += e2 * m[1];
                }
                if m[2] != 0.0 {
                    car += l1 * m[2];
                }
                if m[3] != 0.0 {
                    car += l2 * m[3];
                }
                car
            };
            let modvol = |mut car: f32, m: &Mod| {
                if m[0] > 0.0 {
                    car *= 1.0 - m[0] + m[0] * e1;
                }
                if m[0] < 0.0 {
                    car *= 1.0 + m[0] * e1;
                }
                if m[1] > 0.0 {
                    car *= 1.0 - m[1] + m[1] * e2;
                }
                if m[1] < 0.0 {
                    car *= 1.0 + m[1] * e2;
                }
                if m[2] != 0.0 {
                    car *= 1.0 + m[2] * l1;
                }
                if m[3] != 0.0 {
                    car *= 1.0 + m[3] * l2;
                }
                car.clamp(-MODCLIP, MODCLIP)
            };

            // Oscillator 1: the pulse
            let mut o1f = base[0];
            if any(&pit_m[0]) {
                o1f = [modfreq(o1f[0], &pit_m[0]), modfreq(o1f[1], &pit_m[0])];
            }
            let mut pw = s.pulse_width * 0.01;
            if any(&pw_m) {
                pw = modabs(pw, &pw_m);
            }
            {
                // (never under a sample high or low)
                let one = o1f[0].max(o1f[1]) / sr;
                let lowest = (PW_MIN * 0.01).max(one);
                let highest = (PW_MAX * 0.01).min(1.0 - one);
                pw = if lowest < highest { pw.clamp(lowest, highest) } else { 0.5 };
            }
            let mut ph = p[0];
            if any(&ph_m[0]) {
                ph = [modabs(ph[0], &ph_m[0]), modabs(ph[1], &ph_m[0])];
            }
            let steps = [
                if abs_fraction(ph[0]) < pw { 1.0f32 } else { -1.0 },
                if abs_fraction(ph[1]) < pw { 1.0f32 } else { -1.0 },
            ];
            // Sync out, on the pulse's rise or fall
            for c in 0..2 {
                let rose = steps[c] > self.osc1_last[c];
                let fell = steps[c] < self.osc1_last[c];
                let edge = (s.sync_on_rise && rose) || (s.sync_on_fall && fell);
                for (k, o) in [1usize, 2].into_iter().enumerate() {
                    let on_rise = s.sync_on_rise && rose;
                    let on_fall = s.sync_on_fall && fell;
                    let hits = [on_rise, on_fall].into_iter().filter(|h| *h).count();
                    if s.sync_hard[k] && edge {
                        p[o][c] = d.po[o][c];
                        self.counter[k][c] = d.counter_max;
                    }
                    if s.sync_reverse[k] {
                        for _ in 0..hits {
                            self.invert[k][c] = !self.invert[k][c];
                            self.counter[k][c] = d.counter_max;
                        }
                    }
                }
            }
            self.osc1_last = steps;
            let mut o1 = [
                band_limited_pulse(ph[0], pw, self.sample_rate as f32 / o1f[0]),
                band_limited_pulse(ph[1], pw, self.sample_rate as f32 / o1f[1]),
            ];
            o1[0] *= d.vol[0][0];
            o1[1] *= d.vol[0][1];
            if any(&vol_m[0]) {
                o1 = [modvol(o1[0], &vol_m[0]), modvol(o1[1], &vol_m[0])];
            }
            for c in 0..2 {
                p[0][c] += 1.0 / (sr / o1f[c]);
            }

            // Oscillator 2
            let mut o2f = base[1];
            if any(&pit_m[1]) {
                o2f = [modfreq(o2f[0], &pit_m[1]), modfreq(o2f[1], &pit_m[1])];
            }
            let mut ph = p[1];
            if any(&ph_m[1]) {
                ph = [modabs(ph[0], &ph_m[1]), modabs(ph[1], &ph_m[1])];
            }
            ph = [abs_fraction(ph[0]), abs_fraction(ph[1])];
            let mut o2 = [0.0f32; 2];
            for c in 0..2 {
                let mut pd = (ph[c] - self.ph_last[0][c]).abs();
                if pd > 0.5 {
                    pd = (1.0 - f64::from(pd)) as f32;
                }
                if pd != 0.0 {
                    let mut len = 1.0 / pd;
                    if self.counter[0][c] > 0 {
                        len /= self.counter[0][c] as f32;
                        self.counter[0][c] -= 1;
                    }
                    o2[c] = self.osc(s.osc2_wave, ph[c], len);
                }
            }
            o2[0] *= d.vol[1][0];
            o2[1] *= d.vol[1][1];
            if any(&vol_m[1]) {
                o2 = [modvol(o2[0], &vol_m[1]), modvol(o2[1], &vol_m[1])];
            }
            for c in 0..2 {
                if self.invert[0][c] {
                    o2[c] = -o2[c];
                }
            }
            self.ph_last[0] = ph;
            for c in 0..2 {
                p[1][c] += 1.0 / (sr / o2f[c]);
            }

            // Oscillator 3
            let mut o3f = base[2];
            if any(&pit_m[2]) {
                o3f = [modfreq(o3f[0], &pit_m[2]), modfreq(o3f[1], &pit_m[2])];
            }
            let mut ph = p[2];
            if any(&ph_m[2]) {
                ph = [modabs(ph[0], &ph_m[2]), modabs(ph[1], &ph_m[2])];
            }
            if s.o23 == O23Mod::Pm {
                ph[0] += o2[0] * 0.5;
                ph[1] += o2[1] * 0.5;
            }
            ph = [abs_fraction(ph[0]), abs_fraction(ph[1])];
            let mut o3a = [0.0f32; 2];
            let mut o3b = [0.0f32; 2];
            for c in 0..2 {
                let mut pd = (ph[c] - self.ph_last[1][c]).abs();
                if pd > 0.5 {
                    pd = (1.0 - f64::from(pd)) as f32;
                }
                if pd != 0.0 {
                    let mut len = 1.0 / pd;
                    if self.counter[1][c] > 0 {
                        len /= self.counter[1][c] as f32;
                        self.counter[1][c] -= 1;
                    }
                    o3a[c] = self.osc(s.osc3_waves[0], ph[c], len);
                    o3b[c] = self.osc(s.osc3_waves[1], ph[c], len);
                }
            }
            let mut sub = o3sub;
            if any(&sub_m) {
                sub = modabs(sub, &sub_m).clamp(0.0, 1.0);
            }
            let mut o3 = [lerp(o3a[0], o3b[0], sub), lerp(o3a[1], o3b[1], sub)];
            o3[0] *= d.vol[2][0];
            o3[1] *= d.vol[2][1];
            if any(&vol_m[2]) {
                o3 = [modvol(o3[0], &vol_m[2]), modvol(o3[1], &vol_m[2])];
            }
            if s.o23 == O23Mod::Am {
                for c in 0..2 {
                    o3[c] = (o3[c] * (1.0 + o2[c]).max(0.0)).clamp(-MODCLIP, MODCLIP);
                }
            }
            for c in 0..2 {
                if self.invert[1][c] {
                    o3[c] = -o3[c];
                }
            }
            self.ph_last[1] = ph;
            for c in 0..2 {
                let mut len = 1.0 / (sr / o3f[c]);
                // (FM as PM)
                if s.o23 == O23Mod::Fm {
                    len += o2[c] * d.fm_correction;
                }
                p[2][c] += len;
            }

            // The integrator: a very simple filter
            for c in 0..2 {
                let x = o1[c] + o3[c] + if s.o23 == O23Mod::Mix { o2[c] } else { 0.0 };
                out[f][c] = lerp(x, self.out_last[c], d.integrator);
                self.out_last[c] = x;
            }
        }

        for o in 0..3 {
            for c in 0..2 {
                self.osc_phase[o][c] = abs_fraction(p[o][c] - d.po[o][c]);
            }
        }
        for i in 0..2 {
            self.lfo_phase[i] = abs_fraction(self.lfo_phase[i] - lfo_po[i]);
        }
    }
}

/// The soft square: a sine's rise and fall at its edges
#[inline]
fn soft_square(x: f32) -> f32 {
    let ph = fraction(x);
    if f64::from(ph) < 0.1 {
        sin_sample((f64::from(ph * 5.0) + 0.75) as f32)
    } else if f64::from(ph) < 0.5 {
        1.0
    } else if f64::from(ph) < 0.6 {
        sin_sample((f64::from(ph * 5.0) + 0.75) as f32)
    } else {
        -1.0
    }
}
