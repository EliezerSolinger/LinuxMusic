//! The instrument functions' part of an instrument's schema (F-IJN-002 pitch and bend range, F-IJN-010 envelopes and
//! LFOs, F-IJN-011 filter, F-IJN-013 chords, F-IJN-014 mono/legato/glide, F-IJN-015 arpeggio).
//!
//! The project's track has none of these (lm-doc: a track holds its instrument's node, and the instrument's state is
//! the plugin's tree), so they live in the instrument's own state, under the keys below, which every instrument that
//! takes them adds to its schema with [`add_instrument_functions`]. They come **after** the instrument's own
//! parameters: the index of the first of them is the instrument's parameter count.
//!
//! What may change while playing without allocating is a parameter (automatable; a switch is a stepped 0–1
//! parameter); every choice is a `Choice` field with permanent text keys (U30), and changing one rebuilds the
//! processor, which takes the sound over in `adopt` (U32).

use std::sync::LazyLock;

use lm_dsp::basic_filters::FilterType;
use lm_dsp::envelope::LfoShape;
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use super::chords::CHORD_KEYS;

/// The three targets of the envelopes and LFOs, as key prefixes
pub const TARGETS: [&str; 3] = ["vol", "cut", "res"];

/// The tempo-synced lengths of the LFO's period and the arpeggio's step (legacy `TempoSyncKnob`), `off` free
pub static SYNC_KEYS: [&str; 8] = ["off", "2/1", "1/1", "1/2", "1/4", "1/8", "1/16", "1/32"];

/// Beats (quarter notes) of a synced length; `None` for `off`
pub fn sync_beats(key: &str) -> Option<f64> {
    Some(match key {
        "2/1" => 8.0,
        "1/1" => 4.0,
        "1/2" => 2.0,
        "1/4" => 1.0,
        "1/8" => 0.5,
        "1/16" => 0.25,
        "1/32" => 0.125,
        _ => return None,
    })
}

pub static ARP_DIRECTIONS: [&str; 5] = ["up", "down", "up-down", "down-up", "random"];
pub static ARP_MODES: [&str; 3] = ["free", "sort", "sync"];
/// Poly; mono (a new note starts the voice again, gliding from the last pitch); legato (a new note moves the
/// sounding voice to its key without starting again)
pub static VOICE_MODES: [&str; 3] = ["poly", "mono", "legato"];

