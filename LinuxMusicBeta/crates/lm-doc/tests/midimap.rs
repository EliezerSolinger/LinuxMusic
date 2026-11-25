//! The MIDI map in the document (interfaces.md, I-21.2): `project.midiMap` with targets that go with their node
//! (OnRemove::RemoveOwner) and come back with undo; the automation and recording fields of I-21.4 and I-21.5

mod common;

use common::*;
use lm_state::{NodeId, ROOT, Value};

fn mapping(target: Value) -> Value {
    Value::object([
        ("source", Value::object([("message", text("cc")), ("number", Value::int(7)), ("channel", Value::int(0))])),
        ("target", target),
    ])
}

fn param_target(node: NodeId, key: &str) -> Value {
    Value::object([("kind", text("param")), ("node", id_text(node)), ("param", text(key))])
}

fn map_len(doc: &lm_doc::Document) -> usize {
    doc.root().get("midiMap").map_or(0, Value::len)
}

#[test]
fn a_mapping_goes_with_its_target_and_comes_back_with_undo() {
    let mut doc = new_doc();
    let t = add_track(&mut doc, "a", Some(synth()));
    let m = add_channel(&mut doc, "c");
    let mut id = None;
    doc.transact(label("x"), |e| {
        id = Some(e.insert(ROOT, "midiMap", 0, mapping(param_target(t, "volume")))?);
        e.insert(ROOT, "midiMap", 1, mapping(param_target(m, "pan")))?;
        // The transport is on the root: a field target may point at it
        e.insert(
            ROOT,
            "midiMap",
            2,
            mapping(Value::object([
                ("kind", text("field")),
                ("node", text("r:0")),
                ("path", Value::list([text("transport"), text("loop"), text("on")])),
            ])),
        )?;
        // An action, and a selection-following target, have no node
        e.insert(
            ROOT,
            "midiMap",
            3,
            mapping(Value::object([("kind", text("action")), ("action", text("transport.play"))])),
        )?;
        e.insert(
            ROOT,
            "midiMap",
            4,
            mapping(Value::object([
                ("kind", text("param")),
                ("follow", text("track")),
                ("offset", Value::int(3)),
                ("param", text("volume")),
            ])),
        )?;
        Ok(())
    })
    .unwrap();
    let id = id.unwrap();
    assert_eq!(id.kind, b'k');
    assert_eq!(map_len(&doc), 5);
    let before = content(&doc);
    // Removing the track removes its mapping, and only it
    doc.transact(label("x"), |e| e.remove(t)).unwrap();
    assert_eq!(map_len(&doc), 4);
    assert!(doc.find(id).is_none());
    doc.undo().unwrap();
    assert_eq!(content(&doc), before);
    assert!(doc.find(id).is_some());
    // The defaults of a mapping are complete
    let node = doc.find(id).unwrap();
    assert_eq!(node.get("feedback").and_then(Value::as_bool), Some(true));
    assert_eq!(node.get("transform").and_then(|t| t.get("takeover")).and_then(Value::as_text), Some("pickup"));
    // A bad message kind is refused
    let err = doc
        .transact(label("x"), |e| {
            let bad = mapping(param_target(m, "pan"))
                .with("source", Value::object([("message", text("sysex")), ("number", Value::int(0))]));
            e.insert(ROOT, "midiMap", 0, bad).map(|_| ())
        })
        .unwrap_err();
    assert!(err.key().starts_with("state.schema") || err.key().starts_with("doc.value"), "{err}");
}

#[test]
fn automation_modes_and_recording_fields() {
    let mut doc = new_doc();
    let t = add_track(&mut doc, "a", Some(synth()));
    doc.transact(label("x"), |e| {
        e.set(ROOT, &[k("transport"), k("automationMode")], text("latch"))?;
        e.set(ROOT, &[k("transport"), k("recordMode")], text("layers"))?;
        e.set(
            ROOT,
            &[k("transport"), k("recordQuantize")],
            Value::object([("on", Value::bool(true)), ("grid", Value::int(120))]),
        )?;
        e.set(t, &[k("midiInput")], Value::object([("ports", Value::list([text("Teclado")]))]))?;
        let lane = Value::object([
            ("target", Value::object([("node", id_text(t)), ("param", text("volume"))])),
            ("mode", text("touch")),
            (
                "points",
                Value::list([
                    Value::object([("tick", Value::int(0)), ("value", real(0.5)), ("shape", text("step"))]),
                    Value::object([
                        ("tick", Value::int(960)),
                        ("value", real(0.7)),
                        ("shape", text("smooth")),
                        ("tension", real(0.3)),
                        ("out", real(0.2)),
                    ]),
                ]),
            ),
        ]);
        e.insert(t, "automation", 0, lane).map(|_| ())
    })
    .unwrap();
    let err = doc.transact(label("x"), |e| e.set(ROOT, &[k("transport"), k("automationMode")], text("x"))).unwrap_err();
    assert_eq!(err.key(), "doc.value.not_a_choice", "{err}");
}

/// F-IJN-025: the CC rack's values are parameters of the track ("cc.7" at `cc.c7`), automatable before they are
/// first set; P-38: a v1 MIDI output becomes v2 (`port` → `ports`, on)
#[test]
fn the_cc_rack_and_the_midi_output_upgrade() {
    let mut doc = new_doc();
    let t = add_track(&mut doc, "a", Some(synth()));
    let d = doc.transact(label("x"), |e| e.set(t, &[k("cc"), k("c7")], real(64.0))).expect("a CC value");
    assert!(d.params.iter().any(|(n, key, v)| *n == t && &**key == "cc.7" && *v == 64.0), "{:?}", d.params);
    assert_eq!(lm_doc::param_path("track", "cc.74"), vec![k("cc"), k("c74")]);
    // A lane on a CC not set yet
    let lane = Value::object([
        ("target", Value::object([("node", id_text(t)), ("param", text("cc.74"))])),
        ("points", Value::list([Value::object([("tick", Value::int(0)), ("value", real(10.0))])])),
    ]);
    doc.transact(label("x"), |e| e.insert(t, "automation", 0, lane).map(|_| ())).expect("the lane");
    let bad = Value::object([("target", Value::object([("node", id_text(t)), ("param", text("cc.200"))]))]);
    let err = doc.transact(label("x"), |e| e.insert(t, "automation", 0, bad).map(|_| ())).unwrap_err();
    assert_eq!(err.key(), "doc.automation.no_such_param");
    // The output's upgrade
    let registry = registry();
    let v1 = Value::object([("port", text("Synth")), ("channel", Value::int(2))]);
    let v2 = registry.upgrade("track.midi_output", 1, v1).expect("upgrade");
    assert_eq!(v2.get("ports"), Some(&Value::list([text("Synth")])));
    assert_eq!(v2.get("on").and_then(Value::as_bool), Some(true));
    assert!(v2.get("port").is_none());
    let none = registry.upgrade("track.midi_output", 1, Value::object([("port", text(""))])).expect("upgrade");
    assert_eq!(none.get("ports"), Some(&Value::empty_list()));
}
