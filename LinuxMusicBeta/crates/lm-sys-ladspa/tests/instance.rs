//! The test plugins through the binding: listing, hint ranges and defaults, exact audio, the latency port, missing
//! channels, and no allocation in `process`

#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use lm_sys_ladspa::{Instance, Library};

fn plugin_file() -> PathBuf {
    static FILE: OnceLock<PathBuf> = OnceLock::new();
    FILE.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lm-sys-ladspa-{}", std::process::id()));
        lm_sys_ladspa::build_test_plugin(&dir).expect("the test plugins build")
    })
    .clone()
}

fn library() -> Arc<Library> {
    Library::open(&plugin_file()).expect("open")
}

const BLOCK: usize = 64;

fn ramp() -> Vec<f32> {
    (0..BLOCK).map(|k| k as f32 / 100.0).collect()
}

#[test]
fn the_file_lists_its_plugins_with_their_hints() {
    let lib = library();
    let d = lib.descriptors();
    let labels: Vec<&str> = d.iter().map(|d| d.label.as_str()).collect();
    assert_eq!(labels, ["lm_gain", "lm_stereo_gain"]);
    assert_eq!((d[0].audio_inputs(), d[0].audio_outputs()), (1, 1));
    assert_eq!((d[1].audio_inputs(), d[1].audio_outputs()), (2, 2));
    assert!(d[0].hard_realtime);
    let controls: Vec<_> = d[0].controls().collect();
    assert_eq!(controls.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Gain", "Invert", "Steps"]);
    assert_eq!(controls[0].range(48_000.0), (Some(0.0), Some(4.0)));
    assert_eq!(controls[0].default(48_000.0), 1.0);
    assert!(controls[1].toggled() && controls[1].default(48_000.0) == 0.0);
    // DEFAULT_LOW of 0..8: a quarter of the way up
    assert!(controls[2].integer() && controls[2].default(48_000.0) == 2.0);
    assert!(d[1].latency_port().is_some() && d[0].latency_port().is_none());
    assert!(lm_sys_ladspa::find_files(&[plugin_file().parent().unwrap().to_path_buf()]).contains(&plugin_file()));
}

#[test]
fn a_mono_gain_plays_exactly_and_clamps_its_parameters() {
    let lib = library();
    let mut g = Instance::new(&lib, "lm_gain", 48_000.0, BLOCK).unwrap();
    assert_eq!(g.param_count(), 3);
    assert_eq!(g.param(0), Some(1.0));
    g.set_param(0, 2.0);
    let input = ramp();
    let mut out = vec![9.0f32; BLOCK];
    {
        let ins: [&[f32]; 1] = [&input];
        let mut outs: [&mut [f32]; 1] = [&mut out];
        assert_no_alloc::assert_no_alloc(|| g.process(BLOCK, &ins, &mut outs));
    }
    assert!(out.iter().zip(&input).all(|(y, x)| *y == x * 2.0));
    g.set_param(0, 100.0);
    assert_eq!(g.param(0), Some(4.0), "clamped to the upper bound");
    g.set_param(1, 0.7);
    assert_eq!(g.param(1), Some(1.0), "a toggle rounds");
    g.set_param(0, 1.0);
    {
        let ins: [&[f32]; 1] = [&input];
        let mut outs: [&mut [f32]; 1] = [&mut out];
        g.process(BLOCK, &ins, &mut outs);
    }
    assert!(out.iter().zip(&input).all(|(y, x)| *y == -x));
    assert_eq!(g.latency(), 0);
}

#[test]
fn a_stereo_gain_reports_its_latency_and_takes_missing_channels() {
    let lib = library();
    let mut g = Instance::new(&lib, "lm_stereo_gain", 48_000.0, BLOCK).unwrap();
    assert_eq!(g.latency(), 0, "nothing before the first block");
    let l = ramp();
    let mut out_l = vec![9.0f32; BLOCK];
    let mut out_r = vec![9.0f32; BLOCK];
    {
        // Only the left channel in: the right is silence
        let ins: [&[f32]; 1] = [&l];
        let mut outs: [&mut [f32]; 2] = [&mut out_l, &mut out_r];
        g.process(BLOCK, &ins, &mut outs);
    }
    assert_eq!(out_l, l);
    assert!(out_r.iter().all(|x| *x == 0.0));
    assert_eq!(g.latency(), 5);
    // Only one output taken: the other is dropped without trouble
    let mut one = vec![9.0f32; BLOCK];
    {
        let ins: [&[f32]; 2] = [&l, &l];
        let mut outs: [&mut [f32]; 1] = [&mut one];
        g.process(BLOCK, &ins, &mut outs);
    }
    assert_eq!(one, l);
}

#[test]
fn a_file_that_is_no_plugin_and_an_unknown_label_are_errors() {
    let dir = std::env::temp_dir().join(format!("lm-bad-ladspa-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("broken.so");
    std::fs::write(&bad, b"not a library").unwrap();
    assert!(Library::open(&bad).is_err());
    let _ = std::fs::remove_dir_all(&dir);
    assert!(Instance::new(&library(), "nope", 48_000.0, BLOCK).is_err());
}