pub static FILTER_KEYS: LazyLock<Box<[&'static str]>> =
    LazyLock::new(|| FilterType::ALL.iter().map(|t| t.key()).collect());
pub static LFO_SHAPE_KEYS: LazyLock<Box<[&'static str]>> =
    LazyLock::new(|| LfoShape::ALL.iter().map(|s| s.key()).collect());

/// The parameters, in index order (after the instrument's own)
pub mod p {
    pub const PITCH: usize = 0;
    pub const BEND_RANGE: usize = 1;
    /// The first parameter of target `t` (0 volume, 1 cutoff, 2 resonance); the offsets below follow it
    pub const fn env(t: usize) -> usize {
        2 + t * ENV_PARAMS
    }
    pub const ENV_PARAMS: usize = 13;
    pub const PREDELAY: usize = 0;
    pub const ATTACK: usize = 1;
    pub const HOLD: usize = 2;
    pub const DECAY: usize = 3;
    pub const SUSTAIN: usize = 4;
    pub const RELEASE: usize = 5;
    pub const AMOUNT: usize = 6;
    pub const LFO_PREDELAY: usize = 7;
    pub const LFO_ATTACK: usize = 8;
    pub const LFO_SPEED: usize = 9;
    pub const LFO_AMOUNT: usize = 10;
    pub const LFO_X100: usize = 11;
    pub const LFO_ENV_AMOUNT: usize = 12;
    pub const FILTER_ON: usize = 2 + 3 * ENV_PARAMS;
    pub const CUTOFF: usize = FILTER_ON + 1;
    pub const RESONANCE: usize = FILTER_ON + 2;
    pub const CHORD_ON: usize = FILTER_ON + 3;
    pub const CHORD_RANGE: usize = FILTER_ON + 4;
    pub const ARP_ON: usize = FILTER_ON + 5;
    pub const ARP_RANGE: usize = FILTER_ON + 6;
    pub const ARP_REPEATS: usize = FILTER_ON + 7;
    pub const ARP_CYCLE: usize = FILTER_ON + 8;
    pub const ARP_SKIP: usize = FILTER_ON + 9;
    pub const ARP_MISS: usize = FILTER_ON + 10;
    pub const ARP_TIME: usize = FILTER_ON + 11;
    pub const ARP_GATE: usize = FILTER_ON + 12;
    pub const GLIDE: usize = FILTER_ON + 13;
    /// How many there are
    pub const COUNT: usize = GLIDE + 1;
}

fn switch(key: &'static str) -> ParamSpec {
    ParamSpec::new(key, 0.0, 1.0, 0.0).scale(Scale::Stepped)
}

fn env_params(t: &str) -> Vec<ParamSpec> {
    // The keys are permanent (C5); they are leaked once into 'static (39 short strings, for the life of the program)
    let key = |name: &str| -> &'static str { Box::leak(format!("{t}{name}").into_boxed_str()) };
    vec![
        ParamSpec::new(key("Predelay"), 0.0, 2.0, 0.0),
        ParamSpec::new(key("Attack"), 0.0, 2.0, 0.0),
        ParamSpec::new(key("Hold"), 0.0, 2.0, 0.5),
        ParamSpec::new(key("Decay"), 0.0, 2.0, 0.5),
        ParamSpec::new(key("Sustain"), 0.0, 1.0, 0.5),
        ParamSpec::new(key("Release"), 0.0, 2.0, 0.1),
        ParamSpec::new(key("Amount"), -1.0, 1.0, 0.0),
        ParamSpec::new(key("LfoPredelay"), 0.0, 1.0, 0.0),
        ParamSpec::new(key("LfoAttack"), 0.0, 1.0, 0.0),
        ParamSpec::new(key("LfoSpeed"), 0.001, 1.0, 0.1).scale(Scale::Log),
        ParamSpec::new(key("LfoAmount"), -1.0, 1.0, 0.0),
        switch(key("LfoX100")),
        switch(key("LfoEnvAmount")),
    ]
}

static PARAMS: LazyLock<Vec<ParamSpec>> = LazyLock::new(|| {
    let mut v = vec![
        ParamSpec::new("pitch", -100.0, 100.0, 0.0).unit("cents"),
        ParamSpec::new("bendRange", 1.0, 60.0, 2.0).scale(Scale::Stepped).unit("st").not_automatable(),
    ];
    for t in TARGETS {
        v.extend(env_params(t));
    }
    v.extend([
        switch("filterOn"),
        ParamSpec::new("cutoff", 1.0, 14000.0, 14000.0).scale(Scale::Log).unit("Hz"),
        ParamSpec::new("resonance", 0.01, 10.0, 0.5),
        switch("chordOn"),
        ParamSpec::new("chordRange", 1.0, 9.0, 1.0).scale(Scale::Stepped),
        switch("arpOn"),
        ParamSpec::new("arpRange", 1.0, 9.0, 1.0).scale(Scale::Stepped),
        ParamSpec::new("arpRepeats", 1.0, 8.0, 1.0).scale(Scale::Stepped),
        ParamSpec::new("arpCycle", 0.0, 6.0, 0.0).scale(Scale::Stepped),
        ParamSpec::new("arpSkip", 0.0, 100.0, 0.0).unit("%"),
        ParamSpec::new("arpMiss", 0.0, 100.0, 0.0).unit("%"),
        ParamSpec::new("arpTime", 25.0, 2000.0, 200.0).unit("ms"),
        ParamSpec::new("arpGate", 1.0, 200.0, 100.0).unit("%"),
        ParamSpec::new("glide", 0.0, 2000.0, 0.0).unit("ms"),
    ]);
    debug_assert_eq!(v.len(), p::COUNT);
    v
});

/// The instrument functions' parameters, in index order
pub fn instrument_params() -> &'static [ParamSpec] {
    &PARAMS
}

fn choice(key: &'static str, choices: &'static [&'static str], default: &str) -> FieldSpec {
    FieldSpec::choice(key, choices, default)
}

/// The fields of the instrument functions
pub fn instrument_fields() -> Vec<FieldSpec> {
    let mut v = Vec::new();
    for (t, shape, wave, sync) in [
        ("vol", "volLfoShape", "volLfoWave", "volLfoSync"),
        ("cut", "cutLfoShape", "cutLfoWave", "cutLfoSync"),
        ("res", "resLfoShape", "resLfoWave", "resLfoSync"),
    ] {
        let _ = t;
        v.push(choice(shape, &LFO_SHAPE_KEYS, "sine"));
        v.push(FieldSpec::new(wave, FieldKind::Resource("audio")).nullable().default(Value::NULL));
        v.push(choice(sync, &SYNC_KEYS, "off"));
    }
    v.push(choice("filterType", &FILTER_KEYS, "low-pass"));
    v.push(choice("chord", &CHORD_KEYS, "octave"));
    v.push(choice("arpChord", &CHORD_KEYS, "octave"));
    v.push(choice("arpDirection", &ARP_DIRECTIONS, "up"));
    v.push(choice("arpMode", &ARP_MODES, "free"));
    v.push(choice("arpSync", &SYNC_KEYS, "off"));
    v.push(choice("voiceMode", &VOICE_MODES, "poly"));
    v
}

/// The key of the LFO user wave field of target `t`
pub const LFO_WAVE_FIELDS: [&str; 3] = ["volLfoWave", "cutLfoWave", "resLfoWave"];
pub const LFO_SHAPE_FIELDS: [&str; 3] = ["volLfoShape", "cutLfoShape", "resLfoShape"];
pub const LFO_SYNC_FIELDS: [&str; 3] = ["volLfoSync", "cutLfoSync", "resLfoSync"];

/// An instrument's schema with the instrument functions added after its own fields and parameters
pub fn add_instrument_functions(mut schema: NodeSchema) -> NodeSchema {
    for f in instrument_fields() {
        schema = schema.field(f);
    }
    for p in instrument_params() {
        schema = schema.param(p.clone());
    }
    schema
}
