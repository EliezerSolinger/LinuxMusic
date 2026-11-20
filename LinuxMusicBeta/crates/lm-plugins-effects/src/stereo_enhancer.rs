//! Stereo Enhancer (F-EFX: legacy `plugins/StereoEnhancer`): the right side delayed by the width, in frames, and
//! each side mixed with the other by the sine of half the width in degrees.
//!
//! State, version 1: parameter `width` (0–180, 0).

use lm_base::Result;
use lm_dsp::small_effects::StereoEnhancer;
use lm_plugin::{BuildContext, Descriptor, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{NodeSchema, ParamSpec, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports};

pub const ID: &str = "stereoenhancer";
const KEYS: [&str; 1] = ["width"];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.stereoenhancer.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: None,
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    vec![NodeSchema::new("plugin:stereoenhancer", 1).param(ParamSpec::new("width", 0.0, 180.0, 0.0))]
}

struct Enhancer {
    knobs: Knobs<1>,
    dsp: StereoEnhancer,
}

impl Effect for Enhancer {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        self.dsp.process(frames, self.knobs.get(0));
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
    EffectProcessor::build(cx, || Ok(Enhancer { knobs, dsp: StereoEnhancer::default() }))
}
