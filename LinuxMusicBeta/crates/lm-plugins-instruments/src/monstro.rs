//! Monstro (F-INS-009: legacy `plugins/Monstro`): three oscillators (a band-limited pulse with sync out, a multi-wave
//! one, and one crossfading two waves with a sub mix), two envelopes and two LFOs, and a modulation matrix of volume,
//! phase, pitch, pulse width and sub mix; the second oscillator mixed with, or modulating, the third.
//!
//! State, version 1: parameters, the legacy knobs in camelCase:
//! - oscillator 1: `osc1Volume` (0–200, 10), `osc1Panning` (−100–100, 0), `osc1Coarse` (−24–24 semitones, 0),
//!   `osc1FineLeft` and `osc1FineRight` (−100–100 cents, 0), `osc1StereoPhase` (−180–180°, 0), `osc1PulseWidth`
//!   (0.25–99.75 %, 50), `osc1SyncRise` and `osc1SyncFall` (switches, off);
//! - oscillator 2: `osc2Volume`, `osc2Panning`, `osc2Coarse`, `osc2FineLeft`, `osc2FineRight`, `osc2StereoPhase`,
//!   `osc2SyncHard`, `osc2SyncReverse`;
//! - oscillator 3: `osc3Volume`, `osc3Panning`, `osc3Coarse`, `osc3StereoPhase`, `osc3Sub` (−100–100, 0),
//!   `osc3SyncHard`, `osc3SyncReverse`;
//! - LFOs `n` = 1, 2: `lfo<n>Attack` (0–2000 ms, 0), `lfo<n>Rate` (0.1–10000 ms, 1), `lfo<n>Phase` (−180–180°, 0);
//! - envelopes `n` = 1, 2: `env<n>PreDelay` and `env<n>Attack` (0–2000 ms, 0), `env<n>Hold`, `env<n>Decay` and
//!   `env<n>Release` (0–4000 ms, 0), `env<n>Sustain` (0–1, 1), `env<n>Slope` (−1–1, 0);
//! - the matrix, −1 to 1, 0: `<target><source>` for the targets `osc1Vol`, `osc2Vol`, `osc3Vol`, `osc1Phs`,
//!   `osc2Phs`, `osc3Phs`, `osc1Pit`, `osc2Pit`, `osc3Pit`, `osc1Pw`, `osc3Sub` and the sources `Env1`, `Env2`,
//!   `Lfo1`, `Lfo2` (`osc1VolEnv1`, …);
//!
//! fields `osc2Wave`, `osc3Wave1`, `osc3Wave2` ([`OSC_WAVE_KEYS`], `sine`), `lfo1Wave`, `lfo2Wave`
//! ([`LFO_WAVE_KEYS`], `sine`), `o23Mod` (`mix`, `am`, `fm`, `pm`; `mix`), and for each time knob (the LFOs' attack
//! and rate, the envelopes' five times) a tempo sync `<knob>Sync` (`off`, `2/1` … `1/32`; `off`): synced, the knob is
//! the note length at the song's tempo, at most its maximum (legacy `TempoSyncKnob`); and the instrument functions.
//!
//! As the legacy, the knobs act on the sounding notes, the voice releases itself by its envelopes (the instrument only
//! keeps it for the longer release, at least 1.5 ms), and the host fades the note in.

use std::sync::LazyLock;

use lm_base::Result;
use lm_dsp::monstro::{
    MAX_PERIOD, Mod, MonstroEnvelope, MonstroLfo, MonstroSettings, MonstroVoice, O23Mod, PW_MAX, PW_MIN,
};
use lm_plugin::instrument::{GRID, Instrument, NoteStart, SYNC_KEYS, Synth, add_instrument_functions, sync_beats};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::stereo_out;

pub const ID: &str = "monstro";
pub const VOICES: usize = 32;

