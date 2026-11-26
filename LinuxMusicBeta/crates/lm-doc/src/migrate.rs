//! Project-level migrations that a node's own upgrade cannot do because they need its neighbours.
//!
//! **Chords and arpeggio into note effects (26/11).** Version 2 of every instrument schema dropped the chord and the
//! arpeggio (`lm_plugin::instrument::drop_chords_and_arpeggio`): they are note effects now, plugin nodes in the
//! track's `noteEffects`. A node's upgrade cannot add a node to its track, so the opening does it first, on the track,
//! while the instrument's state is still the old one: [`note_effects_from_instrument`] reads `chordOn`, `arpOn` and
//! their settings and, for each that was on, makes an `arpeggiator` and a `chords` node (the arpeggio first: the legacy
//! chord stacked every step's note). What was off is dropped with the fields. When the project's registry has no such
//! plugin (a registry of a test), the settings are dropped too.

use lm_base::Result;
use lm_state::{NodeId, PARAMS_KEY, SchemaRegistry, Value};

use crate::schemas::{KIND_PLUGIN, plugin_type};

/// The highest number of any id in a tree
pub(crate) fn max_id(v: &Value) -> u64 {
    let mut m = 0;
    for (k, x) in v.entries() {
        if let Some(t) = x.as_text()
            && (k == "id" || k == "parent" || k == "to" || k == "node")
            && let Ok(id) = NodeId::parse(t)
        {
            m = m.max(id.number);
        }
        m = m.max(max_id(x));
    }
    for x in v.items() {
        m = m.max(max_id(x));
    }
    m
}

fn real(v: f64) -> Value {
    Value::real(v).unwrap_or(Value::NULL)
}

/// The arpeggio's and the chord's settings of an old instrument state, to `(field or parameter key in the effect,
/// key in the old state)`
const ARP_FIELDS: [(&str, &str, &str); 4] = [
    ("chord", "arpChord", "octave"),
    ("direction", "arpDirection", "up"),
    ("mode", "arpMode", "free"),
    ("sync", "arpSync", "off"),
];
const ARP_PARAMS: [(&str, &str, f64); 7] = [
    ("range", "arpRange", 1.0),
    ("repeats", "arpRepeats", 1.0),
    ("cycle", "arpCycle", 0.0),
    ("skip", "arpSkip", 0.0),
    ("miss", "arpMiss", 0.0),
    ("time", "arpTime", 200.0),
    ("gate", "arpGate", 100.0),
];

/// A track whose instrument had the chord or the arpeggio on gets them as note effects (`next_id` is the next id
/// number to issue). Tracks with `noteEffects` already, without an instrument or with both off are returned as they
/// are
pub(crate) fn note_effects_from_instrument(
    registry: &SchemaRegistry,
    track: Value,
    next_id: &mut u64,
) -> Result<Value> {
    if track.get("noteEffects").is_some() {
        return Ok(track);
    }
    let Some(state) = track.get("instrument").filter(|i| !i.is_null()).and_then(|i| i.get("state")).cloned() else {
        return Ok(track);
    };
    let params = state.get(PARAMS_KEY);
    let param = |k: &str, d: f64| params.and_then(|p| p.get(k)).and_then(Value::as_real).unwrap_or(d);
    let text = |k: &str, d: &str| state.get(k).and_then(Value::as_text).unwrap_or(d).to_owned();
    let mut list: Vec<Value> = Vec::new();
    let mut node = |plugin: &str, fields: Vec<(&str, Value)>, params: Vec<(&str, Value)>| -> Result<Option<Value>> {
        let Some(schema) = registry.find(&plugin_type(plugin)) else { return Ok(None) };
        let id = NodeId::new(KIND_PLUGIN, *next_id)?;
        *next_id += 1;
        let mut st = Value::empty_object();
        for (k, v) in fields {
            st = st.with(k, v);
        }
        st = st.with(PARAMS_KEY, Value::object(params));
        Ok(Some(Value::object([
            ("id", Value::text(id.to_string())),
            ("plugin", Value::text(plugin)),
            ("version", Value::int(i64::from(schema.version))),
            ("state", st),
            ("bypass", Value::bool(false)),
        ])))
    };
    if param("arpOn", 0.0) >= 0.5 {
        let fields = ARP_FIELDS.iter().map(|(k, old, d)| (*k, Value::text(text(old, d)))).collect();
        let params = ARP_PARAMS.iter().map(|(k, old, d)| (*k, real(param(old, *d)))).collect();
        list.extend(node("arpeggiator", fields, params)?);
    }
    if param("chordOn", 0.0) >= 0.5 {
        let fields = vec![("chord", Value::text(text("chord", "octave")))];
        let params = vec![("range", real(param("chordRange", 1.0)))];
        list.extend(node("chords", fields, params)?);
    }
    if list.is_empty() {
        return Ok(track);
    }
    Ok(track.with("noteEffects", Value::list(list)))
}
