//! Watsyn (F-INS-021: legacy `plugins/Watsyn`): four drawn waves in two pairs (A1 with A2, B1 with B2), the second of
//! each pair mixed with or modulating the first (mix, AM, RM, PM), A1 talking into B2, and the two pairs crossfaded by
//! a mix with its own envelope.
//!
//! State, version 1: for each oscillator `o` of `a1`, `a2`, `b1`, `b2`: parameters `<o>Volume` (0–200, 32),
//! `<o>Panning` (−100–100, 0), `<o>Multiplier` (1–24, 8: the note's frequency times this over 8), `<o>LeftDetune`
//! and `<o>RightDetune` (−600–600 cents, 0); `abMix` (−100 A – 100 B, 0), `envAmount` (−200–200, 0), `envAttack`,
//! `envHold` and `envDecay` (0–2000 ms, 0), `crosstalk` (0–100, 0); fields `<o>Graph` (the wave, 220 points from −1
//! to 1, a sine by default), `aMod` and `bMod` (`mix`, `am`, `rm`, `pm`; `mix`), `antiAliasing` (on), and a tempo
//! sync for each envelope time, `envAttackSync`, `envHoldSync`, `envDecaySync` (`off`, `2/1` … `1/32`; `off`: synced,
//! the knob is the note length at the song's tempo, at most 2000 ms, legacy `TempoSyncKnob`); and the instrument
//! functions.
//!
//! As the legacy, the waves are oversampled 32 times by libsamplerate's fastest sinc when they change (in the build,
//! off the audio thread), with band-limited tables when anti-aliasing is on; a note keeps the pairs' modulations it
//! started with; the other knobs act on the sounding notes.

use std::sync::{Arc, LazyLock};

use lm_base::Result;
use lm_dsp::oscillator::sin_sample;
use lm_dsp::watsyn::{
    GRAPH_LENGTH, RELEASE_MS, WAVE_RATIO, WatsynMod, WatsynSettings, WatsynVoice, WatsynWaves, oversample,
};
use lm_plugin::instrument::{GRID, Instrument, NoteStart, SYNC_KEYS, Synth, add_instrument_functions, sync_beats};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};
use lm_sys_samplerate::{Quality, Resampler};

use crate::stereo_out;

pub const ID: &str = "watsyn";
pub const VOICES: usize = 32;
pub const OSCILLATORS: [&str; 4] = ["a1", "a2", "b1", "b2"];
pub const MOD_KEYS: [&str; 4] = ["mix", "am", "rm", "pm"];
static MODS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| MOD_KEYS.into());
static SYNCS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| SYNC_KEYS.into());
/// Per oscillator: key suffix, minimum, maximum, default, stepped
const PER_OSC: [(&str, f64, f64, f64, bool); 5] = [
    ("Volume", 0.0, 200.0, 32.0, false),
    ("Panning", -100.0, 100.0, 0.0, false),
    ("Multiplier", 1.0, 24.0, 8.0, true),
    ("LeftDetune", -600.0, 600.0, 0.0, false),
    ("RightDetune", -600.0, 600.0, 0.0, false),
];
/// After the oscillators' parameters: key, minimum, maximum, default, unit
pub const GLOBALS: [(&str, f64, f64, f64, &str); 6] = [
    ("abMix", -100.0, 100.0, 0.0, ""),
    ("envAmount", -200.0, 200.0, 0.0, ""),
    ("envAttack", 0.0, 2000.0, 0.0, "ms"),
    ("envHold", 0.0, 2000.0, 0.0, "ms"),
    ("envDecay", 0.0, 2000.0, 0.0, "ms"),
    ("crosstalk", 0.0, 100.0, 0.0, ""),
];
const OSC_PARAMS: usize = 4 * PER_OSC.len();
const PARAMS: usize = OSC_PARAMS + GLOBALS.len();
/// The envelope's time knobs (their index in [`GLOBALS`]) and their tempo-sync fields
pub const SYNC_FIELDS: [(usize, &str); 3] = [(2, "envAttackSync"), (3, "envHoldSync"), (4, "envDecaySync")];
pub const GRAPH_FIELDS: [&str; 4] = ["a1Graph", "a2Graph", "b1Graph", "b2Graph"];

