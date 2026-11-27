//! LADSPA through the helper: the scan names the files and lists each one, probes each plugin; a stereo gain plays
//! exactly through the proxy with its latency port heard by the core; a mono gain on a stereo channel runs as two
//! instances (dual mono) and a parameter change lands at its sample; a file that is no library is blocked

#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use common::{BLOCK, RATE, Runner, ramp};
use lm_base::{Cancel, Progress};
use lm_plugin_proxy::{Deadline, HostConfig, LoadRequest, Scanned, scan, start};
use lm_render::{Event, EventKind};

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d =
            PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lm-plugin-host-ladspa-{}", std::process::id()));
        lm_sys_ladspa::build_test_plugin(&d).expect("the test plugins build");
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
    let r = scan(&config(), "ladspa", &[], &[], Duration::from_secs(20), &Cancel::new(), &Progress::new()).unwrap();
    assert!(r.blocked.is_empty(), "{:?}", r.blocked);
    r.plugins
}

fn request(p: &Scanned, inputs: usize, outputs: usize, params: &[(&str, f32)]) -> LoadRequest {
    LoadRequest {
        format: "ladspa".into(),
        uri: p.uri.clone(),
        file: p.file.clone(),
        rate: RATE,
        max_block: BLOCK,
        inputs,
        outputs,
        sidechain: 0,
        chunk: None,
        params: params.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
    }
}

#[test]
fn the_scan_lists_the_file_and_its_parameters() {
    let p = scanned();
    let uris: Vec<&str> = p.iter().map(|p| p.uri.as_str()).collect();
    assert_eq!(uris, ["lm_test_ladspa:lm_gain", "lm_test_ladspa:lm_stereo_gain"]);
    let (mono, stereo) = (&p[0], &p[1]);
    assert!(!mono.instrument && !mono.midi_in);
    assert_eq!((mono.inputs, mono.outputs, stereo.inputs, stereo.outputs), (1, 1, 2, 2));
    let keys: Vec<&str> = mono.params.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["p2", "p3", "p4"]);
    assert_eq!((mono.params[0].min, mono.params[0].max, mono.params[0].default), (0.0, 4.0, 1.0));
    assert!(mono.params[1].toggled && mono.params[2].integer);
    assert!(stereo.latency && !mono.latency);
    assert!(mono.file.as_deref().is_some_and(|f| f.ends_with("lm_test_ladspa.so")));
}

#[test]
fn a_stereo_gain_plays_exactly_and_its_latency_port_reaches_the_core() {
    let p = scanned();
    let (mut proxy, _hosted) = start(&config(), &request(&p[1], 2, 2, &[("p4", 2.0)])).unwrap();
    let mut r = Runner::effect();
    let start_frame = r.frame;
    let out = r.block(&mut proxy, ramp, &[], true);
    for (ch, o) in out.iter().enumerate() {
        for (k, y) in o.iter().enumerate() {
            assert_eq!(*y, ramp(start_frame + k as u64, ch) * 2.0);
        }
    }
    // The port says 5 once a block has run; the next block carries it to the core
    let t = std::time::Instant::now();
    while lm_render::Processor::latency(&proxy) != 5 {
        assert!(t.elapsed() < Duration::from_secs(10), "the latency never came");
        r.block(&mut proxy, ramp, &[], true);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_mono_gain_on_a_stereo_channel_runs_twice_and_a_parameter_lands_at_its_sample() {
    let p = scanned();
    let (mut proxy, hosted) = start(&config(), &request(&p[0], 2, 2, &[("p2", 2.0)])).unwrap();
    let mut r = Runner::effect();
    let start_frame = r.frame;
    let set = [Event { offset: 100, kind: EventKind::Param { index: 0, value: 3.0 } }];
    let out = r.block(&mut proxy, ramp, &set, true);
    for (ch, o) in out.iter().enumerate() {
        for (k, y) in o.iter().enumerate() {
            let g = if k < 100 { 2.0 } else { 3.0 };
            assert_eq!(*y, ramp(start_frame + k as u64, ch) * g, "channel {ch}, frame {k}");
        }
    }
    // The state is only the parameters, which the core keeps: there is no chunk, the values come back by key
    assert_eq!(hosted.save_state(Duration::from_secs(5)).unwrap(), None);
    let saved = hosted.saved(Duration::from_secs(5)).unwrap();
    assert!(saved.params.contains(&("p2".to_owned(), 3.0)), "{:?}", saved.params);
}

#[test]
fn a_file_that_is_no_library_is_blocked() {
    let bad = std::env::temp_dir().join(format!("lm-bad-ladspa-host-{}", std::process::id()));
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("broken.so"), b"not a library").unwrap();
    let mut cfg = config();
    cfg.paths = vec![bad.clone()];
    let r = scan(&cfg, "ladspa", &[], &[], Duration::from_secs(20), &Cancel::new(), &Progress::new()).unwrap();
    assert!(r.plugins.is_empty());
    assert_eq!(r.blocked.len(), 1);
    assert!(r.blocked[0].uri.starts_with("file:") && r.blocked[0].uri.ends_with("broken.so"));
    let _ = std::fs::remove_dir_all(&bad);
}
