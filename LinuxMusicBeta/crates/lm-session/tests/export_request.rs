//! The export screen's request (`project.export`, lm_port::trees): range, fast or exact, repeats, stems, formats,
//! rate, normalisation, fades, dither; the same request gives the same files

mod common;

use std::path::{Path, PathBuf};

use common::*;
use lm_base::{Cancel, Label, Progress};
use lm_session::Session;
use lm_session::export::{ExportRequest, export_request};
use lm_state::{NodeId, PathStep, ROOT, Value};

fn read(path: &Path) -> (Vec<f32>, u32) {
    let mut r = lm_sys_sndfile::Reader::open(path).expect("open");
    let mut buf = vec![0.0f32; r.info().frames as usize * r.info().channels];
    let n = r.read_f32(&mut buf).expect("read");
    buf.truncate(n * r.info().channels);
    (buf, r.info().rate)
}

/// Two synth tracks on a digital desk, two bars
fn song(dir: &TempDir) -> (Session, NodeId, NodeId) {
    let mut s = session(dir);
    s.create(dir.join("song")).expect("create");
    s.document_mut()
        .transact(Label::new("test"), |e| {
            e.set(ROOT, &[PathStep::key("mixer"), PathStep::key("mode")], Value::text("digital"))
        })
        .expect("digital");
    let a = add_synth_track(&mut s, "Bass");
    add_notes(&mut s, a, &[(36, 0, 3840), (43, 3840, 3840)]);
    let b = add_synth_track(&mut s, "Lead");
    add_notes(&mut s, b, &[(72, 960, 960), (76, 4800, 1920)]);
    settle(&mut s);
    (s, a, b)
}

fn request(dir: &TempDir, name: &str, extra: &[(&str, Value)]) -> ExportRequest {
    let mut v = Value::object([
        ("folder", Value::text(dir.join("out").display().to_string())),
        ("name", Value::text(name)),
        ("formats", Value::list([Value::text("wav")])),
        ("depth", Value::int(32)),
    ]);
    for (k, x) in extra {
        v = v.with(k, x.clone());
    }
    ExportRequest::from_value(&v).expect("request")
}

fn run(s: &Session, req: &ExportRequest) -> Value {
    export_request(s.export_job(), req, &Cancel::new(), &Progress::new()).expect("export")
}

fn out(dir: &TempDir, file: &str) -> PathBuf {
    dir.join("out").join(file)
}

#[test]
fn the_same_request_gives_the_same_files() {
    let dir = TempDir::new("req-same");
    let (s, _, _) = song(&dir);
    let mut formats = vec![Value::text("wav"), Value::text("flac"), Value::text("ogg")];
    if lm_sys_sndfile::is_supported(lm_sys_sndfile::WriteFormat::Mp3 { quality: 1.0 }, 48_000, 2) {
        formats.push(Value::text("mp3"));
    }
    let extra = [
        ("formats", Value::list(formats.clone())),
        ("depth", Value::int(16)),
        ("dither", Value::bool(true)),
        ("rate", Value::int(44_100)),
        ("normalize", Value::object([("kind", Value::text("peak")), ("target", Value::real(-1.0).expect("r"))])),
        ("fadeOut", Value::real(0.5).expect("r")),
    ];
    let report = run(&s, &request(&dir, "a", &extra));
    assert_eq!(report.get("files").and_then(Value::as_int), Some(formats.len() as i64), "{report:?}");
    let first: Vec<Vec<u8>> = formats
        .iter()
        .map(|f| std::fs::read(out(&dir, &format!("a.{}", f.as_text().unwrap_or("")))).expect("file"))
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    run(&s, &request(&dir, "a", &extra));
    for (f, bytes) in formats.iter().zip(first) {
        let again = std::fs::read(out(&dir, &format!("a.{}", f.as_text().unwrap_or("")))).expect("file");
        assert!(again == bytes, "{f:?} differs");
    }
    // The WAV: 44.1 kHz, peak at -1 dBFS (within the 16-bit step and the dither), faded to silence
    let (wav, rate) = read(&out(&dir, "a.wav"));
    assert_eq!(rate, 44_100);
    let peak = wav.iter().fold(0.0f32, |p, x| p.max(x.abs()));
    assert!((20.0 * f64::from(peak).log10() + 1.0).abs() < 0.01, "{peak}");
    assert!(wav[wav.len() - 2..].iter().all(|x| x.abs() < 1.0e-3));
}

