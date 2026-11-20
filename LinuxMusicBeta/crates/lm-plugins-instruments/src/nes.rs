//! Nescaline (F-INS-010: legacy `plugins/Nes`): a NES-like voice of two pulse channels, a triangle and an LFSR
//! noise, with the console's envelopes, sweeps and vibrato, its mixing and its slight output distortion.
//!
//! State, version 1: parameters, the legacy knobs in camelCase, switches stepped 0 to 1:
//! - pulses `n` = 1, 2: `ch<n>Enabled` (on), `ch<n>Coarse` (−24–24 semitones, 0), `ch<n>Volume` (0–15, 15),
//!   `ch<n>EnvEnabled` (off), `ch<n>EnvLooped` (off), `ch<n>EnvLength` (0–15, 0), `ch<n>DutyCycle` (0–3: 12.5, 25,
//!   50, 75 %; 0 on the first, 2 on the second), `ch<n>SweepEnabled` (off), `ch<n>SweepAmount` (−7–7, 0),
//!   `ch<n>SweepRate` (0–7, 0);
//! - the triangle: `ch3Enabled` (on), `ch3Coarse` (−24–24, 0), `ch3Volume` (0–15, 15);
//! - the noise: `ch4Enabled` (off), `ch4Volume` (0–15, 15), `ch4EnvEnabled` (off), `ch4EnvLooped` (off),
//!   `ch4EnvLength` (0–15, 0), `ch4NoiseMode` (off), `ch4NoiseFreqMode` (off: the noise frequency knob; on: the
//!   note's), `ch4NoiseFreq` (0–15, 0), `ch4Sweep` (−7–7, 0), `ch4NoiseQuantize` (on);
//! - `masterVolume` (0–2, 1) and `vibrato` (0–15, 0);
//!
//! and the instrument functions. As the legacy, the knobs act on the sounding notes; the duty cycles are parameters
//! (the legacy button groups were automatable).

use lm_base::Result;
use lm_dsp::nes::{NesSettings, NesVoice, RELEASE_MS};
use lm_plugin::instrument::{GRID, Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{NodeSchema, ParamSpec, Scale, Value};

use crate::stereo_out;

pub const ID: &str = "nes";
pub const VOICES: usize = 32;

/// The parameters: key, minimum, maximum, default, stepped
pub const PARAMS: [(&str, f64, f64, f64, bool); 35] = [
    ("ch1Enabled", 0.0, 1.0, 1.0, true),
    ("ch1Coarse", -24.0, 24.0, 0.0, true),
    ("ch1Volume", 0.0, 15.0, 15.0, true),
    ("ch1EnvEnabled", 0.0, 1.0, 0.0, true),
    ("ch1EnvLooped", 0.0, 1.0, 0.0, true),
    ("ch1EnvLength", 0.0, 15.0, 0.0, true),
    ("ch1DutyCycle", 0.0, 3.0, 0.0, true),
    ("ch1SweepEnabled", 0.0, 1.0, 0.0, true),
    ("ch1SweepAmount", -7.0, 7.0, 0.0, true),
    ("ch1SweepRate", 0.0, 7.0, 0.0, true),
    ("ch2Enabled", 0.0, 1.0, 1.0, true),
    ("ch2Coarse", -24.0, 24.0, 0.0, true),
    ("ch2Volume", 0.0, 15.0, 15.0, true),
    ("ch2EnvEnabled", 0.0, 1.0, 0.0, true),
    ("ch2EnvLooped", 0.0, 1.0, 0.0, true),
    ("ch2EnvLength", 0.0, 15.0, 0.0, true),
    ("ch2DutyCycle", 0.0, 3.0, 2.0, true),
    ("ch2SweepEnabled", 0.0, 1.0, 0.0, true),
    ("ch2SweepAmount", -7.0, 7.0, 0.0, true),
    ("ch2SweepRate", 0.0, 7.0, 0.0, true),
    ("ch3Enabled", 0.0, 1.0, 1.0, true),
    ("ch3Coarse", -24.0, 24.0, 0.0, true),
    ("ch3Volume", 0.0, 15.0, 15.0, true),
    ("ch4Enabled", 0.0, 1.0, 0.0, true),
    ("ch4Volume", 0.0, 15.0, 15.0, true),
    ("ch4EnvEnabled", 0.0, 1.0, 0.0, true),
    ("ch4EnvLooped", 0.0, 1.0, 0.0, true),
    ("ch4EnvLength", 0.0, 15.0, 0.0, true),
    ("ch4NoiseMode", 0.0, 1.0, 0.0, true),
    ("ch4NoiseFreqMode", 0.0, 1.0, 0.0, true),
    ("ch4NoiseFreq", 0.0, 15.0, 0.0, true),
    ("ch4Sweep", -7.0, 7.0, 0.0, true),
    ("ch4NoiseQuantize", 0.0, 1.0, 1.0, true),
    ("masterVolume", 0.0, 2.0, 1.0, false),
    ("vibrato", 0.0, 15.0, 0.0, true),
];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.nes.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: None,
    build,
    // (no tuning probe: the console's channels count whole samples per period, as the legacy: within about two
    // cents, not the one cent of U36; the pitch is tested against the legacy note)
    tests: TestHints { voices: VOICES, ..TestHints::new() },
};

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:nes", 1);
    for (key, min, max, def, stepped) in PARAMS {
        let p = ParamSpec::new(key, min, max, def);
        s = s.param(if stepped { p.scale(Scale::Stepped) } else { p });
    }
    vec![add_instrument_functions(s)]
}

