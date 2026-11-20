//! ReverbSC (F-FX-024: legacy `plugins/ReverbSC`): Csound's `reverbsc` (eight delay lines whose lengths wander, in a
//! feedback network), a DC blocker on each side, input and output gains.
//!
//! State, version 1: parameters `inputGain` (−60–15 dB, 0), `size` (0–1, 0.89), `color` (100–15 000 Hz, 10 000),
//! `outputGain` (−60–15 dB, 0).

use lm_base::Result;
use lm_dsp::reverb_sc::{ReverbSc, ReverbScSettings};
use lm_plugin::{BuildContext, Descriptor, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports};

pub const ID: &str = "reverbsc";
const KEYS: [&str; 4] = ["inputGain", "size", "color", "outputGain"];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.reverbsc.name",
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
        NodeSchema::new("plugin:reverbsc", 1)
            .param(ParamSpec::new("inputGain", -60.0, 15.0, 0.0).unit("dB"))
            .param(ParamSpec::new("size", 0.0, 1.0, 0.89))
            .param(ParamSpec::new("color", 100.0, 15_000.0, 10_000.0).scale(Scale::Log).unit("Hz"))
            .param(ParamSpec::new("outputGain", -60.0, 15.0, 0.0).unit("dB")),
    ]
}

struct Reverb {
    knobs: Knobs<4>,
    dsp: ReverbSc,
}

impl Effect for Reverb {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        let s = ReverbScSettings { input_gain: k.get(0), size: k.get(1), color: k.get(2), output_gain: k.get(3) };
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
    EffectProcessor::build(cx, || Ok(Reverb { knobs, dsp: ReverbSc::new(rate) }))
}
