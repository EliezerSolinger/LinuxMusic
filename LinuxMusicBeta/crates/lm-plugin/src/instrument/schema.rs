//! The instrument functions' part of an instrument's schema: pitch and pitch bend range (F-IJN-002), mono, legato and
//! glide (F-IJN-014), and, for now, the sound shaping (the envelopes, LFOs and filter, in [`super::shaping`]: they move
//! out of the instruments in a next step, see that module).
//!
//! Chords and the arpeggio are **not** here any more: they are note effects (`lm-plugins-noteeffects`, a
//! `Kind::NoteEffect` plugin in the track's `noteEffects` list, before the instrument). Version 2 of every instrument
//! schema dropped their fields and parameters: [`drop_chords_and_arpeggio`] is the upgrade step, and the project's
//! opening moves what was set into note effects (`lm-doc`'s `Upgrader::track`).
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

use lm_base::Result;
use lm_state::{FieldSpec, NodeSchema, PARAMS_KEY, ParamSpec, Scale, Value};

use super::shaping;

/// The tempo-synced lengths of the LFO's period (and of a note effect's step), `off` free
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

/// Poly; mono (a new note starts the voice again, gliding from the last pitch); legato (a new note moves the
/// sounding voice to its key without starting again)
pub static VOICE_MODES: [&str; 3] = ["poly", "mono", "legato"];

/// The parameters, in index order (after the instrument's own): pitch, bend range, the shaping's block, glide
pub mod p {
    use super::shaping::s;

    pub const PITCH: usize = 0;
    pub const BEND_RANGE: usize = 1;
    /// Where the shaping's block starts
    pub const SHAPING: usize = 2;
    pub const GLIDE: usize = SHAPING + s::COUNT;
    /// The first parameter of target `t` (0 volume, 1 cutoff, 2 resonance); the offsets of `shaping::s` follow it
    pub const fn env(t: usize) -> usize {
        SHAPING + s::env(t)
    }
    pub const FILTER_ON: usize = SHAPING + s::FILTER_ON;
    pub const CUTOFF: usize = SHAPING + s::CUTOFF;
    pub const RESONANCE: usize = SHAPING + s::RESONANCE;
    /// How many there are
    pub const COUNT: usize = GLIDE + 1;
}

static PARAMS: LazyLock<Vec<ParamSpec>> = LazyLock::new(|| {
    let mut v = vec![
        ParamSpec::new("pitch", -100.0, 100.0, 0.0).unit("cents"),
        ParamSpec::new("bendRange", 1.0, 60.0, 2.0).scale(Scale::Stepped).unit("st").not_automatable(),
    ];
    v.extend(shaping::params());
    v.push(ParamSpec::new("glide", 0.0, 2000.0, 0.0).unit("ms"));
    debug_assert_eq!(v.len(), p::COUNT);
    v
});

/// The instrument functions' parameters, in index order
pub fn instrument_params() -> &'static [ParamSpec] {
    &PARAMS
}

/// The fields of the instrument functions
pub fn instrument_fields() -> Vec<FieldSpec> {
    let mut v = shaping::fields();
    v.push(FieldSpec::choice("voiceMode", &VOICE_MODES, "poly"));
    v
}

/// The fields and parameters the chords and the arpeggio had in version 1 of an instrument's schema
const REMOVED_FIELDS: [&str; 5] = ["chord", "arpChord", "arpDirection", "arpMode", "arpSync"];
const REMOVED_PARAMS: [&str; 10] = [
    "chordOn",
    "chordRange",
    "arpOn",
    "arpRange",
    "arpRepeats",
    "arpCycle",
    "arpSkip",
    "arpMiss",
    "arpTime",
    "arpGate",
];

/// The upgrade step that takes an instrument's state from the version with the chords and the arpeggio to the next:
/// they are note effects now, so the fields and parameters go. (What was switched on is moved into the track's
/// `noteEffects` before this runs, at the project's opening: a node's upgrade cannot see its track)
pub fn drop_chords_and_arpeggio(mut state: Value) -> Result<Value> {
    for k in REMOVED_FIELDS {
        state = state.without(k);
    }
    if let Some(params) = state.get(PARAMS_KEY).cloned() {
        let mut params = params;
        for k in REMOVED_PARAMS {
            params = params.without(k);
        }
        state = state.with(PARAMS_KEY, params);
    }
    Ok(state)
}

/// An instrument's schema with the instrument functions added after its own fields and parameters. It adds the
/// version step that dropped the chords and the arpeggio (version 1 → 2, the schema's version goes up by one): call it
/// **last**, after the schema's own `upgrade`s, and give the schema the version it had with them
pub fn add_instrument_functions(mut schema: NodeSchema) -> NodeSchema {
    for f in instrument_fields() {
        schema = schema.field(f);
    }
    for p in instrument_params() {
        schema = schema.param(p.clone());
    }
    let from = schema.version;
    schema.version = from + 1;
    schema.upgrade(from, drop_chords_and_arpeggio)
}
