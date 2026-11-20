//! Helpers of the session's tests
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::Duration;

use lm_base::Label;
use lm_session::{Session, SessionConfig};
use lm_state::{NodeId, ROOT, Value};

/// A fresh folder under the system's temporary folder, removed on drop
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("lm-session-{name}-{}", lm_io::new_uuid()));
        std::fs::create_dir_all(&p).expect("temp dir");
        TempDir(p)
    }

    pub fn join(&self, s: &str) -> PathBuf {
        self.0.join(s)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn config(dir: &TempDir) -> SessionConfig {
    SessionConfig::headless(dir.join("untitled")).expect("config")
}

pub fn session(dir: &TempDir) -> Session {
    Session::new(config(dir)).expect("session")
}

pub fn settle(s: &mut Session) {
    assert!(s.settle(Duration::from_secs(20)), "the session did not settle: {:?}", s.activity());
}

/// A track with a TripleOscillator, as the last track
pub fn add_synth_track(s: &mut Session, name: &str) -> NodeId {
    let n = s.document().root().get("tracks").map_or(0, Value::len);
    let mut id = None;
    s.document_mut()
        .transact(Label::new("test.track"), |e| {
            let instrument = Value::object([
                ("plugin", Value::text("tripleoscillator")),
                ("version", Value::int(1)),
                ("state", Value::empty_object()),
            ]);
            let track = Value::object([("name", Value::text(name)), ("instrument", instrument)]);
            id = Some(e.insert(ROOT, "tracks", n, track)?);
            Ok(())
        })
        .expect("add track");
    id.expect("id")
}

pub fn set(s: &mut Session, id: NodeId, key: &str, v: Value) {
    s.document_mut().transact(Label::new("test.set"), |e| e.set(id, &[lm_state::PathStep::key(key)], v)).expect("set");
}

/// Waits for an open operation
pub fn open(s: &mut Session, folder: PathBuf) {
    let _ = s.open(folder);
    settle(s);
}

/// A MIDI item on a track with a few notes (ticks at 960 PPQ)
pub fn add_notes(s: &mut Session, track: NodeId, notes: &[(i64, i64, i64)]) -> NodeId {
    let mut id = None;
    s.document_mut()
        .transact(Label::new("test.item"), |e| {
            let n = e.find(track).and_then(|t| t.get("items").map(Value::len)).unwrap_or(0);
            let end = notes.iter().map(|(_, st, len)| st + len).max().unwrap_or(960);
            let item =
                Value::object([("kind", Value::text("midi")), ("start", Value::int(0)), ("length", Value::int(end))]);
            let it = e.insert(track, "items", n, item)?;
            for (key, start, length) in notes {
                let note = Value::object([
                    ("key", Value::int(*key)),
                    ("start", Value::int(*start)),
                    ("length", Value::int(*length)),
                    ("velocity", Value::real(0.8)?),
                ]);
                e.add_note(it, note)?;
            }
            id = Some(it);
            Ok(())
        })
        .expect("notes");
    id.expect("item")
}

/// A short song: a TripleOscillator playing a chord
pub fn song(s: &mut Session) -> NodeId {
    let t = add_synth_track(s, "Synth");
    add_notes(s, t, &[(60, 0, 960), (64, 480, 960), (67, 960, 960), (48, 0, 1920)]);
    t
}