#[test]
fn an_exact_range_is_the_same_samples_as_in_the_whole_song() {
    let dir = TempDir::new("req-range");
    let (s, _, _) = song(&dir);
    run(&s, &request(&dir, "whole", &[("tail", Value::real(0.0).expect("r"))]));
    let range = Value::object([("from", Value::int(1920)), ("to", Value::int(5760)), ("fast", Value::bool(false))]);
    let report = run(&s, &request(&dir, "part", &[("range", range)]));
    assert_eq!(report.get("identical").and_then(Value::as_bool), Some(true));
    let (whole, _) = read(&out(&dir, "whole.wav"));
    let (part, _) = read(&out(&dir, "part.wav"));
    let map = lm_session::tempo_map(s.document().root()).expect("map");
    let (a, b) = (map.sample_at(1920) as usize, map.sample_at(5760) as usize);
    // The range with its tail: the kept part starts exactly where the range does in the whole song
    assert!(part.len() >= 2 * (b - a));
    assert_eq!(&part[..2 * (b - a)], &whole[2 * a..2 * b]);
    // Fast: a warm-up instead, and the report says it is not identical
    let range = Value::object([
        ("from", Value::int(1920)),
        ("to", Value::int(5760)),
        ("fast", Value::bool(true)),
        ("warmup", Value::real(0.1).expect("r")),
    ]);
    let report = run(&s, &request(&dir, "fast", &[("range", range)]));
    assert_eq!(report.get("identical").and_then(Value::as_bool), Some(false));
}

#[test]
fn repeats_as_a_loop_and_stems() {
    let dir = TempDir::new("req-stems");
    let (s, a, b) = song(&dir);
    let range = Value::object([("from", Value::int(0)), ("to", Value::int(3840)), ("fast", Value::bool(false))]);
    run(
        &s,
        &request(&dir, "loop", &[("range", range.clone()), ("asLoop", Value::bool(true)), ("repeats", Value::int(3))]),
    );
    let (looped, _) = read(&out(&dir, "loop.wav"));
    let map = lm_session::tempo_map(s.document().root()).expect("map");
    let bar = map.sample_at(3840) as usize;
    assert_eq!(looped.len(), 2 * 3 * bar);
    // Stems per track: one file each, each the same as that track exported alone
    let stems = Value::object([("by", Value::text("tracks")), ("nodes", Value::empty_list())]);
    let report =
        run(&s, &request(&dir, "st", &[("range", range.clone()), ("asLoop", Value::bool(true)), ("stems", stems)]));
    assert_eq!(report.get("files").and_then(Value::as_int), Some(2));
    for (track, file) in [(a, "st - Bass.wav"), (b, "st - Lead.wav")] {
        run(
            &s,
            &request(
                &dir,
                "alone",
                &[
                    ("range", range.clone()),
                    ("asLoop", Value::bool(true)),
                    ("onlyTrack", Value::text(track.to_string())),
                ],
            ),
        );
        let (stem, _) = read(&out(&dir, file));
        let (alone, _) = read(&out(&dir, "alone.wav"));
        assert!(stem.iter().any(|x| x.abs() > 0.01));
        assert!(stem == alone, "{file}");
    }
    let _ = (a, b);
}

#[test]
fn normalised_to_a_loudness() {
    let dir = TempDir::new("req-lufs");
    let (s, _, _) = song(&dir);
    let norm = Value::object([("kind", Value::text("lufs")), ("target", Value::real(-20.0).expect("r"))]);
    let report = run(&s, &request(&dir, "n", &[("normalize", norm)]));
    let lufs = report.get("lufs").and_then(Value::as_real).unwrap_or(0.0);
    assert!((lufs + 20.0).abs() < 0.05, "{report:?}");
}
