//! What the contract's generated tests do not reach: the amp's resampling (the contract's model runs at its own
//! rate), the latency it reports, any block size with every part on, a change of IR while it plays, and channel
//! counts other than two

use std::sync::Arc;

use lm_plugin::testing::{Bench, Timed, test_input};
use lm_plugin::{BuildContext, Loaded, Registry, ResourceView};
use lm_plugins_ampandir::DESCRIPTOR;
use lm_render::{Adopted, EventKind, ProcessSpec, Processor, SampleData};
use lm_state::{PARAMS_KEY, Resource, Value};

const RATE: u32 = 44_100;

fn model() -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ab/data/wavenet-48000.nam")).unwrap()
}

fn ir(seed: u32, frames: usize) -> SampleData {
    let side = |k: u32| -> Vec<f32> {
        (0..frames)
            .map(|i| {
                let x =
                    ((i as u32).wrapping_mul(2_654_435_761).wrapping_add(seed * 977 + k) >> 8) as f32 / 16_777_216.0;
                (x * 2.0 - 1.0) * (-(i as f32) / 300.0).exp()
            })
            .collect()
    };
    SampleData::from_channels(RATE, vec![side(0), side(1)]).unwrap()
}

/// A state with the model and an IR (seed), its resources, built for `channels`
fn build(seed: u32, channels: u16, block: usize) -> (Box<dyn Processor>, ProcessSpec) {
    let bytes = model();
    let nam = Resource::of_content("nam", &bytes).unwrap();
    let ir_data = ir(seed, 2_000);
    let ir_key = Resource::of_content("audio", &seed.to_le_bytes()).unwrap();
    let resources = ResourceView::new()
        .with(nam.clone(), Loaded::Bytes(Arc::from(bytes.into_boxed_slice())))
        .with(ir_key.clone(), Loaded::Audio(Arc::new(ir_data)));
    let params =
        Value::empty_object().with("input", Value::real(4.0).unwrap()).with("output", Value::real(-2.0).unwrap());
    let state = Value::empty_object()
        .with(PARAMS_KEY, params)
        .with("nam", Value::resource(nam))
        .with("ir", Value::resource(ir_key));
    let registry = Registry::new(&[&DESCRIPTOR]).unwrap();
    let spec = ProcessSpec { sample_rate: RATE, max_block: block.max(1024) };
    let cx = BuildContext { spec, resources: &resources, channels };
    (registry.build(DESCRIPTOR.id, &state, &cx).unwrap(), spec)
}

fn changes() -> Vec<Timed> {
    vec![
        Timed::new(5_003, EventKind::Param { index: 0, value: -7.5 }),
        Timed::new(9_000, EventKind::Param { index: 1, value: 3.0 }),
        Timed::new(14_321, EventKind::AllSoundOff),
    ]
}

#[test]
fn the_latency_is_the_irs_block_and_the_amps_resampling() {
    let (p, _) = build(1, 2, 256);
    // 256 for the IR; the amp at 44.1 kHz runs its 48 kHz model behind a cushion of 256 (the sinc converters add no
    // delay of their own, as the legacy measure finds)
    assert!(p.latency() >= 512 && p.latency() < 600, "latency {}", p.latency());
}

#[test]
fn any_block_size_gives_the_same_audio_with_everything_on() {
    let (p, spec) = build(1, 2, 32);
    let reference = Bench::effect(p, spec, 2, 0, test_input).render(20_000, 32, &changes(), true);
    assert!(reference[0][4_000..].iter().any(|x| x.abs() > 1.0e-3), "silent");
    for block in [64, 256, 1024] {
        let (p, spec) = build(1, 2, block);
        let out = Bench::effect(p, spec, 2, 0, test_input).render(20_000, block, &changes(), true);
        for (a, b) in reference.iter().zip(&out) {
            let first = a.iter().zip(b).position(|(x, y)| x.to_bits() != y.to_bits());
            assert!(first.is_none(), "blocks of {block}: from frame {first:?}");
        }
    }
    // Blocks off the grid (a loop's cut) cut the pieces elsewhere: libsamplerate's streaming position is a double
    // carried from call to call, so the resampled amp differs by rounding only
    let (p, spec) = build(1, 2, 100);
    let out = Bench::effect(p, spec, 2, 0, test_input).render(20_000, 100, &changes(), true);
    for (a, b) in reference.iter().zip(&out) {
        let worst = a.iter().zip(b).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
        assert!(worst < 1.0e-6, "blocks of 100: {worst}");
    }
}

#[test]
fn the_same_state_rebuilt_goes_on_bit_for_bit_and_another_ir_crossfades() {
    let (p, spec) = build(1, 2, 256);
    let whole = Bench::effect(p, spec, 2, 0, test_input).render(16_384, 256, &[], false);
    let (p, spec) = build(1, 2, 256);
    let mut bench = Bench::effect(p, spec, 2, 0, test_input);
    let first = bench.render(8_192, 256, &[], true);
    let (adopted, old) = bench.replace(build(1, 2, 256).0, true);
    assert_eq!(adopted, Adopted::Yes);
    drop(old);
    let second = bench.render(8_192, 256, &[], true);
    for ch in 0..2 {
        let joined: Vec<f32> = [first[ch].as_slice(), &second[ch]].concat();
        assert!(joined.iter().zip(&whole[ch]).all(|(x, y)| x.to_bits() == y.to_bits()), "channel {ch}");
    }

    let (p, spec) = build(1, 2, 256);
    let mut bench = Bench::effect(p, spec, 2, 0, test_input);
    let first = bench.render(8_192, 256, &[], true);
    let (adopted, old) = bench.replace(build(2, 2, 256).0, true);
    assert_eq!(adopted, Adopted::Yes);
    drop(old);
    let second = bench.render(8_192, 256, &[], true);
    let x: Vec<f32> = [first[0].as_slice(), &second[0]].concat();
    let step = |r: std::ops::Range<usize>| r.map(|i| (x[i + 1] - x[i]).abs()).fold(0.0f32, f32::max);
    let (before, at, after) = (step(4_000..8_000), step(8_180..8_200), step(8_700..12_000));
    assert!(at <= 2.0 * before.max(after) + 1.0e-3, "a click at the change: {at} ({before} before, {after} after)");
}

#[test]
fn one_and_three_channels() {
    for channels in [1u16, 3] {
        let (p, spec) = build(1, channels, 256);
        let out = Bench::effect(p, spec, usize::from(channels), 0, test_input).render(8_000, 256, &[], true);
        assert_eq!(out.len(), usize::from(channels));
        for ch in &out {
            assert!(ch.iter().all(|x| x.is_finite()) && ch[4_000..].iter().any(|x| x.abs() > 1.0e-3));
        }
    }
}