/// The key of parameter `i`
pub fn param_key(i: usize) -> &'static str {
    static KEYS: LazyLock<Vec<String>> =
        LazyLock::new(|| OSCILLATORS.iter().flat_map(|o| PER_OSC.iter().map(move |p| format!("{o}{}", p.0))).collect());
    if i < OSC_PARAMS { KEYS[i].as_str() } else { GLOBALS[i - OSC_PARAMS].0 }
}

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.watsyn.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: None,
    build,
    // (no tuning probe: four oscillators on drawn waves; the pitch is the legacy note's, tested against it)
    tests: TestHints { voices: VOICES, ..TestHints::new() },
};

/// The legacy graph's sine (`setWaveToSine`)
pub fn default_graph() -> [f32; GRAPH_LENGTH] {
    core::array::from_fn(|i| sin_sample(i as f32 / GRAPH_LENGTH as f32))
}

fn schemas() -> Vec<NodeSchema> {
    let sine = Value::list(default_graph().iter().map(|v| Value::real(f64::from(*v)).unwrap_or(Value::NULL)));
    let mut s = NodeSchema::new("plugin:watsyn", 1);
    for i in 0..OSC_PARAMS {
        let (_, min, max, def, stepped) = PER_OSC[i % PER_OSC.len()];
        let p = ParamSpec::new(param_key(i), min, max, def);
        let p = if i % PER_OSC.len() >= 3 { p.unit("ct") } else { p };
        s = s.param(if stepped { p.scale(Scale::Stepped) } else { p });
    }
    for (key, min, max, def, unit) in GLOBALS {
        let p = ParamSpec::new(key, min, max, def);
        s = s.param(if unit.is_empty() { p } else { p.unit(unit) });
    }
    for key in GRAPH_FIELDS {
        s = s.field(
            FieldSpec::new(key, FieldKind::List(Box::new(FieldKind::Real { min: -1.0, max: 1.0 })))
                .default(sine.clone()),
        );
    }
    s = s
        .field(FieldSpec::choice("aMod", &MODS, "mix"))
        .field(FieldSpec::choice("bMod", &MODS, "mix"))
        .field(FieldSpec::bool("antiAliasing", true));
    for (_, key) in SYNC_FIELDS {
        s = s.field(FieldSpec::choice(key, &SYNCS, "off"));
    }
    vec![add_instrument_functions(s)]
}

/// The synth's settings from the parameters' values, the modulations, the synced envelope times (beats, `None` free)
/// and the tempo (a synced time is the note length at `bpm`, at most 2000 ms)
pub fn settings(v: &[f32; PARAMS], mods: [WatsynMod; 2], sync: &[Option<f64>; 3], bpm: f64) -> WatsynSettings {
    let osc = |k: usize| -> [f32; 4] { core::array::from_fn(|o| v[o * PER_OSC.len() + k]) };
    let mut g: [f32; 6] = core::array::from_fn(|i| v[OSC_PARAMS + i]);
    for ((knob, _), beats) in SYNC_FIELDS.iter().zip(sync) {
        if let Some(beats) = beats
            && bpm > 0.0
        {
            let (_, min, max, ..) = GLOBALS[*knob];
            g[*knob] = (beats * 60_000.0 / bpm).clamp(min, max) as f32;
        }
    }
    WatsynSettings {
        volume: osc(0),
        pan: osc(1),
        multiplier: osc(2),
        left_tune: osc(3),
        right_tune: osc(4),
        ab_mix: g[0],
        env_amount: g[1],
        env_attack: g[2],
        env_hold: g[3],
        env_decay: g[4],
        crosstalk: g[5],
        a_mod: mods[0],
        b_mod: mods[1],
    }
}

/// The values of the parameters at their defaults
pub fn default_values() -> [f32; PARAMS] {
    core::array::from_fn(|i| {
        if i < OSC_PARAMS { PER_OSC[i % PER_OSC.len()].3 as f32 } else { GLOBALS[i - OSC_PARAMS].3 as f32 }
    })
}

