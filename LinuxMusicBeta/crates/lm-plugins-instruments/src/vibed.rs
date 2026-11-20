//! Vibed (F-INS-020: legacy `plugins/Vibed`): up to nine plucked strings (Karplus-Strong waveguides, oversampled),
//! each on a harmonic of the note, struck with a drawn impulse, panned and mixed.
//!
//! State, version 1: for each string `n` (1 to 9), parameters `string<n>` (a switch: the legacy power LED, on for the
//! first string), `volume<n>` (0–200 %, 100), `stiffness<n>` (0–0.05, 0), `pick<n>` (0–0.05, 0), `pickup<n>`
//! (0–0.05, 0.05), `panning<n>` (−1–1, 0), `detune<n>` (−0.1–0.1, 0), `fuzziness<n>` (0–0.75, 0), `impulse<n>` (a
//! switch, off: the impulse as drawn, not stretched over the string); fields `harmonic<n>` (`1/4`, `1/2`, `1`, `2`,
//! `3`, `4`, `5`, `6`, `7`; `1`), `length<n>` (1–16, 1: the string's oversampling) and `shape<n>` (the impulse, 128
//! points from −1 to 1, a sine by default); and the instrument functions.
//!
//! The strings run at their own rate, twice their length knob times 44.1 kHz, at any project rate (at 44.1 kHz,
//! exactly the legacy strings; the legacy ran them at a multiple of the project rate, and their decay and tone
//! changed with it).
//!
//! As the legacy plugin, a note's strings are set when it starts (the volume and panning follow the knobs) and are not
//! bent. A string's delay lines are made in the build, long enough for the lowest note (key 0 at the lowest pitch
//! knob) with its harmonic and length, so a note never allocates; a lower pitch (a tuning expression below key 0)
//! plays the string as long as its room. The harmonic and the length decide that room, and are fields (the legacy
//! knobs were automatable): a longer string than the old instrument's voices hold cannot be adopted, the old one plays
//! its tail apart (U32).

use std::sync::LazyLock;

use lm_base::Result;
use lm_dsp::vibed::{IMPULSE_LENGTH, OCTAVES, STRINGS, VibedString, VibedVoice};
use lm_plugin::instrument::{GRID, Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::stereo_out;

pub const ID: &str = "vibed";
pub const VOICES: usize = 16;
/// Parameters per string
const PER_STRING: usize = 9;
const PARAMS: usize = STRINGS * PER_STRING;
const NAMES: [&str; PER_STRING] =
    ["string", "volume", "stiffness", "pick", "pickup", "panning", "detune", "fuzziness", "impulse"];
pub const HARMONIC_KEYS: [&str; 9] = ["1/4", "1/2", "1", "2", "3", "4", "5", "6", "7"];
static HARMONICS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| HARMONIC_KEYS.into());
/// The lowest note a string is made for: key 0 at the pitch knob's lowest (−100 cents)
const LOWEST_HZ: f32 = 8.175_799 * 0.943_874_3;

/// The keys of string `n` (0-based): its parameters, then its fields (harmonic, length, shape)
fn key(name: &str, n: usize) -> &'static str {
    static KEYS: LazyLock<Vec<String>> = LazyLock::new(|| {
        let names = NAMES.iter().chain(["harmonic", "length", "shape"].iter());
        (0..STRINGS).flat_map(|n| names.clone().map(move |name| format!("{name}{}", n + 1))).collect()
    });
    let names = NAMES.len() + 3;
    let i = NAMES.iter().chain(["harmonic", "length", "shape"].iter()).position(|x| *x == name).unwrap_or(0);
    KEYS[n * names + i].as_str()
}

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.vibed.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: None,
    build,
    // (no tuning probe: a plucked string dies away, and its pitch is its delay lines' whole samples, as the legacy:
    // within a few cents, not the one cent of U36)
    tests: TestHints {
        no_adopt: &["length1", "length2", "length3", "length4", "length5", "length6", "length7", "length8", "length9"],
        voices: VOICES,
        ..TestHints::new()
    },
};

/// The legacy graph's sine (`setWaveToSine`) over the impulse's points
pub fn default_shape() -> [f32; IMPULSE_LENGTH] {
    VibedString::default().shape
}