/// The waves of oscillators 2 and 3, in the legacy menu's order
pub const OSC_WAVE_KEYS: [&str; 15] = [
    "sine",
    "triangle",
    "saw",
    "ramp",
    "square",
    "moogSaw",
    "softSquare",
    "absSine",
    "exponential",
    "noise",
    "digitalTriangle",
    "digitalSaw",
    "digitalRamp",
    "digitalSquare",
    "digitalMoogSaw",
];
/// The waves of the LFOs, in the legacy menu's order
pub const LFO_WAVE_KEYS: [&str; 11] = [
    "sine",
    "triangle",
    "saw",
    "ramp",
    "square",
    "moogSaw",
    "softSquare",
    "absSine",
    "exponential",
    "random",
    "randomSmooth",
];
pub const O23_KEYS: [&str; 4] = ["mix", "am", "fm", "pm"];
static OSC_WAVES: LazyLock<Box<[&'static str]>> = LazyLock::new(|| OSC_WAVE_KEYS.into());
static LFO_WAVES: LazyLock<Box<[&'static str]>> = LazyLock::new(|| LFO_WAVE_KEYS.into());
static O23S: LazyLock<Box<[&'static str]>> = LazyLock::new(|| O23_KEYS.into());
static SYNCS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| SYNC_KEYS.into());

/// A parameter: key, minimum, maximum, default, unit, stepped
type Param = (&'static str, f64, f64, f64, &'static str, bool);

const fn knob(key: &'static str, min: f64, max: f64, def: f64, unit: &'static str) -> Param {
    (key, min, max, def, unit, false)
}

const fn switch(key: &'static str) -> Param {
    (key, 0.0, 1.0, 0.0, "", true)
}

/// The knobs before the matrix, in this order (the matrix follows: [`MATRIX_TARGETS`] × [`MATRIX_SOURCES`])
pub const KNOBS: [Param; 44] = [
    knob("osc1Volume", 0.0, 200.0, 10.0, ""),
    knob("osc1Panning", -100.0, 100.0, 0.0, ""),
    ("osc1Coarse", -24.0, 24.0, 0.0, "st", true),
    knob("osc1FineLeft", -100.0, 100.0, 0.0, "ct"),
    knob("osc1FineRight", -100.0, 100.0, 0.0, "ct"),
    knob("osc1StereoPhase", -180.0, 180.0, 0.0, "°"),
    knob("osc1PulseWidth", PW_MIN as f64, PW_MAX as f64, 50.0, "%"),
    switch("osc1SyncRise"),
    switch("osc1SyncFall"),
    knob("osc2Volume", 0.0, 200.0, 10.0, ""),
    knob("osc2Panning", -100.0, 100.0, 0.0, ""),
    ("osc2Coarse", -24.0, 24.0, 0.0, "st", true),
    knob("osc2FineLeft", -100.0, 100.0, 0.0, "ct"),
    knob("osc2FineRight", -100.0, 100.0, 0.0, "ct"),
    knob("osc2StereoPhase", -180.0, 180.0, 0.0, "°"),
    switch("osc2SyncHard"),
    switch("osc2SyncReverse"),
    knob("osc3Volume", 0.0, 200.0, 10.0, ""),
    knob("osc3Panning", -100.0, 100.0, 0.0, ""),
    ("osc3Coarse", -24.0, 24.0, 0.0, "st", true),
    knob("osc3StereoPhase", -180.0, 180.0, 0.0, "°"),
    knob("osc3Sub", -100.0, 100.0, 0.0, ""),
    switch("osc3SyncHard"),
    switch("osc3SyncReverse"),
    knob("lfo1Attack", 0.0, 2000.0, 0.0, "ms"),
    knob("lfo1Rate", 0.1, 10000.0, 1.0, "ms"),
    knob("lfo1Phase", -180.0, 180.0, 0.0, "°"),
    knob("lfo2Attack", 0.0, 2000.0, 0.0, "ms"),
    knob("lfo2Rate", 0.1, 10000.0, 1.0, "ms"),
    knob("lfo2Phase", -180.0, 180.0, 0.0, "°"),
    knob("env1PreDelay", 0.0, 2000.0, 0.0, "ms"),
    knob("env1Attack", 0.0, 2000.0, 0.0, "ms"),
    knob("env1Hold", 0.0, 4000.0, 0.0, "ms"),
    knob("env1Decay", 0.0, 4000.0, 0.0, "ms"),
    knob("env1Sustain", 0.0, 1.0, 1.0, ""),
    knob("env1Release", 0.0, 4000.0, 0.0, "ms"),
    knob("env1Slope", -1.0, 1.0, 0.0, ""),
    knob("env2PreDelay", 0.0, 2000.0, 0.0, "ms"),
    knob("env2Attack", 0.0, 2000.0, 0.0, "ms"),
    knob("env2Hold", 0.0, 4000.0, 0.0, "ms"),
    knob("env2Decay", 0.0, 4000.0, 0.0, "ms"),
    knob("env2Sustain", 0.0, 1.0, 1.0, ""),
    knob("env2Release", 0.0, 4000.0, 0.0, "ms"),
    knob("env2Slope", -1.0, 1.0, 0.0, ""),
];
const KNOB_COUNT: usize = KNOBS.len();
pub const MATRIX_TARGETS: [&str; 11] = [
    "osc1Vol", "osc2Vol", "osc3Vol", "osc1Phs", "osc2Phs", "osc3Phs", "osc1Pit", "osc2Pit", "osc3Pit", "osc1Pw",
    "osc3Sub",
];
pub const MATRIX_SOURCES: [&str; 4] = ["Env1", "Env2", "Lfo1", "Lfo2"];
const PARAMS: usize = KNOB_COUNT + MATRIX_TARGETS.len() * MATRIX_SOURCES.len();

/// The time knobs that follow the tempo when synced: their index in [`KNOBS`]
pub const SYNCED: [usize; 14] = [24, 25, 27, 28, 30, 31, 32, 33, 35, 37, 38, 39, 40, 42];

fn matrix_key(i: usize) -> &'static str {
    static KEYS: LazyLock<Vec<String>> = LazyLock::new(|| {
        MATRIX_TARGETS.iter().flat_map(|t| MATRIX_SOURCES.iter().map(move |s| format!("{t}{s}"))).collect()
    });
    KEYS[i].as_str()
}

fn sync_key(i: usize) -> &'static str {
    static KEYS: LazyLock<Vec<String>> =
        LazyLock::new(|| SYNCED.iter().map(|k| format!("{}Sync", KNOBS[*k].0)).collect());
    KEYS[i].as_str()
}

