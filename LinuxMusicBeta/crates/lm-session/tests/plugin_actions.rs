//! The plugin actions: an `.sbi` into an OpulenZ, one undo step, from a path or from the bytes

mod common;

use common::*;
use lm_base::Label;
use lm_session::Session;
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

/// A mono 16-bit WAV of four clicks, one every quarter second (something to find slices in)
fn clicks_wav() -> Vec<u8> {
    let rate = 44_100u32;
    let frames = rate as usize;
    let mut pcm = vec![0i16; frames];
    for at in [0usize, 11_025, 22_050, 33_075] {
        for (i, v) in pcm.iter_mut().skip(at).take(400).enumerate() {
            *v = (20_000.0 * (1.0 - i as f32 / 400.0) * if i % 2 == 0 { 1.0 } else { -1.0 }) as i16;
        }
    }
    let data_len = (frames * 2) as u32;
    let mut b = b"RIFF".to_vec();
    b.extend((36 + data_len).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(rate.to_le_bytes());
    b.extend((rate * 2).to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(data_len.to_le_bytes());
    for v in pcm {
        b.extend(v.to_le_bytes());
    }
    b
}

fn slicert_track(s: &mut Session) -> lm_state::NodeId {
    let n = s.document().root().get("tracks").map_or(0, Value::len);
    let mut track = None;
    s.document_mut()
        .transact(Label::new("test.track"), |e| {
            let instrument = Value::object([
                ("plugin", Value::text("slicert")),
                ("version", Value::int(1)),
                ("state", Value::empty_object()),
            ]);
            track = Some(e.insert(
                ROOT,
                "tracks",
                n,
                Value::object([("name", Value::text("Slices")), ("instrument", instrument)]),
            )?);
            Ok(())
        })
        .unwrap();
    track.unwrap()
}

#[test]
fn slicert_analyze_refuses_what_it_cannot_analyse() {
    let dir = TempDir::new("slicert-refuse");
    let mut s = session(&dir);
    let track = slicert_track(&mut s);
    let params = Value::object([("node", Value::text(track.to_string()))]);
    assert_eq!(s.run_action("slicert.analyze", &params).unwrap_err().key(), "slicert.no_sample");
    let other = add_synth_track(&mut s, "Synth");
    let params = Value::object([("node", Value::text(other.to_string()))]);
    assert_eq!(s.run_action("slicert.analyze", &params).unwrap_err().key(), "slicert.not_slicert");
    assert_eq!(s.run_action("slicert.analyze", &Value::empty_object()).unwrap_err().key(), "action.params");
}

#[test]
fn a_sample_set_on_a_slicert_is_analysed_by_itself() {
    let dir = TempDir::new("slicert-auto");
    let mut s = session(&dir);
    let track = slicert_track(&mut s);
    let file = dir.0.join("clicks.wav");
    std::fs::write(&file, clicks_wav()).unwrap();
    s.run_action("resources.import", &Value::object([("path", Value::text(file.to_str().unwrap()))])).unwrap();
    let mut resource = None;
    for _ in 0..2000 {
        s.pump();
        if let Some((_, r)) = s.take_imported().into_iter().next() {
            resource = Some(r.unwrap());
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let resource = resource.expect("the import ended");
    let path =
        [lm_state::PathStep::key("instrument"), lm_state::PathStep::key("state"), lm_state::PathStep::key("sample")];
    s.document_mut()
        .transact(Label::new("test.sample"), |e| e.set(track, &path, Value::resource(resource.clone())))
        .unwrap();
    // The pump starts the analysis; its result comes back through the pump too
    let slices = |s: &lm_session::Session| {
        s.document()
            .find(track)
            .and_then(|t| t.get("instrument").and_then(|i| i.get("state")).and_then(|st| st.get("slices")).cloned())
            .map_or(0, |l| l.len())
    };
    for _ in 0..2000 {
        s.pump();
        if slices(&s) > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(slices(&s) > 0, "the slices were found when the sample was set");
    let state = s.document().find(track).unwrap().get("instrument").unwrap().get("state").unwrap().clone();
    let bpm = state.get("params").and_then(|p| p.get("originalBpm")).and_then(Value::as_real).unwrap();
    assert!((1.0..=999.0).contains(&bpm));
}
