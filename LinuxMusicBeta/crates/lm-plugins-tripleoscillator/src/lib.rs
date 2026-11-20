//! TripleOscillator (F-INS-019; legacy `plugins/TripleOscillator`): three stereo oscillators, the first working with
//! the second and the second with the third (phase or frequency modulation, amplitude modulation, mix, or sync), on
//! the oscillator ported to `lm-dsp` with its A/B (eight wave shapes, band-limited tables, the user's wave).
//!
//! Its state, version 1:
//! - per oscillator `n` (1 to 3), parameters `vol<n>` (0–200 %, 20), `pan<n>` (−100–100), `coarse<n>` (−24–24
//!   semitones: 0, −12, −24), `fineL<n>` and `fineR<n>` (−100–100 cents), `phase<n>` (0–360°), `stereoPhase<n>`
//!   (0–360°), `table<n>` (the band-limited tables, on); fields `wave<n>` (a [`WaveShape`] key, `sine`) and
//!   `userWave<n>` (an audio resource or null: the user's wave, its first channel, one period);
//! - fields `modulation1` (how oscillator 1 works with 2) and `modulation2` (2 with 3): a [`ModulationAlgo`] key,
//!   `mix`;
//! - the instrument functions (`lm_plugin::instrument`): envelopes, LFOs, filter, chords, arpeggio, mono/legato.
//!
//! The legacy plugin read the user's wave from a file path in its settings and swapped it without a lock while
//! notes played; here the wave is a resource of the tree, made into its band-limited tables in the build, and a new
//! wave is a new processor that adopts the notes (arquitetura-nova.md, piece 7).

#![forbid(unsafe_code)]

use std::sync::{Arc, LazyLock};

use lm_base::{Error, Result};
use lm_dsp::oscillator::{ModulationAlgo, OscParams, Oscillator, UserWave, WaveShape, prepare_wave_tables, update};
use lm_plugin::instrument::{Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, PluginKind, Port, Ports, StateView, TestHints, TuningProbe, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};

pub const ID: &str = "tripleoscillator";
/// Voices sounding at once (more steal, U27)
pub const VOICES: usize = 64;
const OSCS: usize = 3;
/// Parameters per oscillator
const PER_OSC: usize = 8;

const VOL: usize = 0;
const PAN: usize = 1;
const COARSE: usize = 2;
const FINE_L: usize = 3;
const FINE_R: usize = 4;
const PHASE: usize = 5;
const STEREO_PHASE: usize = 6;
const TABLE: usize = 7;

