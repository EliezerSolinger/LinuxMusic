//! The test plugins through the binding: listing, parameters, exact audio, latency and restart, notes and
//! expressions, transport, the plugin's own parameter changes and state flag, state round trip, and no allocation
//! in `process`

#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use lm_sys_clap::{Event, Instance, Library, ThreadRole, Transport, set_thread_role};

fn plugin_file() -> PathBuf {
    static FILE: OnceLock<PathBuf> = OnceLock::new();
    FILE.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lm-sys-clap-{}", std::process::id()));
        lm_sys_clap::build_test_plugin(&dir).expect("the test plugins build")
    })
    .clone()
}

fn library() -> Arc<Library> {
    Library::open(&plugin_file()).expect("open")
}

const BLOCK: usize = 64;

/// One block as the audio thread, under the allocation trap
fn block(
    inst: &mut Instance,
    inputs: &[Vec<f32>],
    events: &[Event],
    transport: Option<&Transport>,
) -> (Vec<Vec<f32>>, i32) {
    let mut outs = vec![vec![9.0f32; BLOCK]; 2];
    let ins: Vec<&[f32]> = inputs.iter().map(Vec::as_slice).collect();
    let mut o: Vec<&mut [f32]> = outs.iter_mut().map(Vec::as_mut_slice).collect();
    set_thread_role(ThreadRole::Audio);
    let status = assert_no_alloc::assert_no_alloc(|| inst.process(BLOCK, &ins, &mut o, events, transport));
    set_thread_role(ThreadRole::Main);
    (outs, status)
}

fn ramp(start: usize) -> Vec<Vec<f32>> {
    let l: Vec<f32> = (0..BLOCK).map(|k| (start + k) as f32 / 100.0).collect();
    let r: Vec<f32> = l.iter().map(|x| -x).collect();
    vec![l, r]
}

#[test]
fn the_file_lists_its_plugins() {
    let lib = library();
    let d = lib.descriptors();
    let ids: Vec<&str> = d.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, ["lm.test.gain", "lm.test.synth"]);
    assert!(!d[0].is_instrument() && d[1].is_instrument());
    assert_eq!(d[1].name, "Test synth");
    assert!(lm_sys_clap::find_files(&[plugin_file().parent().unwrap().to_path_buf()]).contains(&plugin_file()));
}

#[test]
fn a_gain_with_parameters_latency_and_a_restart() {
    let lib = library();
    let mut g = Instance::new(&lib, "lm.test.gain", 48_000.0, BLOCK as u32).unwrap();
    let keys: Vec<(u32, &str)> = g.params().iter().map(|p| (p.id, p.name.as_str())).collect();
    assert_eq!(keys, [(7, "Gain"), (9, "Latency")]);
    assert!(g.params()[1].stepped);
    assert_eq!(g.audio_inputs()[0].channels, 2);
    assert!(!g.takes_notes());
    assert_eq!(g.latency(), 0);
    g.set_param(7, 2.0);
    let (out, status) = block(&mut g, &ramp(0), &[], None);
    assert_eq!(status, 1);
    let input = ramp(0);
    for (o, i) in out.iter().zip(&input) {
        assert!(o.iter().zip(i).all(|(y, x)| *y == x * 2.0));
    }
    assert_eq!(g.param_value(7), Some(2.0));
    // A latency change asks for a restart: the audio side stops at its next block, the main thread restarts it
    block(&mut g, &ramp(BLOCK), &[Event::Param { time: 0, id: 9, value: 10.0 }], None);
    assert!(g.restart_pending());
    let (silent, _) = block(&mut g, &ramp(2 * BLOCK), &[], None);
    assert!(silent[0].iter().all(|x| *x == 0.0), "silence while it restarts");
    assert!(g.idle(), "the latency changed");
    assert_eq!(g.latency(), 10);
    let (out, _) = block(&mut g, &ramp(0), &[], None);
    assert!(out[0][..10].iter().all(|x| *x == 0.0));
    assert_eq!(out[0][10], ramp(0)[0][0] * 2.0);
    // The state: saved, loaded into a new instance
    let chunk = g.save_state().unwrap().expect("a state");
    let mut h = Instance::new(&lib, "lm.test.gain", 48_000.0, BLOCK as u32).unwrap();
    h.load_state(&chunk).unwrap();
    assert_eq!(h.param_value(7), Some(2.0));
    assert_eq!(h.save_state().unwrap().unwrap(), chunk);
    assert!(h.load_state(&[1, 2, 3]).is_err(), "a short state is refused");
}

#[test]
fn a_synth_takes_notes_expressions_and_the_transport() {
    let lib = library();
    let mut s = Instance::new(&lib, "lm.test.synth", 48_000.0, BLOCK as u32).unwrap();
    assert!(s.takes_notes());
    assert!(s.audio_inputs().is_empty());
    let on = Event::NoteOn { time: 16, note_id: 1, channel: 0, key: 64, velocity: 1.0 };
    let t = Transport { playing: true, tempo: 120.0, beats: 4.0, seconds: 2.0, ..Transport::default() };
    let (out, _) = block(&mut s, &[], &[on], Some(&t));
    assert!(out[0][..16].iter().all(|x| *x == 0.0));
    assert!(out[0][16..].iter().all(|x| *x == 0.5));
    assert!(out[1].iter().all(|x| *x == 0.12), "the tempo reached the plugin");
    // Its own parameter change and its state flag
    assert_eq!(s.changes(), [(2, 64.0)]);
    assert!(s.take_state_dirty());
    assert!(!s.take_state_dirty());
    let vol = Event::Expression {
        time: 0,
        expression: lm_sys_clap::CLAP_NOTE_EXPRESSION_VOLUME,
        note_id: 1,
        channel: 0,
        key: 64,
        value: 2.0,
    };
    let (out, _) = block(&mut s, &[], &[vol], None);
    assert!(out[0].iter().all(|x| *x == 1.0));
    assert!(out[1].iter().all(|x| *x == 0.0), "no transport");
    assert!(s.changes().is_empty());
    let off = Event::NoteOff { time: 0, note_id: 1, channel: 0, key: 64, velocity: 0.0 };
    let (out, _) = block(&mut s, &[], &[off], None);
    assert!(out[0].iter().all(|x| *x == 0.0));
    assert_eq!(s.param_value(2), Some(64.0));
}

#[test]
fn an_unknown_plugin_and_a_missing_file_are_errors() {
    let lib = library();
    assert_eq!(Instance::new(&lib, "lm.test.none", 48_000.0, 64).unwrap_err().key(), "clap.not_found");
    assert_eq!(Library::open(std::path::Path::new("/nonexistent/x.clap")).unwrap_err().key(), "clap.not_found");
}
