//! Compressor (F-FX-009: legacy `plugins/Compressor`, Lost Robot's): a feed-forward or feedback compressor and
//! limiter with knee, hold, range, RMS or peak detection, lookahead, mid/side, balance, tilt of the sidechain,
//! automatic attack, release and make-up, audition, stereo linking, and its own mix.
//!
//! State, version 1: the legacy knobs, same ranges and defaults: `threshold` (−60–0 dBFS, −8), `ratio` (1–20, 1.8),
//! `attack` (0.005–250 ms, 10), `release` (1–2500 ms, 100), `knee` (0–96 dB, 12), `hold` (0–500 ms, 0), `range`
//! (−240–0 dB, −240: off), `rmsSize` (0–250 ms, 1), `lookaheadLength` (0–20 ms, 0), `inputBalance` and
//! `outputBalance` (−1–1, 0), `outputGain` and `inputGain` (−60–30 dB, 0), `blend` (0–3, 1), `stereoBalance` (−1–1,
//! 0), `autoAttack` and `autoRelease` (0–100 %, 0), `tilt` (−6–6 dB, 0), `tiltFrequency` (20–20 000 Hz, 150), `mix`
//! (0–100 %, 100); the switches `midSide`, `peakMode`, `limiter`, `autoMakeupGain`, `audition`, `feedback`,
//! `lookahead` (off); field `stereoLink` (`unlinked`, `maximum`, `average`, `minimum`, `blend`; `maximum`).
//!
//! With the lookahead on, the signal is delayed by the lookahead's ring (20 ms): reported as latency, and asked again
//! when the switch changes. The window's live frame: the input's and the output's peaks of each side since the last
//! frame and the gain applied last to each side (linear), six little-endian `f32`.

use std::sync::LazyLock;

use lm_base::Result;
use lm_dsp::compressor::{CompressorPlugin, CompressorPluginSettings, StereoLink};
use lm_plugin::{BuildContext, Descriptor, DisplaySpec, PluginKind, StateView, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports, put_f32s, raise_peaks};

pub const ID: &str = "compressor";

/// The parameters: key, min, max, default, scale, unit
const PARAMS: [(&str, f64, f64, f64, Scale, &str); 27] = [
    ("threshold", -60.0, 0.0, -8.0, Scale::Linear, "dBFS"),
    ("ratio", 1.0, 20.0, 1.8, Scale::Log, ""),
    ("attack", 0.005, 250.0, 10.0, Scale::Log, "ms"),
    ("release", 1.0, 2500.0, 100.0, Scale::Log, "ms"),
    ("knee", 0.0, 96.0, 12.0, Scale::Linear, "dB"),
    ("hold", 0.0, 500.0, 0.0, Scale::Linear, "ms"),
    ("range", -240.0, 0.0, -240.0, Scale::Linear, "dB"),
    ("rmsSize", 0.0, 250.0, 1.0, Scale::Linear, "ms"),
    ("midSide", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("peakMode", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("lookaheadLength", 0.0, 20.0, 0.0, Scale::Linear, "ms"),
    ("inputBalance", -1.0, 1.0, 0.0, Scale::Linear, ""),
    ("outputBalance", -1.0, 1.0, 0.0, Scale::Linear, ""),
    ("limiter", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("outputGain", -60.0, 30.0, 0.0, Scale::Linear, "dB"),
    ("inputGain", -60.0, 30.0, 0.0, Scale::Linear, "dB"),
    ("blend", 0.0, 3.0, 1.0, Scale::Linear, ""),
    ("stereoBalance", -1.0, 1.0, 0.0, Scale::Linear, ""),
    ("autoMakeupGain", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("audition", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("feedback", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("autoAttack", 0.0, 100.0, 0.0, Scale::Linear, "%"),
    ("autoRelease", 0.0, 100.0, 0.0, Scale::Linear, "%"),
    ("lookahead", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("tilt", -6.0, 6.0, 0.0, Scale::Linear, "dB"),
    ("tiltFrequency", 20.0, 20_000.0, 150.0, Scale::Log, "Hz"),
    ("mix", 0.0, 100.0, 100.0, Scale::Linear, "%"),
];
const KEYS: [&str; 27] = {
    let mut keys = [""; 27];
    let mut i = 0;
    while i < 27 {
        keys[i] = PARAMS[i].0;
        i += 1;
    }
    keys
};

static LINK_KEYS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| StereoLink::ALL.iter().map(|l| l.key()).collect());

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.compressor.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: Some(DisplaySpec { bytes: 24 }),
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:compressor", 1);
    for (key, min, max, def, scale, unit) in PARAMS {
        let mut p = ParamSpec::new(key, min, max, def).scale(scale);
        if !unit.is_empty() {
            p = p.unit(unit);
        }
        s = s.param(p);
    }
    vec![s.field(FieldSpec::choice("stereoLink", &LINK_KEYS, StereoLink::Maximum.key()))]
}

struct Compress {
    knobs: Knobs<27>,
    link: StereoLink,
    dsp: CompressorPlugin,
    /// Input and output peaks since the window's last frame
    peaks: [[f32; 2]; 2],
}

impl Compress {
    fn settings(&self) -> CompressorPluginSettings {
        let k = &self.knobs;
        CompressorPluginSettings {
            threshold: k.get(0),
            ratio: k.get(1),
            attack: k.get(2),
            release: k.get(3),
            knee: k.get(4),
            hold: k.get(5),
            range: k.get(6),
            rms: k.get(7),
            mid_side: k.on(8),
            peak_mode: k.on(9),
            lookahead_length: k.get(10),
            in_balance: k.get(11),
            out_balance: k.get(12),
            limiter: k.on(13),
            out_gain: k.get(14),
            in_gain: k.get(15),
            blend: k.get(16),
            stereo_balance: k.get(17),
            auto_makeup: k.on(18),
            audition: k.on(19),
            feedback: k.on(20),
            auto_attack: k.get(21),
            auto_release: k.get(22),
            lookahead: k.on(23),
            tilt: k.get(24),
            tilt_freq: k.get(25),
            stereo_link: self.link,
            mix: k.get(26),
        }
    }
}

impl Effect for Compress {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        raise_peaks(&mut self.peaks[0], frames);
        let s = self.settings();
        self.dsp.process(frames, &s);
        raise_peaks(&mut self.peaks[1], frames);
    }

    fn reset(&mut self) {
        self.dsp.reset();
    }

    fn adopt(&mut self, old: &mut Self) -> bool {
        std::mem::swap(&mut self.dsp, &mut old.dsp);
        true
    }

    fn latency(&self) -> u32 {
        if self.knobs.on(23) { self.dsp.lookahead_latency() as u32 } else { 0 }
    }

    fn display(&mut self, frame: &mut [u8]) {
        let [input, output] = self.peaks;
        let gains = self.dsp.gains();
        put_f32s(frame, &[input[0], input[1], output[0], output[1], gains[0], gains[1]]);
        self.peaks = [[0.0; 2]; 2];
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let knobs = Knobs::from_state(state, &KEYS)?;
    let link = StateView::new(state).choice("stereoLink", StereoLink::from_key)?;
    let rate = cx.spec.sample_rate as f32;
    EffectProcessor::build(cx, || Ok(Compress { knobs, link, dsp: CompressorPlugin::new(rate), peaks: [[0.0; 2]; 2] }))
}
