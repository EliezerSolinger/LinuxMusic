//! Multitap Echo (F-FX-021: legacy `plugins/MultitapEcho`): up to 32 echoes, one step apart, each low-passed and at
//! its own level, from a ring the length of the longest.
//!
//! State, version 1: parameters `steps` (4–32, 16, stepped), `stepLength` (1–500 ms, 100), `dryGain` (−80–20 dB, 0),
//! `swapInputs` (a switch, off), `lowpassStages` (1–4, 1, stepped); fields `amplitude` (each echo's level, dB, −60–0,
//! 0) and `lowpass` (each echo's low-pass, as log10(f / 20 Hz), 0–3, 3): 32 values each, the first `steps` used.
//! (The legacy graphs had `steps` points and went back to their defaults when the steps changed; here every step
//! keeps its values.)
//!
//! The echoes are written a whole piece of the grid at a time, so a change takes effect at the next grid point.

use lm_base::Result;
use lm_dsp::modulation_effects::{MAX_STEPS, MultitapEcho, MultitapSettings};
use lm_plugin::instrument::GRID;
use lm_plugin::{BuildContext, Descriptor, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports, reals};

pub const ID: &str = "multitapecho";
const KEYS: [&str; 5] = ["steps", "stepLength", "dryGain", "swapInputs", "lowpassStages"];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.multitapecho.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: None,
    build,
    tests: TestHints::new(),
};

fn steps_of(value: f64) -> Value {
    Value::list((0..MAX_STEPS).map(|_| Value::real(value).unwrap_or(Value::NULL)))
}

fn schemas() -> Vec<NodeSchema> {
    let list = |min, max| FieldKind::List(Box::new(FieldKind::Real { min, max }));
    vec![
        NodeSchema::new("plugin:multitapecho", 1)
            .param(ParamSpec::new("steps", 4.0, MAX_STEPS as f64, 16.0).scale(Scale::Stepped))
            .param(ParamSpec::new("stepLength", 1.0, 500.0, 100.0).scale(Scale::Log).unit("ms"))
            .param(ParamSpec::new("dryGain", -80.0, 20.0, 0.0).unit("dB"))
            .param(ParamSpec::new("swapInputs", 0.0, 1.0, 0.0).scale(Scale::Stepped))
            .param(ParamSpec::new("lowpassStages", 1.0, 4.0, 1.0).scale(Scale::Stepped))
            .field(FieldSpec::new("amplitude", list(-60.0, 0.0)).default(steps_of(0.0)))
            .field(FieldSpec::new("lowpass", list(0.0, 3.0)).default(steps_of(3.0))),
    ]
}

/// A graph of the state as the echoes' values: the list's values in order, the steps it does not have at @p rest
fn graph(values: &[f64], rest: f32) -> [f32; MAX_STEPS] {
    core::array::from_fn(|i| values.get(i).map_or(rest, |v| *v as f32))
}

struct Taps {
    knobs: Knobs<5>,
    amp: [f32; MAX_STEPS],
    lp: [f32; MAX_STEPS],
    dsp: MultitapEcho,
}

impl Effect for Taps {
    const CUT_AT_EVENTS: bool = false;

    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        let s = MultitapSettings {
            steps: k.get(0).max(0.0) as usize,
            step_length: k.get(1),
            dry_gain: k.get(2),
            swap_inputs: k.on(3),
            stages: k.get(4).max(1.0) as usize,
            amp: self.amp,
            lp: self.lp,
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
    let amp = graph(&reals(state, "amplitude")?, 0.0);
    let lp = graph(&reals(state, "lowpass")?, 3.0);
    let rate = cx.spec.sample_rate as f32;
    EffectProcessor::build(cx, || Ok(Taps { knobs, amp, lp, dsp: MultitapEcho::new(rate, GRID) }))
}
