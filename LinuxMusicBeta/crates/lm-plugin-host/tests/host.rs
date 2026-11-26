//! The out-of-process path end to end (I-9.6): the core's proxy runs a plugin in an `lm-plugin-host` helper through
//! the shared block, with nothing allocated on the core's audio thread; a crash and a hang silence only that node and
//! are reported; the state goes and comes back; the scan blocks what crashes or hangs it; and an LV2 plugin.

#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use lm_base::{Cancel, Progress};
use lm_plugin_proxy::{Deadline, HostConfig, LoadRequest, scan, start};
use lm_render::{Event, EventKind, NoteId, PluginErrorKey, PluginRequest, Processor};

mod common;

use common::{BLOCK, RATE, Runner, ramp};

fn config() -> HostConfig {
    HostConfig {
        binary: PathBuf::from(env!("CARGO_BIN_EXE_lm-plugin-host")),
        deadline: Deadline::Live(0.6),
        load_timeout: Duration::from_secs(20),
        paths: Vec::new(),
        system_paths: true,
    }
}

fn request(uri: &str, inputs: usize, params: &[(&str, f32)]) -> LoadRequest {
    LoadRequest {
        format: "test".into(),
        uri: uri.into(),
        file: None,
        rate: RATE,
        max_block: BLOCK,
        inputs,
        outputs: 2,
        sidechain: 0,
        chunk: None,
        params: params.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
    }
}

#[test]
fn an_effect_plays_through_the_helper_without_allocating() {
    // Exact, so without a deadline (the export's mode): the test machine runs the other tests meanwhile
    let mut cfg = config();
    cfg.deadline = Deadline::Wait;
    let (mut proxy, hosted) = start(&cfg, &request("gain", 2, &[("gain", 0.5), ("latency", 100.0)])).unwrap();
    assert_eq!(proxy.latency(), 100, "the latency the plugin reports, for the PDC");
    let mut r = Runner::effect();
    let mut all = [Vec::new(), Vec::new()];
    for _ in 0..200 {
        let out = r.block(&mut proxy, ramp, &[], true);
        for ch in 0..2 {
            all[ch].extend(out[ch].iter().copied());
        }
    }
    // Exactly the input, halved, 100 samples late
    for (ch, chan) in all.iter().enumerate() {
        for (k, y) in chan.iter().enumerate() {
            let want = if k < 100 { 0.0 } else { ramp(k as u64 - 100, ch) * 0.5 };
            assert_eq!(*y, want, "channel {ch}, sample {k}");
        }
    }
    assert_eq!(hosted.missed(), 0);
    // A parameter as an event at its block
    let ev = [Event { offset: 0, kind: EventKind::Param { index: 0, value: 2.0 } }];
    r.block(&mut proxy, ramp, &ev, true);
    let out = r.block(&mut proxy, ramp, &[], true);
    let k = 128usize;
    let frame = r.frame - BLOCK as u64 + k as u64;
    assert_eq!(out[0][k], ramp(frame - 100, 0) * 2.0);
}

#[test]
fn live_every_block_is_the_plugin_s_or_silence() {
    // With the live deadline, a block is either the plugin's whole answer or silence, counted as missed
    let (mut proxy, hosted) = start(&config(), &request("gain", 2, &[("gain", 1.0)])).unwrap();
    let mut r = Runner::effect();
    let mut silent = 0;
    for _ in 0..400 {
        let start = r.frame;
        let out = r.block(&mut proxy, ramp, &[], true);
        if out[0].iter().all(|x| *x == 0.0) {
            silent += 1;
        } else {
            for (k, y) in out[0].iter().enumerate() {
                assert_eq!(*y, ramp(start + k as u64, 0));
            }
        }
    }
    assert_eq!(hosted.missed(), silent);
    // How many miss depends on the machine (a shared CI runner misses many): only the whole-or-silence rule and
    // that the plugin is heard at all are the contract
    assert!(silent < 400, "every block missed");
}

#[test]
fn an_instrument_takes_notes() {
    // Notes, not timing: no deadline, so a slow machine cannot turn the answer into a missed block
    let mut cfg = config();
    cfg.deadline = Deadline::Wait;
    let (mut proxy, _hosted) = start(&cfg, &request("synth", 0, &[("level", 0.25)])).unwrap();
    let mut r = Runner::instrument();
    let on = [Event { offset: 0, kind: EventKind::NoteOn { note: NoteId(1), channel: 0, key: 60, velocity: 1.0 } }];
    let out = r.block(&mut proxy, |_, _| 0.0, &on, true);
    assert!(out[0].iter().all(|x| *x == 0.25) && out[1].iter().all(|x| *x == 0.25));
    let off = [Event { offset: 0, kind: EventKind::NoteOff { note: NoteId(1), channel: 0, key: 60, velocity: 0.0 } }];
    let out = r.block(&mut proxy, |_, _| 0.0, &off, true);
    assert!(out[0].iter().all(|x| *x == 0.0));
}

