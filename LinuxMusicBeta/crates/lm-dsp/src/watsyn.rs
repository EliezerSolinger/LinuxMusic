//! The legacy program's Watsyn (Vesa Kivimäki's): four drawn waves in two pairs (A2 into A1, B2 into B1: mixed,
//! amplitude, ring or phase modulated), with crosstalk from A1 to B2, and an A-B mix with its envelope.
//!
//! The waves are drawn as 220 points and played oversampled 32 times by a sinc (the legacy program's libsamplerate):
//! the host makes that oversampling ([`oversample`], with its resampler) away from the audio thread.

use crate::drawn_wave::{DrawnWave, Hold, Table};
use crate::math::exp2f;
use crate::oscillator::lerp;

/// The points of a drawn wave
pub const GRAPH_LENGTH: usize = 220;
/// Its oversampling
pub const WAVE_RATIO: usize = 32;
/// The points of a wave as it plays
pub const WAVE_LENGTH: usize = GRAPH_LENGTH * WAVE_RATIO;
const PMOD_AMT: i32 = (WAVE_LENGTH / 2) as i32;
/// The release, ms
pub const RELEASE_MS: f32 = 1.5;

/// The oscillators
pub const A1: usize = 0;
pub const A2: usize = 1;
pub const B1: usize = 2;
pub const B2: usize = 3;

/// How the second wave of a pair works with the first
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatsynMod {
    Mix,
    Am,
    Rm,
    Pm,
}

impl WatsynMod {
    pub const ALL: [WatsynMod; 4] = [WatsynMod::Mix, WatsynMod::Am, WatsynMod::Rm, WatsynMod::Pm];
}

/// The 220 points of a drawn wave oversampled 32 times, through @p resample: a resampler at a ratio of 32, which
/// reads frames from its input and writes to its output, and says how many of each (the legacy `srccpy`: the points
/// read round and round until the output is full)
pub fn oversample(
    points: &[f32; GRAPH_LENGTH],
    mut resample: impl FnMut(&[f32], &mut [f32]) -> (usize, usize),
) -> Vec<f32> {
    let mut out = vec![0.0f32; WAVE_LENGTH];
    let (mut src, mut dst) = (0usize, 0usize);
    while dst < WAVE_LENGTH {
        let (used, generated) = resample(&points[src..], &mut out[dst..]);
        src = (src + used) % GRAPH_LENGTH;
        dst += generated;
    }
    out
}

/// The four waves the notes play (oversampled), and their band-limited tables with anti-aliasing on
#[derive(Clone, Debug)]
pub struct WatsynWaves {
    waves: [Vec<f32>; 4],
    drawn: Option<[DrawnWave; 4]>,
}

impl WatsynWaves {
    /// From the four oversampled waves (each [`WAVE_LENGTH`] points); made away from the audio thread
    pub fn new(waves: [Vec<f32>; 4], anti_aliasing: bool) -> Self {
        // (up to the 128th harmonic: the oversampling leaves nothing above the 110th)
        let drawn = anti_aliasing.then(|| core::array::from_fn(|i| DrawnWave::new(&waves[i], Hold::Linear, 128)));
        Self { waves, drawn }
    }
}

/// The Watsyn's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WatsynSettings {
    /// 0 to 200, per oscillator (A1, A2, B1, B2)
    pub volume: [f32; 4],
    /// -100 to 100
    pub pan: [f32; 4],
    /// 1 to 24: the frequency times this over 8
    pub multiplier: [f32; 4],
    /// Cents, -600 to 600
    pub left_tune: [f32; 4],
    pub right_tune: [f32; 4],
    /// -100 (A) to 100 (B)
    pub ab_mix: f32,
    /// -200 to 200: how far the envelope moves the mix
    pub env_amount: f32,
    /// ms
    pub env_attack: f32,
    pub env_hold: f32,
    pub env_decay: f32,
    /// 0 to 100: A1 into B2
    pub crosstalk: f32,
    pub a_mod: WatsynMod,
    pub b_mod: WatsynMod,
}

impl Default for WatsynSettings {
    fn default() -> Self {
        Self {
            volume: [32.0; 4],
            pan: [0.0; 4],
            multiplier: [8.0; 4],
            left_tune: [0.0; 4],
            right_tune: [0.0; 4],
            ab_mix: 0.0,
            env_amount: 0.0,
            env_attack: 0.0,
            env_hold: 0.0,
            env_decay: 0.0,
            crosstalk: 0.0,
            a_mod: WatsynMod::Mix,
            b_mod: WatsynMod::Mix,
        }
    }
}

fn left_ch(vol: f32, pan: f32) -> f32 {
    ((if pan <= 0.0 { 1.0 } else { 1.0 - (f64::from(pan) / 100.0) }) * f64::from(vol) / 100.0) as f32
}

fn right_ch(vol: f32, pan: f32) -> f32 {
    ((if pan >= 0.0 { 1.0 } else { 1.0 + (f64::from(pan) / 100.0) }) * f64::from(vol) / 100.0) as f32
}

/// One note of the Watsyn
#[derive(Clone, Debug)]
pub struct WatsynVoice {
    sample_rate: u32,
    /// The pairs' modulations, as the note started
    a_mod: WatsynMod,
    b_mod: WatsynMod,
    phase: [[f32; 4]; 2],
}

