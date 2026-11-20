//! Mallets (F-INS-018; legacy `plugins/Stk/Mallets`): tuneful things to bang on, from the Synthesis ToolKit's
//! instruments, ported to Rust ([`stk`]): nine struck bars (`ModalBar`: marimba, vibraphone, agogo, wood 1, reso,
//! wood 2, beats, two fixed, clump), tubular bells (`TubeBell`, FM) and four banded waveguides (`BandedWG`: uniform
//! bar, tuned bar, glass, Tibetan bowl), plucked or bowed. A per-note instrument on the instrument functions.
//!
//! Its state, version 1:
//! - field `preset` ([`mallets::PRESET_KEYS`], `marimba`);
//! - the struck bars' parameters `hardness` (0–128, 64), `position` (0–64, 64), `vibratoGain` (0–128, 0),
//!   `vibratoFreq` (0–128, 0), `stickMix` (0–128, 0); the bells' `modulator`, `crossfade`, `lfoSpeed`, `lfoDepth`,
//!   `adsr` (0–128, 64); the waveguides' `pressure` and `speed` (0.1–128, 64) and `bowed` (a switch, on: the
//!   legacy's saved "strike" flag, which has no knob); `spread` (0–255 samples at 44.1 kHz, 0: the right side's
//!   delay) and `randomness` (0–1, 0: how much the hardness and position, the modulator and crossfade, or the
//!   pressure and speed vary per note), all with the legacy's ranges and defaults (the legacy's step of 0.1 is the
//!   knob's resolution, not a step of the value);
//! - fields `levelled` (each preset's level trimmed and the strike's level proportional to the velocity, as the
//!   legacy's new instruments: on; a project from before it, off) and `oldVersion` (a project from before the presets'
//!   version: velocities up to twice as hard, and the uniform bar's old gain; off);
//! - the instrument functions (`lm_plugin::instrument`).
//!
//! **Velocity** (U28): it shapes the strike (the STK amplitude is the legacy note volume over 200), so the plugin
//! declares `Timbre` and applies the level itself, as the legacy track did. The velocity `v` is the legacy note
//! volume `200 v` (MIDI 127 is the hardest strike, as the legacy program mapped it), and the output is the legacy's
//! (the plugin's, times the track's `volume / 100`) times one half: the rule that makes every level instrument of the
//! new program at velocity `v` sound as the legacy one at volume `200 v`, 6 dB lower.
//!
//! **Models**: a note's voice is built for one of the three models, and an instrument plays one model; changing the
//! preset within a model is adopted (the notes keep their preset, as in the legacy), changing the model is not
//! (U32: the old notes are released in a tail node). The bowed bars need lines of up to 4096 samples per mode; every
//! voice of every model holding them would cost tens of megabytes per instance.

#![forbid(unsafe_code)]

pub mod mallets;
pub mod stk;

use lm_base::{Error, Result};
use lm_dsp::math::LegacyRand;
use lm_plugin::instrument::{Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, PluginKind, Port, Ports, StateView, TestHints, TuningProbe, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use mallets::{BANDED, Knobs, Model, Note, PRESET_KEYS, Start, TUBE_BELL};
use stk::Rate;
use stk::banded_wg::BandedWG;
use stk::modal_bar::ModalBar;
use stk::tube_bell::TubeBell;

pub const ID: &str = "mallets";
/// Voices sounding at once (more steal, U27)
pub const VOICES: usize = 32;

/// The parameters, in index order
pub const PARAM_KEYS: [&str; 15] = [
    "hardness",
    "position",
    "vibratoGain",
    "vibratoFreq",
    "stickMix",
    "modulator",
    "crossfade",
    "lfoSpeed",
    "lfoDepth",
    "adsr",
    "pressure",
    "speed",
    "bowed",
    "spread",
    "randomness",
];
const PARAMS: usize = PARAM_KEYS.len();

/// The descriptor
pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.mallets.name",
    kind: PluginKind::Instrument,
    schemas,
    ports,
    velocity: Velocity::Timbre,
    display: None,
    build,
    tests: TestHints {
        tuning: Some(TuningProbe { state: probe_state, key: 75, hz: 622.253_967_444_161_6 }),
        no_adopt: &[],
        voices: VOICES,
        max_level: TestHints::new().max_level,
        tail: lm_plugin::DEFAULT_TAIL,
    },
};

/// The probe of the generated tuning tests: the bowed tuned bar at D#5. The generated tests measure a period, and the
/// instrument's sounds have none exactly: the bars and bells are inharmonic by design and decay, and the waveguides'
/// modes are inharmonic too and, their lines being whole samples long at 44.1 kHz (the library's), off their ratios
/// by the truncation, which pulls the measured period by up to about 2 cents. The bowed tuned bar lasts and is close
/// to harmonic (modes 1, 4.02, 10.7, 18.1); at D#5 its truncations cancel (the measured period within 0.02 cent of
/// the key at 44.1, 48, 96 and 192 kHz). Every preset's fundamental mode and level at every rate are tested in
/// `tests/rates.rs`
fn probe_state() -> Value {
    Value::empty_object().with("preset", Value::text("tuned-bar"))
}