static WAVE_KEYS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| WaveShape::ALL.iter().map(|s| s.key()).collect());
static ALGO_KEYS: LazyLock<Box<[&'static str]>> =
    LazyLock::new(|| ModulationAlgo::ALL.iter().map(|a| a.key()).collect());

const PARAM_KEYS: [[&str; PER_OSC]; OSCS] = [
    ["vol1", "pan1", "coarse1", "fineL1", "fineR1", "phase1", "stereoPhase1", "table1"],
    ["vol2", "pan2", "coarse2", "fineL2", "fineR2", "phase2", "stereoPhase2", "table2"],
    ["vol3", "pan3", "coarse3", "fineL3", "fineR3", "phase3", "stereoPhase3", "table3"],
];
const WAVE_FIELDS: [&str; OSCS] = ["wave1", "wave2", "wave3"];
const USER_WAVE_FIELDS: [&str; OSCS] = ["userWave1", "userWave2", "userWave3"];
const MODULATION_FIELDS: [&str; 2] = ["modulation1", "modulation2"];

/// The descriptor
pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.tripleoscillator.name",
    kind: PluginKind::Instrument,
    schemas,
    ports,
    velocity: Velocity::Level,
    display: None,
    build,
    tests: TestHints {
        // The defaults: sines at the key, an octave and two octaves down: the whole repeats at the lowest
        tuning: Some(TuningProbe { state: Value::empty_object, key: 69, hz: 110.0 }),
        no_adopt: &[],
        voices: VOICES,
        max_level: 1.0e3,
        tail: lm_plugin::DEFAULT_TAIL,
    },
};

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:tripleoscillator", 1);
    for (i, keys) in PARAM_KEYS.iter().enumerate() {
        // (20 each: a new instrument as loud as the others, about −12 dB at its loudest: the fork's choice)
        s = s
            .param(ParamSpec::new(keys[VOL], 0.0, 200.0, 20.0).unit("%"))
            .param(ParamSpec::new(keys[PAN], -100.0, 100.0, 0.0))
            .param(ParamSpec::new(keys[COARSE], -24.0, 24.0, -12.0 * i as f64).scale(Scale::Stepped).unit("st"))
            .param(ParamSpec::new(keys[FINE_L], -100.0, 100.0, 0.0).unit("cents"))
            .param(ParamSpec::new(keys[FINE_R], -100.0, 100.0, 0.0).unit("cents"))
            .param(ParamSpec::new(keys[PHASE], 0.0, 360.0, 0.0).unit("°"))
            .param(ParamSpec::new(keys[STEREO_PHASE], 0.0, 360.0, 0.0).unit("°"))
            .param(ParamSpec::new(keys[TABLE], 0.0, 1.0, 1.0).scale(Scale::Stepped));
        s = s
            .field(FieldSpec::choice(WAVE_FIELDS[i], &WAVE_KEYS, "sine"))
            .field(FieldSpec::new(USER_WAVE_FIELDS[i], FieldKind::Resource("audio")).nullable().default(Value::NULL));
    }
    for key in MODULATION_FIELDS {
        s = s.field(FieldSpec::choice(key, &ALGO_KEYS, "mix"));
    }
    vec![add_instrument_functions(s)]
}

fn ports(_: &Value) -> Ports {
    Ports { outputs: vec![Port::new("main", 2)], sidechains: Vec::new() }
}

/// One oscillator's knobs, and what the notes read of them (legacy `OscillatorObject`)
#[derive(Clone, Copy, Debug, Default)]
struct OscKnobs {
    raw: [f32; PER_OSC],
    volume_left: f32,
    volume_right: f32,
    detuning_left: f64,
    detuning_right: f64,
    phase_left: f32,
    phase_right: f32,
    use_table: bool,
}

impl OscKnobs {
    /// The legacy `updateVolume`, `updateDetuning*`, `updatePhaseOffset*`
    fn update(&mut self, rate: f64) {
        let r = &self.raw;
        let (vol, pan) = (r[VOL], r[PAN]);
        if pan >= 0.0 {
            self.volume_left = (1.0 - pan / 100.0) * vol / 100.0;
            self.volume_right = vol / 100.0;
        } else {
            self.volume_left = vol / 100.0;
            self.volume_right = (1.0 + pan / 100.0) * vol / 100.0;
        }
        let coarse = f64::from(r[COARSE]) * 100.0;
        self.detuning_left = libm::exp2((coarse + f64::from(r[FINE_L])) / 1200.0) / rate;
        self.detuning_right = libm::exp2((coarse + f64::from(r[FINE_R])) / 1200.0) / rate;
        self.phase_left = (r[PHASE] + r[STEREO_PHASE]) / 360.0;
        self.phase_right = r[PHASE] / 360.0;
        self.use_table = r[TABLE] >= 0.5;
    }
}

/// The synth: the three oscillators' settings
#[derive(Clone, Debug)]
pub struct TripleOscillator {
    rate: f64,
    knobs: [OscKnobs; OSCS],
    shapes: [WaveShape; OSCS],
    /// How oscillator `i` works with `i + 1` (the last one's is not used)
    algos: [ModulationAlgo; OSCS],
    user_waves: [Option<Arc<UserWave>>; OSCS],
}

