//! The plugin actions: an `.sbi` into an OpulenZ, one undo step, from a path or from the bytes

mod common;

use common::*;
use lm_base::Label;
use lm_state::{ROOT, Value};

fn sbi() -> Vec<u8> {
    let mut b = b"SBI\x1aOrgan".to_vec();
    b.resize(36, 0);
    b.extend([0x21, 0x31, 0x4f, 0x00, 0xf2, 0xd2, 0x52, 0x73, 0x00, 0x01, 0x06]);
    b.resize(52, 0);
    b
}

#[test]
fn an_sbi_sets_the_opulenz_knobs_in_one_undo_step() {
    let dir = TempDir::new("plugin-actions");
    let mut s = session(&dir);
    let n = s.document().root().get("tracks").map_or(0, Value::len);
    let mut track = None;
    s.document_mut()
        .transact(Label::new("test.track"), |e| {
            let instrument = Value::object([
                ("plugin", Value::text("opulenz")),
                ("version", Value::int(1)),
                ("state", Value::empty_object()),
            ]);
            track = Some(e.insert(
                ROOT,
                "tracks",
                n,
                Value::object([("name", Value::text("FM")), ("instrument", instrument)]),
            )?);
            Ok(())
        })
        .unwrap();
    let track = track.unwrap();
    let file = dir.0.join("organ.sbi");
    std::fs::write(&file, sbi()).unwrap();
    let params =
        Value::object([("node", Value::text(track.to_string())), ("path", Value::text(file.to_str().unwrap()))]);
    s.run_action("opulenz.import_sbi", &params).unwrap();
    let state = s.document().find(track).unwrap().get("instrument").unwrap().get("state").unwrap().clone();
    let p = |k: &str| state.get("params").and_then(|v| v.get(k)).and_then(Value::as_real);
    assert_eq!(p("op1Level"), Some(63.0 - 15.0));
    assert_eq!(p("feedback"), Some(3.0));
    assert_eq!(state.get("op2Waveform").and_then(Value::as_text), Some("half-sine"));
    // One undo step
    s.document_mut().undo().unwrap();
    let state = s.document().find(track).unwrap().get("instrument").unwrap().get("state").unwrap().clone();
    let level = state.get("params").and_then(|v| v.get("op1Level")).and_then(Value::as_real);
    assert_ne!(level, Some(63.0 - 15.0), "the undo brings the knobs back");
    // The bytes themselves (a client's file)
    let bytes = Value::list(sbi().into_iter().map(|b| Value::int(i64::from(b))));
    let params = Value::object([("node", Value::text(track.to_string())), ("bytes", bytes)]);
    s.run_action("opulenz.import_sbi", &params).unwrap();
    let state = s.document().find(track).unwrap().get("instrument").unwrap().get("state").unwrap().clone();
    assert_eq!(state.get("params").and_then(|v| v.get("op2Multiplier")).and_then(Value::as_real), Some(1.0));
    // Not an OpulenZ
    let other = add_synth_track(&mut s, "Synth");
    let params =
        Value::object([("node", Value::text(other.to_string())), ("path", Value::text(file.to_str().unwrap()))]);
    assert_eq!(s.run_action("opulenz.import_sbi", &params).unwrap_err().key(), "opulenz.sbi.not_opulenz");
}
