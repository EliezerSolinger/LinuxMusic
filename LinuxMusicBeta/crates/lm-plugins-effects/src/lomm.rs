//! LOMM (F-FX-020: legacy `plugins/LOMM`, Lost Robot's OTT-style multiband compressor): three bands (high, mid,
//! low) split by Linkwitz-Riley crossovers, each compressed downward above a threshold and upward below another, with
//! depth, time, knee, range, stereo link, automatic time, lookahead, feedback, mid/side and its own mix.
//!
//! State, version 1: the legacy knobs in camelCase, same ranges, defaults and units (see [`PARAMS`]); the legacy
//! switches are stepped parameters of 0 to 1. With the lookahead on, the signal is delayed by the lookahead's ring
//! (20 ms): reported as latency, and asked again when the switch changes. The window's live frame: for each band
//! (high, mid, low) and side, the detected level and the level after the gain (dBFS, the legacy meters), twelve
//! little-endian `f32`: the three bands' inputs (left, right), then their outputs.

use lm_base::Result;
use lm_dsp::lomm::{Lomm, LommBand, LommSettings};
use lm_plugin::{BuildContext, Descriptor, DisplaySpec, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports, put_f32s};

pub const ID: &str = "lomm";
const COUNT: usize = 50;

/// The parameters: key, min, max, default, scale, unit
pub const PARAMS: [(&str, f64, f64, f64, Scale, &str); COUNT] = [
    ("depth", 0.0, 1.0, 0.4, Scale::Linear, ""),
    ("time", 0.0, 10.0, 1.0, Scale::Linear, ""),
    ("inputVolume", -48.0, 48.0, 0.0, Scale::Linear, "dB"),
    ("outputVolume", -48.0, 48.0, 8.0, Scale::Linear, "dB"),
    ("upwardDepth", 0.0, 2.0, 1.0, Scale::Linear, ""),
    ("downwardDepth", 0.0, 2.0, 1.0, Scale::Linear, ""),
    ("highMidSplit", 20.0, 20000.0, 2500.0, Scale::Log, "Hz"),
    ("midLowSplit", 20.0, 20000.0, 88.3, Scale::Log, "Hz"),
    ("highMidSplitEnabled", 0.0, 1.0, 1.0, Scale::Stepped, ""),
    ("midLowSplitEnabled", 0.0, 1.0, 1.0, Scale::Stepped, ""),
    ("highBandEnabled", 0.0, 1.0, 1.0, Scale::Stepped, ""),
    ("midBandEnabled", 0.0, 1.0, 1.0, Scale::Stepped, ""),
    ("lowBandEnabled", 0.0, 1.0, 1.0, Scale::Stepped, ""),
    ("highInputVolume", -48.0, 48.0, 0.0, Scale::Linear, "dB"),
    ("midInputVolume", -48.0, 48.0, 0.0, Scale::Linear, "dB"),
    ("lowInputVolume", -48.0, 48.0, 0.0, Scale::Linear, "dB"),
    ("highOutputVolume", -48.0, 48.0, 4.6, Scale::Linear, "dB"),
    ("midOutputVolume", -48.0, 48.0, 0.0, Scale::Linear, "dB"),
    ("lowOutputVolume", -48.0, 48.0, 4.6, Scale::Linear, "dB"),
    ("aboveThresholdHigh", -72.0, 0.0, -30.3, Scale::Linear, "dBFS"),
    ("aboveThresholdMid", -72.0, 0.0, -25.0, Scale::Linear, "dBFS"),
    ("aboveThresholdLow", -72.0, 0.0, -28.6, Scale::Linear, "dBFS"),
    ("aboveRatioHigh", 1.0, 99.99, 99.99, Scale::Linear, ""),
    ("aboveRatioMid", 1.0, 99.99, 66.7, Scale::Linear, ""),
    ("aboveRatioLow", 1.0, 99.99, 66.7, Scale::Linear, ""),
    ("belowThresholdHigh", -72.0, 0.0, -35.6, Scale::Linear, "dBFS"),
    ("belowThresholdMid", -72.0, 0.0, -36.6, Scale::Linear, "dBFS"),
    ("belowThresholdLow", -72.0, 0.0, -35.6, Scale::Linear, "dBFS"),
    ("belowRatioHigh", 1.0, 99.99, 4.17, Scale::Linear, ""),
    ("belowRatioMid", 1.0, 99.99, 4.17, Scale::Linear, ""),
    ("belowRatioLow", 1.0, 99.99, 4.17, Scale::Linear, ""),
    ("attackHigh", 0.0, 1000.0, 13.5, Scale::Linear, "ms"),
    ("attackMid", 0.0, 1000.0, 22.4, Scale::Linear, "ms"),
    ("attackLow", 0.0, 1000.0, 47.8, Scale::Linear, "ms"),
    ("releaseHigh", 0.0, 1000.0, 132.0, Scale::Linear, "ms"),
    ("releaseMid", 0.0, 1000.0, 282.0, Scale::Linear, "ms"),
    ("releaseLow", 0.0, 1000.0, 282.0, Scale::Linear, "ms"),
    ("rmsTime", 0.0, 500.0, 10.0, Scale::Linear, "ms"),
    ("knee", 0.0, 36.0, 6.0, Scale::Linear, "dB"),
    ("range", 0.0, 96.0, 36.0, Scale::Linear, "dB"),
    ("balance", -18.0, 18.0, 0.0, Scale::Linear, "dB"),
    ("depthScaling", 0.0, 1.0, 1.0, Scale::Stepped, ""),
    ("stereoLink", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("autoTime", 0.0, 1.0, 0.0, Scale::Linear, ""),
    ("mix", 0.0, 1.0, 1.0, Scale::Linear, ""),
    ("feedback", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("midSide", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("lookahead", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("lookaheadLength", 0.0, 20.0, 0.0, Scale::Linear, "ms"),
    ("lowSideUpwardSuppress", 0.0, 1.0, 0.0, Scale::Stepped, ""),
];
const KEYS: [&str; COUNT] = {
    let mut keys = [""; COUNT];
    let mut i = 0;
    while i < COUNT {
        keys[i] = PARAMS[i].0;
        i += 1;
    }
    keys
};

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.lomm.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: Some(DisplaySpec { bytes: 48 }),
    build,
    // Its knobs add up to 288 dB of gain (input, band input, upward range, band output and output volume: 48 + 48 +
    // 96 + 48 + 48), all legitimate: the tests' 0.5 can become 1.3e14
    tests: TestHints { max_level: 1.0e15, ..TestHints::new() },
};

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:lomm", 1);
    for (key, min, max, def, scale, unit) in PARAMS {
        let mut p = ParamSpec::new(key, min, max, def).scale(scale);
        if !unit.is_empty() {
            p = p.unit(unit);
        }
        s = s.param(p);
    }
    vec![s]
}

struct Multiband {
    knobs: Knobs<COUNT>,
    dsp: Lomm,
}

impl Multiband {
    fn settings(&self) -> LommSettings {
        let k = &self.knobs;
        // Band j (0 high, 1 mid, 2 low): its knobs are j places after the high band's
        let band = |j: usize| LommBand {
            input: k.get(13 + j),
            output: k.get(16 + j),
            above_threshold: k.get(19 + j),
            above_ratio: k.get(22 + j),
            below_threshold: k.get(25 + j),
            below_ratio: k.get(28 + j),
            attack: k.get(31 + j),
            release: k.get(34 + j),
            enabled: k.on(10 + j),
        };
        LommSettings {
            bands: [band(0), band(1), band(2)],
            depth: k.get(0),
            time: k.get(1),
            input: k.get(2),
            output: k.get(3),
            upward: k.get(4),
            downward: k.get(5),
            splits: [k.get(6), k.get(7)],
            splits_enabled: [k.on(8), k.on(9)],
            rms_time: k.get(37),
            knee: k.get(38),
            range: k.get(39),
            balance: k.get(40),
            depth_scaling: k.on(41),
            stereo_link: k.on(42),
            auto_time: k.get(43),
            mix: k.get(44),
            feedback: k.on(45),
            mid_side: k.on(46),
            lookahead_enabled: k.on(47),
            lookahead: k.get(48),
            low_side_upward_suppress: k.on(49),
        }
    }
}

impl Effect for Multiband {
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

    fn latency(&self) -> u32 {
        if self.knobs.on(47) { self.dsp.lookahead_latency() as u32 } else { 0 }
    }

    fn display(&mut self, frame: &mut [u8]) {
        let (i, o) = (self.dsp.display_in, self.dsp.display_out);
        put_f32s(
            frame,
            &[
                i[0][0], i[0][1], i[1][0], i[1][1], i[2][0], i[2][1], o[0][0], o[0][1], o[1][0], o[1][1], o[2][0],
                o[2][1],
            ],
        );
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let knobs = Knobs::from_state(state, &KEYS)?;
    let rate = cx.spec.sample_rate as f32;
    EffectProcessor::build(cx, || Ok(Multiband { knobs, dsp: Lomm::new(rate) }))
}

/// The legacy knobs' names, for importing legacy projects and the A/B: (legacy name, key)
pub const LEGACY_NAMES: [(&str, &str); COUNT] = [
    ("Depth", "depth"),
    ("Time", "time"),
    ("Input Volume", "inputVolume"),
    ("Output Volume", "outputVolume"),
    ("Upward Depth", "upwardDepth"),
    ("Downward Depth", "downwardDepth"),
    ("High/Mid Split", "highMidSplit"),
    ("Mid/Low Split", "midLowSplit"),
    ("Enable High/Mid Split", "highMidSplitEnabled"),
    ("Enable Mid/Low Split", "midLowSplitEnabled"),
    ("Enable High Band", "highBandEnabled"),
    ("Enable Mid Band", "midBandEnabled"),
    ("Enable Low Band", "lowBandEnabled"),
    ("High Input Volume", "highInputVolume"),
    ("Mid Input Volume", "midInputVolume"),
    ("Low Input Volume", "lowInputVolume"),
    ("High Output Volume", "highOutputVolume"),
    ("Mid Output Volume", "midOutputVolume"),
    ("Low Output Volume", "lowOutputVolume"),
    ("Above Threshold High", "aboveThresholdHigh"),
    ("Above Threshold Mid", "aboveThresholdMid"),
    ("Above Threshold Low", "aboveThresholdLow"),
    ("Above Ratio High", "aboveRatioHigh"),
    ("Above Ratio Mid", "aboveRatioMid"),
    ("Above Ratio Low", "aboveRatioLow"),
    ("Below Threshold High", "belowThresholdHigh"),
    ("Below Threshold Mid", "belowThresholdMid"),
    ("Below Threshold Low", "belowThresholdLow"),
    ("Below Ratio High", "belowRatioHigh"),
    ("Below Ratio Mid", "belowRatioMid"),
    ("Below Ratio Low", "belowRatioLow"),
    ("Attack High", "attackHigh"),
    ("Attack Mid", "attackMid"),
    ("Attack Low", "attackLow"),
    ("Release High", "releaseHigh"),
    ("Release Mid", "releaseMid"),
    ("Release Low", "releaseLow"),
    ("RMS Time", "rmsTime"),
    ("Knee", "knee"),
    ("Range", "range"),
    ("Balance", "balance"),
    ("Scale output volume with Depth", "depthScaling"),
    ("Stereo Link", "stereoLink"),
    ("Auto Time", "autoTime"),
    ("Mix", "mix"),
    ("Feedback", "feedback"),
    ("Mid/Side", "midSide"),
    ("Lookahead", "lookahead"),
    ("Lookahead Length", "lookaheadLength"),
    ("Suppress upward compression for side band", "lowSideUpwardSuppress"),
];
