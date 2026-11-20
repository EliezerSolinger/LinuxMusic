//! Granular Pitch Shifter (F-FX-018: legacy `plugins/GranularPitchShifter`, Lost Robot's): grains read from a ring of
//! the input at another speed, with spray, jitter, twitch, stereo spread, shape and fade, feedback, density and glide.
//!
//! State, version 1: parameters `pitch` (semitones, −48–48, 1), `grainSize` (2–1000 Hz, 10), `spray` (0–40 s,
//! 0.005), `jitter` (0–2, 0), `twitch` (0–1, 0), `pitchStereoSpread` (−24–24 semitones, 0), `sprayStereo` (0–1, 0),
//! `shape` (1–2, 2), `fadeLength` (0.001–1, 1), `feedback` (0–1, 0), `minimumAllowedLatency` (0–1 s, 0.01),
//! `prefilter` (a switch, on), `density` (1–16, 1), `glide` (0–1 s, 0.01); field `ringBufferLength` (`5s`,
//! `10s-size`, `40s-size-pitch`, `40s-size-spray-jitter`, `120s-all`; `5s`). The legacy knobs changed their ranges
//! with the ring's length; here the parameters span the widest, and each is held to the range its ring allows (the
//! legacy knob's range in that mode): grain size from 4 Hz in the 5 s ring, pitch to 24 semitones but in the 40 s
//! (pitch) and 120 s rings, spray to 0.5 s and jitter to 1 but in the 40 s (spray) and 120 s rings.
//!
//! A different ring is a new processor that cannot take the old one's sound (the ring's length changes): the old one
//! plays on in a tail node (U32), as the legacy emptied its ring.

use std::sync::LazyLock;

use lm_base::Result;
use lm_dsp::granular_pitch_shifter::{GranularPitchShifter, GranularSettings};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports};

pub const ID: &str = "granularpitchshifter";
const KEYS: [&str; 14] = [
    "pitch",
    "grainSize",
    "spray",
    "jitter",
    "twitch",
    "pitchStereoSpread",
    "sprayStereo",
    "shape",
    "fadeLength",
    "feedback",
    "minimumAllowedLatency",
    "prefilter",
    "density",
    "glide",
];
pub const RANGE_KEYS: [&str; 5] = ["5s", "10s-size", "40s-size-pitch", "40s-size-spray-jitter", "120s-all"];
static RANGES: LazyLock<Box<[&'static str]>> = LazyLock::new(|| RANGE_KEYS.into());

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.granularpitchshifter.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: None,
    build,
    tests: TestHints { no_adopt: &["ringBufferLength"], ..TestHints::new() },
};

fn schemas() -> Vec<NodeSchema> {
    vec![
        NodeSchema::new("plugin:granularpitchshifter", 1)
            .param(ParamSpec::new("pitch", -48.0, 48.0, 1.0).unit("st"))
            .param(ParamSpec::new("grainSize", 2.0, 1000.0, 10.0).scale(Scale::Log).unit("Hz"))
            .param(ParamSpec::new("spray", 0.0, 40.0, 0.005).unit("s"))
            .param(ParamSpec::new("jitter", 0.0, 2.0, 0.0))
            .param(ParamSpec::new("twitch", 0.0, 1.0, 0.0))
            .param(ParamSpec::new("pitchStereoSpread", -24.0, 24.0, 0.0).unit("st"))
            .param(ParamSpec::new("sprayStereo", 0.0, 1.0, 0.0))
            .param(ParamSpec::new("shape", 1.0, 2.0, 2.0))
            .param(ParamSpec::new("fadeLength", 0.001, 1.0, 1.0).scale(Scale::Log))
            .param(ParamSpec::new("feedback", 0.0, 1.0, 0.0))
            .param(ParamSpec::new("minimumAllowedLatency", 0.0, 1.0, 0.01).unit("s"))
            .param(ParamSpec::new("prefilter", 0.0, 1.0, 1.0).scale(Scale::Stepped))
            .param(ParamSpec::new("density", 1.0, 16.0, 1.0).scale(Scale::Log))
            .param(ParamSpec::new("glide", 0.0, 1.0, 0.01).unit("s"))
            .field(FieldSpec::choice("ringBufferLength", &RANGES, RANGE_KEYS[0])),
    ]
}

/// The legacy knobs' ranges in ring @p range: (lowest grain size, highest pitch, highest spray, highest jitter)
fn limits(range: usize) -> (f32, f32, f32, f32) {
    match range {
        0 => (4.0, 24.0, 0.5, 1.0),
        1 => (2.0, 24.0, 0.5, 1.0),
        2 => (2.0, 48.0, 0.5, 1.0),
        3 => (2.0, 24.0, 20.0, 2.0),
        _ => (2.0, 48.0, 40.0, 2.0),
    }
}

struct Granular {
    knobs: Knobs<14>,
    range: usize,
    dsp: GranularPitchShifter,
}

impl Effect for Granular {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        let (min_size, max_pitch, max_spray, max_jitter) = limits(self.range);
        let mut pitch = k.ramp(0);
        pitch.from = pitch.from.min(max_pitch);
        pitch.to = pitch.to.min(max_pitch);
        let s = GranularSettings {
            pitch,
            pitch_spread: k.ramp(5),
            size: k.get(1).max(min_size),
            spray: k.get(2).min(max_spray),
            jitter: k.get(3).min(max_jitter),
            twitch: k.get(4),
            spray_spread: k.get(6),
            shape: k.get(7),
            fade_length: k.get(8),
            feedback: k.get(9),
            min_latency: k.get(10),
            prefilter: k.on(11),
            density: k.get(12),
            glide: k.get(13),
            range: self.range,
        };
        self.dsp.process(frames, &s);
        self.knobs.settle();
    }

    fn reset(&mut self) {
        self.dsp.reset();
    }

    fn adopt(&mut self, old: &mut Self) -> bool {
        if old.range != self.range {
            return false;
        }
        std::mem::swap(&mut self.dsp, &mut old.dsp);
        self.knobs.adopt(&old.knobs);
        true
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let knobs = Knobs::from_state(state, &KEYS)?;
    let range = StateView::new(state).choice("ringBufferLength", |key| RANGE_KEYS.iter().position(|k| *k == key))?;
    let rate = cx.spec.sample_rate as f32;
    let (_, max_pitch, ..) = limits(range);
    let (pitch, spread) = (knobs.get(0).min(max_pitch), knobs.get(5));
    let mut seed = 0u64;
    EffectProcessor::build(cx, || {
        let dsp = GranularPitchShifter::new(rate, range, pitch, spread, seed);
        seed += 1;
        Ok(Granular { knobs, range, dsp })
    })
}