/// The key of parameter `i` (the knobs, then the matrix)
pub fn param_key(i: usize) -> &'static str {
    if i < KNOB_COUNT { KNOBS[i].0 } else { matrix_key(i - KNOB_COUNT) }
}

/// The tempo-sync field of knob `knob` of [`KNOBS`], if it has one
pub fn sync_field(knob: usize) -> Option<&'static str> {
    SYNCED.iter().position(|k| *k == knob).map(sync_key)
}

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.monstro.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: None,
    build,
    // (no tuning probe: the default patch is three oscillators a hair apart in level and phase, and its pitch is the
    // legacy note's, tested against it)
    tests: TestHints { voices: VOICES, ..TestHints::new() },
};

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:monstro", 1);
    for (key, min, max, def, unit, stepped) in &KNOBS {
        let mut p = ParamSpec::new(key, *min, *max, *def);
        if !unit.is_empty() {
            p = p.unit(unit);
        }
        s = s.param(if *stepped { p.scale(Scale::Stepped) } else { p });
    }
    for i in 0..MATRIX_TARGETS.len() * MATRIX_SOURCES.len() {
        s = s.param(ParamSpec::new(matrix_key(i), -1.0, 1.0, 0.0));
    }
    s = s
        .field(FieldSpec::choice("osc2Wave", &OSC_WAVES, "sine"))
        .field(FieldSpec::choice("osc3Wave1", &OSC_WAVES, "sine"))
        .field(FieldSpec::choice("osc3Wave2", &OSC_WAVES, "sine"))
        .field(FieldSpec::choice("lfo1Wave", &LFO_WAVES, "sine"))
        .field(FieldSpec::choice("lfo2Wave", &LFO_WAVES, "sine"))
        .field(FieldSpec::choice("o23Mod", &O23S, "mix"));
    for i in 0..SYNCED.len() {
        s = s.field(FieldSpec::choice(sync_key(i), &SYNCS, "off"));
    }
    vec![add_instrument_functions(s)]
}

