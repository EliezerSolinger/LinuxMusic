//! Multitap Echo: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::multitap_echo::DESCRIPTOR, "multitapecho");

use common::{Knob, NULL, run_extended};
use lm_plugins_effects::multitap_echo::DESCRIPTOR;
use lm_state::Value;

const KNOBS: &[(&str, Knob)] = &[
    ("Steps", Knob::Param("steps")),
    ("Step length", Knob::Param("stepLength")),
    ("Dry gain", Knob::Param("dryGain")),
    ("Swap inputs", Knob::Param("swapInputs")),
    ("Lowpass stages", Knob::Param("lowpassStages")),
];

/// The case's graphs (each echo's level and low-pass) into the state's fields
fn graphs(case: &serde_json::Value, state: Value) -> Value {
    let Some(graphs) = case["graphs"].as_array() else { return state };
    let mut state = state;
    for (g, key) in graphs.iter().zip(["amplitude", "lowpass"]) {
        let values = g.as_array().unwrap().iter().map(|v| Value::real(v.as_f64().unwrap()).unwrap());
        state = state.with(key, Value::list(values));
    }
    state
}

#[test]
fn ab_against_the_legacy_plugin_at_its_defaults() {
    assert!(run_extended(&DESCRIPTOR, "multitapecho-defaults", KNOBS, 0, false, &graphs) <= NULL);
}

/// Up to the case's change (made at a period of the legacy program: here it would wait for the next grid point,
/// which is the same frame, but the legacy graphs went back to their defaults with the steps)
#[test]
fn ab_against_the_legacy_plugin() {
    assert!(run_extended(&DESCRIPTOR, "multitapecho", KNOBS, 0, false, &graphs) <= NULL);
}
