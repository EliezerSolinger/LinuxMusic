//! Crossover EQ (F-FX-010: legacy `plugins/CrossoverEQ`): four bands split by Linkwitz-Riley crossovers, each with
//! its gain, each one on or off.
//!
//! State, version 1: parameters `crossover12` (50–10 000 Hz, 125), `crossover23` (50–20 000 Hz, 1250),
//! `crossover34` (50–20 000 Hz, 5000), `gain1` to `gain4` (−60–30 dB, 0), `band1Active` to `band4Active` (switches,
//! on: the legacy "Mute Band" LEDs, lit when the band plays).

use lm_base::Result;
use lm_dsp::filter_effects::{CrossoverEq, CrossoverSettings};
use lm_plugin::instrument::GRID;
use lm_plugin::{BuildContext, Descriptor, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports};

pub const ID: &str = "crossovereq";
const KEYS: [&str; 11] = [
    "crossover12",
    "crossover23",
    "crossover34",
    "gain1",
    "gain2",
    "gain3",
    "gain4",
    "band1Active",
    "band2Active",
    "band3Active",
    "band4Active",
];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.crossovereq.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: None,
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:crossovereq", 1)
        .param(ParamSpec::new("crossover12", 50.0, 10_000.0, 125.0).scale(Scale::Log).unit("Hz"))
        .param(ParamSpec::new("crossover23", 50.0, 20_000.0, 1250.0).scale(Scale::Log).unit("Hz"))
        .param(ParamSpec::new("crossover34", 50.0, 20_000.0, 5000.0).scale(Scale::Log).unit("Hz"));
    for key in &KEYS[3..7] {
        s = s.param(ParamSpec::new(key, -60.0, 30.0, 0.0).unit("dB"));
    }
    for key in &KEYS[7..] {
        s = s.param(ParamSpec::new(key, 0.0, 1.0, 1.0).scale(Scale::Stepped));
    }
    vec![s]
}

struct Crossover {
    knobs: Knobs<11>,
    dsp: CrossoverEq,
}

impl Effect for Crossover {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        let s = CrossoverSettings {
            crossovers: [k.get(0), k.get(1), k.get(2)],
            gains: [k.get(3), k.get(4), k.get(5), k.get(6)],
            enabled: [k.on(7), k.on(8), k.on(9), k.on(10)],
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
    EffectProcessor::build(cx, || Ok(Crossover { knobs, dsp: CrossoverEq::new(rate, GRID) }))
}
