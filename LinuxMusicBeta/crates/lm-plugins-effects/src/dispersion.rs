//! Dispersion (F-EFX: legacy `plugins/Dispersion`): a chain of up to 999 all-pass filters, with feedback and an
//! optional DC offset removal.
//!
//! State, version 1: parameters `amount` (0–999 filters, 0, stepped), `frequency` (20–20 000 Hz, 200), `resonance`
//! (0.01–8, 0.707), `feedback` (−1–1, 0), `dcOffsetRemoval` (a switch, off).

use lm_base::Result;
use lm_dsp::distortion_effects::{Dispersion, DispersionSettings, MAX_DISPERSION_FILTERS};
use lm_plugin::{BuildContext, Descriptor, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports};

pub const ID: &str = "dispersion";
const KEYS: [&str; 5] = ["amount", "frequency", "resonance", "feedback", "dcOffsetRemoval"];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.dispersion.name",
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
        NodeSchema::new("plugin:dispersion", 1)
            .param(ParamSpec::new("amount", 0.0, MAX_DISPERSION_FILTERS as f64, 0.0).scale(Scale::Stepped))
            .param(ParamSpec::new("frequency", 20.0, 20_000.0, 200.0).scale(Scale::Log).unit("Hz"))
            .param(ParamSpec::new("resonance", 0.01, 8.0, 0.707))
            .param(ParamSpec::new("feedback", -1.0, 1.0, 0.0))
            .param(ParamSpec::new("dcOffsetRemoval", 0.0, 1.0, 0.0).scale(Scale::Stepped)),
    ]
}

/// The loop's gain at the feedback knob's ends. The all-passes pass every frequency at unity gain, so a feedback of
/// ±1 makes the loop lossless: poles on the unit circle, and an input at one of their frequencies grows without end
/// (the legacy plugin's ends did exactly that). Held a hair inside, the loop rings for seconds and stays bounded; any
/// value within ±0.999 is the legacy one
const MAX_LOOP_GAIN: f32 = 0.999;

struct Disperse {
    knobs: Knobs<5>,
    dsp: Dispersion,
}

impl Effect for Disperse {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        let s = DispersionSettings {
            amount: k.get(0).max(0.0) as usize,
            frequency: k.get(1),
            resonance: k.get(2),
            feedback: k.get(3).clamp(-MAX_LOOP_GAIN, MAX_LOOP_GAIN),
            dc: k.on(4),
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
    EffectProcessor::build(cx, || Ok(Disperse { knobs, dsp: Dispersion::new(rate) }))
}
