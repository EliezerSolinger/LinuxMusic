//! A bench for one note of an instrument of this crate, from a state given by its parameters and fields

#![allow(dead_code)]

use lm_plugin::testing::{Bench, Timed, test_resources};
use lm_plugin::{BuildContext, Descriptor, Registry};
use lm_render::ProcessSpec;
use lm_state::Value;

/// A state with these parameters and fields over the defaults
pub fn state(params: &[(&str, f64)], fields: &[(&str, Value)]) -> Value {
    let mut p = Value::empty_object();
    for (k, v) in params {
        p = p.with(k, Value::real(*v).unwrap());
    }
    let mut s = Value::empty_object().with(lm_state::PARAMS_KEY, p);
    for (k, v) in fields {
        s = s.with(k, v.clone());
    }
    s
}

/// Renders `frames` of `events` (blocks of 256), both sides
pub fn render(
    d: &'static Descriptor,
    state: &Value,
    rate: u32,
    seed: u64,
    frames: usize,
    events: &[Timed],
) -> Vec<Vec<f32>> {
    let registry = Registry::new(&[d]).unwrap();
    let complete = registry.complete(d.id, state).unwrap();
    let spec = ProcessSpec { sample_rate: rate, max_block: 1024 };
    let resources = test_resources(registry.schemas(), &d.type_name(), &complete, rate);
    let cx = BuildContext { spec, resources: &resources, channels: 0 };
    let mut bench = Bench::instrument(registry.build(d.id, state, &cx).unwrap(), spec, 2, seed);
    bench.render(frames, 256, events, false)
}

/// The two agree after the fade-in (at most 85 frames at 44.1 kHz, scaled with the rate), to `tolerance`
pub fn agree(ours: &[f32], theirs: &[f32], rate: u32, tolerance: f32, what: &str) {
    let skip = (85.0 * rate as f32 / 44_100.0).ceil() as usize + 1;
    for (n, (a, b)) in ours.iter().zip(theirs).enumerate().skip(skip) {
        assert!((a - b).abs() <= tolerance, "{what}: frame {n}: {a} and {b}");
    }
}
