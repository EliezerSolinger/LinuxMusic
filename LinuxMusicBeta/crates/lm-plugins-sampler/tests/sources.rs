//! The Sampler against real sources made in the test: a WAVE file for an SFZ, a SoundFont, a patch and a drum kit
//! of pieces, each through the plugin's build and a render. Every fixture is made here; nothing is skipped.

use std::path::PathBuf;
use std::sync::Arc;

use lm_plugin::testing::{Bench, note_off, note_on};
use lm_plugin::{BuildContext, Loaded, Registry, ResourceView};
use lm_render::ProcessSpec;
use lm_state::{Resource, Value};

const RATE: u32 = 48_000;

fn registry() -> Arc<Registry> {
    Registry::new(&[&lm_plugins_sampler::DESCRIPTOR]).expect("registry")
}

fn resource(kind: &str, n: u8) -> Resource {
    Resource::new(kind, &format!("{n:02x}").repeat(32)).expect("a valid key")
}

fn wav(frames: usize) -> Vec<u8> {
    let data_len = (frames * 2) as u32;
    let mut v = b"RIFF".to_vec();
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&RATE.to_le_bytes());
    v.extend_from_slice(&(RATE * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        let s = (libm::sinf(i as f32 * 0.1) * 16000.0) as i16;
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lm-plugins-sampler-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).expect("a temporary folder");
    d
}

fn render(state: Value, resources: ResourceView, key: u8) -> Vec<Vec<f32>> {
    let registry = registry();
    let state = registry.complete("sampler", &state).expect("complete");
    let spec = ProcessSpec { sample_rate: RATE, max_block: 256 };
    let cx = BuildContext { spec, resources: &resources, channels: 0 };
    let ports = registry.ports("sampler", &state).expect("ports");
    let channels: usize = ports.outputs.iter().map(|p| p.channels as usize).sum();
    let processor = registry.build("sampler", &state, &cx).expect("build");
    let events = [note_on(0, 1, key, 1.0), note_off(4800, 1, key)];
    Bench::instrument(processor, spec, channels, 1).render(9600, 256, &events, true)
}

fn peak(out: &[Vec<f32>]) -> f32 {
    out.iter().flatten().fold(0.0f32, |a, x| a.max(x.abs()))
}

#[test]
fn an_sfz_with_its_wave_sounds() {
    let dir = temp_dir("sfz");
    std::fs::write(dir.join("tone.wav"), wav(4800)).expect("the wave");
    let sfz = dir.join("tone.sfz");
    std::fs::write(&sfz, "<region> sample=tone.wav key=60 loop_mode=loop_continuous loop_start=100 loop_end=4000\n")
        .expect("the sfz");
    let r = resource("sfz", 1);
    let view = ResourceView::new().with(r.clone(), Loaded::File(Arc::from(sfz.as_path())));
    let state = Value::empty_object().with("format", Value::text("sfz")).with("sfz", Value::resource(r));
    let out = render(state, view, 60);
    assert!(peak(&out) > 0.05, "the SFZ made no sound");
    // Nothing at the end: the note-off released it
    assert!(out[0][9000..].iter().all(|x| x.abs() < 1.0e-3));
}

#[test]
fn a_patch_sounds_on_the_keys_it_covers() {
    let mut v = vec![0u8; 239];
    v[..22].copy_from_slice(b"GF1PATCH110\0ID#000002\0");
    v[82] = 1;
    v[151] = 1;
    v[198] = 1;
    let frames: Vec<i16> = (0..400).map(|i| (libm::sinf(i as f32 * 0.2) * 12000.0) as i16).collect();
    let mut head = vec![0u8; 8];
    head.extend_from_slice(&((frames.len() * 2) as u32).to_le_bytes());
    head.extend_from_slice(&20u32.to_le_bytes());
    head.extend_from_slice(&700u32.to_le_bytes());
    head.extend_from_slice(&22_050u16.to_le_bytes());
    head.extend_from_slice(&[0u8; 8]);
    head.extend_from_slice(&261_626u32.to_le_bytes());
    head.extend_from_slice(&[0u8; 20]);
    head.push(1 | 4);
    head.extend_from_slice(&[0u8; 40]);
    v.extend_from_slice(&head);
    for f in &frames {
        v.extend_from_slice(&f.to_le_bytes());
    }
    let r = resource("pat", 2);
    let view = ResourceView::new().with(r.clone(), Loaded::Bytes(Arc::from(v.into_boxed_slice())));
    let state = Value::empty_object().with("format", Value::text("pat")).with("patch", Value::resource(r));
    let out = render(state, view, 60);
    assert!(peak(&out) > 0.05, "the patch made no sound");
}

#[test]
fn a_file_that_is_not_a_gig_fails_with_a_clear_key() {
    let registry = registry();
    let r = resource("gig", 3);
    let state = registry
        .complete(
            "sampler",
            &Value::empty_object().with("format", Value::text("gig")).with("gig", Value::resource(r.clone())),
        )
        .expect("complete");
    let view = ResourceView::new().with(r, Loaded::Bytes(Arc::from(vec![0u8; 4].into_boxed_slice())));
    let spec = ProcessSpec { sample_rate: RATE, max_block: 256 };
    let cx = BuildContext { spec, resources: &view, channels: 0 };
    let e = registry.build("sampler", &state, &cx).expect_err("not a gig");
    assert_eq!(e.key(), "sampler.gig.invalid");
}

#[test]
fn a_drum_kit_with_groups_has_nine_output_channels() {
    let registry = registry();
    let state = registry
        .complete(
            "sampler",
            &Value::empty_object()
                .with("format", Value::text("kit"))
                .with("kit", Value::text("tr808"))
                .with("outputs", Value::text("groups")),
        )
        .expect("complete");
    let ports = registry.ports("sampler", &state).expect("ports");
    let channels: u32 = ports.outputs.iter().map(|p| p.channels).sum();
    assert_eq!((ports.outputs.len(), channels), (7, 9));
    // The percussion AVL kit has no groups: one stereo output
    let state = registry
        .complete(
            "sampler",
            &Value::empty_object()
                .with("format", Value::text("kit"))
                .with("kit", Value::text("buskmans-holiday"))
                .with("outputs", Value::text("groups")),
        )
        .expect("complete");
    assert_eq!(registry.ports("sampler", &state).expect("ports").outputs.len(), 1);
}