/// One note: three oscillators for each side (plain values: cloned without allocating)
#[derive(Clone, Debug)]
pub struct Voice {
    left: [Oscillator; OSCS],
    right: [Oscillator; OSCS],
}

impl TripleOscillator {
    fn params(&self, left: bool, freq: f32) -> [OscParams<'_>; OSCS] {
        std::array::from_fn(|i| {
            let k = &self.knobs[i];
            OscParams {
                shape: self.shapes[i],
                algo: self.algos[i],
                freq,
                detuning: if left { k.detuning_left } else { k.detuning_right },
                phase_offset: if left { k.phase_left } else { k.phase_right },
                volume: if left { k.volume_left } else { k.volume_right },
                use_wave_table: k.use_table,
                user_wave: self.user_waves[i].as_deref(),
            }
        })
    }
}

impl Synth for TripleOscillator {
    type Voice = Voice;
    const PARAMS: usize = OSCS * PER_OSC;

    fn voice(&self) -> Voice {
        let rate = self.rate as f32;
        Voice {
            left: std::array::from_fn(|_| Oscillator::new(0.0, rate, 0)),
            right: std::array::from_fn(|_| Oscillator::new(0.0, rate, 0)),
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let (osc, k) = (index / PER_OSC, index % PER_OSC);
        if let Some(knobs) = self.knobs.get_mut(osc) {
            knobs.raw[k] = value;
            knobs.update(self.rate);
        }
    }

    fn start(&self, voice: &mut Voice, _: &NoteStart, rng: &mut Rng) {
        let rate = self.rate as f32;
        for i in 0..OSCS {
            let k = &self.knobs[i];
            voice.left[i] = Oscillator::new(k.phase_left, rate, rng.next_u64());
            voice.right[i] = Oscillator::new(k.phase_right, rate, rng.next_u64());
        }
    }

    fn render(&self, voice: &mut Voice, freq: f32, left: &mut [f32], right: &mut [f32]) {
        update(&mut voice.left, &self.params(true, freq), left, false);
        update(&mut voice.right, &self.params(false, freq), right, false);
    }

    fn release_ms(&self) -> f32 {
        3.0
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    // The built-in tables are made once, here, never on the audio thread
    prepare_wave_tables();
    let rate = f64::from(cx.spec.sample_rate);
    let mut knobs = [OscKnobs::default(); OSCS];
    for (i, keys) in PARAM_KEYS.iter().enumerate() {
        for (k, key) in keys.iter().enumerate() {
            knobs[i].raw[k] = s.param_f32(key)?;
        }
        knobs[i].update(rate);
    }
    let mut shapes = [WaveShape::Sine; OSCS];
    let mut user_waves: [Option<Arc<UserWave>>; OSCS] = [None, None, None];
    for i in 0..OSCS {
        shapes[i] = s.choice(WAVE_FIELDS[i], WaveShape::from_key)?;
        if let Some(r) = s.resource(USER_WAVE_FIELDS[i])? {
            // A wave that cannot be read matters only if the oscillator plays it, as in the legacy program
            match cx.resources.audio(r) {
                Ok(data) if data.frames() > 0 => {
                    user_waves[i] = Some(Arc::new(UserWave::new(data.channel(0).to_vec())))
                }
                Ok(_) if shapes[i] == WaveShape::UserDefined => {
                    return Err(Error::new("plugin.tripleoscillator.empty_wave").arg("oscillator", i as i64 + 1));
                }
                Err(e) if shapes[i] == WaveShape::UserDefined => return Err(e),
                _ => {}
            }
        }
    }
    let mut algos = [ModulationAlgo::SignalMix; OSCS];
    for (i, key) in MODULATION_FIELDS.iter().enumerate() {
        algos[i] = s.choice(key, ModulationAlgo::from_key)?;
    }
    let synth = TripleOscillator { rate, knobs, shapes, algos, user_waves };
    let params = &DESCRIPTOR.schema().params;
    let instrument = Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?;
    Ok(Box::new(instrument))
}
