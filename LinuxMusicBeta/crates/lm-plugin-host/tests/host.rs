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
        format_paths: Default::default(),
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
fn a_made_up_latency_is_limited_before_it_reaches_the_renderer() {
    // The test plugin reports whatever its parameter says: four billion frames would be a delay line of gigabytes
    let (proxy, _hosted) = start(&config(), &request("gain", 2, &[("latency", 4.0e9)])).unwrap();
    assert_eq!(proxy.latency(), lm_plugin_proxy::layout::MAX_LATENCY);
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
    assert_eq!(ok, ["gain", "synth", "crash", "mono", "knob"]);
    assert_eq!(report.skipped, ["hang"]);
    let blocked: Vec<(&str, &str)> = report.blocked.iter().map(|b| (b.uri.as_str(), b.reason.key())).collect();
    assert_eq!(blocked, [("bomb", "plugin_host.crashed"), ("stall", "plugin_host.timeout")]);
    let gain = &report.plugins[0];
    assert!(!gain.instrument && gain.latency && gain.params.len() == 2 && gain.params[1].integer);
    assert!(report.plugins[1].instrument && report.plugins[1].midi_in);
}

#[test]
fn a_plugin_window_is_shown_resized_and_its_gestures_come_back() {
    use lm_plugin_proxy::WindowCommand;
    let mut cfg = config();
    cfg.deadline = Deadline::Wait;
    let (mut proxy, hosted) = start(&cfg, &request("knob", 2, &[])).unwrap();
    let t = Duration::from_secs(5);
    assert!(!hosted.window_is_open());
    let shown = hosted.window(WindowCommand::Show, t).unwrap();
    assert!(shown.open && shown.size == Some((300, 150)) && shown.reason.is_none());
    assert!(hosted.window_is_open());
    // The user turns the knob (the test plugin does it on a resize): begin, the value, end, at the next block
    let turned = hosted.window(WindowCommand::Resize(500, 100), t).unwrap();
    assert_eq!(turned.size, Some((500, 100)));
    let mut r = Runner::effect();
    for _ in 0..4 {
        r.block(&mut proxy, ramp, &[], false);
    }
    let at = |q: PluginRequest| r.requests.iter().position(|x| *x == q);
    let begin = at(PluginRequest::GestureBegin { index: 0 }).expect("a gesture began");
    let end = at(PluginRequest::GestureEnd { index: 0 }).expect("a gesture ended");
    let set = at(PluginRequest::ParamChanged { index: 0, value: 0.5 }).expect("the knob's value");
    assert!(begin != end && set != begin);
    // Hidden, and a plugin with no window says why
    assert!(!hosted.window(WindowCommand::Hide, t).unwrap().open);
    assert!(!hosted.window_is_open());
    let (_, plain) = start(&cfg, &request("gain", 2, &[])).unwrap();
    let none = plain.window(WindowCommand::Show, t).unwrap();
    assert!(!none.open && none.reason.as_deref() == Some("plugin.window.none"));
}

#[test]
fn instances_of_a_group_share_one_helper_process() {
    use lm_plugin_proxy::{Group, start_in};
    let mut cfg = config();
    cfg.deadline = Deadline::Wait;
    let group = Group::new();
    let (mut a, hosted_a) = start_in(&cfg, &request("gain", 2, &[("gain", 0.5)]), Some(&group)).unwrap();
    let (mut b, hosted_b) = start_in(&cfg, &request("gain", 2, &[("gain", 2.0)]), Some(&group)).unwrap();
    let (mut ra, mut rb) = (Runner::effect(), Runner::effect());
    // Each instance has its own parameters and its own audio
    for _ in 0..4 {
        let out_a = ra.block(&mut a, ramp, &[], false);
        let out_b = rb.block(&mut b, ramp, &[], false);
        let start = ra.frame - BLOCK as u64;
        assert_eq!(out_a[0][10], ramp(start + 10, 0) * 0.5);
        assert_eq!(out_b[0][10], ramp(start + 10, 0) * 2.0);
    }
    // Each has its own state, answered to its own request
    assert_eq!(hosted_a.save_state(Duration::from_secs(5)).unwrap().map(|c| c.len()), Some(8));
    assert_eq!(hosted_b.save_state(Duration::from_secs(5)).unwrap().map(|c| c.len()), Some(8));
    // One instance leaves: the other goes on in the same helper
    drop(a);
    drop(hosted_a);
    let out_b = rb.block(&mut b, ramp, &[], false);
    let start = rb.frame - BLOCK as u64;
    assert_eq!(out_b[0][10], ramp(start + 10, 0) * 2.0);
    assert!(!hosted_b.is_dead());
    // A new instance of the group joins the running helper
    let (mut c, _hosted_c) = start_in(&cfg, &request("gain", 2, &[("gain", 1.0)]), Some(&group)).unwrap();
    let mut rc = Runner::effect();
    let out_c = rc.block(&mut c, ramp, &[], false);
    assert_eq!(out_c[0][10], ramp(10, 0));
}