#[test]
fn the_state_goes_and_comes_back() {
    let (mut proxy, hosted) = start(&config(), &request("gain", 2, &[("gain", 0.75), ("latency", 3.0)])).unwrap();
    let mut r = Runner::effect();
    r.block(&mut proxy, ramp, &[], false);
    let chunk = hosted.save_state(Duration::from_secs(5)).unwrap().expect("a state");
    drop(proxy);
    drop(hosted);
    let mut req = request("gain", 2, &[]);
    req.chunk = Some(chunk);
    let (proxy, _h) = start(&config(), &req).unwrap();
    assert_eq!(proxy.latency(), 3, "the restored state's latency");
}

fn wait_until(what: &str, f: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < end, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_crash_silences_only_this_node_and_is_reported() {
    let (mut proxy, hosted) = start(&config(), &request("crash", 2, &[("crash", 1.0)])).unwrap();
    let mut r = Runner::effect();
    // The first block makes the helper abort; the core goes on
    for _ in 0..10 {
        let out = r.block(&mut proxy, ramp, &[], true);
        assert!(out[0].iter().all(|x| *x == 0.0), "a dead plugin is silence");
    }
    wait_until("the crash is noticed", || hosted.is_dead());
    r.block(&mut proxy, ramp, &[], true);
    assert!(r.requests.contains(&PluginRequest::Error(PluginErrorKey("plugin_host.crashed"))), "{:?}", r.requests);
}

#[test]
fn a_hang_misses_its_deadline_then_is_killed() {
    let (mut proxy, hosted) = start(&config(), &request("hang", 2, &[("hang", 1.0)])).unwrap();
    let mut r = Runner::effect();
    let t = Instant::now();
    for _ in 0..(lm_plugin_proxy::HANG_BLOCKS + 4) {
        let b = Instant::now();
        let out = r.block(&mut proxy, ramp, &[], true);
        // Bounded by the deadline (60% of a 5.3 ms block), with a wide margin for a loaded test machine (the
        // scheduler may not run this thread for a while): a hung plugin never holds the block for long
        assert!(b.elapsed() < Duration::from_millis(250), "a block took {:?}", b.elapsed());
        assert!(out[0].iter().all(|x| *x == 0.0));
    }
    assert!(t.elapsed() < Duration::from_secs(5));
    assert!(r.requests.contains(&PluginRequest::Error(PluginErrorKey("plugin_host.hung"))), "{:?}", r.requests);
    wait_until("the hung helper is killed", || hosted.is_dead());
    assert!(hosted.missed() >= lm_plugin_proxy::HANG_BLOCKS);
}

#[test]
fn the_export_waits_without_a_deadline() {
    let mut cfg = config();
    cfg.deadline = Deadline::Wait;
    let (mut proxy, hosted) = start(&cfg, &request("gain", 2, &[("gain", 1.0)])).unwrap();
    let mut r = Runner::effect();
    for _ in 0..50 {
        r.block(&mut proxy, ramp, &[], true);
    }
    assert_eq!(hosted.missed(), 0);
}

#[test]
fn the_scan_blocks_what_crashes_or_hangs_it() {
    let report =
        scan(&config(), "test", &[], &["hang".to_owned()], Duration::from_secs(10), &Cancel::new(), &Progress::new())
            .unwrap();
    let ok: Vec<&str> = report.plugins.iter().map(|p| p.uri.as_str()).collect();
    assert_eq!(ok, ["gain", "synth", "crash", "mono"]);
    assert_eq!(report.skipped, ["hang"]);
    let blocked: Vec<(&str, &str)> = report.blocked.iter().map(|b| (b.uri.as_str(), b.reason.key())).collect();
    assert_eq!(blocked, [("bomb", "plugin_host.crashed"), ("stall", "plugin_host.timeout")]);
    let gain = &report.plugins[0];
    assert!(!gain.instrument && gain.latency && gain.params.len() == 2 && gain.params[1].integer);
    assert!(report.plugins[1].instrument && report.plugins[1].midi_in);
}
