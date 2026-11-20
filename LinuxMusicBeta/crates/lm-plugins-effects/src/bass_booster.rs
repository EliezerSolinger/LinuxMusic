//! Bass Booster (F-EFX: legacy `plugins/BassBooster`): a one-pole low-pass added back to the signal, then a gain.
//!
//! State, version 1: parameters `frequency` (50–200 Hz, 100), `gain` (0.1–5, 1; ramped over the piece in which it
//! changes, as the legacy value buffer over a period), `ratio` (0.1–10, 2).

use lm_base::Result;
use lm_dsp::small_effects::{BassBooster, BassBoosterSettings};
use lm_plugin::{BuildContext, Descriptor, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports};

pub const ID: &str = "bassbooster";
const KEYS: [&str; 3] = ["frequency", "gain", "ratio"];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.bassbooster.name",
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
        NodeSchema::new("plugin:bassbooster", 1)
            .param(ParamSpec::new("frequency", 50.0, 200.0, 100.0).scale(Scale::Log).unit("Hz"))
            .param(ParamSpec::new("gain", 0.1, 5.0, 1.0))
            .param(ParamSpec::new("ratio", 0.1, 10.0, 2.0)),
    ]
}

struct Booster {
    knobs: Knobs<3>,
    dsp: BassBooster,
}

impl Effect for Booster {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        let s = BassBoosterSettings { frequency: k.get(0), gain: k.ramp(1), ratio: k.get(2) };
        self.dsp.process(frames, &s);
        self.knobs.settle();
    }

    fn reset(&mut self) {
        self.dsp.reset();
    }

    fn adopt(&mut self, old: &mut Self) -> bool {
        std::mem::swap(&mut self.dsp, &mut old.dsp);
        self.knobs.adopt(&old.knobs);
        true
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let knobs = Knobs::from_state(state, &KEYS)?;
    let rate = cx.spec.sample_rate as f32;
    EffectProcessor::build(cx, || Ok(Booster { knobs, dsp: BassBooster::new(rate) }))
}
