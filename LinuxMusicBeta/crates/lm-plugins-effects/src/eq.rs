//! Eq (F-FX-015: legacy `plugins/Eq`): input and output gain, a high-pass and a low-pass of 12, 24 or 48 dB/oct,
//! two shelves and four peaks, each band switched on or off.
//!
//! State, version 1: parameters `inputGain` and `outputGain` (−60–20 dB, 0); `hpActive`, `hpFreq` (20–20 000 Hz, 31),
//! `hpRes` (0.003–10, 0.707); `lowShelfActive`, `lowShelfFreq` (80), `lowShelfRes` (0.55–10, 0.707),
//! `lowShelfGain` (−18–18 dB, 0); for each peak `n` (1–4) `peak<n>Active`, `peak<n>Freq` (120, 250, 2000, 4000),
//! `peak<n>Bw` (0.1–4 octaves, 0.3), `peak<n>Gain` (−18–18 dB, 0); `highShelfActive`, `highShelfFreq` (12 000),
//! `highShelfRes`, `highShelfGain`; `lpActive`, `lpFreq` (18 000), `lpRes`. Every `…Active` is a switch, off.
//! Fields `hpSlope` and `lpSlope` (`12db`, `24db`, `48db`; `12db`), and `analyseIn` and `analyseOut` (the window's
//! spectrum of the input and of the output, on; they change nothing in the sound).
//!
//! The resonance of a 24 or 48 dB/oct pass is on one of its sections, the others at the legacy default (0.707): the
//! legacy put it on every section, so that a resonance of 10 at 48 dB/oct peaked at +80 dB. At the default resonance
//! the two are the same.
//!
//! The window's live frame: the peaks of the input and of the output of each side since the last frame, four
//! little-endian `f32` (the legacy faders' meters). The spectrum the window draws is the host's analysis of the
//! channel's audio, not part of the plugin's frame.

use std::sync::LazyLock;

use lm_base::Result;
use lm_dsp::eq::{Eq, EqBand, EqSettings, Slope};
use lm_dsp::spectrum::{BANDS, Spectrum};
use lm_plugin::{BuildContext, Descriptor, DisplaySpec, PluginKind, StateView, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports, put_f32s, raise_peaks};

pub const ID: &str = "eq";

/// The live frame, in `f32` words: `[0..4)` the input's then the output's peaks (left, right; linear), `[4]` and `[5]`
/// the lowest and the highest frequency of the spectrum's bands (Hz, logarithmically spaced from one to the other),
/// `[6]` the bands per spectrum, `[7]` unused (0), `[8..8+BANDS)` the input's spectrum and `[8+BANDS..8+2*BANDS)` the
/// output's, in dBFS (NaN while its analyser is off, the `analyseIn` and `analyseOut` fields)
pub const FRAME_FLOATS: usize = 8 + 2 * BANDS;
/// Where the input's and the output's spectra start in the frame
pub const SPECTRUM_IN: usize = 8;
pub const SPECTRUM_OUT: usize = 8 + BANDS;
const COUNT: usize = 32;
const KEYS: [&str; COUNT] = [
    "inputGain",
    "outputGain",
    "hpActive",
    "hpFreq",
    "hpRes",
    "lowShelfActive",
    "lowShelfFreq",
    "lowShelfRes",
    "lowShelfGain",
    "peak1Active",
    "peak1Freq",
    "peak1Bw",
    "peak1Gain",
    "peak2Active",
    "peak2Freq",
    "peak2Bw",
    "peak2Gain",
    "peak3Active",
    "peak3Freq",
    "peak3Bw",
    "peak3Gain",
    "peak4Active",
    "peak4Freq",
    "peak4Bw",
    "peak4Gain",
    "highShelfActive",
    "highShelfFreq",
    "highShelfRes",
    "highShelfGain",
    "lpActive",
    "lpFreq",
    "lpRes",
];
const PEAK_FREQS: [f64; 4] = [120.0, 250.0, 2000.0, 4000.0];
pub const SLOPE_KEYS: &[&str] = &["12db", "24db", "48db"];

static SLOPES: LazyLock<Box<[&'static str]>> = LazyLock::new(|| SLOPE_KEYS.into());

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.eq.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: Some(DisplaySpec { bytes: 4 * FRAME_FLOATS }),
    build,
    tests: TestHints::new(),
};

fn slope(key: &str) -> Option<Slope> {
    match key {
        "12db" => Some(Slope::Db12),
        "24db" => Some(Slope::Db24),
        "48db" => Some(Slope::Db48),
        _ => None,
    }
}

