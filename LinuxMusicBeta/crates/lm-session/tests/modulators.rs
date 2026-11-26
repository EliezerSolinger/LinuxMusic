//! The modulators as actions: a parameter's modulator, its targets, one per parameter

mod common;

use common::*;
use lm_state::{NodeId, Value};

fn modulators(s: &lm_session::Session, owner: NodeId) -> Vec<Value> {
    s.document()
        .find(owner)
        .and_then(|o| o.get("modulators").cloned())
        .map(|l| l.items().cloned().collect())
        .unwrap_or_default()
}

fn attach(s: &mut lm_session::Session, node: NodeId, param: &str) -> lm_base::Result<()> {
    s.run_action(
        "modulator.attach",
        &Value::object([("node", Value::text(node.to_string())), ("param", Value::text(param))]),
    )
}

#[test]
fn attach_makes_one_lfo_per_parameter_and_undoes_in_one_step() {
    let dir = TempDir::new("modulators");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    let t = add_synth_track(&mut s, "Synth");
    attach(&mut s, t, "volume").expect("attach");
    let ms = modulators(&s, t);
    assert_eq!(ms.len(), 1);
    assert_eq!(ms[0].get("kind").and_then(Value::as_text), Some("lfo"));
    assert_eq!(ms[0].get("targets").map(Value::len), Some(1));
    // The same parameter again: the one it has
    attach(&mut s, t, "volume").expect("again");
    assert_eq!(modulators(&s, t).len(), 1);
    // Another parameter: another modulator, and a second target on the first is refused
    attach(&mut s, t, "cc.1").expect("other");
    assert_eq!(modulators(&s, t).len(), 2);
    let first = NodeId::parse(ms[0].get("id").and_then(Value::as_text).unwrap()).unwrap();
    let again = Value::object([
        ("modulator", Value::text(first.to_string())),
        ("node", Value::text(t.to_string())),
        ("param", Value::text("cc.1")),
    ]);
    assert!(s.run_action("modulator.add_target", &again).is_err());
    s.document_mut().undo().expect("undo");
    assert_eq!(modulators(&s, t).len(), 1);
    s.run_action("modulator.remove", &Value::object([("modulator", Value::text(first.to_string()))])).expect("remove");
    assert!(modulators(&s, t).is_empty());
}
