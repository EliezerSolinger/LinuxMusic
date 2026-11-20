//! Amplifier (F-EFX: legacy `plugins/Amplifier`): volume, panning, and a gain for each side.
//!
//! State, version 1: parameters `volume` (0–200 %, 100), `panning` (−100–100, 0), `leftGain` and `rightGain`
//! (0–200 %, 100). Every knob ramps over the piece in which it changes, as the legacy value buffers did over a
//! period.

use lm_base::Result;
use lm_dsp::small_effects::{AmplifierSettings, amplifier};
use lm_plugin::{BuildContext, Descriptor, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{NodeSchema, ParamSpec, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports};

pub const ID: &str = "amplifier";
const KEYS: [&str; 4] = ["volume", "panning", "leftGain", "rightGain"];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.amplifier.name",
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
        NodeSchema::new("plugin:amplifier", 1)
            .param(ParamSpec::new("volume", 0.0, 200.0, 100.0).unit("%"))
            .param(ParamSpec::new("panning", -100.0, 100.0, 0.0))
            .param(ParamSpec::new("leftGain", 0.0, 200.0, 100.0).unit("%"))
            .param(ParamSpec::new("rightGain", 0.0, 200.0, 100.0).unit("%")),
    ]
}

struct Amplifier {
    knobs: Knobs<4>,
}

impl Effect for Amplifier {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        amplifier(frames, &AmplifierSettings { volume: k.ramp(0), pan: k.ramp(1), left: k.ramp(2), right: k.ramp(3) });
        self.knobs.settle();
    }

    fn reset(&mut self) {}

    fn adopt(&mut self, old: &mut Self) -> bool {
        self.knobs.adopt(&old.knobs);
        true
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let knobs = Knobs::from_state(state, &KEYS)?;
    EffectProcessor::build(cx, || Ok(Amplifier { knobs }))
}
