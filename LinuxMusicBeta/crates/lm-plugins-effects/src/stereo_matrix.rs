//! Stereo Matrix (F-EFX: legacy `plugins/StereoMatrix`): each output side as a mix of both inputs.
//!
//! State, version 1: parameters `leftToLeft` (−1–1, 1), `leftToRight` (−1–1, 0), `rightToLeft` (−1–1, 0),
//! `rightToRight` (−1–1, 1).

use lm_base::Result;
use lm_dsp::small_effects::{StereoMatrixSettings, stereo_matrix};
use lm_plugin::{BuildContext, Descriptor, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{NodeSchema, ParamSpec, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports};

pub const ID: &str = "stereomatrix";
const KEYS: [&str; 4] = ["leftToLeft", "leftToRight", "rightToLeft", "rightToRight"];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.stereomatrix.name",
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
        NodeSchema::new("plugin:stereomatrix", 1)
            .param(ParamSpec::new("leftToLeft", -1.0, 1.0, 1.0))
            .param(ParamSpec::new("leftToRight", -1.0, 1.0, 0.0))
            .param(ParamSpec::new("rightToLeft", -1.0, 1.0, 0.0))
            .param(ParamSpec::new("rightToRight", -1.0, 1.0, 1.0)),
    ]
}

struct StereoMatrix {
    knobs: Knobs<4>,
}

impl Effect for StereoMatrix {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        let s = StereoMatrixSettings {
            left_to_left: k.get(0),
            left_to_right: k.get(1),
            right_to_left: k.get(2),
            right_to_right: k.get(3),
        };
        stereo_matrix(frames, &s);
    }

    fn reset(&mut self) {}

    fn adopt(&mut self, _old: &mut Self) -> bool {
        true
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let knobs = Knobs::from_state(state, &KEYS)?;
    EffectProcessor::build(cx, || Ok(StereoMatrix { knobs }))
}
