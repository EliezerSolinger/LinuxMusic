//! State as an opaque chunk: save → restore → save is identical

mod common;
use common::bundle::{self, GAIN, STATELESS, port};

use lm_sys_lilv::{Instance, WorkerMode, chunk_keys};
use std::time::{Duration, Instant};

fn gain() -> Instance {
    let mut i = bundle::world().instantiate(GAIN, &bundle::config()).unwrap();
    i.activate();
    i
}

fn block(i: &mut Instance) {
    let input = [0.0_f32; 64];
    let mut out = [0.0_f32; 64];
    i.run(64, &[&input], &mut [&mut out]).unwrap();
}

/// Changes the plugin's state through its worker: `n` triggers, each answered (seed becomes n * 17 + 3)
fn change(i: &mut Instance, n: u32) {
    for _ in 0..n {
        i.set_control(port::TRIGGER, 1.0).unwrap();
        block(i);
        i.set_control(port::TRIGGER, 0.0).unwrap();
        block(i);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while i.control(port::WORKED) != Some(n as f32) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
        block(i);
    }
}

#[test]
fn save_restore_save_is_identical() {
    let mut a = gain();
    change(&mut a, 2);
    assert_eq!(a.control(port::SEED), Some(37.0));
    let chunk = a.save_state().unwrap().expect("the gain has a state interface");
    assert_eq!(chunk_keys(&chunk).unwrap(), vec!["urn:linuxmusic:test#blob", "urn:linuxmusic:test#seed"]);
    assert_eq!(a.save_state().unwrap().unwrap(), chunk, "the same state gives the same bytes");

    // A new instance (of a new world: the URIDs differ, the chunk holds URIs) takes the state back
    let mut b = gain();
    let fresh = b.save_state().unwrap().unwrap();
    assert_ne!(fresh, chunk);
    b.restore_state(&chunk).unwrap();
    block(&mut b);
    assert_eq!(b.control(port::SEED), Some(37.0));
    assert_eq!(b.save_state().unwrap().unwrap(), chunk, "save → restore → save is bit-identical");
    assert_eq!(b.control(port::RESTORED), Some(1.0), "the plugin's restore scheduled work, and it was answered");

    // Back to the fresh state too
    b.restore_state(&fresh).unwrap();
    assert_eq!(b.save_state().unwrap().unwrap(), fresh);
}

#[test]
fn restore_waits_for_running_work_and_no_older_response_lands_after_it() {
    let mut a = gain();
    change(&mut a, 1);
    let chunk = a.save_state().unwrap().unwrap(); // seed 20

    let mut b = gain();
    b.set_control(port::WORK_MS, 200.0).unwrap();
    // b's first work also gives seed 20; its second (seed 37) is the one left running during the restore
    change(&mut b, 1);
    b.set_control(port::TRIGGER, 1.0).unwrap();
    block(&mut b); // schedules the second work (seed 37), 200 ms long
    b.set_control(port::TRIGGER, 0.0).unwrap();
    // Restore while that work runs: the restore waits for it and delivers its response first, then restores
    let started = Instant::now();
    b.restore_state(&chunk).unwrap();
    assert!(started.elapsed() >= Duration::from_millis(100), "the restore waited for the worker");
    for _ in 0..5 {
        block(&mut b);
    }
    assert_eq!(b.control(port::SEED), Some(20.0), "the restored state, not the late response");
    assert_eq!(b.save_state().unwrap().unwrap(), chunk);
}

#[test]
fn synchronous_mode_restores_too() {
    let mut config = bundle::config();
    config.worker = WorkerMode::Synchronous;
    let world = bundle::world();
    let mut a = world.instantiate(GAIN, &config).unwrap();
    a.activate();
    change(&mut a, 3);
    let chunk = a.save_state().unwrap().unwrap();
    let mut b = world.instantiate(GAIN, &config).unwrap();
    b.restore_state(&chunk).unwrap(); // before activation: allowed (instantiation class)
    b.activate();
    block(&mut b);
    assert_eq!(b.control(port::SEED), Some(54.0));
    assert_eq!(b.save_state().unwrap().unwrap(), chunk);
}

#[test]
fn bad_chunks_are_refused_and_change_nothing() {
    let mut a = gain();
    change(&mut a, 1);
    let good = a.save_state().unwrap().unwrap();
    let key = |chunk: &[u8], i: &mut Instance| i.restore_state(chunk).unwrap_err().key();
    assert_eq!(key(b"", &mut a), "lilv.bad_chunk");
    assert_eq!(key(b"not a chunk at all", &mut a), "lilv.bad_chunk");
    assert_eq!(key(&good[..good.len() - 1], &mut a), "lilv.bad_chunk", "truncated");
    let mut longer = good.clone();
    longer.push(0);
    assert_eq!(key(&longer, &mut a), "lilv.bad_chunk", "trailing bytes");
    let mut version = good.clone();
    version[8] = 9;
    assert_eq!(key(&version, &mut a), "lilv.chunk_version");
    assert_eq!(a.save_state().unwrap().unwrap(), good, "the refused chunks changed nothing");
}

#[test]
fn an_empty_chunk_resets_to_the_defaults() {
    // An empty chunk is the encoding of no properties; the gain falls back to its defaults (seed 0)
    let mut empty = b"LMLV2ST\0".to_vec();
    empty.extend_from_slice(&1_u32.to_le_bytes());
    empty.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(chunk_keys(&empty).unwrap(), Vec::<String>::new());
    let mut a = gain();
    change(&mut a, 2);
    a.restore_state(&empty).unwrap();
    block(&mut a);
    assert_eq!(a.control(port::SEED), Some(0.0));
}

#[test]
fn a_plugin_without_state_takes_only_an_empty_chunk() {
    let world = bundle::world();
    let mut stateless = world.instantiate(STATELESS, &bundle::config()).unwrap();
    assert!(!stateless.has_state() && !stateless.has_worker());
    assert_eq!(stateless.save_state().unwrap(), None);
    let mut empty = b"LMLV2ST\0".to_vec();
    empty.extend_from_slice(&1_u32.to_le_bytes());
    empty.extend_from_slice(&0_u32.to_le_bytes());
    stateless.restore_state(&empty).unwrap();
    let mut a = gain();
    change(&mut a, 1);
    let chunk = a.save_state().unwrap().unwrap();
    assert_eq!(stateless.restore_state(&chunk).unwrap_err().key(), "lilv.no_state_interface");
}
