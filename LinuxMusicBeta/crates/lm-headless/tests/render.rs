//! `lm-headless render` is the export's path (T19): a saved project renders to the same file as the session's export

use std::path::PathBuf;
use std::time::Duration;

use lm_base::{Cancel, Label, Progress};
use lm_session::export::{ExportTarget, export};
use lm_session::{Session, SessionConfig};
use lm_state::{ROOT, Value};

fn read(path: &std::path::Path) -> Vec<f32> {
    let mut r = lm_sys_sndfile::Reader::open(path).expect("open");
    let mut buf = vec![0.0f32; r.info().frames as usize * r.info().channels];
    let n = r.read_f32(&mut buf).expect("read");
    buf.truncate(n * r.info().channels);
    buf
}

#[test]
fn render_gives_the_export_of_the_saved_project() {
    let dir = std::env::temp_dir().join(format!("lm-headless-{}", lm_io::new_uuid()));
    let folder = dir.join("song");
    let mut s = Session::new(SessionConfig::headless(dir.join("untitled")).expect("config")).expect("session");
    s.create(folder.clone()).expect("create");
    s.document_mut()
        .transact(Label::new("test"), |e| {
            let instrument = Value::object([
                ("plugin", Value::text("tripleoscillator")),
                ("version", Value::int(1)),
                ("state", Value::empty_object()),
            ]);
            let t = e.insert(ROOT, "tracks", 0, Value::object([("instrument", instrument)]))?;
            let it =
                e.insert(t, "items", 0, Value::object([("kind", Value::text("midi")), ("length", Value::int(1920))]))?;
            for (k, st) in [(60, 0), (67, 480)] {
                let note =
                    Value::object([("key", Value::int(k)), ("start", Value::int(st)), ("length", Value::int(960))]);
                e.add_note(it, note)?;
            }
            Ok(())
        })
        .expect("song");
    s.save().expect("save");
    assert!(s.settle(Duration::from_secs(20)));
    let expected = dir.join("session.wav");
    export(s.export_job(), &ExportTarget::wav(expected.clone()), &Cancel::new(), &Progress::new()).expect("export");
    drop(s);
    let out: PathBuf = dir.join("headless.wav");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_lm-headless"))
        .args(["render", folder.to_str().expect("path"), out.to_str().expect("path")])
        .status()
        .expect("run");
    assert!(status.success());
    let (a, b) = (read(&expected), read(&out));
    assert!(!a.is_empty());
    assert!(a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()), "bit for bit");
    let _ = std::fs::remove_dir_all(&dir);
}
