//! MIDI files (agent D; F-PRJ-019, F-PRJ-021, P-39): export then import gives back the notes; an import is an
//! operation and one undo step, with a desk channel per track and CC7/CC10 as automation

mod common;

use common::*;
use lm_smf::{Event, Smf, Timed, Track};
use lm_state::Value;

/// The session's settle does not wait for the generic operations: wait for them too
fn wait_ops(s: &mut lm_session::Session) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        settle(s);
        if s.activity().is_empty() || std::time::Instant::now() > deadline {
            settle(s);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn notes_of(root: &Value, track: usize) -> Vec<(i64, i64, i64)> {
    let t = root.get("tracks").unwrap().at(track).unwrap();
    let mut out = Vec::new();
    for item in t.get("items").unwrap().items() {
        let start = item.get("start").and_then(Value::as_int).unwrap();
        for (_, n) in item.get("notes").unwrap().entries() {
            let g = |k: &str| n.get(k).and_then(Value::as_int).unwrap();
            out.push((g("key"), start + g("start"), g("length")));
        }
    }
    out.sort();
    out
}

#[test]
fn export_then_import_gives_the_notes_back() {
    let dir = TempDir::new("midi");
    let mut s = session(&dir);
    song(&mut s);
    let file = dir.join("song.mid");
    let _ = s.midi_export(file.clone(), None);
    wait_ops(&mut s);
    assert!(file.exists());
    let before = notes_of(s.document().root(), 0);
    let tracks = s.document().root().get("tracks").map_or(0, Value::len);
    let channels = s.document().root().get("mixer").and_then(|m| m.get("channels")).map_or(0, Value::len);
    let _ = s.midi_import(file, 0);
    wait_ops(&mut s);
    let root = s.document().root().clone();
    assert_eq!(root.get("tracks").map_or(0, Value::len), tracks + 1);
    assert_eq!(notes_of(&root, tracks), before, "the same notes, at the same ticks");
    assert_eq!(
        root.get("mixer").and_then(|m| m.get("channels")).map_or(0, Value::len),
        channels + 1,
        "a channel per track (P-39)"
    );
    s.document_mut().undo().unwrap();
    assert_eq!(s.document().root().get("tracks").map_or(0, Value::len), tracks, "one undo step");
}

#[test]
fn controllers_become_automation_and_the_tempo_goes_to_the_map() {
    let dir = TempDir::new("midi-cc");
    let mut s = session(&dir);
    let t = |tick, event| Timed { tick, event };
    let smf = Smf {
        format: 1,
        ppq: 480,
        tracks: vec![
            Track { events: vec![t(0, Event::Tempo(400_000)), t(0, Event::TimeSignature(3, 4))] },
            Track {
                events: vec![
                    t(0, Event::TrackName("Baixo".into())),
                    t(0, Event::Controller { channel: 1, number: 7, value: 90 }),
                    t(0, Event::Controller { channel: 1, number: 10, value: 127 }),
                    t(0, Event::NoteOn { channel: 1, key: 40, velocity: 100 }),
                    t(480, Event::NoteOff { channel: 1, key: 40, velocity: 0 }),
                ],
            },
        ],
    };
    let file = dir.join("cc.mid");
    std::fs::write(&file, lm_smf::write(&smf)).unwrap();
    let _ = s.midi_import(file, 0);
    wait_ops(&mut s);
    let root = s.document().root().clone();
    let map = root.get("tempoMap").unwrap().at(0).unwrap();
    assert_eq!(map.get("bpm").and_then(Value::as_text), Some("150"));
    let track = root.get("tracks").unwrap().items().last().unwrap().clone();
    assert_eq!(track.get("name").and_then(Value::as_text), Some("Baixo"));
    let lane = track.get("automation").unwrap().at(0).unwrap();
    assert_eq!(lane.get("target").and_then(|t| t.get("param")).and_then(Value::as_text), Some("volume"));
    let ch = root.get("mixer").and_then(|m| m.get("channels")).unwrap().items().last().unwrap().clone();
    let pan = ch.get("automation").unwrap().at(0).unwrap();
    assert_eq!(pan.get("target").and_then(|t| t.get("param")).and_then(Value::as_text), Some("pan"));
    assert_eq!(
        notes_of(&root, root.get("tracks").map_or(1, Value::len) - 1),
        vec![(40, 0, 960)],
        "480 file ppq to 960"
    );
}

#[test]
fn a_broken_file_is_a_notice_not_a_change() {
    let dir = TempDir::new("midi-bad");
    let mut s = session(&dir);
    let file = dir.join("bad.mid");
    std::fs::write(&file, b"not a midi file").unwrap();
    let tracks = s.document().root().get("tracks").map_or(0, Value::len);
    let _ = s.midi_import(file, 0);
    wait_ops(&mut s);
    assert_eq!(s.document().root().get("tracks").map_or(0, Value::len), tracks);
    assert!(s.notices().keys().contains(&"session.midi.read"), "{:?}", s.notices().keys());
}

#[test]
fn bend_aftertouch_and_cc_become_control_lanes_of_the_item() {
    let dir = TempDir::new("midi-bend");
    let mut s = session(&dir);
    let t = |tick, event| Timed { tick, event };
    let smf = Smf {
        format: 1,
        ppq: 480,
        tracks: vec![Track {
            events: vec![
                t(0, Event::Controller { channel: 0, number: 1, value: 64 }),
                t(0, Event::NoteOn { channel: 0, key: 60, velocity: 100 }),
                t(0, Event::PitchBend { channel: 0, value: 8192 }),
                t(240, Event::PitchBend { channel: 0, value: 16383 }),
                t(480, Event::NoteOff { channel: 0, key: 60, velocity: 0 }),
            ],
        }],
    };
    let file = dir.join("bend.mid");
    std::fs::write(&file, lm_smf::write(&smf)).unwrap();
    let _ = s.midi_import(file, 0);
    wait_ops(&mut s);
    let root = s.document().root().clone();
    let track = root.get("tracks").unwrap().items().last().unwrap().clone();
    let item = track.get("items").unwrap().at(0).unwrap().clone();
    let lanes: Vec<Value> = item.get("controls").unwrap().items().cloned().collect();
    let bend = lanes.iter().find(|l| l.get("kind").and_then(Value::as_text) == Some("pitchBend")).expect("bend lane");
    let ticks: Vec<i64> = bend.get("points").unwrap().items().filter_map(|p| p.get("tick").and_then(Value::as_int)).collect();
    assert_eq!(ticks, vec![0, 480], "480 file ppq to 960");
    assert!(lanes.iter().any(|l| l.get("kind").and_then(Value::as_text) == Some("cc")
        && l.get("number").and_then(Value::as_int) == Some(1)));
}

#[test]
fn only_an_audio_item_exports_its_sample() {
    let dir = TempDir::new("sample-export");
    let mut s = session(&dir);
    let track = add_synth_track(&mut s, "Synth");
    let item = add_notes(&mut s, track, &[(60, 0, 480)]);
    let err = s
        .run_action(
            "clip.export_sample",
            &Value::object([("item", Value::text(item.to_string())), ("path", Value::text(dir.join("x.flac").to_str().unwrap()))]),
        )
        .unwrap_err();
    assert_eq!(err.key(), "export.sample.no_audio");
}
