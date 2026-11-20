//! ReverbSC: the contract and the A/B against the legacy plugin's reverb

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::reverb_sc::DESCRIPTOR, "reverbsc");

use common::{Knob, NULL, run};
use lm_plugins_effects::reverb_sc::DESCRIPTOR;

const KNOBS: &[(&str, Knob)] = &[
    ("inputGain", Knob::Param("inputGain")),
    ("size", Knob::Param("size")),
    ("color", Knob::Param("color")),
    ("outputGain", Knob::Param("outputGain")),
];

#[test]
fn ab_against_the_legacy_plugin_at_its_defaults() {
    assert!(run(&DESCRIPTOR, "reverbsc-default", KNOBS, 0, true) <= NULL);
}

/// Every knob changed at once in the middle, as parameter events
#[test]
fn ab_against_the_legacy_plugin_with_a_change() {
    assert!(run(&DESCRIPTOR, "reverbsc-dark-small", KNOBS, 0, true) <= NULL);
}
