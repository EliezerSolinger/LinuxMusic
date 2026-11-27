//! CLAP through the helper: the scan names the files without running plugin code, lists each file in a helper of
//! its own and probes each plugin; a gain plays exactly through the proxy with nothing allocated on the core's side,
//! its latency change (a restart on the plugin's main thread) reaches the core; a synth takes notes and gives back its
//! own parameter change and state flag; the state goes and comes back; a file that crashes its listing is blocked

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

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lm-plugin-host-clap-{}", std::process::id()));
        lm_sys_clap::build_test_plugin(&d).expect("the test plugins build");
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
    let r = scan(&config(), "clap", &[], &[], Duration::from_secs(20), &Cancel::new(), &Progress::new()).unwrap();
    assert!(r.blocked.is_empty(), "{:?}", r.blocked);
    r.plugins
}

fn request(p: &Scanned, params: &[(&str, f32)]) -> LoadRequest {
    LoadRequest {
        format: "clap".into(),
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
    let uris: Vec<&str> = p.iter().map(|p| p.uri.as_str()).collect();
    assert_eq!(uris, ["lm.test.gain", "lm.test.synth"]);
    let (gain, synth) = (&p[0], &p[1]);
    assert!(!gain.instrument && synth.instrument && synth.midi_in);
    assert_eq!((gain.inputs, gain.outputs, synth.inputs, synth.outputs), (2, 2, 0, 2));
    let keys: Vec<&str> = gain.params.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["p7", "p9"]);
    assert!(gain.params[1].integer && !gain.params[0].integer);
    assert!(gain.file.as_deref().is_some_and(|f| f.ends_with("lm_test.clap")));
}

#[test]
fn a_gain_plays_exactly_and_its_latency_change_reaches_the_core() {
    let p = scanned();
    let (mut proxy, hosted) = start(&config(), &request(&p[0], &[("p7", 2.0)])).unwrap();
    assert_eq!(proxy.latency(), 0);
    let mut r = Runner::effect();
    let start_frame = r.frame;
    let out = r.block(&mut proxy, ramp, &[], true);
    for (ch, o) in out.iter().enumerate() {
        for (k, y) in o.iter().enumerate() {
            assert_eq!(*y, ramp(start_frame + k as u64, ch) * 2.0);
        }
    }
    // The latency parameter: the plugin asks for a restart, done on its main thread; the core hears of it
    let set = [Event { offset: 0, kind: EventKind::Param { index: 1, value: 10.0 } }];
    r.block(&mut proxy, ramp, &set, true);
    let t = Instant::now();
    while proxy.latency() != 10 {
        assert!(t.elapsed() < Duration::from_secs(10), "the new latency never came");
        r.block(&mut proxy, ramp, &[], true);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(r.requests.contains(&PluginRequest::LatencyChanged));
    let start_frame = r.frame;
    let out = r.block(&mut proxy, ramp, &[], true);
    assert_eq!(out[0][20], ramp(start_frame + 10, 0) * 2.0);
    // The state, into a new helper
    let chunk = hosted.save_state(Duration::from_secs(5)).unwrap().expect("a state");
    let saved = hosted.saved(Duration::from_secs(5)).unwrap();
    assert!(saved.params.contains(&("p7".to_owned(), 2.0)));
    // CLAP saves with the plugin not held: the block right after still plays (never the silence of a held plugin)
    let start_frame = r.frame;
    let out = r.block(&mut proxy, ramp, &[], true);
    assert_eq!(out[0][20], ramp(start_frame + 10, 0) * 2.0, "playing after the save");
    drop(proxy);
    drop(hosted);
    let mut req = request(&p[0], &[]);
    req.chunk = Some(chunk);
    let (proxy, h) = start(&config(), &req).unwrap();
    assert_eq!(proxy.latency(), 10, "the restored latency");
    assert_eq!(h.ready.info.get("restored").and_then(lm_state::Value::as_bool), Some(true));
}

#[test]
fn a_synth_takes_notes_and_speaks_back() {
    let p = scanned();
    let (mut proxy, _hosted) = start(&config(), &request(&p[1], &[("p1", 0.25)])).unwrap();
    let mut r = Runner::instrument();
    let on = [Event { offset: 8, kind: EventKind::NoteOn { note: NoteId(3), channel: 0, key: 61, velocity: 1.0 } }];
    let out = r.block(&mut proxy, |_, _| 0.0, &on, true);
    assert!(out[0][..8].iter().all(|x| *x == 0.0) && out[0][8..].iter().all(|x| *x == 0.25));
    assert!(out[1].iter().all(|x| *x == 0.12), "the transport's tempo (120) reached the plugin");
    // Its own change of "last key" (parameter index 1) and its state flag
    assert!(r.requests.contains(&PluginRequest::ParamChanged { index: 1, value: 61.0 }), "{:?}", r.requests);
    assert!(r.requests.contains(&PluginRequest::StateChanged));
    let off = [Event { offset: 0, kind: EventKind::AllNotesRelease }];
    let out = r.block(&mut proxy, |_, _| 0.0, &off, true);
    assert!(out[0].iter().all(|x| *x == 0.0), "a wildcard note-off ends every note");
    let _ = proxy.latency();
}

#[test]
fn a_file_that_crashes_its_listing_is_blocked() {
    // A ".clap" that is no library: opening it fails in the listing helper; a library that aborts in its init would
    // be blocked the same way (the helper dies)
    let bad = std::env::temp_dir().join(format!("lm-bad-clap-{}", std::process::id()));
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("broken.clap"), b"not a library").unwrap();
    let mut cfg = config();
    cfg.paths = vec![bad.clone()];
    let r = scan(&cfg, "clap", &[], &[], Duration::from_secs(20), &Cancel::new(), &Progress::new()).unwrap();
    assert!(r.plugins.is_empty());
    assert_eq!(r.blocked.len(), 1);
    assert!(r.blocked[0].uri.starts_with("file:") && r.blocked[0].uri.ends_with("broken.clap"));
    // Next time it is skipped
    let again =
        scan(&cfg, "clap", &[], &[r.blocked[0].uri.clone()], Duration::from_secs(20), &Cancel::new(), &Progress::new())
            .unwrap();
    assert_eq!(again.skipped, [r.blocked[0].uri.clone()]);
    let _ = std::fs::remove_dir_all(&bad);
}