fn schemas() -> Vec<NodeSchema> {
    let s = NodeSchema::new("plugin:mallets", 1)
        .param(ParamSpec::new("hardness", 0.0, 128.0, 64.0))
        .param(ParamSpec::new("position", 0.0, 64.0, 64.0))
        .param(ParamSpec::new("vibratoGain", 0.0, 128.0, 0.0))
        .param(ParamSpec::new("vibratoFreq", 0.0, 128.0, 0.0))
        .param(ParamSpec::new("stickMix", 0.0, 128.0, 0.0))
        .param(ParamSpec::new("modulator", 0.0, 128.0, 64.0))
        .param(ParamSpec::new("crossfade", 0.0, 128.0, 64.0))
        .param(ParamSpec::new("lfoSpeed", 0.0, 128.0, 64.0))
        .param(ParamSpec::new("lfoDepth", 0.0, 128.0, 64.0))
        .param(ParamSpec::new("adsr", 0.0, 128.0, 64.0))
        .param(ParamSpec::new("pressure", 0.1, 128.0, 64.0))
        .param(ParamSpec::new("speed", 0.1, 128.0, 64.0))
        .param(ParamSpec::new("bowed", 0.0, 1.0, 1.0).scale(Scale::Stepped))
        .param(ParamSpec::new("spread", 0.0, 255.0, 0.0).scale(Scale::Stepped))
        .param(ParamSpec::new("randomness", 0.0, 1.0, 0.0))
        .field(FieldSpec::choice("preset", &PRESET_KEYS, "marimba"))
        .field(FieldSpec::bool("levelled", true))
        .field(FieldSpec::bool("oldVersion", false));
    vec![add_instrument_functions(s)]
}

fn ports(_: &Value) -> Ports {
    Ports { outputs: vec![Port::new("main", 2)], sidechains: Vec::new() }
}

fn knobs(values: &[f32; PARAMS]) -> Knobs {
    Knobs {
        hardness: values[0],
        position: values[1],
        vibrato_gain: values[2],
        vibrato_freq: values[3],
        stick_mix: values[4],
        modulator: values[5],
        crossfade: values[6],
        lfo_speed: values[7],
        lfo_depth: values[8],
        adsr: values[9],
        pressure: values[10],
        speed: values[11],
        bowed: values[12] >= 0.5,
        spread: values[13],
        randomness: values[14],
    }
}

/// The synth of one model: the knobs, the preset new notes play, and a new voice's state
#[derive(Clone, Debug)]
pub struct Mallets<M> {
    rate: Rate,
    values: [f32; PARAMS],
    knobs: Knobs,
    preset: usize,
    levelled: bool,
    old_version: bool,
    fresh: M,
}

impl<M: Model> Synth for Mallets<M> {
    type Voice = Note<M>;
    const PARAMS: usize = PARAMS;
    const VELOCITY_IS_LEVEL: bool = false;

    fn voice(&self) -> Note<M> {
        Note::new(&self.fresh, self.rate)
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(v) = self.values.get_mut(index) {
            *v = value;
            self.knobs = knobs(&self.values);
        }
    }

    fn start(&self, voice: &mut Note<M>, note: &NoteStart, rng: &mut Rng) {
        // (the legacy drew from the program's one fastRand stream; here each note from a seed of the node's)
        let mut rand = LegacyRand::new(rng.next_u64());
        let start = Start {
            preset: self.preset,
            freq: note.freq,
            volume: mallets::legacy_volume(note.velocity),
            levelled: self.levelled,
            old_version: self.old_version,
        };
        voice.start(&self.fresh, &self.knobs, &start, &mut rand);
    }

    fn render(&self, voice: &mut Note<M>, freq: f32, left: &mut [f32], right: &mut [f32]) {
        voice.render(&self.knobs, self.old_version, freq, left, right);
    }
}

fn instrument<M: Model>(
    s: &StateView<'_>,
    cx: &BuildContext<'_>,
    preset: usize,
    values: [f32; PARAMS],
) -> Result<Box<dyn Processor>> {
    let rate = Rate::new(f64::from(cx.spec.sample_rate));
    let synth = Mallets {
        rate,
        values,
        knobs: knobs(&values),
        preset,
        levelled: s.boolean("levelled")?,
        old_version: s.boolean("oldVersion")?,
        fresh: M::new(rate),
    };
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(Instrument::new(synth, s, params, &cx.spec, cx.resources, VOICES)?))
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    // The waves are decoded once, here, never on the audio thread
    stk::rawwaves::prepare();
    let preset = s.choice("preset", |k| PRESET_KEYS.iter().position(|p| *p == k))?;
    let mut values = [0.0f32; PARAMS];
    for (i, key) in PARAM_KEYS.iter().enumerate() {
        values[i] = s.param_f32(key)?;
    }
    if cx.spec.sample_rate == 0 {
        return Err(Error::new("plugin.mallets.rate"));
    }
    if preset < TUBE_BELL {
        instrument::<ModalBar>(&s, cx, preset, values)
    } else if preset < BANDED {
        instrument::<TubeBell>(&s, cx, preset, values)
    } else {
        instrument::<BandedWG>(&s, cx, preset, values)
    }
}
