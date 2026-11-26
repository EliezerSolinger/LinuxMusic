//! The track's `noteEffects` and the opening of a project saved when the instruments had chords and an arpeggio
//! (decisoes-autonomas.md, "O que deixa de seguir o legado", item 2): what was on becomes note effect nodes

mod common;

use std::sync::Arc;

use common::*;
use lm_doc::{Document, Port, Ports, TrackRef, register_project_schemas};
use lm_state::{FieldSpec, NodeSchema, ParamSpec, SchemaRegistry, Value, from_json, to_json};

fn drop_old(state: Value) -> lm_base::Result<Value> {
    let params = state.get("params").cloned().unwrap_or_default();
    let params = params.without("chordOn").without("chordRange").without("arpOn").without("arpTime");
    Ok(state.without("chord").without("arpChord").with("params", params))
}

/// A registry whose synth has the chord and the arpeggio (`old`) or has dropped them (version 2), with the two note
/// effects' schemas
fn registry_of(old: bool) -> Arc<SchemaRegistry> {
    let mut r = SchemaRegistry::new();
    register_project_schemas(&mut r).unwrap();
    let mut synth = NodeSchema::new("plugin:synth", 1).param(ParamSpec::new("vol1", 0.0, 1.0, 0.8));
    if old {
        synth = synth
            .field(FieldSpec::text("chord", "octave"))
            .field(FieldSpec::text("arpChord", "octave"))
            .param(ParamSpec::new("chordOn", 0.0, 1.0, 0.0))
            .param(ParamSpec::new("chordRange", 1.0, 9.0, 1.0))
            .param(ParamSpec::new("arpOn", 0.0, 1.0, 0.0))
            .param(ParamSpec::new("arpTime", 25.0, 2000.0, 200.0));
    } else {
        synth.version = 2;
        synth = synth.upgrade(1, drop_old);
    }
    r.add(synth).unwrap();
    r.add(
        NodeSchema::new("plugin:arpeggiator", 1)
            .field(FieldSpec::text("chord", "octave"))
            .field(FieldSpec::text("direction", "up"))
            .field(FieldSpec::text("mode", "free"))
            .field(FieldSpec::text("sync", "off"))
            .param(ParamSpec::new("range", 1.0, 9.0, 1.0))
            .param(ParamSpec::new("repeats", 1.0, 8.0, 1.0))
            .param(ParamSpec::new("cycle", 0.0, 6.0, 0.0))
            .param(ParamSpec::new("skip", 0.0, 100.0, 0.0))
            .param(ParamSpec::new("miss", 0.0, 100.0, 0.0))
            .param(ParamSpec::new("time", 25.0, 2000.0, 200.0))
            .param(ParamSpec::new("gate", 1.0, 200.0, 100.0)),
    )
    .unwrap();
    r.add(
        NodeSchema::new("plugin:chords", 1)
            .field(FieldSpec::text("chord", "major"))
            .param(ParamSpec::new("range", 1.0, 9.0, 1.0)),
    )
    .unwrap();
    r.check_complete().unwrap();
    Arc::new(r)
}

fn ports(plugin: &str, _: &Value) -> Option<Ports> {
    match plugin {
        "synth" => Some(Ports { outputs: vec![Port::new("main", 2)], sidechains: vec![] }),
        "arpeggiator" | "chords" => Some(Ports { outputs: vec![], sidechains: vec![] }),
        _ => None,
    }
}

fn opts() -> lm_doc::Options {
    lm_doc::Options { ports: Some(Arc::new(ports)), ..lm_doc::Options::default() }
}

fn old_project(chord_on: bool, arp_on: bool) -> String {
    let mut doc = Document::new_project(registry_of(true), opts()).unwrap();
    let state = Value::object([
        ("chord", text("minor")),
        ("arpChord", text("major")),
        (
            "params",
            Value::object([
                ("vol1", real(0.5)),
                ("chordOn", real(if chord_on { 1.0 } else { 0.0 })),
                ("chordRange", real(2.0)),
                ("arpOn", real(if arp_on { 1.0 } else { 0.0 })),
                ("arpTime", real(120.0)),
            ]),
        ),
    ]);
    add_track(&mut doc, "a", Some(plugin("synth", state)));
    to_json(&doc.saved()).unwrap()
}

#[test]
fn what_was_on_becomes_note_effects_and_the_instrument_loses_the_fields() {
    let opened = Document::open(registry_of(false), from_json(&old_project(true, true)).unwrap(), opts()).unwrap();
    assert!(opened.upgraded);
    let doc = &opened.document;
    let track = TrackRef(doc.tracks()[0].id());
    let effects = track.note_effects(doc);
    assert_eq!(effects.len(), 2, "the arpeggio, then the chord");
    let arp = effects[0].value(doc).unwrap();
    assert_eq!(arp.get("plugin"), Some(&text("arpeggiator")));
    let st = arp.get("state").unwrap();
    assert_eq!(st.get("chord"), Some(&text("major")));
    assert_eq!(st.get("params").and_then(|p| p.get("time")).and_then(Value::as_real), Some(120.0));
    let chords = effects[1].value(doc).unwrap();
    let st = chords.get("state").unwrap();
    assert_eq!(st.get("chord"), Some(&text("minor")));
    assert_eq!(st.get("params").and_then(|p| p.get("range")).and_then(Value::as_real), Some(2.0));
    let inst = track.instrument(doc).unwrap().state(doc).unwrap();
    assert!(inst.get("chord").is_none() && inst.get("params").and_then(|p| p.get("chordOn")).is_none());
    assert_eq!(inst.get("params").and_then(|p| p.get("vol1")).and_then(Value::as_real), Some(0.5));
    // The counter is above the ids made
    assert!(doc.issued() > effects[1].0.number);
    // Saved and opened again: unchanged, no second migration
    let again =
        Document::open(registry_of(false), from_json(&to_json(&doc.saved()).unwrap()).unwrap(), opts()).unwrap();
    assert!(!again.upgraded);
    assert_eq!(TrackRef(again.document.tracks()[0].id()).note_effects(&again.document).len(), 2);
}

#[test]
fn what_was_off_is_only_dropped() {
    let opened = Document::open(registry_of(false), from_json(&old_project(false, true)).unwrap(), opts()).unwrap();
    let track = TrackRef(opened.document.tracks()[0].id());
    let effects = track.note_effects(&opened.document);
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].value(&opened.document).unwrap().get("plugin"), Some(&text("arpeggiator")));
    let none = Document::open(registry_of(false), from_json(&old_project(false, false)).unwrap(), opts()).unwrap();
    assert!(TrackRef(none.document.tracks()[0].id()).note_effects(&none.document).is_empty());
}
