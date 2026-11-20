//! Bitcrush (F-EFX: legacy `plugins/Bitcrush`): input gain and noise, a lower sample rate (each side its own, apart
//! by the stereo difference) and fewer levels, oversampled, then output gain and clip.
//!
//! State, version 1: parameters `inputGain` (−20–20 dB, 0), `inputNoise` (0–100 %, 0), `outputGain` (−20–20 dB, 0),
//! `outputClip` (−20–20 dB, 0), `sampleRate` (20–44 100 Hz, 44 100, stepped), `stereoDifference` (−50–50 %, 0),
//! `levels` (1–256, 256), `rateEnabled` and `depthEnabled` (switches, on).
//!
//! The legacy plugin also reset its rate counters once more for each knob whose change flag its first period left
//! set (its plumbing, not its DSP); here a change resets them once, in the piece where it happens.

use lm_base::Result;
use lm_dsp::distortion_effects::{Bitcrush, BitcrushSettings};
use lm_plugin::instrument::GRID;
use lm_plugin::{BuildContext, Descriptor, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports};

pub const ID: &str = "bitcrush";
const KEYS: [&str; 9] = [
    "inputGain",
    "inputNoise",
    "outputGain",
    "outputClip",
    "sampleRate",
    "stereoDifference",
    "levels",
    "rateEnabled",
    "depthEnabled",
];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.bitcrush.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: None,
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    vec![
        NodeSchema::new("plugin:bitcrush", 1)
            .param(ParamSpec::new("inputGain", -20.0, 20.0, 0.0).unit("dB"))
            .param(ParamSpec::new("inputNoise", 0.0, 100.0, 0.0).unit("%"))
            .param(ParamSpec::new("outputGain", -20.0, 20.0, 0.0).unit("dB"))
            .param(ParamSpec::new("outputClip", -20.0, 20.0, 0.0).unit("dB"))
            .param(ParamSpec::new("sampleRate", 20.0, 44_100.0, 44_100.0).scale(Scale::Stepped).unit("Hz"))
            .param(ParamSpec::new("stereoDifference", -50.0, 50.0, 0.0).unit("%"))
            .param(ParamSpec::new("levels", 1.0, 256.0, 256.0))
            .param(ParamSpec::new("rateEnabled", 0.0, 1.0, 1.0).scale(Scale::Stepped))
            .param(ParamSpec::new("depthEnabled", 0.0, 1.0, 1.0).scale(Scale::Stepped)),
    ]
}

struct Crush {
    knobs: Knobs<9>,
    dsp: Bitcrush,
}

impl Effect for Crush {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        let s = BitcrushSettings {
            in_gain: k.get(0),
            in_noise: k.get(1),
            out_gain: k.get(2),
            out_clip: k.get(3),
            rate: k.get(4),
            stereo_diff: k.get(5),
            levels: k.get(6),
            rate_enabled: k.on(7),
            depth_enabled: k.on(8),
        };
        self.dsp.process(frames, &s);
    }

    fn reset(&mut self) {
        self.dsp.reset();
    }

    fn adopt(&mut self, old: &mut Self) -> bool {
        std::mem::swap(&mut self.dsp, &mut old.dsp);
        true
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let knobs = Knobs::from_state(state, &KEYS)?;
    let rate = cx.spec.sample_rate as f32;
    let mut pair = 0u64;
    EffectProcessor::build(cx, || {
        pair += 1;
        Ok(Crush { knobs, dsp: Bitcrush::new(rate, GRID, pair) })
    })
}
