//! The first slice (interfaces.md, Parte 4, step 10; arquitetura-nova.md, "A ordem de construção"), as one repeatable
//! run over a session: record an input with monitoring, a TripleOscillator playing a MIDI item, one effect on a desk
//! channel through the analog rack and the master, the transport, save, close, reopen, and play back identically.
//!
//! `lm-headless slice` runs it on a real device and prints the report; the tests run it on the Dummy. The report:
//! the device's latencies, the xruns, the take's length and RMS, and whether the export after reopening is the same,
//! bit for bit, as before closing.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use lm_base::{Cancel, Error, Label, Progress, Result};
use lm_state::{NodeId, PathStep, ROOT, Value};

use crate::export::{ExportTarget, export};
use crate::session::Session;

/// What the slice does
#[derive(Clone, Debug)]
pub struct SliceOptions {
    /// The project's folder (made new)
    pub folder: PathBuf,
    /// Seconds recorded
    pub record_seconds: f64,
    /// The device input recorded, from 1
    pub input: i64,
    /// The effect inserted on the desk channel
    pub effect: &'static str,
}

impl SliceOptions {
    pub fn new(folder: PathBuf) -> Self {
        Self { folder, record_seconds: 5.0, input: 1, effect: "reverbsc" }
    }
}

fn wait(s: &mut Session, what: &'static str, timeout: Duration, mut done: impl FnMut(&Session) -> bool) -> Result<()> {
    let start = Instant::now();
    while !done(s) {
        if start.elapsed() > timeout {
            return Err(Error::new("slice.timeout").arg("step", what));
        }
        s.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

fn settle(s: &mut Session, what: &'static str) -> Result<()> {
    if s.settle(Duration::from_secs(60)) {
        Ok(())
    } else {
        Err(Error::new("slice.not_settled").arg("step", what).arg("busy", s.busy().join(", ")))
    }
}

fn id_at(v: Option<&Value>) -> Option<NodeId> {
    v.and_then(|n| n.get("id")).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok())
}

fn file_bytes(path: &std::path::Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| crate::folder::io_error("slice.read", path, &e))
}

/// The samples of an audio file, interleaved
fn samples(path: &std::path::Path) -> Result<(Vec<f32>, usize, u32)> {
    let mut r = lm_sys_sndfile::Reader::open(path)?;
    let channels = r.info().channels;
    let rate = r.info().rate;
    let mut buf = vec![0.0f32; r.info().frames as usize * channels];
    let n = r.read_f32(&mut buf)?;
    buf.truncate(n * channels);
    Ok((buf, channels, rate))
}

