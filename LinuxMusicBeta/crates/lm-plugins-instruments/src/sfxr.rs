//! sfxr (F-INS-015: legacy `plugins/Sfxr`): Tomas Pettersson's retro sound-effect generator: a square, saw, sine or
//! noise wave with slides, arpeggio, vibrato, an envelope, filters, a phaser and repeats, rendered at 44.1 kHz at the
//! note's pitch (A4 plays the sound as drawn); the note ends when the sound does.
//!
//! State, version 1: parameters, 0 to 1 unless said, `attackTime` (0), `sustainTime` (0.3), `sustainPunch` (0),
//! `decayTime` (0.4), `startFrequency` (0.3), `minFrequency` (0), `slide` and `deltaSlide` (−1–1, 0),
//! `vibratoDepth` and `vibratoSpeed` (0), `changeAmount` (−1–1, 0), `changeSpeed` (0), `squareDuty` (0),
//! `dutySweep` (−1–1, 0), `repeatSpeed` (0), `phaserOffset` and `phaserSweep` (−1–1, 0), `lpFilterCutoff` (1),
//! `lpFilterCutoffSweep` (−1–1, 0), `lpFilterResonance` (0), `hpFilterCutoff` (0), `hpFilterCutoffSweep` (−1–1, 0);
//! field `wave` (`square`, `saw`, `sine`, `noise`; `square`); and the instrument functions. The knobs are read when a
//! note starts, as the legacy.
//!
//! The legacy plugin stretched each period of the sound over the note's period at its pitch; here over each piece of
//! the grid (the same for any block size).

use std::sync::LazyLock;

use lm_base::Result;
use lm_dsp::sfxr::{SfxrSettings, SfxrVoice, SfxrWave};
use lm_plugin::instrument::{GRID, Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Value};

use crate::stereo_out;

pub const ID: &str = "sfxr";
pub const VOICES: usize = 32;
/// The parameters: key, signed (−1 to 1) or not (0 to 1), default
pub const PARAMS: [(&str, bool, f64); 22] = [
    ("attackTime", false, 0.0),
    ("sustainTime", false, 0.3),
    ("sustainPunch", false, 0.0),
    ("decayTime", false, 0.4),
    ("startFrequency", false, 0.3),
    ("minFrequency", false, 0.0),
    ("slide", true, 0.0),
    ("deltaSlide", true, 0.0),
    ("vibratoDepth", false, 0.0),
    ("vibratoSpeed", false, 0.0),
    ("changeAmount", true, 0.0),
    ("changeSpeed", false, 0.0),
    ("squareDuty", false, 0.0),
    ("dutySweep", true, 0.0),
    ("repeatSpeed", false, 0.0),
    ("phaserOffset", true, 0.0),
    ("phaserSweep", true, 0.0),
    ("lpFilterCutoff", false, 1.0),
    ("lpFilterCutoffSweep", true, 0.0),
    ("lpFilterResonance", false, 0.0),
    ("hpFilterCutoff", false, 0.0),
    ("hpFilterCutoffSweep", true, 0.0),
];
pub const WAVE_KEYS: [&str; 4] = ["square", "saw", "sine", "noise"];
static WAVES: LazyLock<Box<[&'static str]>> = LazyLock::new(|| WAVE_KEYS.into());

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.sfxr.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: None,
    build,
    // (no tuning probe: a sound effect is short and sweeps; its pitch is tested against the legacy note)
    tests: TestHints { voices: VOICES, ..TestHints::new() },
};

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:sfxr", 1);
    for (key, signed, def) in PARAMS {
        s = s.param(ParamSpec::new(key, if signed { -1.0 } else { 0.0 }, 1.0, def));
    }
    vec![add_instrument_functions(s.field(FieldSpec::choice("wave", &WAVES, "square")))]
}

/// The knobs as the synth reads them
pub fn settings(k: &[f32; 22], wave: SfxrWave) -> SfxrSettings {
    SfxrSettings {
        attack: k[0],
        sustain: k[1],
        punch: k[2],
        decay: k[3],
        start_freq: k[4],
        min_freq: k[5],
        slide: k[6],
        delta_slide: k[7],
        vibrato_depth: k[8],
        vibrato_speed: k[9],
        change_amount: k[10],
        change_speed: k[11],
        square_duty: k[12],
        duty_sweep: k[13],
        repeat_speed: k[14],
        phaser_offset: k[15],
        phaser_sweep: k[16],
        lp_cutoff: k[17],
        lp_cutoff_sweep: k[18],
        lp_resonance: k[19],
        hp_cutoff: k[20],
        hp_cutoff_sweep: k[21],
        wave,
    }
}

/// The pitch the sound is drawn at (the legacy track's base note, A4)
pub const BASE_FREQ: f32 = 440.0;

#[derive(Clone, Debug)]
pub struct Sfxr {
    rate: f32,
    knobs: [f32; 22],
    wave: SfxrWave,
}

/// One note: its sound, and whether it still sounds. Copied into its own room (the crossfade copies voices on the
/// audio thread)
#[derive(Debug)]
pub struct Voice {
    sound: SfxrVoice,
    playing: bool,
}

impl Clone for Voice {
    fn clone(&self) -> Self {
        Self { sound: self.sound.clone(), playing: self.playing }
    }

    fn clone_from(&mut self, source: &Self) {
        self.sound.clone_from(&source.sound);
        self.playing = source.playing;
    }
}

impl Synth for Sfxr {
    type Voice = Voice;
    const PARAMS: usize = 22;

    fn voice(&self) -> Voice {
        Voice { sound: SfxrVoice::new(&settings(&self.knobs, self.wave), 0), playing: false }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(k) = self.knobs.get_mut(index) {
            *k = value;
        }
    }

    fn start(&self, voice: &mut Voice, _: &NoteStart, rng: &mut Rng) {
        voice.sound.restart(&settings(&self.knobs, self.wave), rng.next_u64());
        voice.playing = true;
    }

    fn render(&self, voice: &mut Voice, freq: f32, left: &mut [f32], right: &mut [f32]) {
        let mut buf = [[0.0f32; 2]; GRID];
        let mut at = 0;
        let pitch = f64::from(freq) / f64::from(BASE_FREQ);
        while at < left.len() {
            let n = (left.len() - at).min(GRID);
            if voice.playing {
                voice.playing = voice.sound.render(&mut buf[..n], pitch, self.rate);
            } else {
                buf[..n].fill([0.0; 2]);
            }
            for (i, f) in buf[..n].iter().enumerate() {
                left[at + i] = f[0];
                right[at + i] = f[1];
            }
            at += n;
        }
    }

    fn finished(&self, voice: &Voice) -> bool {
        !voice.playing
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let mut knobs = [0.0f32; 22];
    for (k, (key, ..)) in knobs.iter_mut().zip(PARAMS) {
        *k = s.param_f32(key)?;
    }
    let wave = s.choice("wave", |key| WAVE_KEYS.iter().position(|k| *k == key).map(|i| SfxrWave::ALL[i]))?;
    let synth = Sfxr { rate: cx.spec.sample_rate as f32, knobs, wave };
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?))
}
