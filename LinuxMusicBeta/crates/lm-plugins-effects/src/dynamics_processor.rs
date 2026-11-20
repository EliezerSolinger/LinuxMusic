//! Dynamics Processor (F-FX-014: legacy `plugins/DynamicsProcessor`): a gain curve drawn over the input's level (its
//! RMS, followed with an attack and a release), linked between the sides or not.
//!
//! State, version 1: parameters `inputGain` and `outputGain` (0–5, 1), `attackTime` (1–500 ms, 10), `releaseTime`
//! (1–500 ms, 100); fields `stereoMode` (`maximum`, `average`, `unlinked`; `maximum`) and `curve` (the output level
//! for each input level, 200 points from 0 to 1; a straight line by default; another length is resampled). The
//! window's live frame: the level each side's gain was read at, last, two little-endian `f32` (to draw the point on
//! the curve).

use std::sync::LazyLock;

use lm_base::Result;
use lm_dsp::dynamics_processor::{
    CURVE_POINTS, DynamicsProcessor, DynamicsProcessorSettings, StereoMode, default_curve,
};
use lm_plugin::{BuildContext, Descriptor, DisplaySpec, PluginKind, StateView, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports, put_f32s, reals};
use crate::wave_shaper::curve;

pub const ID: &str = "dynamicsprocessor";
const KEYS: [&str; 4] = ["inputGain", "outputGain", "attackTime", "releaseTime"];

static MODE_KEYS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| StereoMode::ALL.iter().map(|m| m.key()).collect());

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.dynamicsprocessor.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: Some(DisplaySpec { bytes: 8 }),
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    let line = Value::list(default_curve().iter().map(|v| Value::real(f64::from(*v)).unwrap_or(Value::NULL)));
    vec![
        NodeSchema::new("plugin:dynamicsprocessor", 1)
            .param(ParamSpec::new("inputGain", 0.0, 5.0, 1.0))
            .param(ParamSpec::new("outputGain", 0.0, 5.0, 1.0))
            .param(ParamSpec::new("attackTime", 1.0, 500.0, 10.0).scale(Scale::Log).unit("ms"))
            .param(ParamSpec::new("releaseTime", 1.0, 500.0, 100.0).scale(Scale::Log).unit("ms"))
            .field(FieldSpec::choice("stereoMode", &MODE_KEYS, StereoMode::Maximum.key()))
            .field(
                FieldSpec::new("curve", FieldKind::List(Box::new(FieldKind::Real { min: 0.0, max: 1.0 })))
                    .default(line),
            ),
    ]
}

struct Dynamics {
    knobs: Knobs<4>,
    settings: DynamicsProcessorSettings,
    dsp: DynamicsProcessor,
}

impl Effect for Dynamics {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        self.settings.input_gain = k.get(0);
        self.settings.output_gain = k.get(1);
        self.settings.attack = k.get(2);
        self.settings.release = k.get(3);
        self.dsp.process(frames, &self.settings);
    }

    fn reset(&mut self) {
        self.dsp.reset();
    }

    fn adopt(&mut self, old: &mut Self) -> bool {
        std::mem::swap(&mut self.dsp, &mut old.dsp);
        true
    }

    fn display(&mut self, frame: &mut [u8]) {
        put_f32s(frame, &self.dsp.levels());
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let knobs = Knobs::from_state(state, &KEYS)?;
    let stereo_mode = StateView::new(state).choice("stereoMode", StereoMode::from_key)?;
    let points: [f32; CURVE_POINTS] = curve(&reals(state, "curve")?);
    let settings = DynamicsProcessorSettings { stereo_mode, curve: points, ..DynamicsProcessorSettings::default() };
    let rate = cx.spec.sample_rate as f32;
    EffectProcessor::build(cx, || Ok(Dynamics { knobs, settings: settings.clone(), dsp: DynamicsProcessor::new(rate) }))
}
