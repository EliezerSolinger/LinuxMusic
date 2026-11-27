//! VST3 through the helper: the scan finds the `.vst3`, lists it in a helper of its own and probes its plugins; a
//! gain with a separate controller plays exactly through the proxy with nothing allocated on the core's side, and its
//! latency change (the controller learns the value at the helper's main-thread turn, tells the processor and asks
//! for a restart) reaches the core; a synth takes notes, the transport and a MIDI controller through its mapping,
//! and gives back its own output parameter; the state goes into a new helper

#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use common::{BLOCK, RATE, Runner, ramp};
use lm_base::{Cancel, Progress};
use lm_plugin_proxy::{Deadline, HostConfig, LoadRequest, Scanned, scan, start};
use lm_render::{Event, EventKind, NoteId, PluginRequest, Processor};

/// A folder with the test plugins (the `lm-sys-vst3-test` shared library, built next to this test) as a `.vst3`
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let exe = std::env::current_exe().unwrap();
        let name = if cfg!(target_os = "macos") { "liblm_sys_vst3_test.dylib" } else { "liblm_sys_vst3_test.so" };
        let lib = exe.parent().unwrap().join(name);
        let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lm-plugin-host-vst3-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::copy(&lib, d.join("lm_test.vst3")).expect("the test plugins were built");
        d
    })
    .clone()
}

fn config() -> HostConfig {
    HostConfig {
        binary: PathBuf::from(env!("CARGO_BIN_EXE_lm-plugin-host")),
        deadline: Deadline::Wait,
        load_timeout: Duration::from_secs(20),
        paths: vec![dir()],
        format_paths: Default::default(),
        system_paths: false,
    }
}

fn scanned() -> Vec<Scanned> {
    let r = scan(&config(), "vst3", &[], &[], Duration::from_secs(20), &Cancel::new(), &Progress::new()).unwrap();
    assert!(r.blocked.is_empty(), "{:?}", r.blocked);
    r.plugins
}

fn request(p: &Scanned, params: &[(&str, f32)]) -> LoadRequest {
    LoadRequest {
        format: "vst3".into(),
        uri: p.uri.clone(),
        file: p.file.clone(),
        rate: RATE,
        max_block: BLOCK,
        inputs: p.inputs,
        outputs: p.outputs,
        sidechain: 0,
        chunk: None,
        params: params.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
    }
}

#[test]
fn the_scan_lists_the_file_and_probes_its_plugins() {
    let p = scanned();
    let names: Vec<&str> = p.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Test gain", "Test synth"]);
    let (gain, synth) = (&p[0], &p[1]);
    assert_eq!(gain.uri, lm_sys_vst3::cid_hex(&lm_sys_vst3::testing::GAIN));
    assert!(!gain.instrument && synth.instrument && synth.midi_in);
    assert_eq!((gain.inputs, gain.outputs, synth.inputs, synth.outputs), (2, 2, 0, 2));
    let keys: Vec<&str> = gain.params.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["p7", "p9"]);
    assert!(gain.params.iter().all(|p| p.min == 0.0 && p.max == 1.0), "normalized values");
}

#[test]
fn a_gain_plays_exactly_and_its_latency_change_reaches_the_core() {
    let p = scanned();
    let (mut proxy, hosted) = start(&config(), &request(&p[0], &[("p7", 0.5)])).unwrap();
    let mut r = Runner::effect();
    let first = r.frame;
    let out = r.block(&mut proxy, ramp, &[], true);
    for (ch, o) in out.iter().enumerate() {
        for (k, y) in o.iter().enumerate() {
            assert_eq!(*y, ramp(first + k as u64, ch) * 2.0);
        }
    }
    let set = [Event { offset: 0, kind: EventKind::Param { index: 1, value: 0.01 } }];
    r.block(&mut proxy, ramp, &set, true);
    let t = Instant::now();
    while proxy.latency() != 10 {
        assert!(t.elapsed() < Duration::from_secs(10), "the new latency never came");
        r.block(&mut proxy, ramp, &[], true);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(r.requests.contains(&PluginRequest::LatencyChanged));
    let chunk = hosted.save_state(Duration::from_secs(5)).unwrap().expect("a state");
    drop(proxy);
    drop(hosted);
    let mut req = request(&p[0], &[]);
    req.chunk = Some(chunk);
    let (proxy, h) = start(&config(), &req).unwrap();
    assert_eq!(proxy.latency(), 10, "the restored latency");
    let saved = h.saved(Duration::from_secs(5)).unwrap();
    assert!(saved.params.contains(&("p7".to_owned(), 0.5)), "{:?}", saved.params);
}

#[test]
fn a_synth_takes_notes_the_transport_and_a_controller() {
    let p = scanned();
    let (mut proxy, _hosted) = start(&config(), &request(&p[1], &[("p1", 0.25)])).unwrap();
    let mut r = Runner::instrument();
    let on = [Event { offset: 8, kind: EventKind::NoteOn { note: NoteId(3), channel: 0, key: 64, velocity: 1.0 } }];
    let out = r.block(&mut proxy, |_, _| 0.0, &on, true);
    assert!(out[0][..8].iter().all(|x| *x == 0.0) && out[0][8..].iter().all(|x| *x == 0.25));
    assert!(out[1].iter().all(|x| *x == 0.12), "the transport's tempo (120) reached the plugin");
    assert!(
        r.requests.contains(&PluginRequest::ParamChanged { index: 1, value: (64.0f64 / 127.0) as f32 }),
        "{:?}",
        r.requests
    );
    let cc = [Event { offset: 0, kind: EventKind::Midi { bytes: [0xB0, 7, 127] } }];
    let out = r.block(&mut proxy, |_, _| 0.0, &cc, true);
    assert!(out[0].iter().all(|x| *x == 1.0), "controller 7 is the level, through the plugin's mapping");
    let off = [Event { offset: 0, kind: EventKind::AllSoundOff }];
    let out = r.block(&mut proxy, |_, _| 0.0, &off, true);
    assert!(out[0].iter().all(|x| *x == 0.0));
    let _ = proxy.latency();
}