fn on(v: f32) -> bool {
    v >= 0.5
}

/// Sets knob `index` of [`PARAMS`] on `s`
pub fn set(s: &mut NesSettings, index: usize, v: f32) {
    if index < 20 {
        let p = &mut s.pulses[index / 10];
        match index % 10 {
            0 => p.enabled = on(v),
            1 => p.coarse = v,
            2 => p.volume = v,
            3 => p.env_enabled = on(v),
            4 => p.env_looped = on(v),
            5 => p.env_length = v,
            6 => p.duty = v.round().clamp(0.0, 3.0) as usize,
            7 => p.sweep_enabled = on(v),
            8 => p.sweep_amount = v,
            _ => p.sweep_rate = v,
        }
        return;
    }
    match index {
        20 => s.triangle_enabled = on(v),
        21 => s.triangle_coarse = v,
        22 => s.triangle_volume = v,
        23 => s.noise_enabled = on(v),
        24 => s.noise_volume = v,
        25 => s.noise_env_enabled = on(v),
        26 => s.noise_env_looped = on(v),
        27 => s.noise_env_length = v,
        28 => s.noise_mode = on(v),
        29 => s.noise_freq_mode = on(v),
        30 => s.noise_freq = v,
        31 => s.noise_sweep = v,
        32 => s.noise_quantize = on(v),
        33 => s.master = v,
        34 => s.vibrato = v,
        _ => {}
    }
}

#[derive(Clone, Debug)]
pub struct Nes {
    rate: u32,
    knobs: NesSettings,
}

impl Synth for Nes {
    type Voice = NesVoice;
    const PARAMS: usize = PARAMS.len();

    fn voice(&self) -> NesVoice {
        NesVoice::new(&self.knobs, 440.0, self.rate, 0)
    }

    fn set_param(&mut self, index: usize, value: f32) {
        set(&mut self.knobs, index, value);
    }

    fn start(&self, voice: &mut NesVoice, note: &NoteStart, rng: &mut Rng) {
        // (no heap in a voice: a new one in its place allocates nothing)
        *voice = NesVoice::new(&self.knobs, note.freq, self.rate, rng.next_u64());
    }

    fn render(&self, voice: &mut NesVoice, freq: f32, left: &mut [f32], right: &mut [f32]) {
        let mut buf = [[0.0f32; 2]; GRID];
        let mut at = 0;
        while at < left.len() {
            let n = (left.len() - at).min(GRID);
            voice.render(&mut buf[..n], &self.knobs, freq);
            for (i, f) in buf[..n].iter().enumerate() {
                left[at + i] = f[0];
                right[at + i] = f[1];
            }
            at += n;
        }
    }

    fn release_ms(&self) -> f32 {
        RELEASE_MS
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let mut knobs = NesSettings::default();
    for (i, (key, ..)) in PARAMS.iter().enumerate() {
        set(&mut knobs, i, s.param_f32(key)?);
    }
    let synth = Nes { rate: cx.spec.sample_rate, knobs };
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?))
}
