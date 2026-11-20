//! BitInvader (F-INS-003: legacy `plugins/BitInvader`): a drawn wave of up to 200 points played as a note, stepped
//! or interpolated, band limited or as drawn, normalized or not.
//!
//! State, version 1: parameter `normalize` (a switch, off: read when a note starts, as the legacy); fields
//! `sampleLength` (4–200, 200: how many of the points the wave has), `wave` (the points, −1 to 1, 200 of them; a sine
//! by default; a list of another length is read as that many points, at most 200), `interpolation` (off) and
//! `antiAliasing` (on), and the instrument functions. The legacy interpolation and anti-aliasing switches rebuilt the
//! band-limited tables: here they are fields, whose change builds the tables away from the audio thread (the notes
//! cross-fade into the new sound).

use std::sync::Arc;

use lm_base::Result;
use lm_dsp::bit_invader::{BitInvaderVoice, BitInvaderWave, RELEASE_MS, WAVETABLE_SIZE};
use lm_dsp::oscillator::sin_sample;
use lm_plugin::instrument::{GRID, Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, TuningProbe, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::stereo_out;

pub const ID: &str = "bitinvader";
pub const VOICES: usize = 64;

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.bitinvader.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: None,
    build,
    tests: TestHints {
        // The default sine at its key
        tuning: Some(TuningProbe { state: Value::empty_object, key: 69, hz: 440.0 }),
        voices: VOICES,
        ..TestHints::new()
    },
};

/// The legacy graph's sine (`setWaveToSine`): point i is sin(2π i / 200)
pub fn default_wave() -> Vec<f32> {
    (0..WAVETABLE_SIZE).map(|i| sin_sample(i as f32 / WAVETABLE_SIZE as f32)).collect()
}

fn schemas() -> Vec<NodeSchema> {
    let sine = Value::list(default_wave().iter().map(|v| Value::real(f64::from(*v)).unwrap_or(Value::NULL)));
    let s = NodeSchema::new("plugin:bitinvader", 1)
        .param(ParamSpec::new("normalize", 0.0, 1.0, 0.0).scale(Scale::Stepped))
        .field(FieldSpec::int("sampleLength", 4, WAVETABLE_SIZE as i64, WAVETABLE_SIZE as i64))
        .field(FieldSpec::new("wave", FieldKind::List(Box::new(FieldKind::Real { min: -1.0, max: 1.0 }))).default(sine))
        .field(FieldSpec::bool("interpolation", false))
        .field(FieldSpec::bool("antiAliasing", true));
    vec![add_instrument_functions(s)]
}

#[derive(Clone, Debug)]
pub struct BitInvader {
    rate: f32,
    wave: Arc<BitInvaderWave>,
    normalize: bool,
}

impl Synth for BitInvader {
    type Voice = BitInvaderVoice;
    const PARAMS: usize = 1;

    fn voice(&self) -> BitInvaderVoice {
        // (made for the whole wave: a note's start reuses its room)
        BitInvaderVoice::new(&self.wave, true)
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if index == 0 {
            self.normalize = value >= 0.5;
        }
    }

    fn start(&self, voice: &mut BitInvaderVoice, _: &NoteStart, _: &mut Rng) {
        voice.restart(&self.wave, self.normalize);
    }

    fn render(&self, voice: &mut BitInvaderVoice, freq: f32, left: &mut [f32], right: &mut [f32]) {
        let mut buf = [[0.0f32; 2]; GRID];
        let mut at = 0;
        while at < left.len() {
            let n = (left.len() - at).min(GRID);
            voice.render(&mut buf[..n], &self.wave, freq, self.rate);
            for (i, f) in buf[..n].iter().enumerate() {
                left[at + i] = f[0];
                right[at + i] = f[1];
            }
            at += n;
        }
    }

    fn release_ms(&self) -> f32 {
        RELEASE_MS
    }
}

/// The wave the notes play: the state's first `sampleLength` points (as many as it has, if fewer; the default sine
/// if none)
pub fn wave_of(points: &[f64], length: usize) -> Vec<f32> {
    if points.is_empty() {
        return default_wave()[..length.min(WAVETABLE_SIZE)].to_vec();
    }
    points.iter().take(length.clamp(1, WAVETABLE_SIZE)).map(|v| *v as f32).collect()
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let length = s.int("sampleLength")?.clamp(1, WAVETABLE_SIZE as i64) as usize;
    let points: Vec<f64> =
        state.get("wave").map(|l| l.items().filter_map(Value::as_real).collect()).unwrap_or_default();
    let wave = BitInvaderWave::new(&wave_of(&points, length), s.boolean("interpolation")?, s.boolean("antiAliasing")?);
    let synth = BitInvader {
        rate: cx.spec.sample_rate as f32,
        wave: Arc::new(wave),
        normalize: s.param_f32("normalize")? >= 0.5,
    };
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?))
}
