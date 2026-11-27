//! Markers and regions of the ruler (F-REG-011, F-REG-012): the project's version 2 adds them as optional lists, a
//! version-1 project opens unchanged (upgraded), one marker per tick, a region at least a sixteenth of a bar

mod common;

use common::*;
use lm_doc::Document;
use lm_state::{ROOT, Value};

fn marker(name: &str, tick: i64) -> Value {
    Value::object([("name", text(name)), ("tick", Value::int(tick))])
}

fn region(name: &str, from: i64, to: i64) -> Value {
    Value::object([("name", text(name)), ("from", Value::int(from)), ("to", Value::int(to))])
}

#[test]
fn markers_and_regions_are_edited_and_undone() {
    let mut doc = new_doc();
    let mut ids = Vec::new();
    doc.transact(label("x"), |e| {
        ids.push(e.insert(ROOT, "markers", 0, marker("Verso", 3840 * 4))?);
        ids.push(e.insert(ROOT, "regions", 0, region("Refrão", 3840 * 8, 3840 * 16))?);
        Ok(())
    })
    .unwrap();
    assert_eq!(ids[0].kind, b'g');
    assert_eq!(ids[1].kind, b'q');
    assert_eq!(doc.root().get("markers").map(Value::len), Some(1));
    doc.undo().unwrap();
    assert_eq!(doc.root().get("markers").map_or(0, Value::len), 0);
}

#[test]
fn two_markers_on_a_tick_and_a_tiny_region_are_refused() {
    let mut doc = new_doc();
    let r = doc.transact(label("x"), |e| {
        e.insert(ROOT, "markers", 0, marker("a", 960))?;
        e.insert(ROOT, "markers", 1, marker("b", 960))?;
        Ok(())
    });
    assert_eq!(r.unwrap_err().label.key, "doc.marks.invalid");
    let r = doc.transact(label("x"), |e| e.insert(ROOT, "regions", 0, region("tiny", 0, 100)).map(|_| ()));
    assert!(r.is_err());
    assert_eq!(doc.root().get("regions").map_or(0, Value::len), 0, "refused whole");
}

#[test]
fn a_version_1_project_opens_upgraded_and_keeps_its_meaning() {
    let doc = new_doc();
    let saved = doc.saved();
    let old = saved.with("versions", saved.get("versions").unwrap().with("project", Value::int(1)));
    let opened = Document::open(registry(), old, options()).unwrap();
    assert!(opened.upgraded);
    assert!(opened.document.root().get("markers").is_none(), "absent = none");
}

/// The project's notes (F-PRJ-018): an optional text of the root, set and undone like any field
#[test]
fn the_project_notes_are_set_and_undone() {
    let mut doc = new_doc();
    assert!(doc.root().get("notes").is_none(), "absent until written");
    doc.transact(label("notes"), |e| e.set(ROOT, &[lm_state::PathStep::key("notes")], text("take 3 is the one"))).unwrap();
    assert_eq!(doc.root().get("notes").and_then(Value::as_text), Some("take 3 is the one"));
    doc.undo().unwrap();
    assert!(doc.root().get("notes").is_none());
}