/// The menus and modes of a patch (its fields)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Menus {
    pub osc2_wave: usize,
    pub osc3_waves: [usize; 2],
    pub lfo_waves: [usize; 2],
    pub o23: O23Mod,
    /// Beats of each synced knob of [`SYNCED`], `None` free
    pub sync: [Option<f64>; 14],
}

impl Default for Menus {
    fn default() -> Self {
        Self { osc2_wave: 0, osc3_waves: [0; 2], lfo_waves: [0; 2], o23: O23Mod::Mix, sync: [None; 14] }
    }
}

/// The synth's settings from the parameters' values (`values`, in parameter order), the menus and the tempo: a synced
/// knob is its note length at `bpm`, at most its maximum (legacy `TempoSyncKnobModel::calculateTempoSyncTime`)
pub fn settings(values: &[f32; PARAMS], menus: &Menus, bpm: f64) -> MonstroSettings {
    let mut v = *values;
    for (i, knob) in SYNCED.iter().enumerate() {
        if let Some(beats) = menus.sync[i]
            && bpm > 0.0
        {
            let (_, min, max, ..) = KNOBS[*knob];
            v[*knob] = (beats * 60_000.0 / bpm).clamp(min, max) as f32;
        }
    }
    let on = |i: usize| v[i] >= 0.5;
    let m = |t: usize| -> Mod { core::array::from_fn(|s| v[KNOB_COUNT + t * 4 + s]) };
    let lfo = |at: usize, wave: usize| MonstroLfo { wave, attack: v[at], rate: v[at + 1], phase: v[at + 2] };
    let env = |at: usize| MonstroEnvelope {
        pre: v[at],
        attack: v[at + 1],
        hold: v[at + 2],
        decay: v[at + 3],
        sustain: v[at + 4],
        release: v[at + 5],
        slope: v[at + 6],
    };
    MonstroSettings {
        volume: [v[0], v[9], v[17]],
        pan: [v[1], v[10], v[18]],
        coarse: [v[2], v[11], v[19]],
        fine: [[v[3], v[4]], [v[12], v[13]]],
        stereo_phase: [v[5], v[14], v[20]],
        pulse_width: v[6],
        sync_on_rise: on(7),
        sync_on_fall: on(8),
        osc2_wave: menus.osc2_wave,
        sync_hard: [on(15), on(22)],
        sync_reverse: [on(16), on(23)],
        osc3_waves: menus.osc3_waves,
        osc3_sub: v[21],
        lfos: [lfo(24, menus.lfo_waves[0]), lfo(27, menus.lfo_waves[1])],
        envelopes: [env(30), env(37)],
        o23: menus.o23,
        vol_mod: [m(0), m(1), m(2)],
        phase_mod: [m(3), m(4), m(5)],
        pitch_mod: [m(6), m(7), m(8)],
        pw_mod: m(9),
        sub_mod: m(10),
    }
}

/// The values of the parameters at their defaults
pub fn default_values() -> [f32; PARAMS] {
    core::array::from_fn(|i| if i < KNOB_COUNT { KNOBS[i].3 as f32 } else { 0.0 })
}