fn schemas() -> Vec<NodeSchema> {
    let sine = Value::list(default_shape().iter().map(|v| Value::real(f64::from(*v)).unwrap_or(Value::NULL)));
    let mut s = NodeSchema::new("plugin:vibed", 1);
    for n in 0..STRINGS {
        let switch = |k, on: bool| ParamSpec::new(k, 0.0, 1.0, if on { 1.0 } else { 0.0 }).scale(Scale::Stepped);
        s = s
            .param(switch(key("string", n), n == 0))
            .param(ParamSpec::new(key("volume", n), 0.0, 200.0, 100.0).unit("%"))
            .param(ParamSpec::new(key("stiffness", n), 0.0, 0.05, 0.0))
            .param(ParamSpec::new(key("pick", n), 0.0, 0.05, 0.0))
            .param(ParamSpec::new(key("pickup", n), 0.0, 0.05, 0.05))
            .param(ParamSpec::new(key("panning", n), -1.0, 1.0, 0.0))
            .param(ParamSpec::new(key("detune", n), -0.1, 0.1, 0.0))
            .param(ParamSpec::new(key("fuzziness", n), 0.0, 0.75, 0.0))
            .param(switch(key("impulse", n), false));
    }
    for n in 0..STRINGS {
        s = s
            .field(FieldSpec::choice(key("harmonic", n), &HARMONICS, "1"))
            .field(FieldSpec::int(key("length", n), 1, 16, 1))
            .field(
                FieldSpec::new(key("shape", n), FieldKind::List(Box::new(FieldKind::Real { min: -1.0, max: 1.0 })))
                    .default(sine.clone()),
            );
    }
    vec![add_instrument_functions(s)]
}

#[derive(Clone, Debug)]
pub struct Vibed {
    rate: u32,
    /// The strings as the knobs are now (the shapes, harmonics and lengths from the fields)
    knobs: [VibedString; STRINGS],
    rooms: [usize; STRINGS],
}

impl Synth for Vibed {
    type Voice = VibedVoice;
    const PARAMS: usize = PARAMS;

    fn voice(&self) -> VibedVoice {
        VibedVoice::with_rooms(self.rooms)
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let (n, k) = (index / PER_STRING, index % PER_STRING);
        let Some(s) = self.knobs.get_mut(n) else { return };
        match k {
            0 => s.enabled = value >= 0.5,
            1 => s.volume = value,
            2 => s.stiffness = value,
            3 => s.pick = value,
            4 => s.pickup = value,
            5 => s.pan = value,
            6 => s.detune = value,
            7 => s.fuzziness = value,
            _ => s.impulse = value >= 0.5,
        }
    }

    fn start(&self, voice: &mut VibedVoice, note: &NoteStart, rng: &mut Rng) {
        voice.restart(&self.knobs, note.freq, self.rate, rng.next_u64());
    }

    fn render(&self, voice: &mut VibedVoice, _freq: f32, left: &mut [f32], right: &mut [f32]) {
        let mut buf = [[0.0f32; 2]; GRID];
        let mut at = 0;
        while at < left.len() {
            let n = (left.len() - at).min(GRID);
            voice.render(&mut buf[..n], &self.knobs);
            for (i, f) in buf[..n].iter().enumerate() {
                left[at + i] = f[0];
                right[at + i] = f[1];
            }
            at += n;
        }
    }

    fn can_adopt(&self, old: &Self) -> bool {
        self.rooms.iter().zip(&old.rooms).all(|(new, old)| new <= old)
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let mut knobs: [VibedString; STRINGS] = core::array::from_fn(|_| VibedString::default());
    for (n, string) in knobs.iter_mut().enumerate() {
        string.enabled = s.param_f32(key("string", n))? >= 0.5;
        string.volume = s.param_f32(key("volume", n))?;
        string.stiffness = s.param_f32(key("stiffness", n))?;
        string.pick = s.param_f32(key("pick", n))?;
        string.pickup = s.param_f32(key("pickup", n))?;
        string.pan = s.param_f32(key("panning", n))?;
        string.detune = s.param_f32(key("detune", n))?;
        string.fuzziness = s.param_f32(key("fuzziness", n))?;
        string.impulse = s.param_f32(key("impulse", n))? >= 0.5;
        string.harmonic = s.choice(key("harmonic", n), |k| HARMONIC_KEYS.iter().position(|h| *h == k))?;
        string.length = s.int(key("length", n))?.clamp(1, 16) as i32;
        let points: Vec<f32> = state
            .get(key("shape", n))
            .map(|l| l.items().filter_map(Value::as_real).map(|v| v as f32).collect())
            .unwrap_or_default();
        // A drawn impulse of another length: as many of its points as fit, the sine's after
        for (i, p) in string.shape.iter_mut().enumerate().take(points.len()) {
            *p = points[i];
        }
    }
    debug_assert_eq!(OCTAVES.len(), HARMONIC_KEYS.len());
    let rate = cx.spec.sample_rate;
    let rooms = VibedVoice::rooms(&knobs, LOWEST_HZ, rate);
    let synth = Vibed { rate, knobs, rooms };
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?))
}
