//! Organic (F-INS-012: legacy `plugins/Organic`): eight oscillators on harmonics of the note, each with its shape,
//! volume, panning and stereo detuning, mixed, through a soft waveshaper; band limited.
//!
//! State, version 1: for each oscillator `n` (1 to 8), parameters `harmonic<n>` (0–17, stepped: the index into
//! 0.5, 0.75, 1, 2, 3 … 16 times the note; oscillator n starts at n − 1), `volume<n>` (0–100 %, 100), `panning<n>`
//! (−100–100, 0), `detune<n>` (−1200–1200 cents, 0: the left that much higher, the right that much lower), and field
//! `shape<n>` (`sine`, `saw`, `square`, `triangle`, `moog-saw`, `exponential`; `sine`: the legacy waveform knob, a
//! choice of six); parameters `distortion` (0–0.99, 0) and `volume` (0–200 %, 100); and the instrument functions.
//! Each note's oscillators start at random phases, as the legacy, drawn from the node's seed.

use std::sync::LazyLock;

use lm_base::Result;
use lm_dsp::organic::{HARMONICS, OSCILLATORS, OrganicOsc, OrganicSettings, OrganicVoice, SHAPES};
use lm_dsp::oscillator::{WaveShape, prepare_wave_tables};
use lm_plugin::instrument::{GRID, Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, TuningProbe, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::stereo_out;

pub const ID: &str = "organic";
pub const VOICES: usize = 64;
const PER_OSC: usize = 4;
const PARAMS: usize = OSCILLATORS * PER_OSC + 2;

const PARAM_KEYS: [[&str; PER_OSC]; OSCILLATORS] = [
    ["harmonic1", "volume1", "panning1", "detune1"],
    ["harmonic2", "volume2", "panning2", "detune2"],
    ["harmonic3", "volume3", "panning3", "detune3"],
    ["harmonic4", "volume4", "panning4", "detune4"],
    ["harmonic5", "volume5", "panning5", "detune5"],
    ["harmonic6", "volume6", "panning6", "detune6"],
    ["harmonic7", "volume7", "panning7", "detune7"],
    ["harmonic8", "volume8", "panning8", "detune8"],
];
pub const SHAPE_FIELDS: [&str; OSCILLATORS] =
    ["shape1", "shape2", "shape3", "shape4", "shape5", "shape6", "shape7", "shape8"];

static SHAPE_KEYS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| SHAPES.iter().map(|s| s.key()).collect());

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.organic.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: None,
    build,
    tests: TestHints {
        // The defaults play 0.5, 0.75, 1, 2 … 6 times the note: the whole repeats at a quarter of it
        tuning: Some(TuningProbe { state: Value::empty_object, key: 69, hz: 110.0 }),
        voices: VOICES,
        ..TestHints::new()
    },
};

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:organic", 1);
    for (i, keys) in PARAM_KEYS.iter().enumerate() {
        s = s
            .param(ParamSpec::new(keys[0], 0.0, (HARMONICS.len() - 1) as f64, i as f64).scale(Scale::Stepped))
            .param(ParamSpec::new(keys[1], 0.0, 100.0, 100.0).unit("%"))
            .param(ParamSpec::new(keys[2], -100.0, 100.0, 0.0))
            .param(ParamSpec::new(keys[3], -1200.0, 1200.0, 0.0).unit("cents"));
    }
    s = s
        .param(ParamSpec::new("distortion", 0.0, 0.99, 0.0))
        .param(ParamSpec::new("volume", 0.0, 200.0, 100.0).unit("%"));
    for key in SHAPE_FIELDS {
        s = s.field(FieldSpec::choice(key, &SHAPE_KEYS, WaveShape::Sine.key()));
    }
    vec![add_instrument_functions(s)]
}

#[derive(Clone, Debug)]
pub struct Organic {
    rate: f32,
    settings: OrganicSettings,
}

impl Synth for Organic {
    type Voice = OrganicVoice;
    const PARAMS: usize = PARAMS;

    fn voice(&self) -> OrganicVoice {
        OrganicVoice::new(self.rate, GRID, 0)
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let s = &mut self.settings;
        match index {
            i if i < OSCILLATORS * PER_OSC => {
                let o = &mut s.oscillators[i / PER_OSC];
                match i % PER_OSC {
                    0 => o.harmonic = value.clamp(0.0, (HARMONICS.len() - 1) as f32) as usize,
                    1 => o.volume = value,
                    2 => o.pan = value,
                    _ => o.detune = value,
                }
            }
            i if i == OSCILLATORS * PER_OSC => s.distortion = value,
            i if i == OSCILLATORS * PER_OSC + 1 => s.volume = value,
            _ => {}
        }
    }

    fn start(&self, voice: &mut OrganicVoice, _: &NoteStart, rng: &mut Rng) {
        voice.restart(self.rate, rng.next_u64());
    }

    fn render(&self, voice: &mut OrganicVoice, freq: f32, left: &mut [f32], right: &mut [f32]) {
        let mut buf = [[0.0f32; 2]; GRID];
        let mut at = 0;
        while at < left.len() {
            let n = (left.len() - at).min(GRID);
            voice.render(&mut buf[..n], &self.settings, freq, self.rate);
            for (i, f) in buf[..n].iter().enumerate() {
                left[at + i] = f[0];
                right[at + i] = f[1];
            }
            at += n;
        }
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    // The built-in tables are made once, here, never on the audio thread
    prepare_wave_tables();
    let mut settings = OrganicSettings::default();
    for (i, keys) in PARAM_KEYS.iter().enumerate() {
        settings.oscillators[i] = OrganicOsc {
            shape: s.choice(SHAPE_FIELDS[i], WaveShape::from_key)?,
            harmonic: s.param_f32(keys[0])?.clamp(0.0, (HARMONICS.len() - 1) as f32) as usize,
            volume: s.param_f32(keys[1])?,
            pan: s.param_f32(keys[2])?,
            detune: s.param_f32(keys[3])?,
        };
    }
    settings.distortion = s.param_f32("distortion")?;
    settings.volume = s.param_f32("volume")?;
    let synth = Organic { rate: cx.spec.sample_rate as f32, settings };
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?))
}
