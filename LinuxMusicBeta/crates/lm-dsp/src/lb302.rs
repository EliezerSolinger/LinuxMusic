//! The legacy program's LB302: a monophonic imitation of the TB-303. One oscillator (naive or band-limited shapes),
//! a resonant filter (a 2-pole IIR with distortion, or a 3-pole "24 dB" ladder) with its decaying envelope, an
//! amplifier envelope, and slides between notes. Which note plays is the host's: it starts notes and says where the
//! held note ends.

use crate::band_limited_wave::{BandLimitedWaveform, oscillate};
use crate::kicker::SoftDistortion;
use crate::math::{cosf, expf, f32c::PI, powf, tanhf};
use crate::oscillator::{NoiseRng, exp_sample, sin_sample};

/// Frames between the filter envelope's steps
const ENV_INC: u32 = 64;
const DIST_RATIO: f32 = 4.0;
const VCA_ATTACK: f32 = 1.0 - 0.96406088;
const VCA_INITIAL: f32 = 0.5;

/// The oscillator's shapes, in the order of the legacy knob
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VcoShape {
    Sawtooth,
    Triangle,
    Square,
    RoundSquare,
    Moog,
    Sine,
    Exponential,
    WhiteNoise,
    BlSawtooth,
    BlSquare,
    BlTriangle,
    BlMoog,
}

impl VcoShape {
    pub const ALL: [VcoShape; 12] = [
        VcoShape::Sawtooth,
        VcoShape::Triangle,
        VcoShape::Square,
        VcoShape::RoundSquare,
        VcoShape::Moog,
        VcoShape::Sine,
        VcoShape::Exponential,
        VcoShape::WhiteNoise,
        VcoShape::BlSawtooth,
        VcoShape::BlSquare,
        VcoShape::BlTriangle,
        VcoShape::BlMoog,
    ];
}

/// The LB302's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lb302Settings {
    /// 0 to 1.5
    pub cutoff: f32,
    /// 0 to 1.25
    pub resonance: f32,
    /// 0 to 1
    pub env_mod: f32,
    /// 0 to 1
    pub decay: f32,
    /// 0 to 1
    pub distortion: f32,
    pub shape: VcoShape,
    /// 0 to 1
    pub slide_decay: f32,
    pub slide: bool,
    /// A note that does not restart the amplifier or the filter envelope (when it is still sounding)
    pub dead: bool,
    /// The 3-pole filter
    pub db24: bool,
}

