//! Recording (I-11): takes enter the tree in one transaction per recording; a take of a track removed meanwhile goes
//! to a "Take recuperado" track (H8); the records are forgotten once journaled

mod common;

use std::time::{Duration, Instant};

use common::*;
use lm_base::Label;
use lm_session::Session;
use lm_state::{NodeId, PathStep, ROOT, Value};

fn audio_track(s: &mut Session, name: &str, channel: i64) -> NodeId {
    let n = s.document().root().get("tracks").map_or(0, Value::len);
    let mut id = None;
    s.document_mut()
        .transact(Label::new("test.track"), |e| {
            let input =
                Value::object([("device", Value::text("in")), ("channels", Value::list([Value::int(channel)]))]);
            let t = Value::object([("name", Value::text(name)), ("armed", Value::bool(true)), ("input", input)]);
            id = Some(e.insert(ROOT, "tracks", n, t)?);
            Ok(())
        })
        .expect("track");
    id.expect("id")
}

fn run_for(s: &mut Session, d: Duration) {
    let start = Instant::now();
    while start.elapsed() < d {
        s.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn items(s: &Session, track: NodeId) -> usize {
    s.document().find(track).and_then(|t| t.get("items").map(Value::len)).unwrap_or(0)
}

fn wait_until(s: &mut Session, what: &str, mut f: impl FnMut(&Session) -> bool) {
    let start = Instant::now();
    while !f(s) {
        assert!(start.elapsed() < Duration::from_secs(20), "{what}: {:?}", s.notices().keys());
        s.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_recording_of_two_tracks_is_one_undo_step() {
    let dir = TempDir::new("record");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    let a = audio_track(&mut s, "A", 1);
    let b = audio_track(&mut s, "B", 2);
    settle(&mut s);
    s.record(None).expect("record");
    run_for(&mut s, Duration::from_millis(600));
    s.stop();
    wait_until(&mut s, "the takes", |s| items(s, a) == 1 && items(s, b) == 1);
    let lane = s.document().find(a).and_then(|t| t.get("items").and_then(|l| l.at(0).cloned())).expect("item");
    let resource = lane
        .at_path(&[PathStep::key("lanes"), PathStep::Index(0), PathStep::key("source")])
        .and_then(Value::as_resource)
        .cloned()
        .expect("resource");
    assert!(lm_io::resource_path(s.project().folder.root(), resource.key()).expect("path").is_some());
    // One undo takes the whole recording away
    s.document_mut().undo().expect("undo");
    assert_eq!((items(&s, a), items(&s, b)), (0, 0));
    wait_until(&mut s, "the records forgotten", |s| {
        lm_io::find_interrupted(&s.project().folder.audio()).map(|v| v.is_empty()).unwrap_or(false)
    });
}

#[test]
fn a_take_of_a_removed_track_is_recovered() {
    let dir = TempDir::new("record-h8");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    let a = audio_track(&mut s, "Gone", 1);
    settle(&mut s);
    s.record(None).expect("record");
    run_for(&mut s, Duration::from_millis(400));
    s.document_mut().transact(Label::new("test.remove"), |e| e.remove(a)).expect("remove");
    run_for(&mut s, Duration::from_millis(200));
    s.stop();
    wait_until(&mut s, "the recovered take", |s| {
        s.document().root().get("tracks").is_some_and(|l| {
            l.items().any(|t| {
                t.get("name").and_then(Value::as_text) == Some("Take recuperado")
                    && t.get("items").map_or(0, Value::len) == 1
            })
        })
    });
    assert!(s.notices().keys().contains(&"session.take.recovered_track"));
}