/// Runs the slice on a session (its device, its configuration) and gives the report
pub fn run_slice(s: &mut Session, o: &SliceOptions) -> Result<Value> {
    let t0 = Instant::now();
    s.create(o.folder.clone())?;
    // A desk channel with the effect, a TripleOscillator track playing into it, and an armed track monitoring the
    // input (both through the channel's analog rack and the master's)
    let mut channel = None;
    let mut synth = None;
    let mut voice = None;
    let beats = (o.record_seconds * 2.0).ceil() as i64; // 120 BPM: two beats a second
    s.document_mut().transact(Label::new("slice.setup"), |e| {
        let insert = Value::object([
            ("plugin", Value::text(o.effect)),
            ("version", Value::int(1)),
            ("state", Value::empty_object()),
        ]);
        let ch = Value::object([("name", Value::text("FX")), ("inserts", Value::list([insert]))]);
        let c = e.insert(ROOT, "mixer.channels", 0, ch)?;
        let instrument = Value::object([
            ("plugin", Value::text("tripleoscillator")),
            ("version", Value::int(1)),
            ("state", Value::empty_object()),
        ]);
        let t =
            e.insert(ROOT, "tracks", 0, Value::object([("name", Value::text("Synth")), ("instrument", instrument)]))?;
        let item = Value::object([
            ("kind", Value::text("midi")),
            ("start", Value::int(0)),
            ("length", Value::int(beats * 960)),
        ]);
        let it = e.insert(t, "items", 0, item)?;
        let chords = [[48i64, 55, 64], [45, 52, 60], [41, 48, 57], [43, 50, 59]];
        for b in 0..beats {
            for key in chords[(b as usize / 2) % 4] {
                let note = Value::object([
                    ("key", Value::int(key)),
                    ("start", Value::int(b * 960)),
                    ("length", Value::int(900)),
                    ("velocity", Value::real(0.7)?),
                ]);
                e.add_note(it, note)?;
            }
        }
        let input = Value::object([("device", Value::text("in")), ("channels", Value::list([Value::int(o.input)]))]);
        let v = e.insert(
            ROOT,
            "tracks",
            1,
            Value::object([
                ("name", Value::text("Voz")),
                ("armed", Value::bool(true)),
                ("monitor", Value::text("on")),
                ("channels", Value::int(1)),
                ("input", input),
            ]),
        )?;
        channel = Some(c);
        synth = Some(t);
        voice = Some(v);
        Ok(())
    })?;
    let (c, t, v) = match (channel, synth, voice) {
        (Some(c), Some(t), Some(v)) => (c, t, v),
        _ => return Err(Error::new("slice.setup")),
    };
    // Both tracks' outputs into the channel
    s.document_mut().transact(Label::new("slice.route"), |e| {
        for track in [t, v] {
            let out = id_at(e.find(track).and_then(|n| n.get("outputs").and_then(|l| l.at(0).cloned())).as_ref());
            if let Some(out) = out {
                e.set(out, &[PathStep::key("to")], Value::text(c.to_string()))?;
            }
        }
        Ok(())
    })?;
    settle(s, "setup")?;
    let setup = t0.elapsed();
    // Record while the synth plays
    let xruns_before = s.telemetry().xruns;
    s.record(None)?;
    wait(s, "recording starts", Duration::from_secs(10), |s| s.telemetry().position.recording)?;
    let started = Instant::now();
    while started.elapsed().as_secs_f64() < o.record_seconds {
        s.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    s.stop();
    wait(s, "the take", Duration::from_secs(60), |s| {
        s.document().find(v).and_then(|n| n.get("items").map(Value::len)).unwrap_or(0) == 1
    })?;
    settle(s, "after the take")?;
    let xruns = s.telemetry().xruns.saturating_sub(xruns_before);
    // The take
    let item = s
        .document()
        .find(v)
        .and_then(|n| n.get("items").and_then(|l| l.at(0).cloned()))
        .ok_or_else(|| Error::new("slice.no_take"))?;
    let resource = item
        .at_path(&[PathStep::key("lanes"), PathStep::Index(0), PathStep::key("source")])
        .and_then(Value::as_resource)
        .cloned()
        .ok_or_else(|| Error::new("slice.no_take"))?;
    let take_path =
        lm_io::resource_path(s.project().folder.root(), resource.key())?.ok_or_else(|| Error::new("slice.no_take"))?;
    let (take, channels, rate) = samples(&take_path)?;
    let take_seconds = take.len() as f64 / channels.max(1) as f64 / f64::from(rate);
    let rms = (take.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>() / take.len().max(1) as f64).sqrt();
    let rms_db = if rms > 0.0 { 20.0 * rms.log10() } else { -200.0 };
    // Save, export, close, reopen, export again
    s.save()?;
    settle(s, "save")?;
    let before = s.document().root().clone();
    let a = o.folder.join("slice-before.wav");
    let b = o.folder.join("slice-after.wav");
    let report_a = export(s.export_job(), &ExportTarget::wav(a.clone()), &Cancel::new(), &Progress::new())?;
    s.close_project()?;
    let _ = s.open(o.folder.clone());
    settle(s, "reopen")?;
    let same_tree = s.document().root().get("tracks") == before.get("tracks")
        && s.document().root().get("mixer") == before.get("mixer");
    let report_b = export(s.export_job(), &ExportTarget::wav(b.clone()), &Cancel::new(), &Progress::new())?;
    let identical = file_bytes(&a)? == file_bytes(&b)?;
    let first_difference = if identical {
        Value::NULL
    } else {
        let (x, _, _) = samples(&a)?;
        let (y, _, _) = samples(&b)?;
        let at = x.iter().zip(&y).position(|(p, q)| p.to_bits() != q.to_bits()).unwrap_or(x.len().min(y.len()));
        Value::object([
            ("sample", Value::int(at as i64)),
            ("lengths", Value::list([Value::int(x.len() as i64), Value::int(y.len() as i64)])),
        ])
    };
    let info = s.device_info().cloned();
    let real = |x: f64| Value::real(x).unwrap_or(Value::NULL);
    let device = match info {
        Some(i) => Value::object([
            ("name", Value::text(i.name.as_str())),
            ("rate", Value::int(i64::from(i.rate))),
            ("period", Value::int(i64::from(i.period))),
            ("periods", Value::int(i64::from(i.periods))),
            ("inputLatency", Value::int(i64::from(i.input_latency))),
            ("outputLatency", Value::int(i64::from(i.output_latency))),
            ("roundTripMs", real(f64::from(i.input_latency + i.output_latency) * 1000.0 / f64::from(i.rate.max(1)))),
            ("linked", Value::bool(i.linked)),
        ]),
        None => Value::NULL,
    };
    Ok(Value::object([
        ("device", device),
        ("xruns", Value::int(i64::from(xruns))),
        ("takeSeconds", real(take_seconds)),
        ("takeRmsDb", real(rms_db)),
        ("sameTreeAfterReopen", Value::bool(same_tree)),
        ("identicalAfterReopen", Value::bool(identical)),
        ("firstDifference", first_difference),
        ("exportBefore", report_a),
        ("exportAfter", report_b),
        ("setupMs", real(setup.as_secs_f64() * 1000.0)),
        ("totalMs", real(t0.elapsed().as_secs_f64() * 1000.0)),
    ]))
}
