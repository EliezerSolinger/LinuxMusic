//! Dynamics Processor: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::dynamics_processor::DESCRIPTOR, "dynamicsprocessor");

use common::{Knob, NULL, run_extended};
use lm_dsp::dynamics_processor::StereoMode;
use lm_plugins_effects::dynamics_processor::DESCRIPTOR;
use lm_state::Value;

const KNOBS: &[(&str, Knob)] = &[
    ("Input gain", Knob::Param("inputGain")),
    ("Output gain", Knob::Param("outputGain")),
    ("Attack time", Knob::Param("attackTime")),
    ("Release time", Knob::Param("releaseTime")),
    ("Stereo mode", Knob::Choice("stereoMode", |v| StereoMode::ALL[v as usize].key())),
];

/// The case's drawn curve into the state's field
fn graph(case: &serde_json::Value, state: Value) -> Value {
    match case["graphs"][0].as_array() {
        Some(points) => {
            state.with("curve", Value::list(points.iter().map(|v| Value::real(v.as_f64().unwrap()).unwrap())))
        }
        None => state,
    }
}

#[test]
fn ab_against_the_legacy_plugin_at_its_defaults() {
    assert!(run_extended(&DESCRIPTOR, "dynproc-defaults", KNOBS, 0, false, &graph) <= NULL);
}

#[test]
fn ab_against_the_legacy_plugin_compressing() {
    assert!(run_extended(&DESCRIPTOR, "dynproc-compress", KNOBS, 0, false, &graph) <= NULL);
}

#[test]
fn ab_against_the_legacy_plugin_unlinked_gate() {
    assert!(run_extended(&DESCRIPTOR, "dynproc-unlinked-gate", KNOBS, 0, false, &graph) <= NULL);
}
