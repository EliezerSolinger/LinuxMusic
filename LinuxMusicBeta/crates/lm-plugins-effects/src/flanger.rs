//! Flanger (F-FX-016: legacy `plugins/Flanger`, Dave French's): two short delays swept by a quadrature LFO, with
//! feedback (crossed when inverted) and white noise.
//!
//! State, version 1: parameters `delayTime` (0.0001–0.05 s, 0.001), `lfoPeriod` (0.01–60 s, 0.25; the legacy knob
//! "LFO frequency" was a period), `lfoAmount` (0–0.0025 s, 0), `stereoPhase` (0–360°, 90), `feedback` (−1–1, 0),
//! `noise` (0–0.05, 0), `invert` (a switch, off).

use lm_base::Result;
use lm_dsp::modulation_effects::{Flanger, FlangerSettings};
use lm_plugin::{BuildContext, Descriptor, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports};

pub const ID: &str = "flanger";
const KEYS: [&str; 7] = ["delayTime", "lfoPeriod", "lfoAmount", "stereoPhase", "feedback", "noise", "invert"];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.flanger.name",
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
        NodeSchema::new("plugin:flanger", 1)
            .param(ParamSpec::new("delayTime", 0.0001, 0.05, 0.001).scale(Scale::Log).unit("s"))
            .param(ParamSpec::new("lfoPeriod", 0.01, 60.0, 0.25).scale(Scale::Log).unit("s"))
            .param(ParamSpec::new("lfoAmount", 0.0, 0.0025, 0.0).unit("s"))
            .param(ParamSpec::new("stereoPhase", 0.0, 360.0, 90.0).unit("°"))
            .param(ParamSpec::new("feedback", -1.0, 1.0, 0.0))
            .param(ParamSpec::new("noise", 0.0, 0.05, 0.0))
            .param(ParamSpec::new("invert", 0.0, 1.0, 0.0).scale(Scale::Stepped)),
    ]
}

struct Flange {
    knobs: Knobs<7>,
    dsp: Flanger,
}

impl Effect for Flange {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        let s = FlangerSettings {
            delay: k.get(0),
            lfo_period: k.get(1),
            lfo_amount: k.get(2),
            stereo_phase: k.get(3),
            feedback: k.get(4),
            noise: k.get(5),
            invert_feedback: k.on(6),
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
        Ok(Flange { knobs, dsp: Flanger::new(rate, pair) })
    })
}
