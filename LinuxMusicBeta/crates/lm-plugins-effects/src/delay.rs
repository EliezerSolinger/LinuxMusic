//! Delay (F-FX-011: legacy `plugins/Delay`): a stereo delay line of up to 20 s with feedback, its length swept by a
//! sine LFO, and an output gain.
//!
//! State, version 1: parameters `delayTime` (0.01–5 s, 0.5), `feedback` (0–1, 0), `lfoTime` (the LFO's period,
//! 0.01–5 s, 2), `lfoAmount` (the sweep, 0–0.5 s, 0), `outGain` (−60–20 dB, 0). The window's live frame: the peak
//! of each side of the output since the last frame, two little-endian `f32` (the legacy output meters).

use lm_base::Result;
use lm_dsp::delay::{Delay, DelaySettings};
use lm_plugin::{BuildContext, Descriptor, DisplaySpec, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports, put_f32s};

pub const ID: &str = "delay";
const KEYS: [&str; 5] = ["delayTime", "feedback", "lfoTime", "lfoAmount", "outGain"];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.delay.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: Some(DisplaySpec { bytes: 8 }),
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    vec![
        NodeSchema::new("plugin:delay", 1)
            .param(ParamSpec::new("delayTime", 0.01, 5.0, 0.5).scale(Scale::Log).unit("s"))
            .param(ParamSpec::new("feedback", 0.0, 1.0, 0.0))
            .param(ParamSpec::new("lfoTime", 0.01, 5.0, 2.0).scale(Scale::Log).unit("s"))
            .param(ParamSpec::new("lfoAmount", 0.0, 0.5, 0.0).unit("s"))
            .param(ParamSpec::new("outGain", -60.0, 20.0, 0.0).unit("dB")),
    ]
}

struct Echo {
    knobs: Knobs<5>,
    dsp: Delay,
    peaks: [f32; 2],
}

impl Effect for Echo {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        let s = DelaySettings {
            time: k.get(0),
            feedback: k.get(1),
            lfo_time: k.get(2),
            lfo_amount: k.get(3),
            out_gain: k.get(4),
        };
        let peak = self.dsp.process(frames, &s);
        self.peaks = [self.peaks[0].max(peak[0]), self.peaks[1].max(peak[1])];
    }

    fn reset(&mut self) {
        self.dsp.reset();
    }

    fn adopt(&mut self, old: &mut Self) -> bool {
        std::mem::swap(&mut self.dsp, &mut old.dsp);
        true
    }

    fn display(&mut self, frame: &mut [u8]) {
        put_f32s(frame, &self.peaks);
        self.peaks = [0.0; 2];
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let knobs = Knobs::from_state(state, &KEYS)?;
    let rate = cx.spec.sample_rate as f32;
    EffectProcessor::build(cx, || Ok(Echo { knobs, dsp: Delay::new(rate), peaks: [0.0; 2] }))
}
