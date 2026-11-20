//! The legacy desk's channel-level controls through the engine (exports, digital desk so the arithmetic is plain):
//! TRIM, MONO, phase invert, swap, the master's PRE faders; and the rack's live display

mod common;

use common::*;
use lm_base::{Cancel, Label, Progress};
use lm_session::Session;
use lm_session::export::{ExportTarget, export};
use lm_state::{NodeId, PathStep, ROOT, Value};

fn stereo(path: &std::path::Path) -> (Vec<f32>, Vec<f32>) {
    let mut r = lm_sys_sndfile::Reader::open(path).expect("open");
    let mut buf = vec![0.0f32; r.info().frames as usize * 2];
    let n = r.read_f32(&mut buf).expect("read");
    buf.truncate(n * 2);
    (buf.iter().step_by(2).copied().collect(), buf.iter().skip(1).step_by(2).copied().collect())
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |p, v| p.max(v.abs()))
}

/// A digital desk: a sine track into a hard-left channel, into the channel under test `c`
fn setup(s: &mut Session, dir: &TempDir) -> NodeId {
    s.create(dir.join("desk")).expect("create");
    let mut ch = None;
    let mut track = None;
    let mut src = None;
    s.document_mut()
        .transact(Label::new("test"), |e| {
            e.set(ROOT, &[PathStep::key("mixer"), PathStep::key("mode")], Value::text("digital"))?;
            // The channel under test receives a hard-left channel (the pan before it)
            let c = e.insert(ROOT, "mixer.channels", 0, Value::empty_object())?;
            let left = e.insert(ROOT, "mixer.channels", 1, Value::object([("pan", Value::real(-1.0)?)]))?;
            e.set(left, &[PathStep::key("output")], Value::text(c.to_string()))?;
            src = Some(left);
            let inst = Value::object([
                ("plugin", Value::text("testsine")),
                ("version", Value::int(2)),
                ("state", Value::empty_object()),
            ]);
            let t = e.insert(ROOT, "tracks", 0, Value::object([("instrument", inst)]))?;
            let it =
                e.insert(t, "items", 0, Value::object([("kind", Value::text("midi")), ("length", Value::int(960))]))?;
            e.add_note(
                it,
                Value::object([("key", Value::int(69)), ("length", Value::int(960)), ("velocity", Value::real(1.0)?)]),
            )?;
            track = Some(t);
            ch = Some(c);
            Ok(())
        })
        .expect("setup");
    let (c, t) = (ch.expect("channel"), track.expect("track"));
    s.document_mut()
        .transact(Label::new("test.route"), |e| {
            let out = e.find(t).and_then(|n| n.get("outputs").and_then(|l| l.at(0)).and_then(|o| o.get("id").cloned()));
            let out = NodeId::parse(out.and_then(|v| v.as_text().map(str::to_owned)).unwrap_or_default().as_str())?;
            e.set(out, &[PathStep::key("to")], Value::text(src.map(|x| x.to_string()).unwrap_or_default()))
        })
        .expect("route");
    settle(s);
    c
}

fn render(s: &Session, dir: &TempDir, name: &str) -> (Vec<f32>, Vec<f32>) {
    let path = dir.join(name);
    export(s.export_job(), &ExportTarget::wav(path.clone()), &Cancel::new(), &Progress::new()).expect("export");
    stereo(&path)
}

fn set(s: &mut Session, id: NodeId, key: &str, v: Value) {
    s.document_mut().transact(Label::new("test"), |e| e.set(id, &[PathStep::key(key)], v)).expect("set");
    settle(s);
}