/// The four waves from their graphs, oversampled as the legacy (libsamplerate's fastest sinc at a ratio of 32, the
/// points read round and round: `srccpy`). Off the audio thread
pub fn waves(graphs: &[[f32; GRAPH_LENGTH]; 4], anti_aliasing: bool) -> Result<WatsynWaves> {
    let mut out: [Vec<f32>; 4] = Default::default();
    for (wave, graph) in out.iter_mut().zip(graphs) {
        let mut resampler = Resampler::new(Quality::Fastest, 1, WAVE_RATIO as f64)?;
        let mut failed = None;
        *wave = oversample(graph, |input, output| match resampler.process(input, output) {
            Ok(done) => (done.input_frames, done.output_frames),
            Err(e) => {
                failed = Some(e);
                // (nothing more: the wave is refused below)
                (0, output.len())
            }
        });
        if let Some(e) = failed {
            return Err(e);
        }
    }
    Ok(WatsynWaves::new(out, anti_aliasing))
}

#[derive(Clone, Debug)]
pub struct Watsyn {
    rate: u32,
    waves: Arc<WatsynWaves>,
    values: [f32; PARAMS],
    mods: [WatsynMod; 2],
    sync: [Option<f64>; 3],
    bpm: f64,
    settings: WatsynSettings,
}

impl Watsyn {
    fn update(&mut self) {
        self.settings = settings(&self.values, self.mods, &self.sync, self.bpm);
    }
}

/// One note: the synth's voice and the frames it played
#[derive(Clone, Debug)]
pub struct Voice {
    sound: WatsynVoice,
    played: u64,
}

impl Synth for Watsyn {
    type Voice = Voice;
    const PARAMS: usize = PARAMS;

    fn voice(&self) -> Voice {
        Voice { sound: WatsynVoice::new(&self.settings, self.rate), played: 0 }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(v) = self.values.get_mut(index) {
            *v = value;
            self.update();
        }
    }

    fn set_tempo(&mut self, bpm: f64) {
        self.bpm = bpm;
        if self.sync.iter().any(Option::is_some) {
            self.update();
        }
    }

    fn start(&self, voice: &mut Voice, _: &NoteStart, _: &mut Rng) {
        // (no heap in a voice: a new one in its place allocates nothing; it keeps the modulations of now)
        *voice = Voice { sound: WatsynVoice::new(&self.settings, self.rate), played: 0 };
    }

    fn render(&self, voice: &mut Voice, freq: f32, left: &mut [f32], right: &mut [f32]) {
        let mut buf = [[0.0f32; 2]; GRID];
        let mut at = 0;
        while at < left.len() {
            let n = (left.len() - at).min(GRID);
            voice.sound.render(&mut buf[..n], &self.waves, &self.settings, freq, voice.played);
            voice.played += n as u64;
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

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let mut values = [0.0f32; PARAMS];
    for (i, v) in values.iter_mut().enumerate() {
        *v = s.param_f32(param_key(i))?;
    }
    let mut graphs = [default_graph(); 4];
    for (graph, key) in graphs.iter_mut().zip(GRAPH_FIELDS) {
        let points: Vec<f32> = state
            .get(key)
            .map(|l| l.items().filter_map(Value::as_real).map(|v| v as f32).collect())
            .unwrap_or_default();
        // A graph of another length: as many of its points as fit, the sine's after
        for (p, v) in graph.iter_mut().zip(points) {
            *p = v;
        }
    }
    let modulation = |k: &str| s.choice(k, |key| MOD_KEYS.iter().position(|m| *m == key).map(|i| WatsynMod::ALL[i]));
    let mods = [modulation("aMod")?, modulation("bMod")?];
    let mut sync = [None; 3];
    for (beats, (_, key)) in sync.iter_mut().zip(SYNC_FIELDS) {
        *beats = s.choice(key, |k| SYNC_KEYS.contains(&k).then(|| sync_beats(k)))?;
    }
    let anti_aliasing = s.boolean("antiAliasing")?;
    let waves = Arc::new(waves(&graphs, anti_aliasing)?);
    let mut synth =
        Watsyn { rate: cx.spec.sample_rate, waves, values, mods, sync, bpm: 0.0, settings: WatsynSettings::default() };
    synth.update();
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?))
}
