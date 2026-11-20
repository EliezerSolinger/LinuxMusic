//! The export (I-13): deterministic, and the same whether it starts with everything ready or still loading (T8)

mod common;

use std::time::Duration;

use common::*;
use lm_base::{Cancel, Progress};
use lm_session::export::{ExportTarget, export};

fn samples(path: &std::path::Path) -> Vec<f32> {
    let mut r = lm_sys_sndfile::Reader::open(path).expect("open");
    let frames = r.info().frames as usize;
    let mut buf = vec![0.0f32; frames * r.info().channels];
    let n = r.read_f32(&mut buf).expect("read");
    buf.truncate(n * r.info().channels);
    buf
}

#[test]
fn an_export_is_the_same_every_time_and_sounds() {
    let dir = TempDir::new("export");
    let mut s = session(&dir);
    s.create(dir.join("song")).expect("create");
    song(&mut s);
    settle(&mut s);
    let job = s.export_job();
    let a = dir.join("a.wav");
    let b = dir.join("b.wav");
    let report = export(job.clone(), &ExportTarget::wav(a.clone()), &Cancel::new(), &Progress::new()).expect("export");
    export(job, &ExportTarget::wav(b.clone()), &Cancel::new(), &Progress::new()).expect("export");
    let (x, y) = (samples(&a), samples(&b));
    assert!(!x.is_empty());
    assert!(x.iter().zip(&y).all(|(p, q)| p.to_bits() == q.to_bits()) && x.len() == y.len(), "bit for bit");
    let peak = x.iter().fold(0.0f32, |p, v| p.max(v.abs()));
    assert!(peak > 0.01, "the song sounds: {peak}");
    assert!(report.get("lufs").and_then(lm_state::Value::as_real).unwrap_or(-200.0) > -60.0, "{report:?}");
}

#[test]
fn an_export_started_while_loading_equals_one_started_ready() {
    let dir = TempDir::new("export-t8");
    let mut config = config(&dir);
    config.delay = Duration::from_millis(300);
    let mut s = lm_session::Session::new(config).expect("session");
    s.create(dir.join("song")).expect("create");
    song(&mut s);
    // At once: the instrument is still being built in the session
    s.pump();
    let early = s.export_job();
    settle(&mut s);
    let ready = s.export_job();
    let (a, b) = (dir.join("early.wav"), dir.join("ready.wav"));
    export(early, &ExportTarget::wav(a.clone()), &Cancel::new(), &Progress::new()).expect("export");
    export(ready, &ExportTarget::wav(b.clone()), &Cancel::new(), &Progress::new()).expect("export");
    assert_eq!(samples(&a), samples(&b));
}
