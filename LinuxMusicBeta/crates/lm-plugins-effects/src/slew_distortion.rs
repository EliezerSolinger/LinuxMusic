//! Slew Distortion (F-FX-025: legacy `plugins/SlewDistortion`, Lost Robot's): one or two bands (split by a
//! Linkwitz-Riley crossover), each a waveshaper with drive, bias, warp and crush, a limit on how fast the signal may
//! rise and fall, an envelope that drives the dynamics and the slew, its own output volume and mix; oversampled 1 to
//! 32 times.
//!
//! State, version 1: per band `n` (1, 2), parameters `drive<n>` (−24–24 dB, 0), `slewUp<n>` and `slewDown<n>` (−96–6
//! dB, 6), `bias<n>` (−2–2, 0), `warp<n>` (0–0.99, 0), `crush<n>` (0–24 dB, 0), `outVol<n>` (−24–24 dB, 0),
//! `attack<n>` (0.01–200 ms, 2), `release<n>` (0.01–800 ms, 20), `dynamics<n>` (0–1, 0), `dynamicSlew<n>` (−8–8, 0),
//! `mix<n>` (0–1, 1), `slewLink<n>` (a switch, on), and field `type<n>` (a shape key, `hard-clip`); then
//! `dcRemove` (a switch, on), `multiband` (a switch, off), `oversample` (0–5 stages: 2^n times; 0, stepped) and
//! `split` (100–20 000 Hz, 200).

use std::sync::LazyLock;

use lm_base::Result;
use lm_dsp::slew_distortion::{SlewBand, SlewDistortion, SlewDistortionSettings, SlewShape};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports};

pub const ID: &str = "slewdistortion";
const PER_BAND: usize = 13;
const COUNT: usize = 2 * PER_BAND + 4;
const KEYS: [&str; COUNT] = [
    "drive1",
    "slewUp1",
    "slewDown1",
    "bias1",
    "warp1",
    "crush1",
    "outVol1",
    "attack1",
    "release1",
    "dynamics1",
    "dynamicSlew1",
    "mix1",
    "slewLink1",
    "drive2",
    "slewUp2",
    "slewDown2",
    "bias2",
    "warp2",
    "crush2",
    "outVol2",
    "attack2",
    "release2",
    "dynamics2",
    "dynamicSlew2",
    "mix2",
    "slewLink2",
    "dcRemove",
    "multiband",
    "oversample",
    "split",
];
const TYPE_FIELDS: [&str; 2] = ["type1", "type2"];

static SHAPE_KEYS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| SlewShape::ALL.iter().map(|s| s.key()).collect());

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.slewdistortion.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: None,
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:slewdistortion", 1);
    for band in 0..2 {
        let k = |i: usize| KEYS[band * PER_BAND + i];
        s = s
            .param(ParamSpec::new(k(0), -24.0, 24.0, 0.0).unit("dB"))
            .param(ParamSpec::new(k(1), -96.0, 6.0, 6.0).unit("dB"))
            .param(ParamSpec::new(k(2), -96.0, 6.0, 6.0).unit("dB"))
            .param(ParamSpec::new(k(3), -2.0, 2.0, 0.0))
            .param(ParamSpec::new(k(4), 0.0, 0.99, 0.0))
            .param(ParamSpec::new(k(5), 0.0, 24.0, 0.0).unit("dB"))
            .param(ParamSpec::new(k(6), -24.0, 24.0, 0.0).unit("dB"))
            .param(ParamSpec::new(k(7), 0.01, 200.0, 2.0).scale(Scale::Log).unit("ms"))
            .param(ParamSpec::new(k(8), 0.01, 800.0, 20.0).scale(Scale::Log).unit("ms"))
            .param(ParamSpec::new(k(9), 0.0, 1.0, 0.0))
            .param(ParamSpec::new(k(10), -8.0, 8.0, 0.0))
            .param(ParamSpec::new(k(11), 0.0, 1.0, 1.0))
            .param(ParamSpec::new(k(12), 0.0, 1.0, 1.0).scale(Scale::Stepped));
    }
    let hard = SlewShape::HardClip.key();
    vec![
        s.param(ParamSpec::new("dcRemove", 0.0, 1.0, 1.0).scale(Scale::Stepped))
            .param(ParamSpec::new("multiband", 0.0, 1.0, 0.0).scale(Scale::Stepped))
            .param(ParamSpec::new("oversample", 0.0, 5.0, 0.0).scale(Scale::Stepped))
            .param(ParamSpec::new("split", 100.0, 20_000.0, 200.0).scale(Scale::Log).unit("Hz"))
            .field(FieldSpec::choice(TYPE_FIELDS[0], &SHAPE_KEYS, hard))
            .field(FieldSpec::choice(TYPE_FIELDS[1], &SHAPE_KEYS, hard)),
    ]
}

struct Slew {
    knobs: Knobs<COUNT>,
    shapes: [SlewShape; 2],
    dsp: SlewDistortion,
}

impl Slew {
    fn settings(&self) -> SlewDistortionSettings {
        let k = &self.knobs;
        let band = |b: usize| {
            let o = b * PER_BAND;
            SlewBand {
                shape: self.shapes[b],
                drive: k.get(o),
                slew_up: k.get(o + 1),
                slew_down: k.get(o + 2),
                bias: k.get(o + 3),
                warp: k.get(o + 4),
                crush: k.get(o + 5),
                out_volume: k.get(o + 6),
                attack: k.get(o + 7),
                release: k.get(o + 8),
                dynamics: k.get(o + 9),
                dynamic_slew: k.get(o + 10),
                mix: k.get(o + 11),
                slew_link: k.on(o + 12),
            }
        };
        let g = 2 * PER_BAND;
        SlewDistortionSettings {
            bands: [band(0), band(1)],
            dc_remove: k.on(g),
            multiband: k.on(g + 1),
            oversampling: k.get(g + 2).max(0.0) as usize,
            split: k.get(g + 3),
        }
    }
}

impl Effect for Slew {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let s = self.settings();
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
    let view = StateView::new(state);
    let shapes = [view.choice(TYPE_FIELDS[0], SlewShape::from_key)?, view.choice(TYPE_FIELDS[1], SlewShape::from_key)?];
    let rate = cx.spec.sample_rate as f32;
    EffectProcessor::build(cx, || Ok(Slew { knobs, shapes, dsp: SlewDistortion::new(rate) }))
}