impl WatsynVoice {
    pub fn new(settings: &WatsynSettings, sample_rate: u32) -> Self {
        Self { sample_rate, a_mod: settings.a_mod, b_mod: settings.b_mod, phase: [[0.0; 4]; 2] }
    }

    /// The next @p out frames of the note at @p frequency, @p played frames in (its release:
    /// [`crate::instrument::apply_release`])
    pub fn render(
        &mut self,
        out: &mut [[f32; 2]],
        waves: &WatsynWaves,
        s: &WatsynSettings,
        frequency: f32,
        played: u64,
    ) {
        let vol: [[f32; 4]; 2] = [
            core::array::from_fn(|i| left_ch(s.volume[i], s.pan[i])),
            core::array::from_fn(|i| right_ch(s.volume[i], s.pan[i])),
        ];
        let freq: [[f32; 4]; 2] = [
            core::array::from_fn(|i| (s.multiplier[i] / 8.0) * exp2f(s.left_tune[i] / 1200.0)),
            core::array::from_fn(|i| (s.multiplier[i] / 8.0) * exp2f(s.right_tune[i] / 1200.0)),
        ];
        let rate = self.sample_rate as f32;
        let tables: [[Option<&Table>; 4]; 2] = match &waves.drawn {
            Some(drawn) => [
                core::array::from_fn(|i| drawn[i].table_for(frequency * freq[0][i], rate)),
                core::array::from_fn(|i| drawn[i].table_for(frequency * freq[1][i], rate)),
            ],
            None => [[None; 4]; 2],
        };
        let banded = waves.drawn.is_some();
        let wave = |osc: usize, side: usize, phase: f32| -> f32 {
            if banded {
                return tables[side][osc].map_or(0.0, |t| DrawnWave::at(t, phase * (1.0 / WAVE_LENGTH as f32)));
            }
            let points = &waves.waves[osc];
            lerp(
                points[phase as i32 as usize % WAVE_LENGTH],
                points[(phase + 1.0) as i32 as usize % WAVE_LENGTH],
                phase - phase.trunc(),
            )
        };
        let wl = WAVE_LENGTH as f32;
        let pm = |phase: f32, by: f32| {
            let mut p = (phase + by * PMOD_AMT as f32) % wl;
            if p < 0.0 {
                p += wl;
            }
            p
        };
        let combine = |m: WatsynMod, first: f32, second: f32| match m {
            WatsynMod::Mix => (f64::from(first + second) / 2.0) as f32,
            WatsynMod::Am => first * (second + 1.0).max(0.0),
            WatsynMod::Rm => first * second,
            WatsynMod::Pm => first,
        };

        // The mix and its envelope
        let att = (s.env_attack * rate) / 1000.0;
        let hold = (s.env_hold * rate) / 1000.0;
        let dec = (s.env_decay * rate) / 1000.0;
        let env_len = att + dec + hold;
        let tfp0 = played as f32;
        let enveloped = s.env_amount != 0.0 && tfp0 < env_len;

        for (f, frame) in out.iter_mut().enumerate() {
            let mut a = [0.0f32; 2];
            let mut b = [0.0f32; 2];
            for side in 0..2 {
                let ph = self.phase[side];
                // A: A2 into A1
                let a2 = vol[side][A2] * wave(A2, side, ph[A2]);
                let a1_phase = if self.a_mod == WatsynMod::Pm { pm(ph[A1], a2) } else { ph[A1] };
                let a1 = vol[side][A1] * wave(A1, side, a1_phase);
                // B: B2 (with A1's crosstalk) into B1
                let mut b2 = vol[side][B2] * wave(B2, side, ph[B2]);
                if s.crosstalk > 0.0 {
                    b2 += (a1 * s.crosstalk) * 0.01;
                }
                let b1_phase = if self.b_mod == WatsynMod::Pm { pm(ph[B1], b2) } else { ph[B1] };
                let b1 = vol[side][B1] * wave(B1, side, b1_phase);
                a[side] = combine(self.a_mod, a1, a2);
                b[side] = combine(self.b_mod, b1, b2);
            }
            for side in 0..2 {
                for i in 0..4 {
                    self.phase[side][i] += wl / (rate / (frequency * freq[side][i]));
                    self.phase[side][i] %= wl;
                }
            }

            let mut mix = s.ab_mix;
            if enveloped {
                let tfp = tfp0 + f as f32;
                mix = if tfp < att {
                    (mix + (tfp / att * s.env_amount)).clamp(-100.0, 100.0)
                } else if tfp >= att && tfp < att + hold {
                    (mix + s.env_amount).clamp(-100.0, 100.0)
                } else {
                    (mix + s.env_amount - ((tfp - (att + hold)) / dec * s.env_amount)).clamp(-100.0, 100.0)
                };
            }
            let bmix = ((f64::from(mix) + 100.0) / 200.0) as f32;
            let amix = (1.0 - f64::from(bmix)) as f32;
            *frame = [(a[0] * amix) + (b[0] * bmix), (a[1] * amix) + (b[1] * bmix)];
        }
    }
}
