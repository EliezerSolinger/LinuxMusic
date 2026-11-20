//! Kicker (F-INS-007: legacy `plugins/Kicker`): a sine swept from a start to an end frequency, with noise, a click,
//! a distortion that can move over the note, and a short attack; the note lets itself go after its length.
//!
//! State, version 1: parameters `startFrequency` (5–1000 Hz, 150), `endFrequency` (5–1000 Hz, 40), `length`
//! (5–5000 ms, 440), `startDistortion` and `endDistortion` (0–100, 0.8), `gain` (0.1–5, 1), `envelopeSlope`
//! (0.01–1, 0.163), `noise` (0–1, 0), `click` (0–1, 0.4), `frequencySlope` (0.001–1, 0.06), `attack` (0.1–20 ms, 1),
//! `startFromNote` (a switch, on: the sweep starts at the note's pitch) and `endToNote` (a switch, off), and the
//! instrument functions.
//!
//! As the legacy plugin, the note releases itself once its length has played (a linear fade of 12 ms), the knobs are
//! read when the note starts, and the velocity is its level. The legacy plugin let go at the first period after the
//! length; here at the length's sample (the same for any block size).

use lm_base::Result;
use lm_dsp::kicker::{KickerSettings, KickerVoice, RELEASE_MS, release_fade};
use lm_plugin::instrument::{GRID, Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{NodeSchema, ParamSpec, Scale, Value};

use crate::stereo_out;

pub const ID: &str = "kicker";
/// Voices sounding at once (more steal, U27)
pub const VOICES: usize = 32;
const KEYS: [&str; 13] = [
    "startFrequency",
    "endFrequency",
    "length",
    "startDistortion",
    "endDistortion",
    "gain",
    "envelopeSlope",
    "noise",
    "click",
    "frequencySlope",
    "attack",
    "startFromNote",
    "endToNote",
];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.kicker.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: None,
    build,
    // (no tuning probe: a note lasts at most its 5 s, not the 30 min of U35's test; the pitch at every rate is
    // tested in tests/kicker.rs with `probe_state`)
    tests: TestHints { voices: VOICES, ..TestHints::new() },
};

/// A state whose note is a sine at its key for 5 s (swept from the note to the note, without distortion or click)
pub fn probe_state() -> Value {
    let real = |v: f64| Value::real(v).unwrap_or(Value::NULL);
    let params = Value::empty_object()
        .with("endToNote", real(1.0))
        .with("length", real(5000.0))
        .with("startDistortion", real(0.0))
        .with("endDistortion", real(0.0))
        .with("click", real(0.0));
    Value::empty_object().with(lm_state::PARAMS_KEY, params)
}

fn schemas() -> Vec<NodeSchema> {
    let p = ParamSpec::new;
    let s = NodeSchema::new("plugin:kicker", 1)
        .param(p(KEYS[0], 5.0, 1000.0, 150.0).scale(Scale::Log).unit("Hz"))
        .param(p(KEYS[1], 5.0, 1000.0, 40.0).scale(Scale::Log).unit("Hz"))
        .param(p(KEYS[2], 5.0, 5000.0, 440.0).scale(Scale::Log).unit("ms"))
        .param(p(KEYS[3], 0.0, 100.0, 0.8))
        .param(p(KEYS[4], 0.0, 100.0, 0.8))
        .param(p(KEYS[5], 0.1, 5.0, 1.0))
        .param(p(KEYS[6], 0.01, 1.0, 0.163))
        .param(p(KEYS[7], 0.0, 1.0, 0.0))
        .param(p(KEYS[8], 0.0, 1.0, 0.4))
        .param(p(KEYS[9], 0.001, 1.0, 0.06))
        .param(p(KEYS[10], 0.1, 20.0, 1.0).unit("ms"))
        .param(p(KEYS[11], 0.0, 1.0, 1.0).scale(Scale::Stepped))
        .param(p(KEYS[12], 0.0, 1.0, 0.0).scale(Scale::Stepped));
    vec![add_instrument_functions(s)]
}

/// The knobs as the legacy plugin reads them
pub fn settings(k: &[f32; 13]) -> KickerSettings {
    KickerSettings {
        start_freq: k[0],
        end_freq: k[1],
        length: k[2],
        dist_start: k[3],
        dist_end: k[4],
        gain: k[5],
        env_slope: k[6],
        noise: k[7],
        click: k[8],
        freq_slope: k[9],
        attack: k[10],
        start_from_note: k[11] >= 0.5,
        end_to_note: k[12] >= 0.5,
    }
}

#[derive(Clone, Debug)]
pub struct Kicker {
    rate: f32,
    knobs: [f32; 13],
}

/// One note: the sweep, and how far it has played
#[derive(Clone, Debug)]
pub struct Voice {
    sweep: KickerVoice,
    played: u64,
}

impl Kicker {
    fn release_frames(&self) -> f32 {
        RELEASE_MS * self.rate / 1000.0
    }
}

impl Synth for Kicker {
    type Voice = Voice;
    const PARAMS: usize = 13;

    fn voice(&self) -> Voice {
        Voice { sweep: KickerVoice::new(&settings(&self.knobs), 440.0, self.rate, 0), played: 0 }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(k) = self.knobs.get_mut(index) {
            *k = value;
        }
    }

    fn start(&self, voice: &mut Voice, note: &NoteStart, rng: &mut Rng) {
        voice.sweep = KickerVoice::new(&settings(&self.knobs), note.freq, self.rate, rng.next_u64());
        voice.played = 0;
    }

    fn render(&self, voice: &mut Voice, _freq: f32, left: &mut [f32], right: &mut [f32]) {
        let length = voice.sweep.length() as u64;
        let release = self.release_frames();
        let mut buf = [[0.0f32; 2]; GRID];
        let mut at = 0;
        while at < left.len() {
            let n = (left.len() - at).min(GRID);
            let frames = &mut buf[..n];
            voice.sweep.render(frames, self.rate);
            // Past its length, the note lets itself go
            let start = voice.played;
            if start + n as u64 > length {
                let from = length.saturating_sub(start) as usize;
                let done = start.saturating_sub(length);
                release_fade(frames, from, done, release);
            }
            for (i, f) in frames.iter().enumerate() {
                left[at + i] = f[0];
                right[at + i] = f[1];
            }
            voice.played += n as u64;
            at += n;
        }
    }

    fn finished(&self, voice: &Voice) -> bool {
        voice.played as f32 >= voice.sweep.length() + self.release_frames()
    }

    fn release_ms(&self) -> f32 {
        RELEASE_MS
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let mut knobs = [0.0f32; 13];
    for (k, key) in knobs.iter_mut().zip(KEYS) {
        *k = s.param_f32(key)?;
    }
    let synth = Kicker { rate: cx.spec.sample_rate as f32, knobs };
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?))
}
