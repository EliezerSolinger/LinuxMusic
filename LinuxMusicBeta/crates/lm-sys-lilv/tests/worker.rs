//! The LV2 worker: threaded and synchronous, and the worker thread joined before the instance is freed

mod common;
use common::bundle::{self, GAIN, port, report};

use lm_sys_lilv::{Instance, InstanceConfig, WorkerMode};
use std::time::{Duration, Instant};

fn gain(config: &InstanceConfig) -> Instance {
    let mut i = bundle::world().instantiate(GAIN, config).unwrap();
    i.activate();
    i
}

fn block(i: &mut Instance) {
    let input = [0.0_f32; 64];
    let mut out = [0.0_f32; 64];
    i.run(64, &[&input], &mut [&mut out]).unwrap();
}

/// Raises the trigger for one block (the plugin schedules on the rising edge) and lowers it for another
fn trigger(i: &mut Instance) {
    i.set_control(port::TRIGGER, 1.0).unwrap();
    block(i);
    i.set_control(port::TRIGGER, 0.0).unwrap();
    block(i);
}

/// Runs blocks until the plugin has received `n` responses (threaded mode delivers them after a later run)
fn wait_worked(i: &mut Instance, n: f32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while i.control(port::WORKED) != Some(n) {
        assert!(Instant::now() < deadline, "the worker did not answer (worked = {:?})", i.control(port::WORKED));
        std::thread::sleep(Duration::from_millis(1));
        block(i);
    }
}

fn threads_named(name: &str) -> usize {
    // The worker's thread name, as the system lists the process's threads (Linux only; on macOS the count is
    // checked through the join test below)
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return 0;
    };
    tasks
        .filter_map(|t| std::fs::read_to_string(t.ok()?.path().join("comm")).ok())
        .filter(|comm| comm.trim() == name)
        .count()
}

#[test]
fn threaded_work_comes_back_after_a_later_run() {
    let mut i = gain(&bundle::config());
    assert_eq!(i.control(port::SEED), Some(0.0));
    trigger(&mut i);
    wait_worked(&mut i, 1.0);
    assert_eq!(i.control(port::SEED), Some(20.0), "the response carried seed 1 * 17 + 3");
    trigger(&mut i);
    trigger(&mut i);
    wait_worked(&mut i, 3.0);
    assert_eq!(i.control(port::SEED), Some(54.0), "3 * 17 + 3, the last response wins");
}

#[test]
fn synchronous_work_comes_back_after_the_same_run() {
    let mut config = bundle::config();
    config.worker = WorkerMode::Synchronous;
    let mut i = gain(&config);
    // The plugin writes its output ports during run, so a response delivered after one run shows in the next
    i.set_control(port::TRIGGER, 1.0).unwrap();
    block(&mut i);
    block(&mut i);
    assert_eq!(i.control(port::WORKED), Some(1.0), "worked inside the first run, delivered right after it");
    assert_eq!(i.control(port::SEED), Some(20.0));
}

#[test]
fn the_worker_is_joined_before_the_instance_is_freed() {
    // The work takes 300 ms; the instance is dropped right after scheduling it. The plugin logs when its work
    // begins and ends and when its cleanup runs: the cleanup must come after the work ended
    let id = 4242;
    let mut i = gain(&bundle::config());
    i.set_control(port::REPORT, id as f32).unwrap();
    i.set_control(port::WORK_MS, 300.0).unwrap();
    trigger(&mut i);
    // Wait until the work has started, so that the drop really races a running work
    let deadline = Instant::now() + Duration::from_secs(10);
    while !report(id).iter().any(|l| l == "work-begin") {
        assert!(Instant::now() < deadline, "the work never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    let started = Instant::now();
    drop(i);
    assert!(started.elapsed() >= Duration::from_millis(200), "the drop waited for the work");
    assert_eq!(report(id), vec!["work-begin", "work-end", "cleanup"]);
}

#[test]
fn work_still_queued_is_worked_before_the_instance_is_freed() {
    // Two requests, each 100 ms: when the instance is dropped the second has not started yet. Both run (the plugin
    // may have handed over memory in them), and only then the cleanup
    let id = 4343;
    let mut i = gain(&bundle::config());
    i.set_control(port::REPORT, id as f32).unwrap();
    i.set_control(port::WORK_MS, 100.0).unwrap();
    trigger(&mut i);
    trigger(&mut i);
    drop(i);
    assert_eq!(report(id), vec!["work-begin", "work-end", "work-begin", "work-end", "cleanup"]);
}

#[test]
fn an_inactive_instance_with_a_worker_drops_cleanly() {
    let id = 4444;
    let mut i = bundle::world().instantiate(GAIN, &bundle::config()).unwrap();
    i.set_control(port::REPORT, id as f32).unwrap();
    drop(i);
    // Never ran: the plugin never saw the report id, and nothing was logged; the drop did not hang
    assert!(report(id).is_empty());
}

#[test]
fn each_instance_has_one_worker_thread_and_it_ends() {
    let before = threads_named("lv2-worker");
    let instances: Vec<Instance> = (0..3).map(|_| gain(&bundle::config())).collect();
    if cfg!(target_os = "linux") {
        assert_eq!(threads_named("lv2-worker"), before + 3);
    }
    drop(instances);
    if cfg!(target_os = "linux") {
        assert_eq!(threads_named("lv2-worker"), before, "joined, not left running");
    }
    let mut config = bundle::config();
    config.worker = WorkerMode::Synchronous;
    let synchronous = gain(&config);
    if cfg!(target_os = "linux") {
        assert_eq!(threads_named("lv2-worker"), before, "synchronous mode makes no thread");
    }
    drop(synchronous);
}

#[test]
fn a_full_response_ring_refuses_the_response() {
    // Rings of 1100 bytes: a response of the test plugin (1008 bytes and its 4-byte header) fits once. Three works
    // finish while no run drains the ring: the first response is kept, the next two are refused (the plugin's
    // `respond` returns "no space") and never delivered
    let mut config = bundle::config();
    config.worker_capacity = 1100;
    let mut i = gain(&config);
    i.set_control(port::WORK_MS, 30.0).unwrap();
    for _ in 0..3 {
        trigger(&mut i);
    }
    std::thread::sleep(Duration::from_millis(400));
    block(&mut i);
    block(&mut i);
    assert_eq!(i.control(port::WORKED), Some(1.0));
    assert_eq!(i.control(port::SEED), Some(20.0), "the first response, not a later one");
}