#[test]
fn trim_mono_phase_swap_and_the_pre_faders() {
    let dir = TempDir::new("desk");
    let mut s = session(&dir);
    let c = setup(&mut s, &dir);
    let (l0, r0) = render(&s, &dir, "plain.wav");
    assert!(peak(&l0) > 0.1 && peak(&r0) < 1.0e-6, "hard left");
    // TRIM -6 dB
    set(&mut s, c, "trim", Value::real(-6.0).expect("r"));
    let (l, _) = render(&s, &dir, "trim.wav");
    let ratio = peak(&l) / peak(&l0);
    assert!((ratio - 0.501).abs() < 0.01, "trim: {ratio}");
    set(&mut s, c, "trim", Value::real(0.0).expect("r"));
    // Swap: the left goes right
    set(&mut s, c, "swap", Value::bool(true));
    let (l, r) = render(&s, &dir, "swap.wav");
    assert!(peak(&l) < 1.0e-6 && (peak(&r) - peak(&l0)).abs() < 1.0e-6, "swap");
    set(&mut s, c, "swap", Value::bool(false));
    // Phase invert: the same, upside down
    set(&mut s, c, "phaseInvert", Value::bool(true));
    let (l, _) = render(&s, &dir, "phase.wav");
    assert!(l.iter().zip(&l0).all(|(a, b)| (a + b).abs() < 1.0e-6), "phase");
    set(&mut s, c, "phaseInvert", Value::bool(false));
    // Mono: the hard-left input sums to (L+R)/2 on both sides
    set(&mut s, c, "mono", Value::bool(true));
    let (l, r) = render(&s, &dir, "mono.wav");
    assert!(
        (peak(&l) - peak(&l0) / 2.0).abs() < 1.0e-3 && (peak(&r) - peak(&l0) / 2.0).abs() < 1.0e-3,
        "mono: {} {} {}",
        peak(&l),
        peak(&r),
        peak(&l0)
    );
    set(&mut s, c, "mono", Value::bool(false));
    // The master's PRE faders: left at 0.5
    set(&mut s, lm_doc::MASTER, "preLeft", Value::real(0.5).expect("r"));
    let (l, _) = render(&s, &dir, "pre.wav");
    let ratio = peak(&l) / peak(&l0);
    assert!((ratio - 0.5).abs() < 0.01, "pre: {ratio}");
}

#[test]
fn the_master_rack_shows_its_meters() {
    let dir = TempDir::new("display");
    let mut s = session(&dir);
    s.create(dir.join("d")).expect("create");
    song(&mut s);
    settle(&mut s);
    s.play();
    let start = std::time::Instant::now();
    let mut seen = None;
    while start.elapsed() < std::time::Duration::from_secs(10) {
        s.pump();
        if let Some(d) = s.rack_display(lm_doc::MASTER)
            && d[lm_session::rack::display::CORRELATION].is_finite()
            && d[lm_session::rack::display::GONIO..].iter().any(|v| *v != 0.0)
        {
            seen = Some(d);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let d = seen.expect("the master's display");
    assert!(d[lm_session::rack::display::SAT].is_finite() && d[lm_session::rack::display::BUS_GR].is_finite());
}

#[test]
fn an_inserts_wet_dry_mix() {
    let dir = TempDir::new("wetdry");
    let mut s = session(&dir);
    let c = setup(&mut s, &dir);
    let (l0, _) = render(&s, &dir, "dry.wav");
    // An amplifier at 200 % on the channel
    let mut insert = None;
    s.document_mut()
        .transact(Label::new("test.insert"), |e| {
            let state = Value::object([("params", Value::object([("volume", Value::real(200.0)?)]))]);
            let p = Value::object([("plugin", Value::text("amplifier")), ("version", Value::int(1)), ("state", state)]);
            insert = Some(e.insert(c, "inserts", 0, p)?);
            Ok(())
        })
        .expect("insert");
    settle(&mut s);
    let p = insert.expect("p");
    let ratio = |s: &Session, name: &str| {
        let (l, _) = render(s, &dir, name);
        peak(&l) / peak(&l0)
    };
    assert!((ratio(&s, "wet-absent.wav") - 2.0).abs() < 0.01, "no mix: all wet");
    set(&mut s, p, "wet", Value::real(0.0).expect("r"));
    assert!((ratio(&s, "wet0.wav") - 1.0).abs() < 0.01, "all dry");
    set(&mut s, p, "wet", Value::real(0.5).expect("r"));
    assert!((ratio(&s, "wet05.wav") - 1.5).abs() < 0.01, "half: 0.5·2 + 0.5·1");
}