fn schemas() -> Vec<NodeSchema> {
    let switch = |key| ParamSpec::new(key, 0.0, 1.0, 0.0).scale(Scale::Stepped);
    let freq = |key, def| ParamSpec::new(key, 20.0, 20_000.0, def).scale(Scale::Log).unit("Hz");
    let gain = |key| ParamSpec::new(key, -18.0, 18.0, 0.0).unit("dB");
    let mut s = NodeSchema::new("plugin:eq", 1)
        .param(ParamSpec::new(KEYS[0], -60.0, 20.0, 0.0).unit("dB"))
        .param(ParamSpec::new(KEYS[1], -60.0, 20.0, 0.0).unit("dB"))
        .param(switch(KEYS[2]))
        .param(freq(KEYS[3], 31.0))
        .param(ParamSpec::new(KEYS[4], 0.003, 10.0, 0.707))
        .param(switch(KEYS[5]))
        .param(freq(KEYS[6], 80.0))
        .param(ParamSpec::new(KEYS[7], 0.55, 10.0, 0.707))
        .param(gain(KEYS[8]));
    for (p, def) in PEAK_FREQS.iter().enumerate() {
        let k = 9 + 4 * p;
        s = s
            .param(switch(KEYS[k]))
            .param(freq(KEYS[k + 1], *def))
            .param(ParamSpec::new(KEYS[k + 2], 0.1, 4.0, 0.3).unit("oct"))
            .param(gain(KEYS[k + 3]));
    }
    s = s
        .param(switch(KEYS[25]))
        .param(freq(KEYS[26], 12_000.0))
        .param(ParamSpec::new(KEYS[27], 0.55, 10.0, 0.707))
        .param(gain(KEYS[28]))
        .param(switch(KEYS[29]))
        .param(freq(KEYS[30], 18_000.0))
        .param(ParamSpec::new(KEYS[31], 0.003, 10.0, 0.707));
    vec![
        s.field(FieldSpec::choice("hpSlope", &SLOPES, "12db"))
            .field(FieldSpec::choice("lpSlope", &SLOPES, "12db"))
            .field(FieldSpec::bool("analyseIn", true))
            .field(FieldSpec::bool("analyseOut", true)),
    ]
}

struct Equalizer {
    knobs: Knobs<COUNT>,
    slopes: [Slope; 2],
    dsp: Eq,
    peaks: [[f32; 2]; 2],
    /// The analysers (input, output), made with the plugin; `None`: that analyser is off
    spectra: [Option<Spectrum>; 2],
}

impl Equalizer {
    fn settings(&self) -> EqSettings {
        let k = &self.knobs;
        let band = |at: usize, gain: bool| EqBand {
            active: k.on(at),
            freq: k.get(at + 1),
            q: k.get(at + 2),
            gain: if gain { k.get(at + 3) } else { 0.0 },
        };
        EqSettings {
            in_gain: k.get(0),
            out_gain: k.get(1),
            high_pass: band(2, false),
            high_pass_slope: self.slopes[0],
            low_shelf: band(5, true),
            peaks: [band(9, true), band(13, true), band(17, true), band(21, true)],
            high_shelf: band(25, true),
            low_pass: band(29, false),
            low_pass_slope: self.slopes[1],
            resonance_on_every_section: false,
        }
    }
}

impl Effect for Equalizer {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        raise_peaks(&mut self.peaks[0], frames);
        if let Some(a) = &mut self.spectra[0] {
            a.push(frames);
        }
        let s = self.settings();
        let out = self.dsp.process(frames, &s);
        if let Some(a) = &mut self.spectra[1] {
            a.push(frames);
        }
        self.peaks[1] = [self.peaks[1][0].max(out[0]), self.peaks[1][1].max(out[1])];
    }

    fn reset(&mut self) {
        self.dsp.reset();
        for a in self.spectra.iter_mut().flatten() {
            a.reset();
        }
    }

    fn adopt(&mut self, old: &mut Self) -> bool {
        std::mem::swap(&mut self.dsp, &mut old.dsp);
        true
    }

    fn display(&mut self, frame: &mut [u8]) {
        let [i, o] = self.peaks;
        put_f32s(frame, &[i[0], i[1], o[0], o[1]]);
        self.peaks = [[0.0; 2]; 2];
        let mut words = |at: usize, values: &[f32]| {
            for (chunk, v) in frame[4 * at..].as_chunks_mut::<4>().0.iter_mut().zip(values) {
                chunk.copy_from_slice(&v.to_le_bytes());
            }
        };
        if let Some(a) = self.spectra.iter().flatten().next() {
            let (lo, hi) = a.range_hz();
            words(4, &[lo, hi, BANDS as f32, 0.0]);
        }
        for (k, at) in [SPECTRUM_IN, SPECTRUM_OUT].into_iter().enumerate() {
            match &mut self.spectra[k] {
                Some(a) => {
                    a.update();
                    words(at, a.db());
                }
                None => words(at, &[f32::NAN; BANDS]),
            }
        }
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let knobs = Knobs::from_state(state, &KEYS)?;
    let view = StateView::new(state);
    let slopes = [view.choice("hpSlope", slope)?, view.choice("lpSlope", slope)?];
    let rate = cx.spec.sample_rate as f32;
    let analyse = [view.boolean("analyseIn")?, view.boolean("analyseOut")?];
    EffectProcessor::build(cx, || {
        let spectra = analyse.map(|on| on.then(|| Spectrum::new(rate)));
        Ok(Equalizer { knobs, slopes, dsp: Eq::new(rate), peaks: [[0.0; 2]; 2], spectra })
    })
}
