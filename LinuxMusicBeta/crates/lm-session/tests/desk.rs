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

/// WIDTH: 0 sums the hard-left input to mono (half on each side), 200 doubles the side (the right goes negative)
#[test]
fn the_channel_width_scales_the_side() {
    let dir = TempDir::new("desk");
    let mut s = session(&dir);
    let c = setup(&mut s, &dir);
    let (l0, _) = render(&s, &dir, "plain.wav");
    set(&mut s, c, "width", Value::real(0.0).expect("r"));
    let (l, r) = render(&s, &dir, "narrow.wav");
    assert!((peak(&l) - peak(&l0) / 2.0).abs() < 1.0e-3 && (peak(&r) - peak(&l0) / 2.0).abs() < 1.0e-3, "width 0");
    set(&mut s, c, "width", Value::real(200.0).expect("r"));
    let (l, r) = render(&s, &dir, "wide.wav");
    assert!((peak(&l) - peak(&l0) * 1.5).abs() < 1.0e-2 && (peak(&r) - peak(&l0) / 2.0).abs() < 1.0e-2, "width 200");
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

mod latency {
    //! A click instrument and an insert that delays by `LATENCY` samples and reports it
    use std::any::Any;

    use lm_base::Result;
    use lm_plugin::{BuildContext, Descriptor, PluginKind, Port, Ports, TestHints, Velocity};
    use lm_render::{EventKind, ProcessContext, Processor};
    use lm_state::{NodeSchema, Value};

    pub const LATENCY: usize = 100;

    struct Click;
    impl Processor for Click {
        fn process(&mut self, cx: &mut ProcessContext<'_>) {
            for e in cx.events {
                if let EventKind::NoteOn { .. } = e.kind {
                    for ch in 0..cx.buffers.outputs() {
                        cx.buffers.output(ch)[e.offset as usize] = 1.0;
                    }
                }
            }
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    struct Late {
        memory: Vec<Vec<f32>>,
        pos: usize,
    }
    impl Processor for Late {
        fn process(&mut self, cx: &mut ProcessContext<'_>) {
            let start = self.pos;
            for ch in 0..cx.buffers.outputs() {
                let mut pos = start;
                let mem = &mut self.memory[ch];
                for x in cx.buffers.in_out(ch).iter_mut() {
                    std::mem::swap(&mut mem[pos], x);
                    pos = (pos + 1) % LATENCY;
                }
                self.pos = pos;
            }
        }
        fn latency(&self) -> u32 {
            LATENCY as u32
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    fn click(_: &Value, _: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
        Ok(Box::new(Click))
    }
    fn late(_: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
        Ok(Box::new(Late { memory: vec![vec![0.0; LATENCY]; usize::from(cx.channels).max(1)], pos: 0 }))
    }

    pub static CLICK: Descriptor = Descriptor {
        id: "click",
        name: "plugin.click.name",
        kind: PluginKind::Instrument,
        schemas: || vec![NodeSchema::new("plugin:click", 1)],
        ports: |_| Ports { outputs: vec![Port::new("main", 2)], sidechains: Vec::new() },
        velocity: Velocity::Ignored,
        display: None,
        build: click,
        tests: TestHints::new(),
    };
    pub static LATE: Descriptor = Descriptor {
        id: "late",
        name: "plugin.late.name",
        kind: PluginKind::Effect,
        schemas: || vec![NodeSchema::new("plugin:late", 1)],
        ports: |_| Ports::default(),
        velocity: Velocity::Ignored,
        display: None,
        build: late,
        tests: TestHints::new(),
    };
}

#[test]
fn the_dry_side_is_delayed_by_the_inserts_latency() {
    let dir = TempDir::new("wetdry-latency");
    let mut config = config(&dir);
    config.registry = lm_plugin::Registry::new(&[&latency::CLICK, &latency::LATE, &lm_plugins_testsine::DESCRIPTOR])
        .expect("registry");
    let mut s = Session::new(config).expect("session");
    s.create(dir.join("lat")).expect("create");
    let mut ch = None;
    let mut track = None;
    s.document_mut()
        .transact(Label::new("test"), |e| {
            e.set(ROOT, &[PathStep::key("mixer"), PathStep::key("mode")], Value::text("digital"))?;
            let late = Value::object([
                ("plugin", Value::text("late")),
                ("version", Value::int(1)),
                ("wet", Value::real(0.5)?),
            ]);
            let c = e.insert(ROOT, "mixer.channels", 0, Value::object([("inserts", Value::list([late]))]))?;
            let inst = Value::object([("plugin", Value::text("click")), ("version", Value::int(1))]);
            let t = e.insert(ROOT, "tracks", 0, Value::object([("instrument", inst)]))?;
            let it =
                e.insert(t, "items", 0, Value::object([("kind", Value::text("midi")), ("length", Value::int(960))]))?;
            e.add_note(
                it,
                Value::object([("key", Value::int(60)), ("start", Value::int(480)), ("length", Value::int(240))]),
            )?;
            ch = Some(c);
            track = Some(t);
            Ok(())
        })
        .expect("setup");
    let (c, t) = (ch.expect("c"), track.expect("t"));
    s.document_mut()
        .transact(Label::new("route"), |e| {
            let out = e.find(t).and_then(|n| n.get("outputs").and_then(|l| l.at(0)).and_then(|o| o.get("id").cloned()));
            let out = NodeId::parse(out.and_then(|v| v.as_text().map(str::to_owned)).unwrap_or_default().as_str())?;
            e.set(out, &[PathStep::key("to")], Value::text(c.to_string()))
        })
        .expect("route");
    settle(&mut s);
    let (l, _) = render(&s, &dir, "lat.wav");
    let hits: Vec<(usize, f32)> =
        l.iter().enumerate().filter(|(_, v)| v.abs() > 1.0e-6).map(|(i, v)| (i, *v)).collect();
    // One click of 0.5 + 0.5 at one sample: the dry half waited exactly as long as the wet half
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!((hits[0].1 - 1.0).abs() < 1.0e-6, "{hits:?}");
}

#[test]
fn a_resource_of_another_kind_reaches_its_plugin_as_bytes() {
    // AmpAndIr's `nam` (lm-io's kinds, U24): imported into state/, loaded as bytes in an operation, given to the
    // build, and to the export
    let dir = TempDir::new("kinds");
    let mut config = common::config(&dir);
    let mut all: Vec<&'static lm_plugin::Descriptor> =
        vec![&lm_plugins_testsine::DESCRIPTOR, &lm_plugins_ampandir::DESCRIPTOR];
    all.extend(lm_plugins_effects::ALL.iter().copied());
    config.registry = lm_plugin::Registry::new(&all).expect("registry");
    let mut s = Session::new(config).expect("session");
    let c = setup(&mut s, &dir);
    let model =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../lm-plugins-ampandir/ab/data/wavenet-48000.nam");
    let imported = lm_io::import_file_as(&model, s.project().folder.root(), "nam", &Cancel::new(), &Progress::new())
        .expect("import");
    let nam = lm_state::Resource::new("nam", &imported.key).expect("resource");
    let mut insert = None;
    s.document_mut()
        .transact(Label::new("test.insert"), |e| {
            let state = Value::object([("nam", Value::NULL)]);
            let p = Value::object([("plugin", Value::text("ampandir")), ("version", Value::int(1)), ("state", state)]);
            insert = Some(e.insert(c, "inserts", 0, p)?);
            Ok(())
        })
        .expect("insert");
    settle(&mut s);
    let p = insert.expect("p");
    let (plain, _) = render(&s, &dir, "no-model.wav");
    s.document_mut()
        .transact(Label::new("test.model"), |e| {
            e.set(p, &[PathStep::key("state"), PathStep::key("nam")], Value::resource(nam.clone()))
        })
        .expect("model");
    settle(&mut s);
    assert_eq!(s.resource_state_of(&nam), lm_io::ResourceState::Ready);
    assert_eq!(s.loading(&lm_port::Target::Node(p)), lm_port::Loading::Ready, "{:?}", s.notices_keys());
    let (amp, _) = render(&s, &dir, "model.wav");
    assert_eq!(plain.len(), amp.len());
    assert!(plain.iter().zip(&amp).any(|(a, b)| (a - b).abs() > 1.0e-3), "the model shapes the sound");
    // A model the project does not have: Missing, the action "locate file" (H17)
    let gone = lm_state::Resource::new("nam", &"0".repeat(lm_io::KEY_LEN)).expect("resource");
    s.document_mut()
        .transact(Label::new("test.gone"), |e| {
            e.set(p, &[PathStep::key("state"), PathStep::key("nam")], Value::resource(gone.clone()))
        })
        .expect("gone");
    settle(&mut s);
    assert_eq!(s.resource_state_of(&gone), lm_io::ResourceState::Missing);
    assert!(matches!(s.loading(&lm_port::Target::Node(p)), lm_port::Loading::Missing { .. }));
}