impl Default for Lb302Settings {
    fn default() -> Self {
        Self {
            cutoff: 0.75,
            resonance: 0.75,
            env_mod: 0.1,
            decay: 0.1,
            distortion: 0.0,
            shape: VcoShape::BlSawtooth,
            slide_decay: 0.6,
            slide: false,
            dead: false,
            db24: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct KnobState {
    cutoff: f32,
    reso: f32,
    envmod: f32,
    envdecay: f32,
    dist: f32,
}

/// The envelope shared by both filters
#[derive(Clone, Copy, Debug, Default)]
struct Vcf {
    c0: f32,
    e: [f32; 2],
    res_coeff: f32,
}

#[derive(Clone, Copy, Debug)]
struct Iir2 {
    vcf: Vcf,
    d: [f32; 2],
    a: f32,
    b: f32,
    c: f32,
    dist: SoftDistortion,
}

#[derive(Clone, Copy, Debug, Default)]
struct ThreePole {
    vcf: Vcf,
    kfcn: f32,
    kp: f32,
    kp1h: f32,
    kres: f32,
    ay: [f32; 3],
    lastin: f32,
    value: f32,
}

fn base_recalc(vcf: &mut Vcf, fs: &KnobState, sample_rate: f32) {
    vcf.e[1] = expf(6.109 + 1.5876 * fs.envmod + 2.1553 * fs.cutoff - 1.2 * (1.0 - fs.reso));
    vcf.e[0] = expf(5.613 - 0.8 * fs.envmod + 2.1553 * fs.cutoff - 0.7696 * (1.0 - fs.reso));
    let pi_sr = PI / sample_rate;
    vcf.e[0] *= pi_sr;
    vcf.e[1] *= pi_sr;
    vcf.e[1] -= vcf.e[0];
    vcf.res_coeff = expf(-1.20 + 3.455 * fs.reso);
}

impl Iir2 {
    fn recalc(&mut self, fs: &KnobState, sample_rate: f32) {
        base_recalc(&mut self.vcf, fs, sample_rate);
        self.dist.threshold = fs.dist * 75.0;
    }

    fn env_recalc(&mut self, fs: &KnobState) {
        self.vcf.c0 *= fs.envdecay;
        let w = self.vcf.e[0] + self.vcf.c0;
        let k = expf(-w / self.vcf.res_coeff);
        self.a = 2.0 * cosf(2.0 * w) * k;
        self.b = -k * k;
        self.c = 1.0 - self.a - self.b;
    }

    fn process(&mut self, x: f32, fs: &KnobState) -> f32 {
        let mut ret = self.a * self.d[0] + self.b * self.d[1] + self.c * x;
        self.d[1] = self.d[0];
        self.d[0] = ret;
        if fs.dist > 0.0 {
            ret = self.dist.next(ret);
        }
        ret
    }

    fn reset(&mut self, fs: &KnobState) {
        self.vcf.c0 = 0.0;
        self.d = [0.0; 2];
        self.env_recalc(fs);
    }
}

impl ThreePole {
    fn recalc(&mut self) {
        self.vcf.e[0] = 0.000001;
        self.vcf.e[1] = 1.0;
    }

    fn env_recalc(&mut self, fs: &KnobState, sample_rate: f32) {
        self.vcf.c0 *= fs.envdecay;
        let w = self.vcf.e[0] + self.vcf.c0;
        let k = fs.cutoff.min(0.975);
        // (44.1 kHz on purpose: the cutoff's behavior does not change with the rate)
        const SAMPLE_RATE_CUTOFF: f32 = 44100.0;
        let kfco = 50.0
            + k * ((2300.0 - 1600.0 * fs.envmod)
                + w * (700.0 + 1500.0 * k + (1500.0 + k * (SAMPLE_RATE_CUTOFF / 2.0 - 6000.0)) * fs.envmod));
        self.kfcn = 2.0 * kfco / sample_rate;
        self.kp = ((-2.7528 * self.kfcn + 3.0429) * self.kfcn + 1.718) * self.kfcn - 0.9984;
        let kp1 = self.kp + 1.0;
        self.kp1h = 0.5 * kp1;
        self.kres = fs.reso * (((-2.7079 * kp1 + 10.963) * kp1 - 14.934) * kp1 + 8.4974);
        self.value = 1.0 + (fs.dist * (1.5 + 2.0 * self.kres * (1.0 - self.kfcn)));
    }

    fn process(&mut self, x: f32, fs: &KnobState) -> f32 {
        let ax1 = self.lastin;
        let ay1 = [self.ay[0], self.ay[1]];
        self.lastin = x - tanhf(self.kres * self.ay[2]);
        self.ay[0] = self.kp1h * (self.lastin + ax1) - self.kp * self.ay[0];
        self.ay[1] = self.kp1h * (self.ay[0] + ay1[0]) - self.kp * self.ay[1];
        self.ay[2] = self.kp1h * (self.ay[1] + ay1[1]) - self.kp * self.ay[2];
        tanhf(self.ay[2] * self.value) * 3.0 / (1.0 + fs.dist)
    }

    fn reset(&mut self, fs: &KnobState, sample_rate: f32) {
        self.vcf.c0 = 0.0;
        self.ay = [0.0; 3];
        self.lastin = 0.0;
        self.env_recalc(fs, sample_rate);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VcaMode {
    Attack,
    Decay,
    Idle,
    NeverPlayed,
}

/// The legacy `panningToVolumeVector`: each side's gain for @p pan (-100 to 100) at @p volume
pub fn panning_to_volume(pan: f32, volume: f32) -> [f32; 2] {
    let mut v = [volume; 2];
    let pf = pan / 100.0;
    v[if pan >= 0.0 { 0 } else { 1 }] *= 1.0 - pf.abs();
    v
}

/// The LB302 (one per instrument: it is monophonic)
#[derive(Clone, Debug)]
pub struct Lb302 {
    sample_rate: f32,
    knobs: Lb302Settings,
    fs: KnobState,
    iir2: Iir2,
    three_pole: ThreePole,
    vco_inc: f32,
    vco_k: f32,
    vco_c: f32,
    new_freq: bool,
    true_freq: f32,
    slide: f32,
    slide_inc: f32,
    slide_base: f32,
    vca: f32,
    vca_mode: VcaMode,
    vcf_env_pos: u32,
    noise: NoiseRng,
}

impl Lb302 {
    /// At @p sample_rate, its knobs at their defaults (the first [`Self::prepare_period`] sets them); @p seed is
    /// the white noise's
    pub fn new(sample_rate: f32, seed: u64) -> Self {
        let fs = KnobState::default();
        let mut three_pole = ThreePole::default();
        three_pole.env_recalc(&fs, sample_rate);
        let mut s = Self {
            sample_rate,
            knobs: Lb302Settings::default(),
            fs,
            iir2: Iir2 {
                vcf: Vcf::default(),
                d: [0.0; 2],
                a: 0.0,
                b: 0.0,
                c: 1.0,
                dist: SoftDistortion { threshold: 1.0, gain: 1.0 },
            },
            three_pole,
            vco_inc: 0.0,
            vco_k: 0.0,
            vco_c: 0.0,
            new_freq: false,
            true_freq: 0.0,
            slide: 0.0,
            slide_inc: 0.0,
            slide_base: 0.0,
            vca: 0.0,
            vca_mode: VcaMode::NeverPlayed,
            vcf_env_pos: ENV_INC,
            noise: NoiseRng::new(seed),
        };
        s.filter_changed();
        s.decay_changed();
        s
    }

    fn filter_changed(&mut self) {
        let k = &self.knobs;
        self.fs.cutoff = k.cutoff;
        self.fs.reso = k.resonance;
        self.fs.envmod = k.env_mod;
        self.fs.dist = k.distortion * DIST_RATIO;
        self.recalc_filter();
    }

    fn decay_changed(&mut self) {
        let d = (2.3 * self.knobs.decay + 0.2) * self.sample_rate;
        self.fs.envdecay = powf(0.1, 1.0 / d * ENV_INC as f32);
    }

    fn recalc_filter(&mut self) {
        if self.knobs.db24 {
            self.three_pole.recalc();
        } else {
            self.iir2.recalc(&self.fs, self.sample_rate);
        }
        self.vcf_env_pos = ENV_INC;
    }

    /// The knobs for the next period: what changed reaches the filter
    pub fn prepare_period(&mut self, s: &Lb302Settings) {
        let old = self.knobs;
        self.knobs = *s;
        let filter = old.cutoff != s.cutoff
            || old.resonance != s.resonance
            || old.env_mod != s.env_mod
            || old.distortion != s.distortion;
        if filter {
            self.filter_changed();
        } else if old.db24 != s.db24 {
            self.recalc_filter();
        }
        if old.decay != s.decay {
            self.decay_changed();
        }
    }

    pub fn reset(&mut self) {
        self.vco_inc = 0.0;
        self.vco_k = 0.0;
        self.vco_c = 0.0;
        self.new_freq = false;
        self.true_freq = 0.0;
        self.slide = 0.0;
        self.slide_inc = 0.0;
        self.slide_base = 0.0;
        self.vca = 0.0;
        self.vca_mode = VcaMode::NeverPlayed;
        self.vcf_env_pos = ENV_INC;
        self.iir2.reset(&self.fs);
        self.three_pole.reset(&self.fs, self.sample_rate);
    }

    /// A new note at @p frequency (it plays from the next [`Self::render`])
    pub fn note_on(&mut self, frequency: f32) {
        self.new_freq = true;
        self.true_freq = frequency;
        let inc = frequency / self.sample_rate;
        if self.knobs.slide {
            self.slide_base = inc;
        } else {
            self.vco_inc = inc;
        }
    }

    /// The playing note's pitch moves to @p frequency (a bend, a tuning) without starting it again: the legacy
    /// synth read its note's frequency each period
    pub fn set_frequency(&mut self, frequency: f32) {
        self.true_freq = frequency;
        let inc = frequency / self.sample_rate;
        if self.knobs.slide {
            self.slide_base = inc;
        } else {
            self.vco_inc = inc;
        }
    }

    /// A period into @p out: the note held until frame @p release_frame (decaying from there; while @p playing is
    /// false it decays from the start, unless a note starts), at the side gains @p volume
    pub fn render(&mut self, out: &mut [[f32; 2]], release_frame: usize, playing: bool, volume: [f32; 2]) {
        if release_frame == 0 || !playing {
            self.vca_mode = VcaMode::Decay;
        }
        let db24 = self.knobs.db24;
        if self.new_freq {
            self.new_freq = false;
            let dead = self.knobs.dead;
            self.vco_inc = self.true_freq / self.sample_rate;
            // A note restarts the amplifier, a dead one only once it has decayed
            self.vca_mode = if !dead || matches!(self.vca_mode, VcaMode::Decay | VcaMode::NeverPlayed) {
                VcaMode::Attack
            } else {
                VcaMode::Idle
            };
            if self.slide_inc != 0.0 {
                // A slide from the last note
                self.slide = self.vco_inc - self.slide_inc;
                self.slide_base = self.vco_inc;
                self.slide_inc = 0.0;
            } else {
                self.slide = 0.0;
            }
            if self.knobs.slide {
                self.slide_inc = self.vco_inc;
            }
            self.recalc_filter();
            if !dead {
                if db24 {
                    self.three_pole.vcf.c0 = self.three_pole.vcf.e[1];
                } else {
                    self.iir2.vcf.c0 = self.iir2.vcf.e[1];
                }
                self.vcf_env_pos = ENV_INC;
            }
        }

        let gate = 1.0f32 / 65536.0;
        let decay = powf(gate, 1.0 / (0.245260770975 * self.sample_rate));
        let sample_ratio = 44100.0 / self.sample_rate;
        for (i, frame) in out.iter_mut().enumerate() {
            if i >= release_frame {
                self.vca_mode = VcaMode::Decay;
            }
            if self.vcf_env_pos >= ENV_INC {
                if db24 {
                    self.three_pole.env_recalc(&self.fs, self.sample_rate);
                } else {
                    self.iir2.env_recalc(&self.fs);
                }
                self.vcf_env_pos = 0;
                if self.slide != 0.0 {
                    self.vco_inc = self.slide_base - self.slide;
                    self.slide -= self.slide * (0.1 - self.knobs.slide_decay * 0.0999) * sample_ratio;
                }
            }
            self.vcf_env_pos += 1;

            self.vco_c += self.vco_inc;
            if self.vco_c > 0.5 {
                self.vco_c -= 1.0;
            }
            let c = self.vco_c;
            let bl = |wave| {
                if self.vco_inc == 0.0 { 0.0 } else { oscillate(c + 0.5, 1.0 / self.vco_inc, wave) }
            };
            self.vco_k = match self.knobs.shape {
                VcoShape::Sawtooth => c,
                VcoShape::Triangle => {
                    let k = c * 2.0 + 0.5;
                    if k > 0.5 { 1.0 - k } else { k }
                }
                VcoShape::Square => {
                    if c < 0.0 {
                        0.5
                    } else {
                        -0.5
                    }
                }
                VcoShape::RoundSquare => {
                    if c < 0.0 {
                        (1.0 - (c * c * 4.0)).sqrt() - 0.5
                    } else {
                        -0.5
                    }
                }
                VcoShape::Moog => {
                    let mut k = c * 2.0 + 0.5;
                    if k > 1.0 {
                        k = -0.5;
                    } else if k > 0.5 {
                        let w = 2.0 * (k - 0.5) - 1.0;
                        k = 0.5 - (1.0 - (w * w)).sqrt();
                    }
                    k * 2.0
                }
                VcoShape::Sine => 0.5 * sin_sample(c),
                VcoShape::Exponential => 0.5 * exp_sample(c),
                VcoShape::WhiteNoise => 0.5 * self.noise.sample(),
                VcoShape::BlSawtooth => bl(BandLimitedWaveform::Saw) * 0.5,
                VcoShape::BlSquare => bl(BandLimitedWaveform::Square) * 0.5,
                VcoShape::BlTriangle => bl(BandLimitedWaveform::Triangle) * 0.5,
                VcoShape::BlMoog => bl(BandLimitedWaveform::Moog),
            };

            let filtered = if db24 {
                self.three_pole.process(self.vco_k, &self.fs)
            } else {
                self.iir2.process(self.vco_k, &self.fs)
            };
            let s = filtered * self.vca;
            *frame = [s * volume[0], s * volume[1]];

            match self.vca_mode {
                VcaMode::Attack => self.vca += (VCA_INITIAL - self.vca) * VCA_ATTACK,
                VcaMode::Decay => {
                    self.vca *= decay;
                    if self.vca < gate {
                        self.vca = 0.0;
                        self.vca_mode = VcaMode::NeverPlayed;
                    }
                }
                _ => {}
            }
        }
    }
}
