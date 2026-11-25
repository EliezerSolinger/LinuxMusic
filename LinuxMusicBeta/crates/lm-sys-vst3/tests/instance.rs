//! The test plugins through the binding: the classes, the parameters, exact audio, a latency change through the
//! separate controller (a host message to the processor and a restart), notes, note expressions, the transport, the
//! processor's own output parameter, a MIDI controller through the mapping, the state round trip, and no allocation
//! in `process`

#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

use std::path::PathBuf;
use std::sync::Arc;

use lm_sys_vst3::{Event, Expression, Instance, Module, Transport, cid_hex};

/// The shared library `cargo test` built from `examples/test_vst3.rs`, next to this test's own directory
fn library() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().parent().unwrap().join("examples");
    let name = if cfg!(target_os = "macos") { "libtest_vst3.dylib" } else { "libtest_vst3.so" };
    dir.join(name)
}

fn module() -> Arc<Module> {
    Module::open(&library()).expect("the test plugins open")
}

const BLOCK: usize = 64;

fn block(inst: &mut Instance, inputs: &[Vec<f32>], events: &[Event], t: Option<&Transport>) -> Vec<Vec<f32>> {
    let mut outs = vec![vec![9.0f32; BLOCK]; 2];
    let ins: Vec<&[f32]> = inputs.iter().map(Vec::as_slice).collect();
    let mut o: Vec<&mut [f32]> = outs.iter_mut().map(Vec::as_mut_slice).collect();
    assert!(assert_no_alloc::assert_no_alloc(|| inst.process(BLOCK, &ins, &mut o, events, t)));
    outs
}

fn ramp(start: usize) -> Vec<Vec<f32>> {
    let l: Vec<f32> = (0..BLOCK).map(|k| (start + k) as f32 / 100.0).collect();
    let r: Vec<f32> = l.iter().map(|x| -x).collect();
    vec![l, r]
}

#[test]
fn the_module_lists_its_audio_classes() {
    let m = module();
    let c = m.classes();
    let names: Vec<&str> = c.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Test gain", "Test synth"], "the controller class is not a plugin");
    assert!(!c[0].is_instrument() && c[1].is_instrument());
    assert_eq!(c[0].cid, cid_hex(&lm_sys_vst3::testing::GAIN));
    // Found by its extension, without running it; a copy named .vst3 opens the same
    let dir = std::env::temp_dir().join(format!("lm-sys-vst3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("lm_test.vst3");
    std::fs::copy(library(), &copy).unwrap();
    assert_eq!(lm_sys_vst3::find_files(std::slice::from_ref(&dir)), [copy.as_path()]);
    assert_eq!(Module::open(&copy).unwrap().classes().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_gain_with_a_separate_controller() {
    let m = module();
    let mut g = Instance::new(&m, &cid_hex(&lm_sys_vst3::testing::GAIN), 48_000.0, BLOCK as u32).unwrap();
    let ids: Vec<(u32, &str, i32)> = g.params().iter().map(|p| (p.id, p.title.as_str(), p.steps)).collect();
    assert_eq!(ids, [(7, "Gain", 0), (9, "Latency", 1000)]);
    assert_eq!(g.audio_inputs()[0].channels, 2);
    assert!(!g.takes_notes());
    assert_eq!(g.latency(), 0);
    g.set_param(7, 0.5);
    let out = block(&mut g, &ramp(0), &[], None);
    let input = ramp(0);
    for (o, i) in out.iter().zip(&input) {
        assert!(o.iter().zip(i).all(|(y, x)| *y == x * 2.0));
    }
    // The controller learns the host's values at the main thread's turn
    g.idle();
    assert_eq!(g.param_value(7), Some(0.5));
    // A latency change: the controller tells the processor (a host message) and asks for a restart
    block(&mut g, &ramp(BLOCK), &[Event::Param { time: 0, id: 9, value: 0.01 }], None);
    g.idle();
    assert!(g.restart_pending());
    let silent = block(&mut g, &ramp(2 * BLOCK), &[], None);
    assert!(silent[0].iter().all(|x| *x == 0.0), "silence while it restarts");
    assert!(g.idle(), "the latency changed");
    assert_eq!(g.latency(), 10);
    let out = block(&mut g, &ramp(0), &[], None);
    assert!(out[0][..10].iter().all(|x| *x == 0.0));
    assert_eq!(out[0][10], ramp(0)[0][0] * 2.0);
    // The state into a new instance
    let chunk = g.save_state().unwrap();
    let mut h = Instance::new(&m, &cid_hex(&lm_sys_vst3::testing::GAIN), 48_000.0, BLOCK as u32).unwrap();
    h.load_state(&chunk).unwrap();
    assert_eq!(h.param_value(7), Some(0.5), "the controller learned the component's state");
    assert_eq!(h.save_state().unwrap(), chunk);
    assert!(h.load_state(b"nonsense").is_err());
}

#[test]
fn a_synth_takes_notes_expressions_controllers_and_the_transport() {
    let m = module();
    let mut s = Instance::new(&m, &cid_hex(&lm_sys_vst3::testing::SYNTH), 48_000.0, BLOCK as u32).unwrap();
    assert!(s.takes_notes());
    assert!(s.audio_inputs().is_empty());
    let on = Event::NoteOn { time: 16, note_id: 1, channel: 0, key: 64, velocity: 1.0 };
    let t = Transport {
        playing: true,
        tempo: 120.0,
        beats: 4.0,
        samples: 96_000,
        signature: Some((4, 4)),
        ..Transport::default()
    };
    let out = block(&mut s, &[], &[on], Some(&t));
    assert!(out[0][..16].iter().all(|x| *x == 0.0));
    assert!(out[0][16..].iter().all(|x| *x == 0.5));
    assert!(out[1].iter().all(|x| *x == 0.12), "the tempo reached the plugin");
    assert_eq!(s.changes(), [(2, 64.0 / 127.0)], "its own output parameter");
    let vol = Event::Expression { time: 0, kind: Expression::Volume, note_id: 1, value: 0.5 };
    let out = block(&mut s, &[], &[vol], None);
    assert!(out[0].iter().all(|x| *x == 1.0));
    assert!(out[1].iter().all(|x| *x == 0.0), "no transport");
    // Controller 7 is "Level" through the plugin's MIDI mapping
    let cc = Event::Midi { time: 0, data: [0xB0, 7, 127] };
    let out = block(&mut s, &[], &[cc], None);
    assert!(out[0].iter().all(|x| *x == 2.0));
    let off = Event::ReleaseAll { time: 0 };
    let out = block(&mut s, &[], &[off], None);
    assert!(out[0].iter().all(|x| *x == 0.0), "every held note released");
}

#[test]
fn an_unknown_class_and_a_missing_file_are_errors() {
    let m = module();
    assert_eq!(Instance::new(&m, &"0".repeat(32), 48_000.0, 64).unwrap_err().key(), "vst3.not_found");
    assert_eq!(Module::open(std::path::Path::new("/nonexistent/x.vst3")).unwrap_err().key(), "vst3.not_found");
}