#[derive(Clone, Debug)]
pub struct Monstro {
    rate: u32,
    values: [f32; PARAMS],
    menus: Menus,
    bpm: f64,
    settings: MonstroSettings,
}

impl Monstro {
    fn update(&mut self) {
        self.settings = settings(&self.values, &self.menus, self.bpm);
    }
}

/// One note: the synth's voice, the frames it played, and whether it was let go. Copied into its own buffers (the
/// crossfade copies voices on the audio thread)
#[derive(Debug)]
pub struct Voice {
    sound: MonstroVoice,
    played: i64,
    released: bool,
}

impl Clone for Voice {
    fn clone(&self) -> Self {
        Self { sound: self.sound.clone(), played: self.played, released: self.released }
    }

    fn clone_from(&mut self, source: &Self) {
        self.sound.clone_from(&source.sound);
        self.played = source.played;
        self.released = source.released;
    }
}

impl Synth for Monstro {
    type Voice = Voice;
    const PARAMS: usize = PARAMS;
    const RELEASE_FADE: bool = false;

    fn voice(&self) -> Voice {
        Voice { sound: MonstroVoice::new(&self.settings, self.rate, 0), played: 0, released: false }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(v) = self.values.get_mut(index) {
            *v = value;
            self.update();
        }
    }

    fn set_tempo(&mut self, bpm: f64) {
        self.bpm = bpm;
        if self.menus.sync.iter().any(Option::is_some) {
            self.update();
        }
    }

    fn start(&self, voice: &mut Voice, _: &NoteStart, rng: &mut Rng) {
        voice.sound.restart(&self.settings, rng.next_u64());
        voice.played = 0;
        voice.released = false;
    }

    fn render(&self, voice: &mut Voice, freq: f32, left: &mut [f32], right: &mut [f32]) {
        const _: () = assert!(GRID <= MAX_PERIOD);
        let mut buf = [[0.0f32; 2]; GRID];
        let mut at = 0;
        while at < left.len() {
            let n = (left.len() - at).min(GRID);
            voice.sound.set(&self.settings);
            voice.sound.render(&mut buf[..n], freq, voice.played, voice.released.then_some(0));
            voice.played += n as i64;
            for (i, f) in buf[..n].iter().enumerate() {
                left[at + i] = f[0];
                right[at + i] = f[1];
            }
            at += n;
        }
    }

    fn release(&self, voice: &mut Voice) {
        voice.released = true;
    }

    fn release_ms(&self) -> f32 {
        self.settings.envelopes[0].release.max(self.settings.envelopes[1].release).max(1.5)
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let mut values = [0.0f32; PARAMS];
    for (i, v) in values.iter_mut().enumerate() {
        *v = s.param_f32(param_key(i))?;
    }
    let pick = |keys: &'static [&'static str], k: &str| s.choice(k, |key| keys.iter().position(|w| *w == key));
    let mut menus = Menus {
        osc2_wave: pick(&OSC_WAVE_KEYS, "osc2Wave")?,
        osc3_waves: [pick(&OSC_WAVE_KEYS, "osc3Wave1")?, pick(&OSC_WAVE_KEYS, "osc3Wave2")?],
        lfo_waves: [pick(&LFO_WAVE_KEYS, "lfo1Wave")?, pick(&LFO_WAVE_KEYS, "lfo2Wave")?],
        o23: O23Mod::ALL[pick(&O23_KEYS, "o23Mod")?],
        sync: [None; 14],
    };
    for (i, sync) in menus.sync.iter_mut().enumerate() {
        *sync = s.choice(sync_key(i), |k| SYNC_KEYS.contains(&k).then(|| sync_beats(k)))?;
    }
    let mut synth =
        Monstro { rate: cx.spec.sample_rate, values, menus, bpm: 0.0, settings: MonstroSettings::default() };
    synth.update();
    let params = &DESCRIPTOR.schema().params;
    Ok(Box::new(Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?))
}
