//! Frequency Shifter (F-FX-017: legacy `plugins/FrequencyShifter`): a single-sideband frequency shifter (or ring
//! modulator) with a delay in its feedback, an LFO on the shift, and the barberpole phaser it makes.
//!
//! State, version 1: parameters `mix` (0–1, 1), `frequencyShift` (−20 000–20 000 Hz, 0), `spreadShift` (−50–50 Hz,
//! 0), `ring` (0–1, 0), `feedback` (0–1, 0), `delayLength` (0–2000 ms, 0), `fineDelayLength` (0–20 ms, 0),
//! `delayDamping` (100–22 000 Hz, 22 000), `delayGlide` (0–1 s, 0.05), `lfoAmount` (0–2000 Hz, 0), `lfoRate`
//! (0–200 Hz, 0.2), `lfoStereoPhase` (0–1 turn, 0), `glide` (0–1 s, 0.05), `tone` (100–22 000 Hz, 22 000), `phase`
//! (−2–2 turns, 0), `antireflect` (a switch, off), `harmonics` (0–1, 0), and the buttons `resetShifter` and
//! `resetLfo` (0–1: going to 1 restarts the shifter's oscillators or the LFO, as the legacy buttons); field
//! `routeMode` (`series`, `parallel-add`, `parallel-mix`; `series`).

use std::sync::LazyLock;

use lm_base::Result;
use lm_dsp::frequency_shifter::{FrequencyShifter, FrequencyShifterSettings, RouteMode};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports};

pub const ID: &str = "frequencyshifter";
const KEYS: [&str; 19] = [
    "mix",
    "frequencyShift",
    "spreadShift",
    "ring",
    "feedback",
    "delayLength",
    "fineDelayLength",
    "delayDamping",
    "delayGlide",
    "lfoAmount",
    "lfoRate",
    "lfoStereoPhase",
    "glide",
    "tone",
    "phase",
    "antireflect",
    "harmonics",
    "resetShifter",
    "resetLfo",
];
const RESET_SHIFTER: usize = 17;
const RESET_LFO: usize = 18;

static ROUTE_KEYS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| RouteMode::ALL.iter().map(|m| m.key()).collect());

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.frequencyshifter.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: None,
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    let p = ParamSpec::new;
    let switch = |key| ParamSpec::new(key, 0.0, 1.0, 0.0).scale(Scale::Stepped);
    vec![
        NodeSchema::new("plugin:frequencyshifter", 1)
            .param(p("mix", 0.0, 1.0, 1.0))
            .param(p("frequencyShift", -20_000.0, 20_000.0, 0.0).unit("Hz"))
            .param(p("spreadShift", -50.0, 50.0, 0.0).unit("Hz"))
            .param(p("ring", 0.0, 1.0, 0.0))
            .param(p("feedback", 0.0, 1.0, 0.0))
            .param(p("delayLength", 0.0, 2000.0, 0.0).unit("ms"))
            .param(p("fineDelayLength", 0.0, 20.0, 0.0).unit("ms"))
            .param(p("delayDamping", 100.0, 22_000.0, 22_000.0).scale(Scale::Log).unit("Hz"))
            .param(p("delayGlide", 0.0, 1.0, 0.05).unit("s"))
            .param(p("lfoAmount", 0.0, 2000.0, 0.0).unit("Hz"))
            .param(p("lfoRate", 0.0, 200.0, 0.2).unit("Hz"))
            .param(p("lfoStereoPhase", 0.0, 1.0, 0.0))
            .param(p("glide", 0.0, 1.0, 0.05).unit("s"))
            .param(p("tone", 100.0, 22_000.0, 22_000.0).scale(Scale::Log).unit("Hz"))
            .param(p("phase", -2.0, 2.0, 0.0))
            .param(switch("antireflect"))
            .param(p("harmonics", 0.0, 1.0, 0.0))
            .param(switch("resetShifter"))
            .param(switch("resetLfo"))
            .field(FieldSpec::choice("routeMode", &ROUTE_KEYS, RouteMode::Series.key())),
    ]
}

struct Shifter {
    knobs: Knobs<19>,
    route: RouteMode,
    dsp: FrequencyShifter,
}

impl Effect for Shifter {
    fn set_param(&mut self, index: usize, value: f32) {
        // The buttons act when pressed (going to 1), like the legacy ones
        let pressed = value >= 0.5 && !self.knobs.on(index.min(KEYS.len() - 1));
        if index == RESET_SHIFTER && pressed {
            self.dsp.reset_shifter();
        }
        if index == RESET_LFO && pressed {
            self.dsp.reset_lfo();
        }
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        let s = FrequencyShifterSettings {
            mix: k.get(0),
            shift: k.get(1),
            spread: k.get(2),
            ring: k.get(3),
            feedback: k.get(4),
            delay: k.get(5),
            fine_delay: k.get(6),
            delay_damp: k.get(7),
            delay_glide: k.get(8),
            lfo_amount: k.get(9),
            lfo_rate: k.get(10),
            lfo_stereo_phase: k.get(11),
            glide: k.get(12),
            tone: k.get(13),
            phase: k.get(14),
            antireflect: k.on(15),
            route: self.route,
            harmonics: k.get(16),
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
    let route = StateView::new(state).choice("routeMode", RouteMode::from_key)?;
    let rate = cx.spec.sample_rate as f32;
    EffectProcessor::build(cx, || Ok(Shifter { knobs, route, dsp: FrequencyShifter::new(rate) }))
}
