//! Dual Filter (F-FX-013: legacy `plugins/DualFilter`): two of the legacy filters on the same input, each with its
//! type, cutoff, resonance and gain, mixed from all the first to all the second.
//!
//! State, version 1: per filter `n` (1, 2): parameters `filter<n>Enabled` (a switch, on), `cutoff<n>` (1–20 000 Hz,
//! 7000), `resonance<n>` (0.01–10, 0.5), `gain<n>` (0–200 %, 100); field `filter<n>Type` (a filter key, `lowpass`);
//! parameter `mix` (−1 all the first, 1 all the second; 0). Cutoff, resonance, gain and mix ramp over the piece in
//! which they change, frame by frame, as the legacy value buffers over a period.

use std::sync::LazyLock;

use lm_base::Result;
use lm_dsp::basic_filters::{FilterType, MIN_Q};
use lm_dsp::filter_effects::{DualFilter, DualFilterSettings, DualFilterSide};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports};

pub const ID: &str = "dualfilter";
const KEYS: [&str; 9] =
    ["filter1Enabled", "cutoff1", "resonance1", "gain1", "mix", "filter2Enabled", "cutoff2", "resonance2", "gain2"];
const TYPE_FIELDS: [&str; 2] = ["filter1Type", "filter2Type"];

static TYPE_KEYS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| FilterType::ALL.iter().map(|t| t.key()).collect());

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.dualfilter.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: None,
    build,
    tests: TestHints::new(),
};

fn side_params(s: NodeSchema, n: usize) -> NodeSchema {
    let k = |i: usize| KEYS[if n == 1 { i } else { 5 + i }];
    s.param(ParamSpec::new(k(0), 0.0, 1.0, 1.0).scale(Scale::Stepped))
        .param(ParamSpec::new(k(1), 1.0, 20_000.0, 7000.0).scale(Scale::Log).unit("Hz"))
        .param(ParamSpec::new(k(2), f64::from(MIN_Q), 10.0, 0.5))
        .param(ParamSpec::new(k(3), 0.0, 200.0, 100.0).unit("%"))
}

fn schemas() -> Vec<NodeSchema> {
    let s = side_params(NodeSchema::new("plugin:dualfilter", 1), 1).param(ParamSpec::new("mix", -1.0, 1.0, 0.0));
    let s = side_params(s, 2);
    let low = FilterType::LowPass.key();
    vec![s.field(FieldSpec::choice(TYPE_FIELDS[0], &TYPE_KEYS, low)).field(FieldSpec::choice(
        TYPE_FIELDS[1],
        &TYPE_KEYS,
        low,
    ))]
}

struct Dual {
    knobs: Knobs<9>,
    types: [FilterType; 2],
    dsp: DualFilter,
}

impl Effect for Dual {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        let side = |n: usize| {
            let o = if n == 0 { 0 } else { 5 };
            DualFilterSide {
                enabled: k.on(o),
                filter: self.types[n],
                cutoff: k.ramp(o + 1),
                resonance: k.ramp(o + 2),
                gain: k.ramp(o + 3),
            }
        };
        let s = DualFilterSettings { sides: [side(0), side(1)], mix: k.ramp(4) };
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
    let view = StateView::new(state);
    let types =
        [view.choice(TYPE_FIELDS[0], FilterType::from_key)?, view.choice(TYPE_FIELDS[1], FilterType::from_key)?];
    let rate = cx.spec.sample_rate as f32;
    EffectProcessor::build(cx, || Ok(Dual { knobs, types, dsp: DualFilter::new(rate) }))
}
